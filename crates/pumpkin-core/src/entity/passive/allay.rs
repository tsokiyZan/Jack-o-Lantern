use std::sync::{
    Arc, Mutex, PoisonError, Weak,
    atomic::{AtomicBool, AtomicI64, Ordering},
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::Block;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::{EnchantmentsImpl, EquipmentSlot, PotionContentsImpl};
use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Tag, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use uuid::Uuid;

use crate::entity::ai::brain::behavior::utils::{is_alive, throw_item};
use crate::entity::ai::brain::memory::{PackedMemories, types};
use crate::entity::ai::brain::{Brain, BrainTick};
use crate::entity::ai::control::flying_move_control::FlyingMoveControl;
use crate::entity::ai::pathfinder::{FlyingPathNavigation, Navigator};
use crate::entity::item::ItemEntity;
use crate::entity::mob::{Mob, MobEntity, mob_hurt_server};
use crate::entity::passive::allay_ai;
use crate::entity::player::Player;
use crate::entity::r#type::from_type;
use crate::entity::{Entity, EntityBase};
use crate::world::World;
use crate::world::game_event::vibrations::{
    self, VibrationData, VibrationListener, VibrationSystem, VibrationUser,
};
use crate::world::game_event::{
    DynamicGameEventListener, GameEventContext, GameEventListener, PositionSource,
    notification_radius,
};

/// Vanilla `Allay.ITEM_PICKUP_REACH`.
const ITEM_PICKUP_REACH: Vector3<i32> = Vector3::new(1, 1, 1);
/// Vanilla `Allay.DUPLICATION_COOLDOWN_TICKS`.
const DUPLICATION_COOLDOWN_TICKS: i64 = 6000;
/// Vanilla `Allay.MAX_NOTEBLOCK_DISTANCE`.
pub const MAX_NOTEBLOCK_DISTANCE: i32 = 1024;
/// Vanilla `Allay.THROW_SOUND_PITCHES`.
pub const THROW_SOUND_PITCHES: [f32; 16] = [
    0.5625, 0.625, 0.75, 0.9375, 1.0, 1.0, 1.125, 1.25, 1.5, 1.875, 2.0, 2.25, 2.5, 3.0, 3.75, 4.0,
];
/// The required path length vanilla `Allay.createNavigation` sets.
const REQUIRED_PATH_LENGTH: f32 = 48.0;
/// The max turn vanilla `Allay`'s constructor gives its `FlyingMoveControl`.
const MAX_TURN: i32 = 20;
/// Vanilla `Allay.VibrationUser.VIBRATION_EVENT_LISTENER_RANGE`.
const VIBRATION_EVENT_LISTENER_RANGE: i32 = 16;
/// The interval of vanilla `Allay.aiStep`'s healing, one health each time.
const HEAL_INTERVAL: i32 = 10;
/// The interval at which vanilla `Allay.aiStep` checks whether to stop dancing.
const STOP_DANCING_CHECK_INTERVAL: i32 = 20;
/// What vanilla `BehaviorUtils.throwItem(thrower, item, targetPos)` throws with.
const DEFAULT_THROW_VELOCITY: f32 = 0.3;

/// Vanilla `SimpleContainer(1)`, the allay's `InventoryCarrier` inventory.
pub struct AllayInventory {
    item: Mutex<ItemStack>,
}

impl AllayInventory {
    fn new() -> Self {
        Self {
            item: Mutex::new(ItemStack::EMPTY.clone()),
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, ItemStack> {
        self.item.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slot().is_empty()
    }

    /// Vanilla `SimpleContainer.canAddItem`.
    #[must_use]
    pub fn can_add_item(&self, stack: &ItemStack) -> bool {
        let target = self.slot();
        target.is_empty()
            || target.are_items_and_components_equal(stack)
                && target.item_count < target.get_max_stack_size()
    }

    /// Vanilla `SimpleContainer.addItem`: returns what did not fit.
    pub fn add_item(&self, stack: &ItemStack) -> ItemStack {
        Self::add_item_to(&mut self.slot(), stack)
    }

    fn add_item_to(target: &mut ItemStack, stack: &ItemStack) -> ItemStack {
        if stack.is_empty() {
            return ItemStack::EMPTY.clone();
        }
        let mut remaining = stack.clone();
        // `moveItemToOccupiedSlotsWithSameType`, then `moveItemToEmptySlots`.
        if !target.is_empty() && target.are_items_and_components_equal(&remaining) {
            let moved = remaining.item_count.min(
                target
                    .get_max_stack_size()
                    .saturating_sub(target.item_count),
            );
            target.increment(moved);
            remaining.decrement(moved);
            if remaining.is_empty() {
                return ItemStack::EMPTY.clone();
            }
        }
        if target.is_empty() {
            *target = remaining;
            return ItemStack::EMPTY.clone();
        }
        remaining
    }

    /// Vanilla `SimpleContainer.removeItem(0, count)`.
    pub fn remove_item(&self, count: u8) -> ItemStack {
        let mut target = self.slot();
        if target.is_empty() {
            return ItemStack::EMPTY.clone();
        }
        target.split(count)
    }

    /// Vanilla `SimpleContainer.removeAllItems`.
    pub fn remove_all_items(&self) -> Vec<ItemStack> {
        let item = std::mem::replace(&mut *self.slot(), ItemStack::EMPTY.clone());
        if item.is_empty() {
            Vec::new()
        } else {
            vec![item]
        }
    }

    /// Vanilla `InventoryCarrier.writeInventoryToTag`.
    fn write_inventory_to_tag(&self, nbt: &mut NbtCompound) {
        let mut items = Vec::new();
        let item = self.slot();
        if !item.is_empty() {
            let mut item_nbt = NbtCompound::new();
            item.write_item_stack(&mut item_nbt);
            items.push(NbtTag::Compound(item_nbt));
        }
        nbt.put_list("Inventory", items);
    }

    /// Vanilla `InventoryCarrier.readInventoryFromTag`, through `SimpleContainer.fromItemList`.
    fn read_inventory_from_tag(&self, nbt: &NbtCompound) {
        let Some(items) = nbt.get_list("Inventory") else {
            return;
        };
        let mut target = self.slot();
        *target = ItemStack::EMPTY.clone();
        for item in items {
            if let Some(stack) = item.extract_compound().and_then(ItemStack::read_item_stack) {
                Self::add_item_to(&mut target, &stack);
            }
        }
    }
}

/// Vanilla `EnchantmentHelper.has(stack, PREVENT_EQUIPMENT_DROP)`.
pub(crate) fn prevents_equipment_drop(stack: &ItemStack) -> bool {
    stack
        .get_data_component::<EnchantmentsImpl>()
        .is_some_and(|enchantments| {
            enchantments
                .enchantment
                .iter()
                .any(|(enchantment, _)| enchantment.effects.prevent_equipment_drop)
        })
}

pub struct AllayEntity {
    pub mob_entity: MobEntity,
    pub inventory: AllayInventory,
    jukebox_pos: AtomicCell<Option<BlockPos>>,
    duplication_cooldown: AtomicI64,
    /// `DATA_DANCING`.
    dancing: AtomicBool,
    /// `DATA_CAN_DUPLICATE`.
    can_duplicate: AtomicBool,
    /// The `LIKED_PLAYER` memory, mirrored for `isLikedPlayer`: the player sensor asks
    /// `considersEntityAsAlly` from inside this allay's own brain tick, where the brain is
    /// locked.
    liked_player: AtomicCell<Option<Uuid>>,
    vibration_data: Mutex<VibrationData>,
    /// Vanilla `Allay.VibrationUser.positionSource`, the allay at its eye height.
    vibration_position_source: PositionSource,
}

/// Vanilla `Allay.JukeboxListener`.
struct JukeboxListener {
    allay: Weak<AllayEntity>,
    listener_source: PositionSource,
    listener_radius: i32,
}

impl GameEventListener for JukeboxListener {
    fn get_listener_source(&self) -> &PositionSource {
        &self.listener_source
    }

    fn get_listener_radius(&self) -> i32 {
        self.listener_radius
    }

    fn handle_game_event(
        &self,
        _world: &World,
        event: GameEvent,
        _context: &GameEventContext<'_>,
        source_position: Vector3<f64>,
    ) -> bool {
        let is_playing = match event {
            GameEvent::JukeboxPlay => true,
            GameEvent::JukeboxStopPlay => false,
            _ => return false,
        };
        if let Some(allay) = self.allay.upgrade() {
            allay.set_jukebox_playing(BlockPos::floored_v(source_position), is_playing);
        }
        true
    }
}

impl AllayEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `Allay.createNavigation`, made first as in `Mob`'s constructor.
        let mut navigator = Navigator::new(FlyingPathNavigation::new());
        navigator.set_can_open_doors(false);
        navigator.set_can_float(true);
        *mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = navigator;
        mob_entity.set_required_path_length(REQUIRED_PATH_LENGTH);
        // Vanilla `Allay`'s constructor. Its `setCanPickUpLoot(canPickUpLoot())` stores
        // false, the flag's default, as a new allay holds nothing.
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            Box::new(FlyingMoveControl::new(MAX_TURN, true));
        let allay = Arc::new_cyclic(|allay: &Weak<Self>| {
            let vibration_position_source = PositionSource::Entity {
                entity: allay.clone(),
                y_offset: mob_entity
                    .living_entity
                    .entity
                    .entity_dimension
                    .load()
                    .eye_height,
            };
            let dynamic_vibration_listener = DynamicGameEventListener::new(Arc::new(
                VibrationListener::new(allay.clone(), vibration_position_source.clone()),
            ));
            let dynamic_jukebox_listener =
                DynamicGameEventListener::new(Arc::new(JukeboxListener {
                    allay: allay.clone(),
                    listener_source: vibration_position_source.clone(),
                    listener_radius: notification_radius(GameEvent::JukeboxPlay),
                }));
            let _ = mob_entity
                .living_entity
                .entity
                .dynamic_game_event_listeners
                .set(Box::new([
                    dynamic_vibration_listener,
                    dynamic_jukebox_listener,
                ]));
            Self {
                mob_entity,
                inventory: AllayInventory::new(),
                jukebox_pos: AtomicCell::new(None),
                duplication_cooldown: AtomicI64::new(0),
                dancing: AtomicBool::new(false),
                can_duplicate: AtomicBool::new(true),
                liked_player: AtomicCell::new(None),
                vibration_data: Mutex::new(VibrationData::default()),
                vibration_position_source,
            }
        });
        allay.mob_entity.init_brain(allay.as_ref());
        allay
    }

    fn main_hand_item(&self) -> ItemStack {
        self.mob_entity.get_item_slot(&EquipmentSlot::MAIN_HAND)
    }

    /// Vanilla `Allay.hasItemInHand`.
    #[must_use]
    pub fn has_item_in_hand(&self) -> bool {
        !self.main_hand_item().is_empty()
    }

    /// Vanilla `Allay.isLikedPlayer`.
    fn is_liked_player(&self, other: Option<&dyn EntityBase>) -> bool {
        other
            .and_then(EntityBase::get_player)
            .is_some_and(|player| self.liked_player.load() == Some(player.gameprofile.id))
    }

    fn set_liked_player(&self, brain: &mut Brain, player: Option<Uuid>) {
        brain.set_optional(types::LIKED_PLAYER, player);
        self.liked_player.store(player);
    }

    fn is_panicking(&self) -> bool {
        self.mob_entity.with_brain(self, |tick| {
            tick.brain.has_memory_value(types::IS_PANICKING.id())
        })
    }

    /// Vanilla `Allay.setJukeboxPlaying`.
    pub fn set_jukebox_playing(&self, jukebox: BlockPos, is_playing: bool) {
        if is_playing {
            if !self.is_dancing() {
                self.jukebox_pos.store(Some(jukebox));
                self.set_dancing(true);
            }
        } else if self.jukebox_pos.load().is_none_or(|pos| pos == jukebox) {
            self.jukebox_pos.store(None);
            self.set_dancing(false);
        }
    }

    #[must_use]
    pub fn is_dancing(&self) -> bool {
        self.dancing.load(Ordering::Relaxed)
    }

    /// Vanilla `Allay.setDancing`.
    pub fn set_dancing(&self, is_dancing: bool) {
        if !self.mob_entity.is_no_ai() && (!is_dancing || !self.is_panicking()) {
            self.dancing.store(is_dancing, Ordering::Relaxed);
            self.get_entity()
                .set_synced_data(tracked_data::allay::DATA_DANCING, is_dancing);
        }
    }

    /// Vanilla `Allay.shouldStopDancing`.
    fn should_stop_dancing(&self) -> bool {
        let Some(jukebox_pos) = self.jukebox_pos.load() else {
            return true;
        };
        let entity = self.get_entity();
        let radius = f64::from(notification_radius(GameEvent::JukeboxPlay));
        jukebox_pos
            .to_centered_f64()
            .squared_distance_to_vec(&entity.pos.load())
            >= radius * radius
            || entity.world.load().get_block(&jukebox_pos) != &Block::JUKEBOX
    }

    /// Vanilla `Allay.allayConsidersItemEqual`: the same item and the same potion.
    fn allay_considers_item_equal(item1: &ItemStack, item2: &ItemStack) -> bool {
        item1.item == item2.item
            && item1.get_data_component::<PotionContentsImpl>()
                == item2.get_data_component::<PotionContentsImpl>()
    }

    /// Vanilla `Allay.wantsToPickUp`.
    fn wants_to_pick_up_item(&self, stack: &ItemStack) -> bool {
        let item_in_hand = self.main_hand_item();
        !item_in_hand.is_empty()
            && self
                .get_entity()
                .world
                .load()
                .level_info
                .load()
                .game_rules
                .mob_griefing
            && self.inventory.can_add_item(stack)
            && Self::allay_considers_item_equal(&item_in_hand, stack)
    }

    /// Vanilla `Allay.pickUpItem`, which is `InventoryCarrier.pickUpItem`.
    fn pick_up_item(&self, item_entity: &ItemEntity) {
        let stack = item_entity
            .get_item_stack()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if !self.wants_to_pick_up_item(&stack) || !self.inventory.can_add_item(&stack) {
            return;
        }
        let count = stack.item_count;
        let mut slot = self.inventory.slot().clone();
        let remainder = AllayInventory::add_item_to(&mut slot, &stack);
        // Vanilla `Mob.take`; Pumpkin's lets plugins cancel the pickup.
        if !self.mob_entity.living_entity.pickup(
            item_entity.get_entity(),
            u32::from(count - remainder.item_count),
        ) {
            return;
        }
        *self.inventory.slot() = slot;
        if remainder.is_empty() {
            item_entity.get_entity().remove();
        } else {
            item_entity
                .get_item_stack()
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .set_count(remainder.item_count);
            item_entity.init_data_tracker();
        }
    }

    /// Vanilla `Allay.updateDuplicationCooldown`.
    fn update_duplication_cooldown(&self) {
        let cooldown = self.duplication_cooldown.load(Ordering::Relaxed);
        if cooldown > 0 {
            self.set_duplication_cooldown(cooldown - 1);
        }
    }

    /// Vanilla `Allay.setDuplicationCooldown`.
    fn set_duplication_cooldown(&self, duplication_cooldown: i64) {
        self.duplication_cooldown
            .store(duplication_cooldown, Ordering::Relaxed);
        let can_duplicate = duplication_cooldown == 0;
        self.can_duplicate.store(can_duplicate, Ordering::Relaxed);
        self.get_entity()
            .set_synced_data(tracked_data::allay::DATA_CAN_DUPLICATE, can_duplicate);
    }

    /// Vanilla `Allay.duplicateAllay`.
    fn duplicate_allay(&self) {
        let entity = self.get_entity();
        let world = entity.world.load_full();
        let allay = from_type(
            &EntityType::ALLAY,
            entity.pos.load(),
            &world,
            Uuid::new_v4(),
        );
        if let Some(new_allay) = allay.cast_any().downcast_ref::<Self>() {
            new_allay
                .mob_entity
                .persistence_required
                .store(true, Ordering::Relaxed);
            new_allay.reset_duplication_cooldown();
        }
        self.reset_duplication_cooldown();
        world.spawn_entity(allay);
    }

    fn reset_duplication_cooldown(&self) {
        self.set_duplication_cooldown(DUPLICATION_COOLDOWN_TICKS);
    }

    fn can_duplicate(&self) -> bool {
        self.can_duplicate.load(Ordering::Relaxed)
    }

    /// Vanilla `Level.playSound(player, allay, ...)`: everyone near but the player, whose
    /// client plays it itself.
    fn play_sound_except(&self, player: &Player, sound: Sound, volume: f32) {
        let entity = self.get_entity();
        entity.world.load().play_sound_raw_expect(
            player,
            sound as u16,
            SoundCategory::Neutral,
            &entity.pos.load(),
            volume,
            1.0,
        );
    }
}

impl Mob for AllayEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn make_brain(&self, packed: &PackedMemories) -> Brain {
        allay_ai::ALLAY_PROVIDER.make_brain(self, packed)
    }

    /// The rest of vanilla `Allay.customServerAiStep`: the activity update.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        allay_ai::update_activity(tick);
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
    }

    /// Vanilla `Allay.travel`.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let living = &self.mob_entity.living_entity;
        living.travel_flying(caller, living.speed.load());
        true
    }

    /// Vanilla `Allay.hurtServer`: the liked player can't hurt it.
    fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if self.is_liked_player(cause) {
            return false;
        }
        mob_hurt_server(self, caller, amount, damage_type, position, source, cause)
    }

    /// Vanilla `Allay.considersEntityAsAlly`.
    fn mob_considers_entity_as_ally(&self, other: &dyn EntityBase) -> bool {
        self.is_liked_player(Some(other))
    }

    /// Vanilla `Allay.checkFallDamage` does nothing.
    fn mob_check_fall_damage(
        &self,
        _caller: &dyn crate::entity::EntityBase,
        _ya: f64,
        _on_ground: bool,
        _state: &'static pumpkin_data::BlockState,
        _pos: pumpkin_util::math::position::BlockPos,
    ) {
    }

    /// Vanilla `Allay.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        Some(if self.has_item_in_hand() {
            Sound::EntityAllayAmbientWithItem
        } else {
            Sound::EntityAllayAmbientWithoutItem
        })
    }

    /// Vanilla `Allay.removeWhenFarAway`.
    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        false
    }

    /// Vanilla `Allay.isFlapping`.
    fn is_flapping(&self) -> bool {
        !self.get_entity().on_ground.load(Ordering::Relaxed)
    }

    /// Vanilla `Allay.canPickUpLoot`.
    fn can_pick_up_loot(&self, brain: &Brain) -> bool {
        !brain.has_memory_value(types::ITEM_PICKUP_COOLDOWN_TICKS.id()) && self.has_item_in_hand()
    }

    /// Vanilla `Allay.wantsToPickUp`.
    fn wants_to_pick_up(&self, _brain: &Brain, stack: &ItemStack) -> bool {
        self.wants_to_pick_up_item(stack)
    }

    /// The looting at the end of vanilla `Mob.aiStep`, then the rest of `Allay.aiStep`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        if self
            .mob_entity
            .with_brain(self, |tick| self.can_pick_up_loot(tick.brain))
        {
            self.mob_entity
                .for_each_item_in_reach(ITEM_PICKUP_REACH, |item_entity| {
                    let wanted = self.wants_to_pick_up_item(
                        &item_entity
                            .get_item_stack()
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner),
                    );
                    if wanted {
                        self.pick_up_item(item_entity);
                    }
                });
        }

        let tick_count = self.get_entity().tick_count.load(Ordering::Relaxed);
        if is_alive(self) && tick_count % HEAL_INTERVAL == 0 {
            self.mob_entity.living_entity.heal(1.0);
        }

        if self.is_dancing()
            && self.should_stop_dancing()
            && tick_count % STOP_DANCING_CHECK_INTERVAL == 0
        {
            self.set_dancing(false);
            self.jukebox_pos.store(None);
        }

        self.update_duplication_cooldown();
    }

    /// The server half of vanilla `Allay.tick` after `super.tick()`.
    fn post_tick(&self) {
        vibrations::tick(
            &self.get_entity().world.load_full(),
            &self.vibration_data,
            self,
        );
        if self.is_panicking() {
            self.set_dancing(false);
        }
    }

    /// Vanilla `Allay.mobInteract`, after `Entity.interact`'s leash handling.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if self.mob_entity.mob_interact(player, item_stack) {
            return true;
        }

        let item_in_hand = self.main_hand_item();
        if self.is_dancing()
            && item_stack
                .item
                .has_tag(&tag::Item::MINECRAFT_DUPLICATES_ALLAYS)
            && self.can_duplicate()
        {
            self.duplicate_allay();
            let entity = self.get_entity();
            entity.world.load().send_entity_status(
                entity,
                EntityStatus::InLoveHearts,
                Some(ActorEventID::InLoveHearts),
            );
            self.play_sound_except(player, Sound::BlockAmethystBlockChime, 2.0);
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            return true;
        }

        if item_in_hand.is_empty() && !item_stack.is_empty() {
            let item_to_give = item_stack.copy_with_count(1);
            self.mob_entity
                .set_item_slot(&EquipmentSlot::MAIN_HAND, item_to_give);
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            self.play_sound_except(player, Sound::EntityAllayItemGiven, 2.0);
            self.mob_entity.with_brain(self, |tick| {
                self.set_liked_player(tick.brain, Some(player.gameprofile.id));
            });
            return true;
        }

        // Pumpkin only interacts with the main hand.
        if !item_in_hand.is_empty() && item_stack.is_empty() {
            self.mob_entity
                .set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::EMPTY.clone());
            self.play_sound_except(player, Sound::EntityAllayItemTaken, 2.0);
            self.mob_entity.living_entity.swing_hand();
            let entity = self.get_entity();
            for stack in self.inventory.remove_all_items() {
                throw_item(
                    entity,
                    stack,
                    entity.pos.load(),
                    Vector3::new(
                        f64::from(DEFAULT_THROW_VELOCITY),
                        f64::from(DEFAULT_THROW_VELOCITY),
                        f64::from(DEFAULT_THROW_VELOCITY),
                    ),
                    DEFAULT_THROW_VELOCITY,
                );
            }
            self.mob_entity
                .with_brain(self, |tick| self.set_liked_player(tick.brain, None));
            // Vanilla `Player.addItem`, which loses what doesn't fit. The interaction writes
            // `item_stack` back to the held slot, so it takes whatever landed there.
            let mut returned = item_in_hand;
            player.inventory.insert_stack_anywhere(&mut returned);
            *item_stack = player.inventory.held_item();
            return true;
        }

        false
    }

    /// Vanilla `Allay.dropEquipment`.
    fn drop_equipment(&self) {
        let entity = self.get_entity();
        for stack in self.inventory.remove_all_items() {
            entity.spawn_at_location(stack);
        }
        let item_stack = self.main_hand_item();
        if !item_stack.is_empty() && !prevents_equipment_drop(&item_stack) {
            entity.spawn_at_location(item_stack);
            self.mob_entity
                .set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::EMPTY.clone());
        }
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        // Vanilla `Mob` saves the overridden `canPickUpLoot`, not the stored flag.
        let can_pick_up_loot = self
            .mob_entity
            .with_brain(self, |tick| self.can_pick_up_loot(tick.brain));
        nbt.put_bool("CanPickUpLoot", can_pick_up_loot);
        self.inventory.write_inventory_to_tag(nbt);
        nbt.put_compound(
            VibrationData::NBT_TAG_KEY,
            self.vibration_data
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .write_nbt(),
        );
        nbt.put_long(
            "DuplicationCooldown",
            self.duplication_cooldown.load(Ordering::Relaxed),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.inventory.read_inventory_from_tag(nbt);
        *self
            .vibration_data
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = VibrationData::read_nbt(nbt);
        self.set_duplication_cooldown(i64::from(
            nbt.get("DuplicationCooldown")
                .and_then(NbtTag::as_numeric_int)
                .unwrap_or(0),
        ));
        let liked_player = self
            .mob_entity
            .with_brain(self, |tick| tick.brain.get(types::LIKED_PLAYER).copied());
        self.liked_player.store(liked_player);
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        entity.set_synced_data(tracked_data::allay::DATA_DANCING, self.is_dancing());
        entity.set_synced_data(
            tracked_data::allay::DATA_CAN_DUPLICATE,
            self.can_duplicate(),
        );
    }
}

impl VibrationSystem for AllayEntity {
    fn get_vibration_data(&self) -> &Mutex<VibrationData> {
        &self.vibration_data
    }

    fn get_vibration_user(&self) -> &dyn VibrationUser {
        self
    }
}

/// Vanilla `Allay.VibrationUser`.
impl VibrationUser for AllayEntity {
    fn get_listener_radius(&self) -> i32 {
        VIBRATION_EVENT_LISTENER_RANGE
    }

    fn get_position_source(&self) -> &PositionSource {
        &self.vibration_position_source
    }

    fn can_receive_vibration(
        &self,
        world: &World,
        pos: BlockPos,
        _event: GameEvent,
        _context: &GameEventContext<'_>,
    ) -> bool {
        if self.mob_entity.is_no_ai() {
            return false;
        }
        let liked_noteblock_pos = self.mob_entity.with_brain(self, |tick| {
            tick.brain.get(types::LIKED_NOTEBLOCK_POSITION).copied()
        });
        liked_noteblock_pos.is_none_or(|global_pos| {
            global_pos.is_close_enough(
                &world.dimension,
                self.get_entity().block_pos.load(),
                MAX_NOTEBLOCK_DISTANCE,
            ) && global_pos.pos == pos
        })
    }

    fn on_receive_vibration(
        &self,
        _world: &Arc<World>,
        pos: BlockPos,
        event: GameEvent,
        _source_entity: Option<&dyn EntityBase>,
        _projectile_owner: Option<&dyn EntityBase>,
        _receiving_distance: f32,
    ) {
        if event == GameEvent::NoteBlockPlay {
            self.mob_entity
                .with_brain(self, |tick| allay_ai::hear_noteblock(tick, pos));
        }
    }

    fn get_listenable_events(&self) -> &'static Tag {
        &tag::GameEvent::MINECRAFT_ALLAY_CAN_LISTEN
    }
}
