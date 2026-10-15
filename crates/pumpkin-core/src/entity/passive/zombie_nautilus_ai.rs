//! Vanilla `ZombieNautilusAi`.

use pumpkin_data::environment_attribute::Activity;
use pumpkin_data::sound::Sound;

use crate::entity::ai::brain::BrainProvider;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::{
    self, ChargeAttack, CountDownCooldownTicks, FollowTemptation, Timed,
};
use crate::entity::ai::brain::memory::types;
use crate::entity::passive::nautilus_ai::{
    ATTACK_KNOCKBACK_FORCE, MAX_CHARGE_DISTANCE, MAX_TARGET_DETECTION_DISTANCE, MEMORY_TYPES,
    SENSOR_TYPES, TIME_BETWEEN_ATTACKS, attack_target_conditions, fight_conditions, idle_movement,
    start_attacking, tempted_close_enough_distance,
};

const SPEED_MULTIPLIER_WHEN_TEMPTED: f32 = 0.9;
const SPEED_WHEN_ATTACKING: f32 = 0.5;

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(behavior::LookAtTargetSink::new(45, 90))),
            Box::new(Timed::new(behavior::MoveToTargetSink::default())),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::TEMPTATION_COOLDOWN_TICKS,
            ))),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::CHARGE_COOLDOWN_TICKS,
            ))),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::ATTACK_TARGET_COOLDOWN,
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
                        .with_close_enough_distance_fn(tempted_close_enough_distance),
                )),
            ),
            (2, start_attacking()),
            (3, idle_movement()),
        ],
    )
}

fn init_fight_activity() -> ActivityData {
    ActivityData::with_pairs_and_conditions(
        Activity::Fight,
        vec![(
            0,
            Box::new(Timed::new(ChargeAttack::new(
                TIME_BETWEEN_ATTACKS,
                attack_target_conditions(),
                SPEED_WHEN_ATTACKING,
                ATTACK_KNOCKBACK_FORCE,
                MAX_CHARGE_DISTANCE,
                MAX_TARGET_DETECTION_DISTANCE,
                Sound::EntityZombieNautilusDash,
            ))),
        )],
        fight_conditions(),
    )
}

pub static ZOMBIE_NAUTILUS_PROVIDER: BrainProvider = BrainProvider {
    memory_types: MEMORY_TYPES,
    sensor_types: SENSOR_TYPES,
    activities: |_| {
        vec![
            init_core_activity(),
            init_idle_activity(),
            init_fight_activity(),
        ]
    },
};
