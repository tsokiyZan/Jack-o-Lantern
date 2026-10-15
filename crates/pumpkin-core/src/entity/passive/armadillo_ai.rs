//! Vanilla `ArmadilloAi`, with `ArmadilloBallUp` and `ArmadilloPanic`.

use pumpkin_data::entity::EntityType;
use pumpkin_data::environment_attribute::Activity;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag;
use rand::RngExt;

use crate::entity::EntityBase;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::{
    self, AnimalMakeLove, AnimalPanic, Behavior, CountDownCooldownTicks, DoNothing,
    FollowTemptation, LookAtTargetSink, MoveToTargetSink, OneShot, RandomLookAround, Swim, Timed,
};
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, NEVER_EXPIRE, types};
use crate::entity::ai::brain::sensing::SensorType;
use crate::entity::ai::brain::{BrainProvider, BrainTick};

use super::armadillo::{ArmadilloEntity, ArmadilloState};

const SPEED_MULTIPLIER_WHEN_PANICKING: f32 = 2.0;
const SPEED_MULTIPLIER_WHEN_IDLING: f32 = 1.0;
const SPEED_MULTIPLIER_WHEN_TEMPTED: f32 = 1.25;
const SPEED_MULTIPLIER_WHEN_FOLLOWING_ADULT: f32 = 1.25;
const SPEED_MULTIPLIER_WHEN_MAKING_LOVE: f32 = 1.0;
const DEFAULT_CLOSE_ENOUGH_DIST: f64 = 2.0;
const BABY_CLOSE_ENOUGH_DIST: f64 = 1.0;
/// Vanilla `ArmadilloAi.ADULT_FOLLOW_RANGE`.
const ADULT_FOLLOW_RANGE: (i32, i32) = (5, 16);

fn armadillo<'a>(tick: &'a BrainTick<'_>) -> Option<&'a ArmadilloEntity> {
    tick.mob.cast_any().downcast_ref::<ArmadilloEntity>()
}

/// Vanilla `ArmadilloAi.ARMADILLO_ROLLING_OUT`: once the danger is forgotten, unrolls.
fn armadillo_rolling_out() -> OneShot {
    OneShot::new(
        "ArmadilloRollingOut",
        vec![(
            types::DANGER_DETECTED_RECENTLY.id(),
            MemoryStatus::ValueAbsent,
        )],
        |tick| {
            let Some(body) = armadillo(tick) else {
                return false;
            };
            if body.is_scared() {
                body.roll_out();
                true
            } else {
                false
            }
        },
    )
}

/// Vanilla `ArmadilloAi.initCoreActivity`'s `MoveToTargetSink`, which never starts a
/// path while the armadillo is scared.
struct ArmadilloMoveToTargetSink(MoveToTargetSink);

impl Behavior for ArmadilloMoveToTargetSink {
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
        if armadillo(tick).is_some_and(ArmadilloEntity::is_scared) {
            false
        } else {
            self.0.check_extra_start_conditions(tick)
        }
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        self.0.start(tick);
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        self.0.tick(tick);
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        self.0.stop(tick);
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        self.0.can_still_use(tick)
    }

    fn debug_name(&self) -> &'static str {
        self.0.debug_name()
    }
}

/// Vanilla `ArmadilloAi.ArmadilloPanic`: unrolls before panicking.
struct ArmadilloPanic(AnimalPanic);

impl ArmadilloPanic {
    const fn new(speed_multiplier: f32) -> Self {
        Self(AnimalPanic::with_damage_types(
            speed_multiplier,
            &tag::DamageType::MINECRAFT_PANIC_ENVIRONMENTAL_CAUSES,
        ))
    }
}

impl Behavior for ArmadilloPanic {
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
        if let Some(body) = armadillo(tick) {
            body.roll_out();
        }
        self.0.start(tick);
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        self.0.tick(tick);
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        self.0.stop(tick);
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        self.0.can_still_use(tick)
    }

    fn debug_name(&self) -> &'static str {
        "ArmadilloPanic"
    }
}

/// Vanilla `ArmadilloBallUp.BALL_UP_STAY_IN_STATE`.
const BALL_UP_STAY_IN_STATE: i32 = 5 * 60 * 20;
/// Vanilla `ArmadilloBallUp.DANGER_DETECTED_RECENTLY_DANGER_THRESHOLD`.
const DANGER_DETECTED_RECENTLY_DANGER_THRESHOLD: i64 = 75;

/// Vanilla `ArmadilloAi.ArmadilloBallUp`: rolls up, peeks out now and then while the
/// danger lasts, and starts unrolling as it fades.
struct ArmadilloBallUp {
    next_peek_timer: i32,
    danger_was_around: bool,
}

impl ArmadilloBallUp {
    const fn new() -> Self {
        Self {
            next_peek_timer: 0,
            danger_was_around: false,
        }
    }

    fn pick_next_peek_timer(tick: &BrainTick<'_>) -> i32 {
        ArmadilloState::Scared.animation_duration() + tick.mob.get_random().random_range(100..=400)
    }
}

impl Behavior for ArmadilloBallUp {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &[]
    }

    fn min_duration(&self) -> i32 {
        BALL_UP_STAY_IN_STATE
    }

    fn max_duration(&self) -> i32 {
        BALL_UP_STAY_IN_STATE
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        let entity = tick.mob.get_entity();
        entity.on_ground.load(std::sync::atomic::Ordering::Relaxed) && !entity.is_in_liquid()
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        armadillo(tick).is_some_and(|body| body.get_state().is_threatened())
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(body) = armadillo(tick) {
            body.roll_up();
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(body) = armadillo(tick)
            && !body.can_stay_rolled_up_with(tick.brain)
        {
            body.roll_out();
        }
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let Some(body) = armadillo(tick) else {
            return;
        };
        if self.next_peek_timer > 0 {
            self.next_peek_timer -= 1;
        }

        let on_ground = tick
            .mob
            .get_entity()
            .on_ground
            .load(std::sync::atomic::Ordering::Relaxed);
        if body.should_switch_to_scared_state() {
            body.switch_to_state(ArmadilloState::Scared);
            if on_ground {
                body.play_sound_at_self(Sound::EntityArmadilloLand, 1.0, 1.0);
            }
            return;
        }

        let state = body.get_state();
        // An absent memory reads as never expiring, like vanilla's empty slot.
        let danger_tick_counter = tick
            .brain
            .time_until_expiry(types::DANGER_DETECTED_RECENTLY.id())
            .unwrap_or(NEVER_EXPIRE);
        let danger_is_around = danger_tick_counter > DANGER_DETECTED_RECENTLY_DANGER_THRESHOLD;
        if danger_is_around != self.danger_was_around {
            self.next_peek_timer = Self::pick_next_peek_timer(tick);
        }
        self.danger_was_around = danger_is_around;

        let unrolling_duration = i64::from(ArmadilloState::Unrolling.animation_duration());
        if state == ArmadilloState::Scared {
            if self.next_peek_timer == 0 && on_ground && danger_is_around {
                tick.world.send_entity_status(
                    body.get_entity(),
                    pumpkin_data::entity::EntityStatus::ArmadilloPeek,
                    None,
                );
                self.next_peek_timer = Self::pick_next_peek_timer(tick);
            }
            if danger_tick_counter < unrolling_duration {
                body.play_sound_at_self(Sound::EntityArmadilloUnrollStart, 1.0, 1.0);
                body.switch_to_state(ArmadilloState::Unrolling);
            }
        } else if state == ArmadilloState::Unrolling && danger_tick_counter > unrolling_duration {
            body.switch_to_state(ArmadilloState::Scared);
        }
    }

    fn debug_name(&self) -> &'static str {
        "ArmadilloBallUp"
    }
}

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(Swim::new(0.8))),
            Box::new(Timed::new(ArmadilloPanic::new(
                SPEED_MULTIPLIER_WHEN_PANICKING,
            ))),
            Box::new(Timed::new(LookAtTargetSink::new(45, 90))),
            Box::new(Timed::new(ArmadilloMoveToTargetSink(
                MoveToTargetSink::default(),
            ))),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::TEMPTATION_COOLDOWN_TICKS,
            ))),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::GAZE_COOLDOWN_TICKS,
            ))),
            Box::new(armadillo_rolling_out()),
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
                    &EntityType::ARMADILLO,
                    SPEED_MULTIPLIER_WHEN_MAKING_LOVE,
                    1,
                ))),
            ),
            (
                2,
                Box::new(behavior::run_one(vec![
                    (
                        Box::new(Timed::new(
                            FollowTemptation::new(|_| SPEED_MULTIPLIER_WHEN_TEMPTED)
                                .with_close_enough_distance_fn(|tick| {
                                    if tick.mob.get_entity().is_baby() {
                                        BABY_CLOSE_ENOUGH_DIST
                                    } else {
                                        DEFAULT_CLOSE_ENOUGH_DIST
                                    }
                                }),
                        )),
                        1,
                    ),
                    (
                        Box::new(behavior::baby_follow_adult(
                            ADULT_FOLLOW_RANGE.0,
                            ADULT_FOLLOW_RANGE.1,
                            |_| SPEED_MULTIPLIER_WHEN_FOLLOWING_ADULT,
                            false,
                        )),
                        1,
                    ),
                ])),
            ),
            (
                3,
                Box::new(Timed::new(RandomLookAround::new(
                    (150, 250),
                    30.0,
                    0.0,
                    0.0,
                ))),
            ),
            (
                4,
                Box::new(behavior::run_one_with_conditions(
                    vec![(types::WALK_TARGET.id(), MemoryStatus::ValueAbsent)],
                    vec![
                        (
                            Box::new(behavior::stroll(SPEED_MULTIPLIER_WHEN_IDLING, true)),
                            1,
                        ),
                        (
                            Box::new(behavior::set_walk_target_from_look_target(
                                |_| true,
                                SPEED_MULTIPLIER_WHEN_IDLING,
                                3,
                            )),
                            1,
                        ),
                        (Box::new(DoNothing::new(30, 60)), 1),
                    ],
                )),
            ),
        ],
    )
}

fn init_scared_activity() -> ActivityData {
    ActivityData::with_pairs_and_conditions(
        Activity::Panic,
        vec![(0, Box::new(Timed::new(ArmadilloBallUp::new())))],
        vec![
            (
                types::DANGER_DETECTED_RECENTLY.id(),
                MemoryStatus::ValuePresent,
            ),
            (types::IS_PANICKING.id(), MemoryStatus::ValueAbsent),
        ],
    )
}

/// Vanilla `ArmadilloAi.updateActivity`.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    tick.brain
        .set_active_activity_to_first_valid(&[Activity::Panic, Activity::Idle]);
}

/// Vanilla `Armadillo.BRAIN_PROVIDER`'s sensors.
const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::HurtBy,
    SensorType::FoodTemptations,
    SensorType::NearestAdult,
    SensorType::ArmadilloScareDetected,
];

pub static ARMADILLO_PROVIDER: BrainProvider = BrainProvider {
    memory_types: &[],
    sensor_types: SENSOR_TYPES,
    activities: |_| {
        vec![
            init_core_activity(),
            init_idle_activity(),
            init_scared_activity(),
        ]
    },
};
