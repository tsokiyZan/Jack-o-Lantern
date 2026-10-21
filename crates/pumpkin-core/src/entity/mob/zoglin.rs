use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::{EntityPose, EntityStatus, EntityType};
use pumpkin_data::environment_attribute::Activity;
use pumpkin_data::sound::Sound;
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::utils::{
    is_alive, is_other_target_much_further_away_than_current_attack_target,
};
use crate::entity::ai::brain::behavior::{
    self, DoNothing, LookAtTargetSink, MoveToTargetSink, Timed,
};
use crate::entity::ai::brain::memory::{PackedMemories, types};
use crate::entity::ai::brain::sensing::{SensorType, is_entity_attackable};
use crate::entity::ai::brain::{Brain, BrainProvider, BrainTick};
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::{Mob, MobEntity, hoglin_base, mob_hurt_server};
use crate::entity::{Entity, EntityBase};
use crate::world::World;

/// Vanilla `Zoglin.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::ZOGLIN_BABY_DIMENSIONS;
const BABY_ATTACK_DAMAGE: f64 = 0.5;
const ATTACK_INTERVAL: i32 = 40;
const BABY_ATTACK_INTERVAL: i32 = 15;
const ATTACK_DURATION: i64 = 200;
const SPEED_MULTIPLIER_WHEN_IDLING: f32 = 0.4;
/// Vanilla `SetEntityLookTargetSometimes.create(8.0F, UniformInt.of(30, 60))`.
const LOOK_DISTANCE: f32 = 8.0;
const LOOK_INTERVAL: (i32, i32) = (30, 60);

pub struct ZoglinEntity {
    pub mob_entity: MobEntity,
    is_baby: AtomicBool,
    attack_animation_remaining_ticks: AtomicI32,
}

impl ZoglinEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let zoglin = Self {
            mob_entity: MobEntity::new(entity),
            is_baby: AtomicBool::new(false),
            attack_animation_remaining_ticks: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(zoglin);
        mob_arc.mob_entity.init_brain(mob_arc.as_ref());
        mob_arc
    }

    #[must_use]
    pub fn is_baby(&self) -> bool {
        self.is_baby.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_adult(&self) -> bool {
        !self.is_baby()
    }

    /// Vanilla `Zoglin.setBaby`; growing up keeps the lowered attack damage, as in vanilla.
    pub fn set_baby(&self, baby: bool) {
        self.mob_entity
            .set_baby_flag(&self.is_baby, tracked_data::zoglin::DATA_BABY_ID, baby);
        self.refresh_dimensions();
        if baby {
            self.mob_entity
                .living_entity
                .set_attribute_base(&Attributes::ATTACK_DAMAGE, BABY_ATTACK_DAMAGE);
        }
    }

    #[must_use]
    pub fn get_attack_animation_remaining_ticks(&self) -> i32 {
        self.attack_animation_remaining_ticks
            .load(Ordering::Relaxed)
    }

    /// Vanilla `Zoglin.setAttackTarget`.
    fn set_attack_target(brain: &mut Brain, target: Arc<dyn EntityBase>) {
        brain.erase(types::CANT_REACH_WALK_TARGET_SINCE.id());
        brain.set_with_expiry(types::ATTACK_TARGET, target, ATTACK_DURATION);
    }

    /// Vanilla `Zoglin.updateActivity`.
    fn update_activity(&self, tick: &mut BrainTick<'_>) {
        let old_activity = tick.brain.get_active_non_core_activity();
        tick.brain
            .set_active_activity_to_first_valid(&[Activity::Fight, Activity::Idle]);
        let new_activity = tick.brain.get_active_non_core_activity();
        if new_activity == Some(Activity::Fight) && old_activity != Some(Activity::Fight) {
            self.play_angry_sound();
        }
        let has_attack_target = tick.brain.has_memory_value(types::ATTACK_TARGET.id());
        self.mob_entity.set_attacking(has_attack_target);
    }

    /// Vanilla `Zoglin.playAngrySound`.
    fn play_angry_sound(&self) {
        self.make_sound(Sound::EntityZoglinAngry);
    }
}

fn is_adult(tick: &BrainTick<'_>) -> bool {
    !tick.mob.get_entity().is_baby()
}

/// Vanilla `Zoglin.initCoreActivity`.
fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(LookAtTargetSink::new(45, 90))),
            Box::new(Timed::new(MoveToTargetSink::default())),
        ],
    )
}

/// Vanilla `Zoglin.initIdleActivity`.
fn init_idle_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Idle,
        10,
        vec![
            Box::new(behavior::start_attacking(
                |_| true,
                find_nearest_valid_attack_target,
            )),
            Box::new(behavior::set_entity_look_target_sometimes(
                |_| true,
                LOOK_DISTANCE,
                LOOK_INTERVAL.0,
                LOOK_INTERVAL.1,
            )),
            Box::new(behavior::run_one(vec![
                (
                    Box::new(behavior::stroll(SPEED_MULTIPLIER_WHEN_IDLING, true)),
                    2,
                ),
                (
                    Box::new(behavior::set_walk_target_from_look_target(
                        |_| true,
                        SPEED_MULTIPLIER_WHEN_IDLING,
                        3,
                    )),
                    2,
                ),
                (Box::new(DoNothing::new(30, 60)), 1),
            ])),
        ],
    )
}

/// Vanilla `Zoglin.initFightActivity`.
fn init_fight_activity() -> ActivityData {
    ActivityData::with_gate_memory(
        Activity::Fight,
        10,
        vec![
            Box::new(behavior::set_walk_target_from_attack_target_if_target_out_of_reach(1.0)),
            Box::new(behavior::one_shot::sequence(
                "TriggerIfAdult",
                |tick| is_adult(tick),
                behavior::melee_attack(|_| true, ATTACK_INTERVAL),
            )),
            Box::new(behavior::one_shot::sequence(
                "TriggerIfBaby",
                |tick| !is_adult(tick),
                behavior::melee_attack(|_| true, BABY_ATTACK_INTERVAL),
            )),
            Box::new(behavior::stop_attacking_if_target_invalid(
                |_, _| false,
                |_, _| {},
                true,
            )),
        ],
        types::ATTACK_TARGET.id(),
    )
}

/// Vanilla `Zoglin.findNearestValidAttackTarget`: anything but zoglins and creepers.
fn find_nearest_valid_attack_target(tick: &BrainTick<'_>) -> Option<Arc<dyn EntityBase>> {
    let ctx = tick.visibility();
    ctx.brain
        .get(types::NEAREST_VISIBLE_LIVING_ENTITIES)?
        .find_closest(&ctx, |target| {
            let target_type = target.get_entity().entity_type;
            target_type != &EntityType::ZOGLIN
                && target_type != &EntityType::CREEPER
                && is_entity_attackable(&ctx, target.as_ref())
        })
}

/// Vanilla `Zoglin.BRAIN_PROVIDER`'s sensors.
const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::NearestPlayers,
];

pub static ZOGLIN_PROVIDER: BrainProvider = BrainProvider {
    memory_types: &[],
    sensor_types: SENSOR_TYPES,
    activities: |_| {
        vec![
            init_core_activity(),
            init_idle_activity(),
            init_fight_activity(),
        ]
    },
};

impl Mob for ZoglinEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn make_brain(&self, packed: &PackedMemories) -> Brain {
        ZOGLIN_PROVIDER.make_brain(self, packed)
    }

    /// Vanilla `Zoglin.customServerAiStep`'s `updateActivity`.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        self.update_activity(tick);
    }

    /// Vanilla `Zoglin.customServerAiStep`.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
    }

    /// Vanilla `Zoglin.aiStep`, before `super.aiStep()`.
    fn ai_step(&self, _caller: &dyn EntityBase) {
        if self.get_attack_animation_remaining_ticks() > 0 {
            self.attack_animation_remaining_ticks
                .fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Vanilla `Zoglin.doHurtTarget`.
    fn do_hurt_target(&self, tick: &mut BrainTick<'_>, target: &Arc<dyn EntityBase>) -> bool {
        if target.get_living_entity().is_none() {
            return false;
        }
        self.attack_animation_remaining_ticks
            .store(hoglin_base::ATTACK_ANIMATION_DURATION, Ordering::Relaxed);
        tick.world
            .send_entity_status(self.get_entity(), EntityStatus::StartAttacking, None);
        self.make_sound(Sound::EntityZoglinAttack);
        hoglin_base::hurt_and_throw_target(self, target.as_ref())
    }

    /// Vanilla `Zoglin.blockedByItem`.
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

    /// Vanilla `Zoglin.hurtServer`. The new attack target reaches the brain on the zoglin's
    /// next brain tick, since the attacker may be holding its own brain.
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
                let can_attack = attacker
                    .get_living_entity()
                    .is_some_and(|living| tick.mob.can_attack(living));
                if can_attack
                    && !is_other_target_much_further_away_than_current_attack_target(
                        tick.brain,
                        tick.mob,
                        attacker.as_ref(),
                        4.0,
                    )
                {
                    Self::set_attack_target(tick.brain, attacker);
                }
            }));
        }
        was_hurt
    }

    /// Vanilla `Zoglin.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    /// Vanilla `Zoglin.finalizeSpawn`.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        // `level.getRandom()`.
        let roll = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_f32();
        if roll < hoglin_base::PROBABILITY_OF_SPAWNING_AS_BABY {
            self.set_baby(true);
        }
        self.mob_entity.finalize_spawn_base();
        group_data
    }

    fn spawn_as_baby(&self) -> bool {
        self.set_baby(true);
        true
    }

    /// Vanilla `Zoglin.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        let has_attack_target = self.mob_entity.with_brain(self, |tick| {
            tick.brain.has_memory_value(types::ATTACK_TARGET.id())
        });
        Some(if has_attack_target {
            Sound::EntityZoglinAngry
        } else {
            Sound::EntityZoglinAmbient
        })
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("IsBaby", self.is_baby());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.set_baby(nbt.get_bool("IsBaby").unwrap_or(false));
    }
}
