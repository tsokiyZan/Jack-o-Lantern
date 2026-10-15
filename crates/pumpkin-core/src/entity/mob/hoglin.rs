use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::{EntityPose, EntityStatus, EntityType};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Effect;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;
use uuid::Uuid;

use crate::entity::ageable::{AgeableData, AgeableMob};
use crate::entity::ai::brain::behavior::utils::is_alive;
use crate::entity::ai::brain::memory::PackedMemories;
use crate::entity::ai::brain::{Brain, BrainTick};
use crate::entity::mob::conversion::convert_single;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::{Mob, MobEntity, hoglin_ai, hoglin_base, mob_hurt_server};
use crate::entity::passive::animal::{Animal, set_animal_pathfinding_malus};
use crate::entity::player::Player;
use crate::entity::r#type::from_type;
use crate::entity::{Entity, EntityBase};
use crate::world::World;

/// Vanilla `Hoglin.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    EntityDimensions::scalable(0.75, 0.85).with_eye_height(0.625);
const ATTACK_DAMAGE: f64 = 6.0;
const BABY_ATTACK_DAMAGE: f64 = 0.5;
/// Vanilla `Hoglin`'s `xpReward`, which `ageBoundaryReached` keeps in step with the age.
const XP_REWARD: u32 = 5;
const BABY_XP_REWARD: u32 = 3;
/// Vanilla `MobEffectInstance(MobEffects.NAUSEA, 200, 0)` given to the new zoglin.
const CONVERSION_NAUSEA_DURATION: i32 = 200;

pub struct HoglinEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    attack_animation_remaining_ticks: AtomicI32,
    time_in_overworld: AtomicI32,
    immune_to_zombification: AtomicBool,
    cannot_be_hunted: AtomicBool,
    /// The brain's `NEAREST_REPELLENT`, kept by the sensor for `getWalkTargetValue`, which
    /// the brain's own behaviours call while it is locked.
    nearest_repellent: AtomicCell<Option<BlockPos>>,
}

impl HoglinEntity {
    pub const CONVERSION_TIME: i32 = 300;

    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        let hoglin = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            attack_animation_remaining_ticks: AtomicI32::new(0),
            time_in_overworld: AtomicI32::new(0),
            immune_to_zombification: AtomicBool::new(false),
            cannot_be_hunted: AtomicBool::new(false),
            nearest_repellent: AtomicCell::new(None),
        };
        let mob_arc = Arc::new(hoglin);
        mob_arc.mob_entity.init_brain(mob_arc.as_ref());
        mob_arc
    }

    #[must_use]
    pub fn is_adult(&self) -> bool {
        !self.is_baby()
    }

    pub fn set_time_in_overworld(&self, time_in_overworld: i32) {
        self.time_in_overworld
            .store(time_in_overworld, Ordering::Relaxed);
    }

    pub fn set_immune_to_zombification(&self, immune: bool) {
        self.immune_to_zombification
            .store(immune, Ordering::Relaxed);
        self.get_entity()
            .set_synced_data(tracked_data::hoglin::DATA_IMMUNE_TO_ZOMBIFICATION, immune);
    }

    fn is_immune_to_zombification(&self) -> bool {
        self.immune_to_zombification.load(Ordering::Relaxed)
    }

    /// Vanilla `Hoglin.isConverting`.
    #[must_use]
    pub fn is_converting(&self, world: &World) -> bool {
        !self.is_immune_to_zombification()
            && !self.mob_entity.is_no_ai()
            && world.dimension.piglins_zombify
    }

    fn set_cannot_be_hunted(&self, cannot_be_hunted: bool) {
        self.cannot_be_hunted
            .store(cannot_be_hunted, Ordering::Relaxed);
    }

    /// Vanilla `Hoglin.canBeHunted`.
    #[must_use]
    pub fn can_be_hunted(&self) -> bool {
        self.is_adult() && !self.cannot_be_hunted.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn get_attack_animation_remaining_ticks(&self) -> i32 {
        self.attack_animation_remaining_ticks
            .load(Ordering::Relaxed)
    }

    pub fn set_nearest_repellent(&self, repellent: Option<BlockPos>) {
        self.nearest_repellent.store(repellent);
    }

    /// Vanilla `Hoglin.finishConversion`.
    fn finish_conversion(&self) {
        convert_single(self, &EntityType::ZOGLIN, true, false, |zoglin| {
            zoglin.get_mob_entity().living_entity.add_effect(Effect {
                effect_type: &StatusEffect::NAUSEA,
                duration: CONVERSION_NAUSEA_DURATION,
                amplifier: 0,
                ambient: false,
                show_particles: true,
                show_icon: true,
                blend: false,
            });
        });
    }
}

impl AgeableMob for HoglinEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }

    /// Vanilla `Hoglin.ageBoundaryReached`.
    fn age_boundary_reached(&self) {
        let attack_damage = if self.is_baby() {
            BABY_ATTACK_DAMAGE
        } else {
            ATTACK_DAMAGE
        };
        self.mob_entity
            .living_entity
            .set_attribute_base(&Attributes::ATTACK_DAMAGE, attack_damage);
    }
}

impl Animal for HoglinEntity {
    /// Vanilla `Hoglin.isFood`.
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_HOGLIN_FOOD)
    }

    /// Vanilla `Hoglin.canFallInLove`.
    fn can_fall_in_love(&self) -> bool {
        !self
            .mob_entity
            .with_brain(self, |tick| hoglin_ai::is_pacified(tick.brain))
            && !self.mob_entity.is_in_love()
    }

    /// Vanilla `Hoglin.getBreedOffspring`.
    fn get_breed_offspring(
        &self,
        world: &Arc<World>,
        _partner: &dyn EntityBase,
    ) -> Option<Arc<dyn EntityBase>> {
        let offspring = from_type(
            &EntityType::HOGLIN,
            self.get_entity().pos.load(),
            world,
            Uuid::new_v4(),
        );
        if let Some(mob) = offspring.get_mob() {
            mob.get_mob_entity()
                .persistence_required
                .store(true, Ordering::Relaxed);
        }
        Some(offspring)
    }
}

impl Mob for HoglinEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn make_brain(&self, packed: &PackedMemories) -> Brain {
        hoglin_ai::HOGLIN_PROVIDER.make_brain(self, packed)
    }

    /// Vanilla `Hoglin.customServerAiStep`'s `HoglinAi.updateActivity`.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        hoglin_ai::update_activity(tick);
    }

    /// Vanilla `Hoglin.customServerAiStep`: the brain, then zombification.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
        let world = self.get_entity().world.load();
        if self.is_converting(&world) {
            let time_in_overworld = self.time_in_overworld.fetch_add(1, Ordering::Relaxed) + 1;
            if time_in_overworld > Self::CONVERSION_TIME {
                self.make_sound(Sound::EntityHoglinConvertedToZombified);
                self.finish_conversion();
            }
        } else {
            self.time_in_overworld.store(0, Ordering::Relaxed);
        }
    }

    /// Vanilla `Hoglin.aiStep`, before `super.aiStep()`.
    fn ai_step(&self, _caller: &dyn EntityBase) {
        if self.get_attack_animation_remaining_ticks() > 0 {
            self.attack_animation_remaining_ticks
                .fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Vanilla `AgeableMob.aiStep`, the end of `Hoglin`'s `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        if self.get_entity().is_alive() {
            self.ageable_ai_step();
        }
    }

    /// Vanilla `Hoglin.doHurtTarget`.
    fn do_hurt_target(&self, tick: &mut BrainTick<'_>, target: &Arc<dyn EntityBase>) -> bool {
        if target.get_living_entity().is_none() {
            return false;
        }
        self.attack_animation_remaining_ticks
            .store(hoglin_base::ATTACK_ANIMATION_DURATION, Ordering::Relaxed);
        tick.world
            .send_entity_status(self.get_entity(), EntityStatus::StartAttacking, None);
        self.make_sound(Sound::EntityHoglinAttack);
        hoglin_ai::on_hit_target(tick, target);
        hoglin_base::hurt_and_throw_target(self, target.as_ref())
    }

    /// Vanilla `Hoglin.blockedByItem`.
    fn blocked_by_item(
        &self,
        defender: &dyn EntityBase,
        _damage_type: DamageType,
        _damage: f32,
        _fully_blocked: bool,
    ) {
        if self.is_adult() {
            hoglin_base::throw_target(self, defender);
        }
    }

    /// Vanilla `Hoglin.hurtServer`. `wasHurtBy` reaches the brain on the hoglin's next brain
    /// tick, since the attacker may be holding its own brain.
    fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        let was_hurt = mob_hurt_server(self, caller, amount, damage_type, position, source, cause);
        if was_hurt
            && let Some(attacker) = cause
                .and_then(|cause| {
                    self.get_entity()
                        .world
                        .load()
                        .get_entity_by_id(cause.get_entity().entity_id)
                })
                .filter(|attacker| attacker.get_living_entity().is_some())
        {
            self.mob_entity.post_to_brain(Box::new(move |tick| {
                hoglin_ai::was_hurt_by(tick, &attacker);
            }));
        }
        was_hurt
    }

    /// Vanilla `Hoglin.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    /// Vanilla `Hoglin.finalizeSpawn`.
    fn finalize_spawn(
        &self,
        _world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        if self.get_random().random::<f32>() < hoglin_base::PROBABILITY_OF_SPAWNING_AS_BABY {
            self.set_baby(true);
        }
        let group_data = self.finalize_ageable_spawn(group_data);
        self.mob_entity.finalize_spawn_base();
        Some(group_data)
    }

    /// Vanilla `Hoglin.getWalkTargetValue`.
    fn get_walk_target_value(&self, pos: &BlockPos) -> f32 {
        if hoglin_ai::is_pos_near_nearest_repellent(self.nearest_repellent.load(), pos) {
            return -1.0;
        }
        let world = self.get_entity().world.load();
        if world.get_block(&pos.down()) == &pumpkin_data::Block::CRIMSON_NYLIUM {
            10.0
        } else {
            0.0
        }
    }

    /// Vanilla `Hoglin.mobInteract`.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let interaction_succeeded =
            self.animal_interact(player, item_stack, Sound::EntityHoglinAmbient);
        if interaction_succeeded {
            self.mob_entity
                .persistence_required
                .store(true, Ordering::Relaxed);
        }
        interaction_succeeded
    }

    /// Vanilla `Hoglin.shouldDropExperience`: babies drop it too.
    fn should_drop_experience(&self) -> bool {
        true
    }

    /// Vanilla `Hoglin.getBaseExperienceReward`.
    fn get_base_experience_reward(&self) -> u32 {
        if self.is_baby() {
            BABY_XP_REWARD
        } else {
            XP_REWARD
        }
    }

    /// Vanilla `Hoglin.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        self.mob_entity.with_brain(self, |tick| {
            hoglin_ai::get_sound_for_current_activity(tick.brain, self.is_converting(tick.world))
        })
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        if self.is_baby() {
            entity.set_synced_data(tracked_data::hoglin::DATA_BABY_ID, true);
        }
        if self.is_immune_to_zombification() {
            entity.set_synced_data(tracked_data::hoglin::DATA_IMMUNE_TO_ZOMBIFICATION, true);
        }
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
        self.write_animal_nbt(nbt);
        nbt.put_bool("IsImmuneToZombification", self.is_immune_to_zombification());
        nbt.put_int(
            "TimeInOverworld",
            self.time_in_overworld.load(Ordering::Relaxed),
        );
        nbt.put_bool(
            "CannotBeHunted",
            self.cannot_be_hunted.load(Ordering::Relaxed),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        self.read_animal_nbt(nbt);
        self.set_immune_to_zombification(nbt.get_bool("IsImmuneToZombification").unwrap_or(false));
        self.set_time_in_overworld(nbt.get_int("TimeInOverworld").unwrap_or(0));
        self.set_cannot_be_hunted(nbt.get_bool("CannotBeHunted").unwrap_or(false));
    }
}
