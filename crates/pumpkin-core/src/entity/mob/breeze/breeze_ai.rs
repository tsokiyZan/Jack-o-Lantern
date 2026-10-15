//! Vanilla `BreezeAi`, with `BreezeAi.SlideToTargetSink`.

use std::sync::Arc;

use pumpkin_data::entity::EntityPose;
use pumpkin_data::environment_attribute::Activity;
use pumpkin_data::sound::Sound;

use super::long_jump::LongJump;
use super::shoot::Shoot;
use super::shoot_when_stuck::ShootWhenStuck;
use super::slide::Slide;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::{
    self, Behavior, DoNothing, LookAtTargetSink, MoveToTargetSink, Swim, Timed,
};
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, types};
use crate::entity::ai::brain::sensing::{SensorType, was_entity_attackable_last_n_ticks};
use crate::entity::ai::brain::{BrainProvider, BrainTick};

pub const SPEED_MULTIPLIER_WHEN_SLIDING: f32 = 0.6;
pub const JUMP_CIRCLE_INNER_RADIUS: f32 = 4.0;
pub const JUMP_CIRCLE_MIDDLE_RADIUS: f32 = 8.0;
const TICKS_TO_REMEMBER_SEEN_TARGET: i32 = 100;
/// How long `SlideToTargetSink.stop` readies a shot for.
const SHOOT_AFTER_SLIDE_TICKS: i64 = 60;

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(Swim::new(0.8))),
            Box::new(Timed::new(LookAtTargetSink::new(45, 90))),
        ],
    )
}

fn init_idle_activity() -> ActivityData {
    ActivityData::with_pairs(
        Activity::Idle,
        vec![
            (
                0,
                Box::new(behavior::start_attacking(
                    |_| true,
                    |tick| tick.brain.get(types::NEAREST_ATTACKABLE).map(Arc::clone),
                )),
            ),
            (
                1,
                Box::new(behavior::start_attacking(
                    |_| true,
                    |tick| super::get_hurt_by(tick.brain),
                )),
            ),
            (2, Box::new(Timed::new(SlideToTargetSink::new(20, 40)))),
            (
                3,
                Box::new(behavior::run_one(vec![
                    (Box::new(DoNothing::new(20, 100)), 1),
                    (
                        Box::new(behavior::stroll(SPEED_MULTIPLIER_WHEN_SLIDING, true)),
                        2,
                    ),
                ])),
            ),
        ],
    )
}

fn init_fight_activity() -> ActivityData {
    let was_attackable = was_entity_attackable_last_n_ticks(TICKS_TO_REMEMBER_SEEN_TARGET);
    ActivityData::with_pairs_and_conditions(
        Activity::Fight,
        vec![
            (
                0,
                Box::new(behavior::stop_attacking_if_target_invalid(
                    move |tick, target| !was_attackable(tick, target.as_ref()),
                    |_, _| {},
                    true,
                )),
            ),
            (1, Box::new(Timed::new(Shoot))),
            (2, Box::new(Timed::new(LongJump))),
            (3, Box::new(Timed::new(ShootWhenStuck))),
            (4, Box::new(Timed::new(Slide))),
        ],
        vec![
            (types::ATTACK_TARGET.id(), MemoryStatus::ValuePresent),
            (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
        ],
    )
}

/// Vanilla `BreezeAi.updateActivity`.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    tick.brain
        .set_active_activity_to_first_valid(&[Activity::Fight, Activity::Idle]);
}

/// Vanilla `Breeze.BRAIN_PROVIDER`'s sensors.
const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::HurtBy,
    SensorType::NearestPlayers,
    SensorType::BreezeAttackEntitySensor,
];

pub static BREEZE_PROVIDER: BrainProvider = BrainProvider {
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

/// Vanilla `BreezeAi.SlideToTargetSink`: walks to the walk target in the sliding pose,
/// then readies a shot if it has a target.
pub struct SlideToTargetSink(MoveToTargetSink);

impl SlideToTargetSink {
    #[must_use]
    pub const fn new(min_timeout: i32, max_timeout: i32) -> Self {
        Self(MoveToTargetSink::new(min_timeout, max_timeout))
    }
}

impl Behavior for SlideToTargetSink {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        self.0.entry_conditions()
    }

    fn min_duration(&self) -> i32 {
        self.0.min_duration()
    }

    fn max_duration(&self) -> i32 {
        self.0.max_duration()
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        self.0.check_extra_start_conditions(tick)
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        self.0.start(tick);
        tick.mob
            .play_sound_at_self(Sound::EntityBreezeSlide, 1.0, 1.0);
        tick.mob.set_pose(EntityPose::Sliding);
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        self.0.tick(tick);
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        self.0.stop(tick);
        tick.mob.set_pose(EntityPose::Standing);
        if tick.brain.has_memory_value(types::ATTACK_TARGET.id()) {
            tick.brain
                .set_with_expiry(types::BREEZE_SHOOT, (), SHOOT_AFTER_SLIDE_TICKS);
        }
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        self.0.can_still_use(tick)
    }

    fn times_out(&self) -> bool {
        self.0.times_out()
    }

    fn debug_name(&self) -> &'static str {
        "SlideToTargetSink"
    }
}
