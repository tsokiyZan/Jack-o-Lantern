use std::sync::{
    Arc, PoisonError,
    atomic::{AtomicI32, Ordering},
};

use pumpkin_data::frog_variant::FrogVariant;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::optional_int::OptionalInt;
use pumpkin_protocol::codec::var_int::VarInt;

use super::frog_ai;
use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::brain::{Brain, BrainTick, behavior::utils::is_alive, memory::types},
    ai::control::smooth_swimming_move_control::SmoothSwimmingMoveControl,
    ai::goal::breed::BreedGoal,
    ai::pathfinder::{AmphibiousPathNavigation, Navigator, node::PathType},
    mob::{Mob, MobEntity, spawn::SpawnGroupData},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};
use crate::world::World;

/// Vanilla `Frog.FROG_FALL_DAMAGE_REDUCTION`.
const FROG_FALL_DAMAGE_REDUCTION: i32 = 5;
/// Vanilla `Frog.getHeadRotSpeed`.
const HEAD_ROT_SPEED: f32 = 35.0;
/// Vanilla `Frog.getMaxHeadYRot`.
const MAX_HEAD_Y_ROT: f32 = 5.0;
/// No tongue target, vanilla's empty `OptionalInt`.
const NO_TONGUE_TARGET: i32 = -1;

/// Represents a Frog, an amphibious mob that can eat small slimes and magma cubes.
///
/// Wiki: <https://minecraft.wiki/w/Frog>
pub struct FrogEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub variant: AtomicI32,
    pub tongue_target_id: AtomicI32,
}

impl FrogEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        *mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            Navigator::new(AmphibiousPathNavigation::frog().steered_by_move_control());
        // Vanilla `Animal`'s and `Frog`'s constructors; after the navigator, which holds
        // the maluses.
        set_animal_pathfinding_malus(&mob_entity);
        mob_entity.set_pathfinding_malus(PathType::Water, 4.0);
        mob_entity.set_pathfinding_malus(PathType::Trapdoor, -1.0);
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            Box::new(SmoothSwimmingMoveControl::new(85, 10, 0.02, 0.1, true));
        let frog = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            variant: AtomicI32::new(FrogVariant::Temperate.id() as i32),
            tongue_target_id: AtomicI32::new(NO_TONGUE_TARGET),
        };
        let mob_arc = Arc::new(frog);
        mob_arc.mob_entity.init_brain(mob_arc.as_ref());
        mob_arc
    }

    #[must_use]
    pub fn get_variant(&self) -> FrogVariant {
        FrogVariant::from_id(self.variant.load(Ordering::Relaxed) as u32)
    }

    pub fn set_variant(&self, variant: FrogVariant) {
        self.variant.store(variant.id() as i32, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            tracked_data::frog::DATA_VARIANT_ID,
            VarInt(variant.id() as i32),
        );
    }

    /// Vanilla `Frog.eraseTongueTarget`.
    pub fn erase_tongue_target(&self) {
        self.set_tongue_target_id(NO_TONGUE_TARGET);
    }

    /// Vanilla `Frog.getTongueTarget`.
    #[must_use]
    pub fn get_tongue_target(&self) -> Option<Arc<dyn EntityBase>> {
        let id = self.tongue_target_id.load(Ordering::Relaxed);
        if id == NO_TONGUE_TARGET {
            return None;
        }
        self.get_entity().world.load().get_entity_by_id(id)
    }

    /// Vanilla `Frog.setTongueTarget`.
    pub fn set_tongue_target(&self, target: &dyn EntityBase) {
        self.set_tongue_target_id(target.get_entity().entity_id);
    }

    fn set_tongue_target_id(&self, id: i32) {
        self.tongue_target_id.store(id, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            tracked_data::frog::DATA_TONGUE_TARGET_ID,
            OptionalInt((id != NO_TONGUE_TARGET).then_some(id)),
        );
    }
}

/// Vanilla `VariantUtils.selectVariantToSpawn` over the frog variants' spawn conditions:
/// the warm and cold biome tags at priority one, temperate anywhere else.
fn select_variant_to_spawn(world: &World, entity: &Entity) -> FrogVariant {
    let biome = world.get_biome(&entity.block_pos.load()).registry_id;
    let biome = biome.strip_prefix("minecraft:").unwrap_or(biome);
    let warm = tag::WorldgenBiome::MINECRAFT_SPAWNS_WARM_VARIANT_FROGS
        .0
        .contains(&biome);
    let cold = tag::WorldgenBiome::MINECRAFT_SPAWNS_COLD_VARIANT_FROGS
        .0
        .contains(&biome);
    match (warm, cold) {
        // `Util.getRandomSafe` over the variants that matched, in registry order.
        (true, true) => {
            if rand::random::<bool>() {
                FrogVariant::Cold
            } else {
                FrogVariant::Warm
            }
        }
        (true, false) => FrogVariant::Warm,
        (false, true) => FrogVariant::Cold,
        (false, false) => FrogVariant::Temperate,
    }
}

/// The `minecraft:frog/variant` component of `entity`, when it is a frog.
#[must_use]
pub fn frog_variant_of(entity: &dyn EntityBase) -> Option<FrogVariant> {
    entity
        .cast_any()
        .downcast_ref::<FrogEntity>()
        .map(FrogEntity::get_variant)
}

/// Vanilla `Frog.canEat`: tiny slimes and magma cubes, and anything else in the
/// `frog_food` entity tag.
#[must_use]
pub fn can_eat(entity: &dyn EntityBase) -> bool {
    if let Some(cube) = entity
        .cast_any()
        .downcast_ref::<crate::entity::mob::cube_mob::CubeMobEntity>()
        && cube.get_size() != 1
    {
        return false;
    }
    entity
        .get_entity()
        .entity_type
        .has_tag(&tag::EntityType::MINECRAFT_FROG_FOOD)
}

impl AgeableMob for FrogEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }

    /// Vanilla `Frog.canBeABaby`.
    fn can_be_a_baby(&self) -> bool {
        false
    }
}

impl Animal for FrogEntity {
    /// Vanilla `Frog.isFood`.
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_FROG_FOOD)
    }

    /// Vanilla `Frog.playEatingSound`.
    fn play_eating_sound(&self, _sound: Sound) {
        let entity = self.get_entity();
        entity.world.load().play_sound_fine(
            Sound::EntityFrogEat,
            SoundCategory::Neutral,
            &entity.pos.load(),
            2.0,
            1.0,
        );
    }

    /// Vanilla `Frog.spawnChildFromBreeding`: no baby, the frog carries the spawn.
    fn spawn_child_from_breeding(&self, tick: &mut BrainTick<'_>, partner: &dyn EntityBase) {
        BreedGoal::finalize_spawn_child_from_breeding(tick.mob, partner);
        tick.brain.set(types::IS_PREGNANT, ());
    }
}

impl Mob for FrogEntity {
    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn get_max_look_yaw_change(&self) -> f32 {
        HEAD_ROT_SPEED
    }

    fn get_max_head_rotation(&self) -> f32 {
        MAX_HEAD_Y_ROT
    }

    /// Vanilla `Frog.FrogLookControl.resetXRotOnTick`.
    fn look_control_resets_x_rot(&self) -> bool {
        self.get_tongue_target().is_none()
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_animal_nbt(nbt);
        nbt.put_string("variant", self.get_variant().asset_id().to_string());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_animal_nbt(nbt);
        if let Some(variant) = nbt.get_string("variant").and_then(FrogVariant::from_name) {
            self.set_variant(variant);
        }
    }

    fn mob_set_variant_name(&self, name: &str) {
        if let Some(variant) = FrogVariant::from_name(name) {
            self.set_variant(variant);
        }
    }

    fn make_brain(&self, packed: &crate::entity::ai::brain::memory::PackedMemories) -> Brain {
        frog_ai::FROG_PROVIDER.make_brain(self, packed)
    }

    /// Vanilla `Frog.customServerAiStep`'s `FrogAi.updateActivity`.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        frog_ai::update_activity(tick);
    }

    /// Vanilla `Frog.customServerAiStep`.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
    }

    /// Vanilla `AgeableMob.aiStep`, the end of `Frog`'s `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        if self.get_entity().is_alive() {
            self.ageable_ai_step();
        }
    }

    /// Vanilla `Frog.finalizeSpawn`.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.set_variant(select_variant_to_spawn(world, self.get_entity()));
        self.mob_entity.with_brain(self, frog_ai::init_memories);
        let group_data = self.finalize_ageable_spawn(group_data);
        self.mob_entity.finalize_spawn_base();
        Some(group_data)
    }

    /// Vanilla `Frog.calculateFallDamage`.
    fn calculate_fall_damage(&self, base_damage: i32) -> i32 {
        base_damage - FROG_FALL_DAMAGE_REDUCTION
    }

    /// Vanilla `Frog.isPushedByFluid`.
    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }

    /// Vanilla `Frog.travelInWater`.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        if !entity.is_in_water() {
            return false;
        }
        entity.update_velocity_from_input(living.movement_input.load(), living.get_speed(caller));
        living.make_move(caller);
        entity.velocity.store(entity.velocity.load() * 0.9);
        true
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        entity.set_synced_data(
            tracked_data::frog::DATA_VARIANT_ID,
            VarInt(self.get_variant().id() as i32),
        );
        entity.set_synced_data(tracked_data::frog::DATA_TONGUE_TARGET_ID, OptionalInt(None));
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        self.animal_interact(player, item_stack, Sound::EntityFrogAmbient)
    }
}
