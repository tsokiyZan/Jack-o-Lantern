//! Vanilla `TadpoleAi`.

use pumpkin_data::entity::EntityType;
use pumpkin_data::environment_attribute::Activity;
use pumpkin_data::tag;

use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::one_shot::trigger_if;
use crate::entity::ai::brain::behavior::{
    self, AnimalPanic, CountDownCooldownTicks, FollowTemptation, GateBehavior, LookAtTargetSink,
    MoveToTargetSink, OrderPolicy, RunningPolicy, Timed,
};
use crate::entity::ai::brain::memory::{MemoryStatus, types};
use crate::entity::ai::brain::sensing::SensorType;
use crate::entity::ai::brain::{BrainProvider, BrainTick};

const SPEED_MULTIPLIER_WHEN_PANICKING: f32 = 2.0;
const SPEED_MULTIPLIER_WHEN_IDLING_IN_WATER: f32 = 0.5;
const SPEED_MULTIPLIER_WHEN_TEMPTED: f32 = 1.25;

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(AnimalPanic::with_damage_types(
                SPEED_MULTIPLIER_WHEN_PANICKING,
                &tag::DamageType::MINECRAFT_PANIC_CAUSES,
            ))),
            Box::new(Timed::new(LookAtTargetSink::new(45, 90))),
            Box::new(Timed::new(MoveToTargetSink::default())),
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
                Box::new(Timed::new(FollowTemptation::new(|_| {
                    SPEED_MULTIPLIER_WHEN_TEMPTED
                }))),
            ),
            (
                2,
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
                            Box::new(behavior::set_walk_target_from_look_target(
                                |_| true,
                                SPEED_MULTIPLIER_WHEN_IDLING_IN_WATER,
                                3,
                            )),
                            3,
                        ),
                        (
                            Box::new(trigger_if("TriggerIf", Vec::new(), |tick| {
                                tick.mob.get_entity().is_in_water()
                            })),
                            5,
                        ),
                    ],
                )),
            ),
        ],
    )
}

/// Vanilla `TadpoleAi.updateActivity`.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    tick.brain
        .set_active_activity_to_first_valid(&[Activity::Idle]);
}

pub const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::NearestPlayers,
    SensorType::HurtBy,
    SensorType::FrogTemptations,
];

pub static TADPOLE_PROVIDER: BrainProvider = BrainProvider {
    memory_types: &[],
    sensor_types: SENSOR_TYPES,
    activities: |_| vec![init_core_activity(), init_idle_activity()],
};
