use crate::entity::player::statistics::StatisticCategory;
use crate::server::Server;
use crate::world::World;
use core::f32;
use crossbeam::atomic::AtomicCell;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::{BundleContentsImpl, ContainerImpl};
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::Taggable;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::bedrock::client::CAddItemActor;
use pumpkin_protocol::bedrock::network_item::ItemStackWrapper;
use pumpkin_protocol::bedrock::server::actor_event::{ActorEventID, SActorEvent};
use pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::codec::var_long::VarLong;
use pumpkin_protocol::codec::var_ulong::VarULong;
use pumpkin_protocol::java::client::play::{CSetEntityMetadata, Metadata};
use pumpkin_util::math::atomic_f32::AtomicF32;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use std::sync::atomic::Ordering::{AcqRel, Relaxed};
use uuid::Uuid;

use std::sync::{
    Arc, Mutex,
    atomic::{
        AtomicBool, AtomicU8, AtomicU32,
        Ordering::{self},
    },
};

use super::{
    Entity, EntityBase, entity_reference::EntityReference, living::LivingEntity, player::Player,
};

pub struct ItemEntity {
    entity: Entity,
    item_age: AtomicU32,
    // These cannot be atomic values because we mutate their state based on what they are; we run
    // into the ABA problem
    item_stack: Mutex<ItemStack>,
    pickup_delay: AtomicU8,
    health: AtomicF32,
    never_despawn: AtomicBool,
    never_pickup: AtomicBool,
    merge_reserved: AtomicBool,
    /// Vanilla `ItemEntity.thrower` (NBT `Thrower`).
    thrower: EntityReference,
    /// Vanilla `ItemEntity.target` (NBT `Owner`): the only player that may pick it up.
    target: AtomicCell<Option<Uuid>>,
}

struct ItemMergeReservation<'a> {
    low: &'a AtomicBool,
    high: &'a AtomicBool,
}

impl<'a> ItemMergeReservation<'a> {
    fn acquire(low: &'a ItemEntity, high: &'a ItemEntity) -> Option<Self> {
        if std::ptr::eq(low, high)
            || low
                .merge_reserved
                .compare_exchange(false, true, AcqRel, Ordering::Acquire)
                .is_err()
        {
            return None;
        }

        if high
            .merge_reserved
            .compare_exchange(false, true, AcqRel, Ordering::Acquire)
            .is_err()
        {
            low.merge_reserved.store(false, Ordering::Release);
            return None;
        }

        Some(Self {
            low: &low.merge_reserved,
            high: &high.merge_reserved,
        })
    }
}

impl Drop for ItemMergeReservation<'_> {
    fn drop(&mut self) {
        self.high.store(false, Ordering::Release);
        self.low.store(false, Ordering::Release);
    }
}

/// Vanilla `ItemEntity.LIFETIME`.
const LIFETIME: u32 = 6000; // 5 minutes in ticks
/// Vanilla `ItemEntity.merge(to, from, 64)`: cap on top of `getMaxStackSize`.
const MERGE_MAX_COUNT: u8 = 64;

impl ItemEntity {
    pub const DEFAULT_PICKUP_DELAY: u8 = 10;

    /// Vanilla `ItemEntity(type, level)`: the `bobOffs` field initialiser's `nextFloat`
    /// (client rendering only, so not kept), then `setYRot(random.nextFloat() * 360.0F)`.
    fn take_construction_draws(entity: &Entity) {
        let yaw = {
            let mut random = entity.random();
            random.next_f32();
            random.next_f32() * 360.0
        };
        entity.yaw.store(yaw);
    }

    /// Vanilla `ItemEntity(level, x, y, z, stack)`.
    pub fn new(entity: Entity, item_stack: ItemStack) -> Self {
        Self::take_construction_draws(&entity);
        let velocity = {
            let mut random = entity.random();
            let x = random.next_f64() * 0.2 - 0.1;
            let z = random.next_f64() * 0.2 - 0.1;
            Vector3::new(x, 0.2, z)
        };
        entity.velocity.store(velocity);
        Self::update_fire_immune(&entity, &item_stack);

        Self {
            entity,
            item_stack: Mutex::new(item_stack),
            item_age: AtomicU32::new(0),
            pickup_delay: AtomicU8::new(Self::DEFAULT_PICKUP_DELAY),
            health: AtomicF32::new(5.0),
            never_despawn: AtomicBool::new(false),
            never_pickup: AtomicBool::new(false),
            merge_reserved: AtomicBool::new(false),
            thrower: EntityReference::none(),
            target: AtomicCell::new(None),
        }
    }

    pub fn new_with_velocity(
        entity: Entity,
        item_stack: ItemStack,
        velocity: Vector3<f64>,
        pickup_delay: u8,
    ) -> Self {
        Self::take_construction_draws(&entity);
        entity.velocity.store(velocity);
        Self::update_fire_immune(&entity, &item_stack);

        Self {
            entity,
            item_stack: Mutex::new(item_stack),
            item_age: AtomicU32::new(0),
            pickup_delay: AtomicU8::new(pickup_delay), // Vanilla pickup delay is 10 ticks
            health: AtomicF32::new(5.0),
            never_despawn: AtomicBool::new(false),
            never_pickup: AtomicBool::new(false),
            merge_reserved: AtomicBool::new(false),
            thrower: EntityReference::none(),
            target: AtomicCell::new(None),
        }
    }

    /// Creates an `ItemEntity` for restoring from NBT without random velocity.
    /// The velocity and position will be set by `Entity::read_nbt_non_mut`.
    pub fn new_empty(entity: Entity) -> Self {
        Self {
            entity,
            item_stack: Mutex::new(ItemStack::new(1, &pumpkin_data::item::Item::AIR)),
            item_age: AtomicU32::new(0),
            pickup_delay: AtomicU8::new(0),
            health: AtomicF32::new(5.0),
            never_despawn: AtomicBool::new(false),
            never_pickup: AtomicBool::new(false),
            merge_reserved: AtomicBool::new(false),
            thrower: EntityReference::none(),
            target: AtomicCell::new(None),
        }
    }

    pub const fn get_item_stack(&self) -> &Mutex<ItemStack> {
        &self.item_stack
    }

    pub fn get_pickup_delay(&self) -> u8 {
        self.pickup_delay.load(Ordering::Relaxed)
    }

    pub fn set_pickup_delay(&self, pickup_delay: u8) {
        self.pickup_delay.store(pickup_delay, Ordering::Relaxed);
    }

    pub const fn get_entity(&self) -> &Entity {
        &self.entity
    }

    /// Vanilla `ItemEntity.setThrower`.
    pub fn set_thrower(&self, thrower: &Entity) {
        self.thrower.set(Some(thrower));
    }

    /// Vanilla `ItemEntity.getOwner`: the thrower, in any dimension.
    #[must_use]
    pub fn get_owner(&self) -> Option<Arc<dyn EntityBase>> {
        self.thrower.get(&self.entity.world.load())
    }

    /// Vanilla `ItemEntity.setTarget`.
    pub fn set_target(&self, target: Option<Uuid>) {
        self.target.store(target);
    }

    /// Vanilla `ItemEntity.fireImmune`, cached since the item only changes on load.
    fn update_fire_immune(entity: &Entity, stack: &ItemStack) {
        entity.fire_immune.store(
            !stack.can_be_hurt_by(&DamageType::IN_FIRE),
            Ordering::Relaxed,
        );
    }

    /// Vanilla `ItemEntity.areMergable`.
    #[must_use]
    pub fn are_mergeable(stack: &ItemStack, other: &ItemStack) -> bool {
        u16::from(stack.item_count) + u16::from(other.item_count)
            <= u16::from(other.get_max_stack_size())
            && stack.are_items_and_components_equal(other)
    }

    /// Vanilla `ItemEntity.isMergable`.
    fn is_mergeable(&self) -> bool {
        if self.entity.removed.load(Ordering::SeqCst)
            || self.never_pickup.load(Ordering::Relaxed)
            || self.never_despawn.load(Ordering::Relaxed)
            || self.item_age.load(Ordering::Relaxed) >= LIFETIME
        {
            return false;
        }
        let Ok(item_stack) = self.item_stack.try_lock() else {
            return false;
        };

        item_stack.item_count < item_stack.get_max_stack_size()
    }

    /// Vanilla `ItemEntity.mergeWithNeighbours`.
    fn merge_with_neighbours(&self) {
        if !self.is_mergeable() {
            return;
        }

        let bounding_box = self.entity.bounding_box.load().expand(0.5, 0.0, 0.5);
        let world = self.entity.world.load();
        for other in world.get_entities_at_box(&bounding_box) {
            let Some(item) = other.get_item_entity() else {
                continue;
            };
            if item.entity.entity_id == self.entity.entity_id || !item.is_mergeable() {
                continue;
            }

            self.try_merge_with(item);
            if self.entity.removed.load(Ordering::SeqCst) {
                break;
            }
        }
    }

    #[expect(clippy::too_many_lines)]
    fn try_merge_with(&self, other: &Self) {
        // Always lock in entity_id order to prevent deadlock when two
        // items try to merge with each other concurrently.
        let (low, high) = if self.entity.entity_id < other.entity.entity_id {
            (self, other)
        } else {
            (other, self)
        };
        let Some(_reservation) = ItemMergeReservation::acquire(low, high) else {
            return;
        };

        let (expected_low, expected_high, target_is_self) = {
            let low_stack = low
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let high_stack = high
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let (self_stack, other_stack) = if self.entity.entity_id < other.entity.entity_id {
                (&*low_stack, &*high_stack)
            } else {
                (&*high_stack, &*low_stack)
            };

            if self.target.load() != other.target.load()
                || !Self::are_mergeable(self_stack, other_stack)
            {
                return;
            }

            (
                low_stack.clone(),
                high_stack.clone(),
                other_stack.item_count < self_stack.item_count,
            )
        };
        let (target, source) = if target_is_self {
            (self, other)
        } else {
            (other, self)
        };
        if self.entity.removed.load(Ordering::SeqCst) || other.entity.removed.load(Ordering::SeqCst)
        {
            return;
        }

        let mut event = crate::plugin::api::events::entity::item_merge::ItemMergeEvent {
            entity_id: target.entity.entity_id,
            target_id: source.entity.entity_id,
            cancelled: false,
        };
        let server = self.entity.world.load().server.upgrade();
        if let Some(server) = server {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled
            || self.entity.removed.load(Ordering::SeqCst)
            || other.entity.removed.load(Ordering::SeqCst)
        {
            return;
        }

        let low_stack = low
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let high_stack = high
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if low_stack.uid != expected_low.uid
            || !low_stack.are_equal(&expected_low)
            || high_stack.uid != expected_high.uid
            || !high_stack.are_equal(&expected_high)
            || self.entity.removed.load(Ordering::SeqCst)
            || other.entity.removed.load(Ordering::SeqCst)
        {
            return;
        }

        let (self_stack, other_stack) = if self.entity.entity_id < other.entity.entity_id {
            (low_stack, high_stack)
        } else {
            (high_stack, low_stack)
        };
        if !Self::are_mergeable(&self_stack, &other_stack)
            || (other_stack.item_count < self_stack.item_count) != target_is_self
        {
            return;
        }
        let (mut stack1, mut stack2) = if target_is_self {
            (self_stack, other_stack)
        } else {
            (other_stack, self_stack)
        };

        // Vanilla `ItemEntity.merge(to, from, 64)`.
        let max_size = stack1.get_max_stack_size().min(MERGE_MAX_COUNT);
        let moved = stack2
            .item_count
            .min(max_size.saturating_sub(stack1.item_count));
        stack1.increment(moved);
        stack2.decrement(moved);
        let source_empty = stack2.is_empty();
        drop(stack1);
        drop(stack2);

        // `is_mergeable` already excluded never-despawn and never-pickup items.
        target.pickup_delay.fetch_max(
            source.pickup_delay.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        target
            .item_age
            .fetch_min(source.item_age.load(Ordering::Relaxed), Ordering::Relaxed);

        target.on_count_changed();
        if source_empty {
            source.entity.remove();
        } else {
            source.on_count_changed();
        }
    }

    /// Vanilla `Item.onDestroyed`: containers (`BlockItem`, e.g. shulker boxes and bundles
    /// spill their contents where the item died, whatever the damage type was).
    fn on_destroyed(&self, world: &Arc<World>) {
        let contents = {
            let mut stack = self
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut contents = Vec::new();
            if let Some(container) = stack.get_data_component::<ContainerImpl>()
                && !container.items.is_empty()
            {
                contents.extend(container.items.iter().map(|(_, item)| item.clone()));
                stack.set_data_component(ContainerImpl { items: Vec::new() });
            }
            if let Some(bundle) = stack.get_data_component::<BundleContentsImpl>()
                && !bundle.items.is_empty()
            {
                contents.extend(bundle.items.iter().cloned());
                stack.set_data_component(BundleContentsImpl {
                    items: Vec::new(),
                    selected_item: -1,
                });
            }
            contents
        };

        // no pickup delay.
        let pos = self.entity.pos.load();
        for item in contents.into_iter().filter(|item| !item.is_empty()) {
            let spilled = Self::new(Entity::new(world.clone(), pos, &EntityType::ITEM), item);
            spilled.set_pickup_delay(0);
            world.spawn_entity(Arc::new(spilled));
        }
    }

    /// Resends the stack after its count changed. Bedrock item actors ignore the `ITEM`
    /// metadata, so they get the count as `UpdateStackSize`
    fn on_count_changed(&self) {
        self.init_data_tracker();

        let count = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .item_count;
        let packet = SActorEvent {
            target_runtime_id: VarULong(self.entity.entity_id as u64),
            event_id: ActorEventID::UpdateStackSize,
            data: VarInt(i32::from(count)),
            fire_at_position: None,
        };
        self.entity
            .world
            .load()
            .send_to_tracking_players_bedrock(&self.entity, &packet);
    }

    fn decrement_pickup_delay(&self) {
        self.pickup_delay
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |val| {
                Some(val.saturating_sub(1))
            })
            .ok();
    }

    fn apply_fluid_drag_or_gravity(&self, mut velo: Vector3<f64>) -> Vector3<f64> {
        let entity = &self.entity;

        if entity.touching_water.load(Ordering::SeqCst) && entity.water_height.load() > 0.1 {
            velo.x *= 0.99;
            velo.z *= 0.99;
            if velo.y < 0.06 {
                velo.y += 5.0e-4;
            }
        } else if entity.touching_lava.load(Ordering::SeqCst) && entity.lava_height.load() > 0.1 {
            velo.x *= 0.95;
            velo.z *= 0.95;
            if velo.y < 0.06 {
                velo.y += 5.0e-4;
            }
        } else {
            velo.y -= <Self as EntityBase>::get_gravity(self);
        }

        velo
    }

    fn update_no_physics_and_push_out(&self) {
        let entity = &self.entity;
        let pos = entity.pos.load();
        let bounding_box = entity.bounding_box.load();

        let no_physics = !entity
            .world
            .load()
            .is_space_empty(bounding_box.expand(-1.0e-7, -1.0e-7, -1.0e-7));

        entity.no_physics.store(no_physics, Ordering::Relaxed);

        if no_physics {
            entity.push_out_of_blocks(Vector3::new(
                pos.x,
                f64::midpoint(bounding_box.min.y, bounding_box.max.y),
                pos.z,
            ));
        }
    }

    /// a resting item only moves every 4th `tickCount`, offset by its id.
    fn should_tick_move(&self, move_velo: Vector3<f64>) -> bool {
        let entity = &self.entity;

        !entity.on_ground.load(Ordering::SeqCst)
            || move_velo.horizontal_length_squared() > 1.0e-5
            || entity
                .tick_count
                .load(Ordering::Relaxed)
                .wrapping_add(entity.entity_id)
                % 4
                == 0
    }

    fn move_and_apply_friction(&self, caller: &dyn EntityBase, move_velo: Vector3<f64>) {
        let entity = &self.entity;

        entity.move_entity(caller, move_velo);
        entity.tick_block_collisions(caller);

        // air drag
        let air_drag = 0.98;
        let mut friction = air_drag;
        let on_ground = entity.on_ground.load(Ordering::SeqCst);

        let mut velo = entity.velocity.load();
        if on_ground {
            let block_affecting_velo = entity.get_block_with_y_offset(0.999_999).1;
            friction *= f64::from(block_affecting_velo.slipperiness);
        }

        velo = velo.multiply(friction, air_drag, friction);

        // a landing item bounces back up with half its speed
        if on_ground && velo.y < 0.0 {
            velo.y *= -0.5;
        }

        entity.velocity.store(velo);
    }

    fn process_age_and_merge(&self) -> bool {
        if self.never_despawn.load(Ordering::Relaxed) {
            return true;
        }

        let entity = &self.entity;
        let age = self.item_age.fetch_add(1, Ordering::Relaxed) + 1;

        if age >= LIFETIME {
            let entity_id = entity.entity_id;
            let world = entity.world.load_full();
            let mut despawn_event =
                crate::plugin::api::events::entity::item_despawn::ItemDespawnEvent::new(entity_id);
            if let Some(server) = world.server.upgrade() {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut despawn_event);
            }
            if !despawn_event.cancelled
                && let Some(e) = world.get_entity_by_id(entity_id)
            {
                e.get_entity().remove();
            }
            return false;
        }

        // merge rate on `tickCount`: 2 while the item changes block cell, else 40.
        let moved =
            BlockPos::floored_v(entity.last_pos.load()) != BlockPos::floored_v(entity.pos.load());
        let rate = if moved { 2 } else { 40 };
        if entity.tick_count.load(Ordering::Relaxed) % rate == 0 {
            self.merge_with_neighbours();
        }

        true
    }

    /// Vanilla `ItemEntity.tick` `needsSync`: fluid contact or velocity change.
    fn mark_needs_sync(&self, caller: &dyn EntityBase, original_velo: Vector3<f64>) {
        let entity = &self.entity;

        // Vanilla runs `updateFluidInteraction` again here, pushing by the current twice.
        let in_fluid = entity.update_fluid_interaction(caller);

        if in_fluid || entity.velocity.load().sub(&original_velo).length_squared() > 0.01 {
            entity.velocity_dirty.store(true, Ordering::SeqCst);
        }
    }
}

impl EntityBase for ItemEntity {
    /// Vanilla `ItemEntity.isAttackable`.
    fn is_attackable(&self) -> bool {
        false
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        let entity = &self.entity;
        if self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
        {
            entity.remove();
            return;
        }

        // Vanilla `super.tick()`: `xo/yo/zo`, fluids, fire, portals and the void.
        entity.tick(caller, server);
        if entity.removed.load(Ordering::SeqCst) {
            return;
        }

        self.decrement_pickup_delay();

        let original_velo = entity.velocity.load();
        entity
            .velocity
            .store(self.apply_fluid_drag_or_gravity(original_velo));

        self.update_no_physics_and_push_out();

        let move_velo = entity.velocity.load(); // In case push_out_of_blocks modifies it

        if self.should_tick_move(move_velo) {
            self.move_and_apply_friction(caller, move_velo);
        } else {
            entity.apply_effects_from_blocks_for_last_movements(caller);
        }

        if self.process_age_and_merge() {
            self.mark_needs_sync(caller, original_velo);
        }
    }

    fn init_data_tracker(&self) {
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::item::ITEM,
            ItemStackSerializer::from(
                self.item_stack
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            ),
        );
    }

    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        _source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        // Vanilla `ItemEntity.hurtServer`.
        let entity = &self.entity;
        if entity.is_invulnerable_to(&damage_type, cause) {
            return false;
        }
        let world = entity.world.load_full();
        if !world.level_info.load().game_rules.mob_griefing
            && cause.is_some_and(|cause| cause.get_mob().is_some())
        {
            return false;
        }
        // E.g. netherite ignores fire, the nether star explosions.
        let can_be_hurt = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .can_be_hurt_by(&damage_type);
        if !can_be_hurt {
            return false;
        }

        let mut event = crate::plugin::api::events::entity::entity_damage::EntityDamageEvent::new(
            entity.entity_id,
            damage_type,
            amount,
        );
        if let Some(server) = world.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled {
            return false;
        }

        // Vanilla `markHurt`: resend the motion.
        entity.velocity_dirty.store(true, Ordering::SeqCst);

        // Vanilla keeps item health as an int: `(int)(health - damage)`.
        let destroyed = loop {
            let current = self.health.load(Relaxed);
            let new = (current - event.damage).trunc();
            if self
                .health
                .compare_exchange(current, new, AcqRel, Relaxed)
                .is_ok()
            {
                break current > 0.0 && new <= 0.0;
            }
        };

        world.game_event_from(cause, GameEvent::EntityDamage, entity.pos.load());
        if destroyed {
            self.on_destroyed(&world);
            entity.remove();
        }
        true
    }

    fn on_player_collision(&self, player: &Arc<Player>) {
        if self.pickup_delay.load(Ordering::Relaxed) > 0
            || player.living_entity.health.load() <= 0.0
            || player.is_spectator()
            || self
                .target
                .load()
                .is_some_and(|target| target != player.get_entity().entity_uuid)
        {
            return;
        }

        let (item_id, org_count, mut stack) = {
            let stack = self
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (stack.item.id, stack.item_count, stack.clone())
        };
        let creative = player.is_creative();
        // Pumpkin's pickup event goes before the inventory changes, when any of it could go in.
        if !creative && !player.inventory.has_room_for(&stack) {
            return;
        }
        let living = &player.living_entity;
        if !living.fire_pickup_event(&self.entity, org_count.into()) {
            return;
        }

        // Vanilla `Inventory.add`: true only once all of it went in; with infinite materials
        // whatever didn't fit is gone.
        let mut added = player.inventory.insert_stack_anywhere(&mut stack);
        if creative && !added {
            stack.set_count(0);
            added = true;
        }
        let is_empty = stack.is_empty();
        *self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = stack;
        if !added {
            return;
        }

        player.inventory_changed.store(true, Ordering::Relaxed);
        living.take(&self.entity, org_count.into());
        if is_empty {
            self.entity.remove();
        }
        player.increment_stat(
            StatisticCategory::PickedUp,
            item_id as i32,
            i32::from(org_count),
        );
        living.on_item_pickup(self);

        player
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send_content_updates();
    }

    /// Vanilla `ItemEntity.shouldPlayLavaHurtSound`.
    fn should_play_lava_hurt_sound(&self) -> bool {
        self.health.load(Ordering::Relaxed) <= 0.0
            || self.entity.tick_count.load(Ordering::Relaxed) % 10 == 0
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn get_item_entity(&self) -> Option<&ItemEntity> {
        Some(self)
    }

    fn get_gravity(&self) -> f64 {
        0.04
    }

    fn bedrock_y_offset(&self) -> f64 {
        0.125
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        let item = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut item_compound = NbtCompound::new();
        item.write_item_stack(&mut item_compound);
        nbt.put_compound("Item", item_compound);

        nbt.put_short("Age", self.item_age.load(Ordering::Relaxed) as i16);
        nbt.put_short(
            "PickupDelay",
            self.pickup_delay.load(Ordering::Relaxed) as i16,
        );
        nbt.put_short("Health", self.health.load(Relaxed) as i16);
        if let Some(thrower) = self.thrower.uuid() {
            nbt.put_uuid("Thrower", thrower);
        }
        if let Some(target) = self.target.load() {
            nbt.put_uuid("Owner", target);
        }
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        // Restore the item stack from the "Item" compound
        if let Some(item_compound) = nbt.get_compound("Item")
            && let Some(stack) = ItemStack::read_item_stack(item_compound)
        {
            // `new_empty` had no item yet to derive this from.
            Self::update_fire_immune(&self.entity, &stack);
            *self
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = stack;
        }

        // Vanilla stores Age as a short
        self.item_age
            .store(nbt.get_short("Age").unwrap_or(0) as u32, Ordering::Relaxed);

        // Vanilla stores PickupDelay as a short
        if let Some(delay) = nbt.get_short("PickupDelay") {
            self.pickup_delay.store(delay as u8, Ordering::Relaxed);
        }

        // Vanilla stores Health as a short
        if let Some(health) = nbt.get_short("Health") {
            self.health.store(health as f32, Relaxed);
        }

        self.target.store(nbt.get_uuid("Owner"));
        self.thrower.set_uuid(nbt.get_uuid("Thrower"));
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn movement_emission(&self) -> crate::entity::movement_emission::MovementEmission {
        crate::entity::movement_emission::MovementEmission::None
    }

    /// Vanilla `ItemEntity.dampensVibrations`.
    fn dampens_vibrations(&self) -> bool {
        self.item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .item
            .has_tag(&pumpkin_data::tag::Item::MINECRAFT_DAMPENS_VIBRATIONS)
    }

    fn send_bedrock_spawn_packet(&self, client: &crate::net::bedrock::BedrockClient) {
        let entity = &self.entity;
        let runtime_id = entity.entity_id as u64;
        let data = {
            let item_stack = self
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let packet = CAddItemActor {
                target_actor_id: VarLong(runtime_id as i64),
                target_runtime_id: VarULong(runtime_id),
                item: ItemStackWrapper::from(&*item_stack),
                position: self.bedrock_pos().to_f32_lossy(),
                velocity: entity.velocity.load().to_f32_lossy(),
                entity_data: entity.bedrock_metadata(),
                is_from_fishing: false,
            };
            client.serialize_packet(&packet).ok()
        };
        if let Some(data) = data {
            client.try_enqueue_packet(data);
        }
    }

    fn send_java_spawn_packet(&self, client: &crate::net::java::JavaClient) {
        let spawn_packet = self.entity.create_spawn_packet();
        if let Ok(data) = client.serialize_packet(&spawn_packet) {
            client.try_enqueue_packet(data);
        }

        let metadata = Metadata::new(
            pumpkin_data::tracked_data::item::ITEM,
            ItemStackSerializer::from(
                self.item_stack
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            ),
        );
        let mut data = Vec::new();
        if metadata
            .write(&mut data, &pumpkin_data::packet::CURRENT_MC_VERSION)
            .is_ok()
        {
            data.push(255);
            let meta_packet = CSetEntityMetadata::new(self.entity.entity_id.into(), data.into());
            if let Ok(meta_data) = client.serialize_packet(&meta_packet) {
                client.try_enqueue_packet(meta_data);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ItemEntity;
    use pumpkin_data::data_component_impl::{CustomDataImpl, CustomNameImpl};
    use pumpkin_data::item::Item;
    use pumpkin_data::item_stack::ItemStack;
    use pumpkin_data::{Enchantment, damage::DamageType};
    use pumpkin_nbt::compound::NbtCompound;
    use pumpkin_util::text::TextComponent;

    #[test]
    fn different_counts_merge_up_to_max_stack() {
        let stone = |count| ItemStack::new(count, &Item::STONE);
        assert!(ItemEntity::are_mergeable(&stone(1), &stone(5)));
        assert!(ItemEntity::are_mergeable(&stone(32), &stone(32)));
        assert!(!ItemEntity::are_mergeable(&stone(33), &stone(32)));
    }

    #[test]
    fn unstackable_items_never_merge() {
        let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        assert!(!ItemEntity::are_mergeable(&sword, &sword.clone()));
    }

    #[test]
    fn different_components_never_merge() {
        let plain = ItemStack::new(1, &Item::STONE);

        let mut named = plain.clone();
        named.set_data_component(CustomNameImpl {
            name: TextComponent::text("a"),
        });
        let mut renamed = plain.clone();
        renamed.set_data_component(CustomNameImpl {
            name: TextComponent::text("b"),
        });
        assert!(!ItemEntity::are_mergeable(&plain, &named));
        assert!(!ItemEntity::are_mergeable(&named, &renamed));
        assert!(ItemEntity::are_mergeable(&named, &named.clone()));

        let mut enchanted = plain.clone();
        enchanted.add_enchantment(&Enchantment::UNBREAKING, 1);
        let mut stronger = plain.clone();
        stronger.add_enchantment(&Enchantment::UNBREAKING, 2);
        assert!(!ItemEntity::are_mergeable(&plain, &enchanted));
        assert!(!ItemEntity::are_mergeable(&enchanted, &stronger));

        let mut data = NbtCompound::new();
        data.put_string("id", "a".to_string());
        let mut tagged = plain.clone();
        tagged.set_data_component(CustomDataImpl::new(data));
        assert!(!ItemEntity::are_mergeable(&plain, &tagged));
    }

    #[tokio::test]
    async fn thrower_and_owner_round_trip_through_nbt() {
        use crate::entity::{Entity, EntityBase};
        use pumpkin_data::entity::EntityType;
        use pumpkin_util::math::vector3::Vector3;

        let (_folder, world) = crate::world::test_support::test_world();
        let new_item = || {
            ItemEntity::new(
                Entity::new(
                    world.clone(),
                    Vector3::new(0.0, 0.0, 0.0),
                    &EntityType::ITEM,
                ),
                ItemStack::new(1, &Item::STONE),
            )
        };
        let thrower = Entity::new(world.clone(), Vector3::new(0.0, 0.0, 0.0), &EntityType::PIG);
        let owner = uuid::Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);

        let item = new_item();
        let mut plain = NbtCompound::new();
        item.write_custom_nbt(&mut plain);
        assert!(plain.get_int_array("Thrower").is_none());
        assert!(plain.get_int_array("Owner").is_none());

        item.set_thrower(&thrower);
        item.set_target(Some(owner));
        let mut nbt = NbtCompound::new();
        item.write_custom_nbt(&mut nbt);
        assert_eq!(nbt.get_uuid("Thrower"), Some(thrower.entity_uuid));
        assert_eq!(nbt.get_uuid("Owner"), Some(owner));

        let loaded = new_item();
        loaded.read_custom_nbt(&nbt);
        assert_eq!(loaded.thrower.uuid(), Some(thrower.entity_uuid));
        assert_eq!(loaded.target.load(), Some(owner));
        loaded.read_custom_nbt(&plain);
        assert_eq!(loaded.thrower.uuid(), None);
        assert_eq!(loaded.target.load(), None);
    }

    #[test]
    fn damage_resistant_items() {
        let sword = ItemStack::new(1, &Item::NETHERITE_SWORD);
        assert!(!sword.can_be_hurt_by(&DamageType::LAVA));
        assert!(sword.can_be_hurt_by(&DamageType::CACTUS));

        let star = ItemStack::new(1, &Item::NETHER_STAR);
        assert!(!star.can_be_hurt_by(&DamageType::EXPLOSION));
        assert!(star.can_be_hurt_by(&DamageType::LAVA));
    }
}
