//! Vanilla `AxolotlAi`, with `PlayDead` and `ValidatePlayDead`.

use std::sync::Arc;

use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::environment_attribute::Activity;
use pumpkin_data::potion::Effect;
use pumpkin_data::tag::{self, Taggable};

use crate::entity::EntityBase;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::one_shot::{OneShot, trigger_if};
use crate::entity::ai::brain::behavior::utils::is_breeding;
use crate::entity::ai::brain::behavior::{
    self, AnimalMakeLove, Behavior, CountDownCooldownTicks, FollowTemptation, GateBehavior,
    OrderPolicy, RunningPolicy, Timed,
};
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, types};
use crate::entity::ai::brain::sensing::SensorType;
use crate::entity::ai::brain::{BrainProvider, BrainTick};

/// Vanilla `AxolotlAi.ADULT_FOLLOW_RANGE`.
const ADULT_FOLLOW_RANGE: (i32, i32) = (5, 16);
const SPEED_MULTIPLIER_WHEN_MAKING_LOVE: f32 = 0.2;
const SPEED_MULTIPLIER_ON_LAND: f32 = 0.15;
const SPEED_MULTIPLIER_WHEN_IDLING_IN_WATER: f32 = 0.5;
const SPEED_MULTIPLIER_WHEN_CHASING_IN_WATER: f32 = 0.6;
const SPEED_MULTIPLIER_WHEN_FOLLOWING_ADULT_IN_WATER: f32 = 0.6;
/// Vanilla `MeleeAttack.create(20)`.
const MELEE_ATTACK_COOLDOWN: i32 = 20;
/// Vanilla `PlayDead`'s duration and its regeneration.
const PLAY_DEAD_DURATION: i32 = 200;
/// Vanilla `updateActivity`: a hunt ends with a 2400-tick hunting cooldown.
const HUNTING_COOLDOWN_TICKS: i64 = 2400;

fn in_water(tick: &BrainTick<'_>) -> bool {
    tick.mob.get_entity().is_in_water()
}

/// Vanilla `AxolotlAi.getSpeedModifierChasing`.
fn speed_modifier_chasing(tick: &BrainTick<'_>) -> f32 {
    if in_water(tick) {
        SPEED_MULTIPLIER_WHEN_CHASING_IN_WATER
    } else {
        SPEED_MULTIPLIER_ON_LAND
    }
}

/// Vanilla `AxolotlAi.getSpeedModifierFollowingAdult`.
fn speed_modifier_following_adult(tick: &BrainTick<'_>) -> f32 {
    if in_water(tick) {
        SPEED_MULTIPLIER_WHEN_FOLLOWING_ADULT_IN_WATER
    } else {
        SPEED_MULTIPLIER_ON_LAND
    }
}

/// Vanilla `AxolotlAi.getSpeedModifier`.
fn speed_modifier(tick: &BrainTick<'_>) -> f32 {
    if in_water(tick) {
        SPEED_MULTIPLIER_WHEN_IDLING_IN_WATER
    } else {
        SPEED_MULTIPLIER_ON_LAND
    }
}

/// Vanilla `AxolotlAi.canSetWalkTargetFromLookTarget`: only to a look target in the
/// same medium the axolotl is in.
fn can_set_walk_target_from_look_target(tick: &BrainTick<'_>) -> bool {
    let Some(look_target) = tick.brain.get(types::LOOK_TARGET) else {
        return false;
    };
    let pos = look_target.current_block_position();
    let water_at = tick
        .world
        .get_fluid(&pos)
        .has_tag(&tag::Fluid::MINECRAFT_WATER);
    water_at == in_water(tick)
}

/// Vanilla `AxolotlAi.findNearestValidAttackTarget`.
fn find_nearest_valid_attack_target(tick: &BrainTick<'_>) -> Option<Arc<dyn EntityBase>> {
    if is_breeding(tick.brain) {
        None
    } else {
        tick.brain.get(types::NEAREST_ATTACKABLE).map(Arc::clone)
    }
}

/// Vanilla `PlayDead`: lies still in water, healing, until its ticks run out.
struct PlayDead {
    conditions: [(MemoryModuleId, MemoryStatus); 2],
}

impl PlayDead {
    const fn new() -> Self {
        Self {
            conditions: [
                (types::PLAY_DEAD_TICKS.id(), MemoryStatus::ValuePresent),
                (types::HURT_BY_ENTITY.id(), MemoryStatus::ValuePresent),
            ],
        }
    }
}

impl Behavior for PlayDead {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn min_duration(&self) -> i32 {
        PLAY_DEAD_DURATION
    }

    fn max_duration(&self) -> i32 {
        PLAY_DEAD_DURATION
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        in_water(tick)
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        in_water(tick) && tick.brain.has_memory_value(types::PLAY_DEAD_TICKS.id())
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        tick.brain.erase(types::WALK_TARGET.id());
        tick.brain.erase(types::LOOK_TARGET.id());
        tick.mob.get_mob_entity().living_entity.add_effect(Effect {
            effect_type: &StatusEffect::REGENERATION,
            duration: PLAY_DEAD_DURATION,
            amplifier: 0,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        });
    }

    fn debug_name(&self) -> &'static str {
        "PlayDead"
    }
}

/// Vanilla `ValidatePlayDead`: counts the play-dead ticks down and, at zero, forgets
/// the attacker and returns to the default activity.
fn validate_play_dead() -> OneShot {
    OneShot::with_required(
        "ValidatePlayDead",
        vec![(types::PLAY_DEAD_TICKS.id(), MemoryStatus::ValuePresent)],
        vec![types::HURT_BY_ENTITY.id()],
        |tick| {
            let ticks = tick.brain.get(types::PLAY_DEAD_TICKS).copied().unwrap_or(0);
            if ticks <= 0 {
                tick.brain.erase(types::PLAY_DEAD_TICKS.id());
                tick.brain.erase(types::HURT_BY_ENTITY.id());
                tick.brain.use_default_activity();
            } else {
                tick.brain.set(types::PLAY_DEAD_TICKS, ticks - 1);
            }
            true
        },
    )
}

fn init_play_dead_activity() -> ActivityData {
    ActivityData::new(
        Activity::PlayDead,
        vec![
            (0, Box::new(Timed::new(PlayDead::new()))),
            (
                1,
                Box::new(behavior::erase_memory_if(
                    "EraseMemoryIf",
                    |tick| is_breeding(tick.brain),
                    types::PLAY_DEAD_TICKS.id(),
                )),
            ),
        ],
        vec![(types::PLAY_DEAD_TICKS.id(), MemoryStatus::ValuePresent)],
        vec![types::PLAY_DEAD_TICKS.id()],
    )
}

fn init_fight_activity() -> ActivityData {
    ActivityData::with_gate_memory(
        Activity::Fight,
        0,
        vec![
            Box::new(behavior::stop_attacking_if_target_invalid(
                |_, _| false,
                super::axolotl::on_stop_attacking,
                true,
            )),
            Box::new(
                behavior::set_walk_target_from_attack_target_if_target_out_of_reach_with(
                    speed_modifier_chasing,
                ),
            ),
            Box::new(behavior::melee_attack(|_| true, MELEE_ATTACK_COOLDOWN)),
            Box::new(behavior::erase_memory_if(
                "EraseMemoryIf",
                |tick| is_breeding(tick.brain),
                types::ATTACK_TARGET.id(),
            )),
        ],
        types::ATTACK_TARGET.id(),
    )
}

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(behavior::LookAtTargetSink::new(45, 90))),
            Box::new(Timed::new(behavior::MoveToTargetSink::default())),
            Box::new(validate_play_dead()),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::TEMPTATION_COOLDOWN_TICKS,
            ))),
        ],
    )
}

fn init_idle_activity() -> ActivityData {
    ActivityData::with_pairs(
        Activity::Idle,
        vec![
            (
                0,
                Box::new(behavior::set_entity_look_target_sometimes(
                    |entity| entity.get_entity().entity_type == &EntityType::PLAYER,
                    6.0,
                    30,
                    60,
                )),
            ),
            (
                1,
                Box::new(Timed::new(AnimalMakeLove::new(
                    &EntityType::AXOLOTL,
                    SPEED_MULTIPLIER_WHEN_MAKING_LOVE,
                    2,
                ))),
            ),
            (
                2,
                Box::new(behavior::run_one(vec![
                    (
                        Box::new(Timed::new(FollowTemptation::new(speed_modifier))),
                        1,
                    ),
                    (
                        Box::new(behavior::baby_follow_adult(
                            ADULT_FOLLOW_RANGE.0,
                            ADULT_FOLLOW_RANGE.1,
                            speed_modifier_following_adult,
                            false,
                        )),
                        1,
                    ),
                ])),
            ),
            (
                3,
                Box::new(behavior::start_attacking(
                    |_| true,
                    find_nearest_valid_attack_target,
                )),
            ),
            (
                3,
                Box::new(behavior::try_find_liquid(
                    6,
                    SPEED_MULTIPLIER_ON_LAND,
                    &tag::Fluid::MINECRAFT_AXOLOTL_TRIES_TO_FIND,
                )),
            ),
            (
                4,
                Box::new(GateBehavior::new(
                    "Gate",
                    vec![(types::WALK_TARGET.id(), MemoryStatus::ValueAbsent)],
                    Vec::new(),
                    OrderPolicy::Ordered,
                    RunningPolicy::TryAll,
                    vec![
                        (
                            Box::new(behavior::swim(SPEED_MULTIPLIER_WHEN_IDLING_IN_WATER)),
                            2,
                        ),
                        (
                            Box::new(behavior::stroll(SPEED_MULTIPLIER_ON_LAND, false)),
                            2,
                        ),
                        (
                            Box::new(behavior::set_walk_target_from_look_target_with(
                                can_set_walk_target_from_look_target,
                                speed_modifier,
                                3,
                            )),
                            3,
                        ),
                        (Box::new(trigger_if("TriggerIf", Vec::new(), in_water)), 5),
                        (
                            Box::new(trigger_if("TriggerIf", Vec::new(), |tick| {
                                tick.mob
                                    .get_entity()
                                    .on_ground
                                    .load(std::sync::atomic::Ordering::Relaxed)
                            })),
                            5,
                        ),
                    ],
                )),
            ),
        ],
    )
}

/// Vanilla `AxolotlAi.updateActivity`.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    let old_activity = tick.brain.get_active_non_core_activity();
    if old_activity == Some(Activity::PlayDead) {
        return;
    }
    tick.brain.set_active_activity_to_first_valid(&[
        Activity::PlayDead,
        Activity::Fight,
        Activity::Idle,
    ]);
    if old_activity == Some(Activity::Fight)
        && tick.brain.get_active_non_core_activity() != Some(Activity::Fight)
    {
        tick.brain
            .set_with_expiry(types::HAS_HUNTING_COOLDOWN, true, HUNTING_COOLDOWN_TICKS);
    }
}

pub const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::NearestAdult,
    SensorType::HurtBy,
    SensorType::AxolotlAttackables,
    SensorType::FoodTemptations,
];

pub static AXOLOTL_PROVIDER: BrainProvider = BrainProvider {
    memory_types: &[],
    sensor_types: SENSOR_TYPES,
    activities: |_| {
        vec![
            init_core_activity(),
            init_idle_activity(),
            init_fight_activity(),
            init_play_dead_activity(),
        ]
    },
};
