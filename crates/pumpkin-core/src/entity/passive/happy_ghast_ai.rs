//! Vanilla `HappyGhastAi`, the ghastling's brain.

use pumpkin_data::environment_attribute::Activity;

use crate::entity::EntityBase;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::{
    self, AnimalPanic, CountDownCooldownTicks, FollowTemptation, Swim, Timed,
};
use crate::entity::ai::brain::memory::{MemoryStatus, types};
use crate::entity::ai::brain::sensing::SensorType;
use crate::entity::ai::brain::{BrainProvider, BrainTick};

/// Vanilla `HappyGhastAi.SPEED_MULTIPLIER_WHEN_IDLING`.
const SPEED_MULTIPLIER_WHEN_IDLING: f32 = 1.0;
/// Vanilla `HappyGhastAi.SPEED_MULTIPLIER_WHEN_TEMPTED`.
const SPEED_MULTIPLIER_WHEN_TEMPTED: f32 = 1.25;
/// Vanilla `HappyGhastAi.SPEED_MULTIPLIER_WHEN_FOLLOWING_ADULT`.
const SPEED_MULTIPLIER_WHEN_FOLLOWING_ADULT: f32 = 1.1;
/// Vanilla `HappyGhastAi.BABY_GHAST_CLOSE_ENOUGH_DIST`.
const BABY_GHAST_CLOSE_ENOUGH_DIST: f64 = 3.0;
/// Vanilla `HappyGhastAi.ADULT_FOLLOW_RANGE`.
const ADULT_FOLLOW_RANGE: (i32, i32) = (3, 16);
/// Vanilla `HappyGhast.SPEED_MULTIPLIER_WHEN_PANICKING`.
const SPEED_MULTIPLIER_WHEN_PANICKING: f32 = 2.0;

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(Swim::new(0.8))),
            Box::new(Timed::new(AnimalPanic::flying(
                SPEED_MULTIPLIER_WHEN_PANICKING,
                0,
            ))),
            Box::new(Timed::new(behavior::LookAtTargetSink::new(45, 90))),
            Box::new(Timed::new(behavior::MoveToTargetSink::default())),
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
                1,
                Box::new(Timed::new(
                    FollowTemptation::new(|_| SPEED_MULTIPLIER_WHEN_TEMPTED)
                        .with_close_enough_distance(BABY_GHAST_CLOSE_ENOUGH_DIST, true),
                )),
            ),
            (
                2,
                Box::new(behavior::baby_follow_nearest(
                    ADULT_FOLLOW_RANGE.0,
                    ADULT_FOLLOW_RANGE.1,
                    |_| SPEED_MULTIPLIER_WHEN_FOLLOWING_ADULT,
                    types::NEAREST_VISIBLE_PLAYER,
                    |player| player.clone() as std::sync::Arc<dyn EntityBase>,
                    true,
                )),
            ),
            (
                3,
                Box::new(behavior::baby_follow_adult(
                    ADULT_FOLLOW_RANGE.0,
                    ADULT_FOLLOW_RANGE.1,
                    |_| SPEED_MULTIPLIER_WHEN_FOLLOWING_ADULT,
                    true,
                )),
            ),
            (
                4,
                Box::new(behavior::run_one(vec![
                    (Box::new(behavior::fly(SPEED_MULTIPLIER_WHEN_IDLING)), 1),
                    (
                        Box::new(behavior::set_walk_target_from_look_target(
                            |_| true,
                            SPEED_MULTIPLIER_WHEN_IDLING,
                            3,
                        )),
                        1,
                    ),
                ])),
            ),
        ],
    )
}

fn init_panic_activity() -> ActivityData {
    ActivityData::with_priority_start_and_conditions(
        Activity::Panic,
        0,
        Vec::new(),
        vec![(types::IS_PANICKING.id(), MemoryStatus::ValuePresent)],
    )
}

/// Vanilla `HappyGhastAi.updateActivity`.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    tick.brain
        .set_active_activity_to_first_valid(&[Activity::Panic, Activity::Idle]);
}

pub const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::HurtBy,
    SensorType::FoodTemptations,
    SensorType::NearestAdultAnyType,
    SensorType::NearestPlayers,
];

pub static HAPPY_GHAST_PROVIDER: BrainProvider = BrainProvider {
    memory_types: &[],
    sensor_types: SENSOR_TYPES,
    activities: |_| {
        vec![
            init_core_activity(),
            init_idle_activity(),
            init_panic_activity(),
        ]
    },
};
