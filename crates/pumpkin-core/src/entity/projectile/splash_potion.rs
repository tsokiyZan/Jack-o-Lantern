use pumpkin_data::world::WorldEvent;
use std::sync::{Arc, PoisonError, RwLock};

use crate::item::potion::{PotionApplicationSource, PotionContents};
use crate::world::World;
use crate::world::game_event::GameEventContext;
use crate::{
    entity::{
        Entity, EntityBase,
        projectile::{ProjectileHit, ThrownItemEntity},
    },
    server::Server,
};
use pumpkin_data::block_properties::{
    CampfireLikeProperties, CandleLikeProperties, RedstoneOreLikeProperties,
};
use pumpkin_data::damage::DamageType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockDirection, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;

const GRAVITY: f64 = 0.05;

pub struct SplashPotionEntity {
    pub thrown: ThrownItemEntity,
    pub item_stack: RwLock<ItemStack>,
}

impl SplashPotionEntity {
    pub fn new(entity: Entity) -> Self {
        entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        let thrown = ThrownItemEntity::unowned(entity, GRAVITY);

        Self {
            thrown,
            item_stack: RwLock::new(ItemStack::new(1, &pumpkin_data::item::Item::SPLASH_POTION)),
        }
    }

    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        thrown.entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        Self {
            thrown,
            item_stack: RwLock::new(ItemStack::new(1, &pumpkin_data::item::Item::SPLASH_POTION)),
        }
    }

    pub fn set_item_stack(&self, item_stack: ItemStack) {
        let mut write = self
            .item_stack
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *write = item_stack;
    }
}

/// Vanilla `AbstractThrownPotion.douseFire`.
fn douse_fire(potion: &dyn EntityBase, world: &Arc<World>, pos: BlockPos) {
    let state = world.get_block_state(&pos);
    let block = Block::from_state_id(state.id);
    if block.has_tag(&tag::Block::MINECRAFT_FIRE) {
        // `level.destroyBlock(pos, false, this)`: no break event for a fire block, no drops;
        // a fire holds no fluid, so air is left behind.
        world.set_block_state(&pos, Block::AIR.default_state.id, BlockFlags::NOTIFY_ALL);
        world.game_event(
            GameEvent::BlockDestroy,
            pos.to_centered_f64(),
            GameEventContext::of(Some(potion), Some(state)),
        );
    } else if is_lit_candle(block, state.id) {
        // `AbstractCandleBlock.extinguish(null, state, level, pos)`: `setLit` with flags 11,
        // the smoke is client-side, the extinguish sound, then BLOCK_CHANGE without a source.
        let lit_off = if block.has_tag(&tag::Block::MINECRAFT_CANDLES) {
            let mut properties = CandleLikeProperties::from_state_id(state.id);
            properties.lit = false;
            properties.to_state_id(block)
        } else {
            let mut properties = RedstoneOreLikeProperties::from_state_id(state.id);
            properties.lit = false;
            properties.to_state_id(block)
        };
        // Flags 11: `UPDATE_IMMEDIATE` (8) only matters on the client.
        world.set_block_state(&pos, lit_off, BlockFlags::NOTIFY_ALL);
        world.play_sound_fine(
            Sound::BlockCandleExtinguish,
            SoundCategory::Blocks,
            &pos.to_centered_f64(),
            1.0,
            1.0,
        );
        world.game_event(
            GameEvent::BlockChange,
            pos.to_centered_f64(),
            GameEventContext::of(None, Some(world.get_block_state(&pos))),
        );
    } else if block.has_tag(&tag::Block::MINECRAFT_CAMPFIRES)
        && CampfireLikeProperties::from_state_id(state.id).lit
    {
        // `levelEvent(null, 1009, pos, 0)`, `CampfireBlock.douse(owner, ...)` (BLOCK_CHANGE
        // from the owner; its particles are client-side), then `setBlockAndUpdate` unlit.
        world.sync_world_event(WorldEvent::SoundExtinguishFire, pos, 0);
        let owner = potion
            .get_owner_id()
            .and_then(|id| world.get_entity_by_id(id));
        world.game_event(
            GameEvent::BlockChange,
            pos.to_centered_f64(),
            GameEventContext::of(owner.as_deref(), Some(state)),
        );
        let mut properties = CampfireLikeProperties::from_state_id(state.id);
        properties.lit = false;
        world.set_block_state(&pos, properties.to_state_id(block), BlockFlags::NOTIFY_ALL);
    }
}

/// Vanilla `AbstractCandleBlock.isLit`: a lit candle or candle cake.
fn is_lit_candle(block: &'static Block, state_id: BlockStateId) -> bool {
    if block.has_tag(&tag::Block::MINECRAFT_CANDLES) {
        CandleLikeProperties::from_state_id(state_id).lit
    } else if block.has_tag(&tag::Block::MINECRAFT_CANDLE_CAKES) {
        RedstoneOreLikeProperties::from_state_id(state_id).lit
    } else {
        false
    }
}

/// Vanilla `AbstractThrownPotion.onHitBlock`: with a `#douses_fire` potion, douse the block
/// in front of the hit face, the hit block itself, and the four horizontal neighbours of the
/// one in front.
fn on_hit_block(
    potion: &dyn EntityBase,
    stack: &ItemStack,
    hit_pos: BlockPos,
    face: BlockDirection,
) {
    if !potion_is(stack, &pumpkin_data::tag::Potion::MINECRAFT_DOUSES_FIRE) {
        return;
    }
    let world = potion.get_entity().world.load_full();
    let effect_pos = hit_pos.offset(face.to_offset());
    douse_fire(potion, &world, effect_pos);
    douse_fire(
        potion,
        &world,
        effect_pos.offset(face.opposite().to_offset()),
    );
    // `Direction.Plane.HORIZONTAL`.
    for direction in [
        BlockDirection::North,
        BlockDirection::East,
        BlockDirection::South,
        BlockDirection::West,
    ] {
        douse_fire(potion, &world, effect_pos.offset(direction.to_offset()));
    }
}

/// Whether the potion in `stack` is in a potion `tag` (vanilla `PotionContents.is`).
fn potion_is(stack: &ItemStack, tag: &pumpkin_data::tag::Tag) -> bool {
    stack
        .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
        .and_then(|pc| pc.potion_id)
        .is_some_and(|id| tag.1.contains(&(id as u16)))
}

/// Whether `stack` hurts water-sensitive entities (vanilla `Enderman.hurtWithCleanWater`).
#[must_use]
pub fn hurts_water_sensitive_entities(stack: &ItemStack) -> bool {
    potion_is(
        stack,
        &pumpkin_data::tag::Potion::MINECRAFT_HURTS_WATER_SENSITIVE_ENTITIES,
    )
}

/// The item of a thrown splash or lingering potion (vanilla `AbstractThrownPotion.getItem`).
#[must_use]
pub fn thrown_potion_item(entity: &dyn EntityBase) -> Option<ItemStack> {
    let any = entity.cast_any();
    let lock = if let Some(splash) = any.downcast_ref::<SplashPotionEntity>() {
        &splash.item_stack
    } else {
        &any.downcast_ref::<super::lingering_potion::LingeringPotionEntity>()?
            .item_stack
    };
    Some(lock.read().unwrap_or_else(PoisonError::into_inner).clone())
}

/// Vanilla `AbstractThrownPotion.affectEntitiesAround`, run on every hit before the effects.
pub(crate) fn affect_entities_around(potion: &dyn EntityBase, stack: &ItemStack) {
    let hurts = hurts_water_sensitive_entities(stack);
    let extinguishes = potion_is(
        stack,
        &pumpkin_data::tag::Potion::MINECRAFT_EXTINGUISHES_ENTITIES,
    );
    let entity = potion.get_entity();
    let world = entity.world.load_full();
    let pos = entity.pos.load();
    let aabb = entity.bounding_box.load().expand(4.0, 2.0, 4.0);
    if hurts || extinguishes {
        let owner = potion
            .get_owner_id()
            .and_then(|id| world.get_entity_by_id(id));
        // `getEntitiesOfClass(LivingEntity.class, aabb, WATER_SENSITIVE_OR_ON_FIRE)`.
        for target in world.get_all_at_box(&aabb) {
            if target.get_living_entity().is_none() {
                continue;
            }
            let sensitive = target
                .get_mob()
                .is_some_and(crate::entity::mob::Mob::is_sensitive_to_water);
            let target_entity = target.get_entity();
            if !sensitive && !target_entity.is_on_fire() {
                continue;
            }
            // `this.distanceToSqr(entity)`: positions.
            if pos.squared_distance_to_vec(&target_entity.pos.load()) < 16.0 {
                if hurts && sensitive {
                    target.damage_with_context(
                        target.as_ref(),
                        1.0,
                        DamageType::INDIRECT_MAGIC,
                        None,
                        Some(potion),
                        owner.as_deref(),
                    );
                }
                if extinguishes && target_entity.is_on_fire() && target_entity.is_alive() {
                    target_entity.extinguish();
                }
            }
        }
    }
    if potion_is(
        stack,
        &pumpkin_data::tag::Potion::MINECRAFT_REHYDRATES_AXOLOTLS,
    ) {
        for target in world.get_all_at_box(&aabb) {
            if let Some(axolotl) = target
                .cast_any()
                .downcast_ref::<crate::entity::passive::axolotl::AxolotlEntity>()
            {
                axolotl.rehydrate();
            }
        }
    }
}

/// Vanilla `AbstractThrownPotion.onHit` after `Projectile.onHit` (whose `onHitBlock` douses
/// fire): `affectEntitiesAround`, `onHitAsPotion` when the potion has effects, the splash
/// particles and sound level events at the potion's block, then the discard (done by the
/// caller).
pub(crate) fn on_hit_thrown_potion(
    potion: &dyn EntityBase,
    stack: &ItemStack,
    hit: &ProjectileHit,
    on_hit_as_potion: impl FnOnce(),
) {
    if let ProjectileHit::Block { pos, face, .. } = hit {
        on_hit_block(potion, stack, *pos, *face);
    }
    affect_entities_around(potion, stack);
    if PotionContents::has_effects(stack) {
        on_hit_as_potion();
    }
    let entity = potion.get_entity();
    let world = entity.world.load();
    let block_pos = entity.block_pos.load();
    let color = PotionContents::color(stack);
    let (particles, sound) = if PotionContents::potion_has_instant_effects(stack) {
        (
            WorldEvent::ParticlesInstantPotionSplash,
            WorldEvent::SoundInstantPotionSplash,
        )
    } else {
        (
            WorldEvent::ParticlesSpellPotionSplash,
            WorldEvent::SoundSpellPotionSplash,
        )
    };
    world.sync_world_event(particles, block_pos, color);
    if !entity.is_silent() {
        world.sync_world_event(sound, block_pos, 0);
    }
}

/// Vanilla `ThrownSplashPotion.onHitAsPotion`: every living entity (not spectators) in the
/// potion's box at the hit, inflated by 4, 2, 4, that potions affect gets the effects scaled by
/// its box's distance (inflated by `computeMargin`) to the potion's box, if under 4.
fn on_hit_as_potion(potion: &SplashPotionEntity, stack: &ItemStack, hit_pos: Vector3<f64>) {
    let entity = potion.get_entity();
    let world = entity.world.load_full();
    let effects = PotionContents::read_potion_effects(stack);
    let duration_scale = stack
        .get_data_component::<pumpkin_data::data_component_impl::PotionDurationScaleImpl>()
        .map_or(1.0, |component| component.scale);
    let potion_aabb = entity
        .bounding_box
        .load()
        .shift(hit_pos.sub(&entity.pos.load()));
    let effect_aabb = potion_aabb.expand(4.0, 2.0, 4.0);
    // `getEntitiesOfClass(LivingEntity.class, effectAabb)`: `NO_SPECTATORS`.
    let entities: Vec<Arc<dyn EntityBase>> = world
        .get_all_at_box(&effect_aabb)
        .into_iter()
        .filter(|candidate| candidate.get_living_entity().is_some() && !candidate.is_spectator())
        .collect();
    let margin = f64::from(super::compute_margin(entity));

    let mut affected: Vec<(Arc<dyn EntityBase>, f64)> = Vec::new();
    for candidate in entities {
        let Some(living) = candidate.get_living_entity() else {
            continue;
        };
        if !living.is_affected_by_potions(candidate.as_ref()) {
            continue;
        }
        let target_box = candidate
            .get_entity()
            .bounding_box
            .load()
            .expand_all(margin);
        let dist = potion_aabb.distance_to_sqr_box(&target_box);
        if dist < 16.0 {
            affected.push((candidate, 1.0 - dist.sqrt() / 4.0));
        }
    }

    if let Some(server) = world.server.upgrade() {
        let affected_ids: Vec<i32> = affected
            .iter()
            .map(|(candidate, _)| candidate.get_entity().entity_id)
            .collect();
        let mut event = crate::plugin::api::events::entity::potion_splash::PotionSplashEvent::new(
            entity.entity_id,
            entity.block_pos.load(),
            stack.item.registry_key.to_string(),
            affected_ids,
        );
        server.plugin_manager.fire_blocking(&server, &mut event);
        if event.cancelled {
            return;
        }
        affected.retain(|(candidate, _)| {
            event
                .affected_entities
                .contains(&candidate.get_entity().entity_id)
        });
    }

    let owner = potion
        .get_owner_id()
        .and_then(|id| world.get_entity_by_id(id));
    for (candidate, scale) in affected {
        if let Some(living) = candidate.get_living_entity() {
            PotionContents::apply_effects_to(
                living,
                effects.clone(),
                duration_scale,
                PotionApplicationSource::Splash(scale),
                Some(potion),
                owner.as_deref(),
            );
        }
    }
}

impl EntityBase for SplashPotionEntity {
    fn get_projectile(&self) -> Option<&crate::entity::projectile::Projectile> {
        Some(&self.thrown.projectile)
    }

    fn init_data_tracker(&self) {
        let entity = self.get_entity();
        let stack = self
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Sync the item stack
        entity.set_synced_data(
            pumpkin_data::tracked_data::splash_potion::ITEM_STACK,
            pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer::from(stack.clone()),
        );
    }

    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        self.thrown.process_tick(caller);
    }

    /// Vanilla `ThrowableItemProjectile.addAdditionalSaveData`: the `Item`.
    fn write_custom_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        let mut item = pumpkin_nbt::compound::NbtCompound::new();
        self.item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .write_item_stack(&mut item);
        nbt.put_compound("Item", item);
    }

    /// Vanilla `ThrowableItemProjectile.readAdditionalSaveData`: `setItem` with the saved
    /// `Item`, or the default item.
    fn read_custom_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        let mut stack = nbt
            .get_compound("Item")
            .and_then(ItemStack::read_item_stack)
            .unwrap_or_else(|| ItemStack::new(1, &pumpkin_data::item::Item::SPLASH_POTION));
        // `source.copyWithCount(1)`.
        stack.item_count = 1;
        self.set_item_stack(stack.clone());
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::splash_potion::ITEM_STACK,
            pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer::from(stack),
        );
    }

    fn get_entity(&self) -> &Entity {
        self.thrown.get_entity()
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        let stack = self
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let hit_pos = hit.hit_pos();
        on_hit_thrown_potion(self, &stack, &hit, || {
            on_hit_as_potion(self, &stack, hit_pos);
        });
    }
}
