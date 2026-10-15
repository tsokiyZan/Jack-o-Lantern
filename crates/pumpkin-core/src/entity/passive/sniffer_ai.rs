//! Vanilla `SnifferAi`, with its `Digging`, `FeelingHappy`, `FinishedDigging`, `Scenting`,
//! `Searching` and `Sniffing` behaviours.

use pumpkin_data::entity::EntityType;
use pumpkin_data::environment_attribute::Activity;

use crate::entity::EntityBase;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::{
    self, AnimalMakeLove, AnimalPanic, Behavior, CountDownCooldownTicks, DoNothing,
    FollowTemptation, LookAtTargetSink, MoveToTargetSink, Swim, Timed,
};
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, WalkTarget, types};
use crate::entity::ai::brain::sensing::SensorType;
use crate::entity::ai::brain::{BrainProvider, BrainTick};

use super::sniffer::{SnifferEntity, SnifferState};

/// Vanilla `SnifferAi.MAX_LOOK_DISTANCE`.
const MAX_LOOK_DISTANCE: f32 = 6.0;
/// Vanilla `SnifferAi.SNIFFING_COOLDOWN_TICKS`.
const SNIFFING_COOLDOWN_TICKS: i64 = 9600;
/// Vanilla `SnifferAi.SPEED_MULTIPLIER_WHEN_IDLING`.
const SPEED_MULTIPLIER_WHEN_IDLING: f32 = 1.0;
/// Vanilla `SnifferAi.SPEED_MULTIPLIER_WHEN_PANICKING`.
const SPEED_MULTIPLIER_WHEN_PANICKING: f32 = 2.0;
/// Vanilla `SnifferAi.SPEED_MULTIPLIER_WHEN_SNIFFING`.
const SPEED_MULTIPLIER_WHEN_SNIFFING: f32 = 1.25;
/// Vanilla `SnifferAi.SPEED_MULTIPLIER_WHEN_TEMPTED`.
const SPEED_MULTIPLIER_WHEN_TEMPTED: f32 = 1.25;
/// The close-enough distances `initIdleActivity` gives `FollowTemptation`.
const BABY_TEMPTED_CLOSE_ENOUGH_DIST: f64 = 2.5;
const ADULT_TEMPTED_CLOSE_ENOUGH_DIST: f64 = 3.5;
/// The durations `getActivities` gives the sniffer behaviours.
const DIGGING_DURATION: (i32, i32) = (160, 180);
const FINISHED_DIGGING_DURATION: i32 = 40;
const FEELING_HAPPY_DURATION: (i32, i32) = (40, 100);
const SCENTING_DURATION: (i32, i32) = (40, 80);
const SNIFFING_DURATION: (i32, i32) = (40, 80);
/// Vanilla `Searching`'s timeout.
const SEARCHING_DURATION: i32 = 600;

/// The sniffer whose brain is ticking, for as long as the mob reference lives.
fn sniffer<'m>(tick: &BrainTick<'m>) -> Option<&'m SnifferEntity> {
    tick.mob.cast_any().downcast_ref::<SnifferEntity>()
}

/// Vanilla `SnifferAi.resetSniffing`.
fn reset_sniffing(tick: &mut BrainTick<'_>) {
    tick.brain.erase(types::SNIFFER_DIGGING.id());
    tick.brain.erase(types::SNIFFER_SNIFFING_TARGET.id());
    if let Some(body) = sniffer(tick) {
        body.transition_to(SnifferState::Idling);
    }
}

/// Vanilla `SnifferAi`'s anonymous `AnimalPanic`, `AnimalMakeLove` and `FollowTemptation`
/// subclasses, which reset sniffing as they start.
struct ResetSniffingOnStart<B>(B);

impl<B: Behavior> Behavior for ResetSniffingOnStart<B> {
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
        reset_sniffing(tick);
        self.0.start(tick);
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        self.0.tick(tick);
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        self.0.stop(tick);
    }

    fn stop_with_timeout(&mut self, tick: &mut BrainTick<'_>, timed_out: bool) {
        self.0.stop_with_timeout(tick, timed_out);
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        self.0.can_still_use(tick)
    }

    fn times_out(&self) -> bool {
        self.0.times_out()
    }

    fn debug_name(&self) -> &'static str {
        self.0.debug_name()
    }
}

/// Vanilla `SnifferAi.Digging`.
struct Digging;

const DIGGING_CONDITIONS: [(MemoryModuleId, MemoryStatus); 4] = [
    (types::IS_PANICKING.id(), MemoryStatus::ValueAbsent),
    (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
    (types::SNIFFER_DIGGING.id(), MemoryStatus::ValuePresent),
    (types::SNIFF_COOLDOWN.id(), MemoryStatus::ValueAbsent),
];

impl Behavior for Digging {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &DIGGING_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        DIGGING_DURATION.0
    }

    fn max_duration(&self) -> i32 {
        DIGGING_DURATION.1
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        sniffer(tick).is_some_and(|body| body.can_sniff(tick.brain))
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        tick.brain.has_memory_value(types::SNIFFER_DIGGING.id())
            && sniffer(tick)
                .is_some_and(|body| body.can_dig(tick.brain) && !body.mob_entity.is_in_love())
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(body) = sniffer(tick) {
            body.transition_to(SnifferState::Digging);
        }
    }

    fn stop_with_timeout(&mut self, tick: &mut BrainTick<'_>, finished: bool) {
        if finished {
            tick.brain
                .set_with_expiry(types::SNIFF_COOLDOWN, (), SNIFFING_COOLDOWN_TICKS);
        } else {
            reset_sniffing(tick);
        }
    }

    fn debug_name(&self) -> &'static str {
        "Digging"
    }
}

/// Vanilla `SnifferAi.FeelingHappy`.
struct FeelingHappy;

const FEELING_HAPPY_CONDITIONS: [(MemoryModuleId, MemoryStatus); 1] =
    [(types::SNIFFER_HAPPY.id(), MemoryStatus::ValuePresent)];

impl Behavior for FeelingHappy {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &FEELING_HAPPY_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        FEELING_HAPPY_DURATION.0
    }

    fn max_duration(&self) -> i32 {
        FEELING_HAPPY_DURATION.1
    }

    fn can_still_use(&mut self, _tick: &BrainTick<'_>) -> bool {
        true
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(body) = sniffer(tick) {
            body.transition_to(SnifferState::FeelingHappy);
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(body) = sniffer(tick) {
            body.transition_to(SnifferState::Idling);
        }
        tick.brain.erase(types::SNIFFER_HAPPY.id());
    }

    fn debug_name(&self) -> &'static str {
        "FeelingHappy"
    }
}

/// Vanilla `SnifferAi.FinishedDigging`.
struct FinishedDigging;

const FINISHED_DIGGING_CONDITIONS: [(MemoryModuleId, MemoryStatus); 4] = [
    (types::IS_PANICKING.id(), MemoryStatus::ValueAbsent),
    (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
    (types::SNIFFER_DIGGING.id(), MemoryStatus::ValuePresent),
    (types::SNIFF_COOLDOWN.id(), MemoryStatus::ValuePresent),
];

impl Behavior for FinishedDigging {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &FINISHED_DIGGING_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        FINISHED_DIGGING_DURATION
    }

    fn max_duration(&self) -> i32 {
        FINISHED_DIGGING_DURATION
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        tick.brain.has_memory_value(types::SNIFFER_DIGGING.id())
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(body) = sniffer(tick) {
            body.transition_to(SnifferState::Rising);
        }
    }

    fn stop_with_timeout(&mut self, tick: &mut BrainTick<'_>, finished: bool) {
        if let Some(body) = sniffer(tick) {
            body.transition_to(SnifferState::Idling)
                .on_digging_complete(tick.brain, finished);
        }
        tick.brain.erase(types::SNIFFER_DIGGING.id());
        tick.brain.set(types::SNIFFER_HAPPY, true);
    }

    fn debug_name(&self) -> &'static str {
        "FinishedDigging"
    }
}

/// Vanilla `SnifferAi.Scenting`.
struct Scenting;

const SCENTING_CONDITIONS: [(MemoryModuleId, MemoryStatus); 5] = [
    (types::IS_PANICKING.id(), MemoryStatus::ValueAbsent),
    (types::SNIFFER_DIGGING.id(), MemoryStatus::ValueAbsent),
    (
        types::SNIFFER_SNIFFING_TARGET.id(),
        MemoryStatus::ValueAbsent,
    ),
    (types::SNIFFER_HAPPY.id(), MemoryStatus::ValueAbsent),
    (types::BREED_TARGET.id(), MemoryStatus::ValueAbsent),
];

impl Behavior for Scenting {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &SCENTING_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        SCENTING_DURATION.0
    }

    fn max_duration(&self) -> i32 {
        SCENTING_DURATION.1
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        !SnifferEntity::is_tempted(tick.brain)
    }

    fn can_still_use(&mut self, _tick: &BrainTick<'_>) -> bool {
        true
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(body) = sniffer(tick) {
            body.transition_to(SnifferState::Scenting);
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(body) = sniffer(tick) {
            body.transition_to(SnifferState::Idling);
        }
    }

    fn debug_name(&self) -> &'static str {
        "Scenting"
    }
}

/// Vanilla `SnifferAi.Searching`.
struct Searching;

const SEARCHING_CONDITIONS: [(MemoryModuleId, MemoryStatus); 3] = [
    (types::WALK_TARGET.id(), MemoryStatus::ValuePresent),
    (types::IS_PANICKING.id(), MemoryStatus::ValueAbsent),
    (
        types::SNIFFER_SNIFFING_TARGET.id(),
        MemoryStatus::ValuePresent,
    ),
];

impl Behavior for Searching {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &SEARCHING_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        SEARCHING_DURATION
    }

    fn max_duration(&self) -> i32 {
        SEARCHING_DURATION
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        sniffer(tick).is_some_and(|body| body.can_sniff(tick.brain))
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        let Some(body) = sniffer(tick) else {
            return false;
        };
        if !body.can_sniff(tick.brain) {
            body.transition_to(SnifferState::Idling);
            return false;
        }
        let walk_target = tick
            .brain
            .get(types::WALK_TARGET)
            .map(|walk_target| walk_target.target().current_block_position());
        let sniffing_target = tick.brain.get(types::SNIFFER_SNIFFING_TARGET);
        walk_target.is_some_and(|walk_target| sniffing_target == Some(&walk_target))
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(body) = sniffer(tick) {
            body.transition_to(SnifferState::Searching);
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        if sniffer(tick).is_some_and(|body| body.can_dig(tick.brain) && body.can_sniff(tick.brain))
        {
            tick.brain.set(types::SNIFFER_DIGGING, true);
        }
        tick.brain.erase(types::WALK_TARGET.id());
        tick.brain.erase(types::SNIFFER_SNIFFING_TARGET.id());
    }

    fn debug_name(&self) -> &'static str {
        "Searching"
    }
}

/// Vanilla `SnifferAi.Sniffing`.
struct Sniffing;

const SNIFFING_CONDITIONS: [(MemoryModuleId, MemoryStatus); 3] = [
    (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
    (
        types::SNIFFER_SNIFFING_TARGET.id(),
        MemoryStatus::ValueAbsent,
    ),
    (types::SNIFF_COOLDOWN.id(), MemoryStatus::ValueAbsent),
];

impl Behavior for Sniffing {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &SNIFFING_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        SNIFFING_DURATION.0
    }

    fn max_duration(&self) -> i32 {
        SNIFFING_DURATION.1
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        sniffer(tick).is_some_and(|body| !body.get_entity().is_baby() && body.can_sniff(tick.brain))
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        sniffer(tick).is_some_and(|body| body.can_sniff(tick.brain))
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(body) = sniffer(tick) {
            body.transition_to(SnifferState::Sniffing);
        }
    }

    fn stop_with_timeout(&mut self, tick: &mut BrainTick<'_>, finished: bool) {
        let Some(body) = sniffer(tick) else {
            return;
        };
        body.transition_to(SnifferState::Idling);
        if finished && let Some(position) = body.calculate_dig_position(tick.brain) {
            tick.brain.set(types::SNIFFER_SNIFFING_TARGET, position);
            tick.brain.set(
                types::WALK_TARGET,
                WalkTarget::from_block_pos(position, SPEED_MULTIPLIER_WHEN_SNIFFING, 0),
            );
        }
    }

    fn debug_name(&self) -> &'static str {
        "Sniffing"
    }
}

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(Swim::new(0.8))),
            Box::new(Timed::new(ResetSniffingOnStart(AnimalPanic::new(
                SPEED_MULTIPLIER_WHEN_PANICKING,
            )))),
            Box::new(Timed::new(MoveToTargetSink::new(500, 700))),
            Box::new(Timed::new(CountDownCooldownTicks::new(
                types::TEMPTATION_COOLDOWN_TICKS,
            ))),
        ],
    )
}

fn init_sniffing_activity() -> ActivityData {
    ActivityData::with_pairs_and_conditions(
        Activity::Sniff,
        vec![(0, Box::new(Timed::new(Searching)))],
        vec![
            (types::IS_PANICKING.id(), MemoryStatus::ValueAbsent),
            (
                types::SNIFFER_SNIFFING_TARGET.id(),
                MemoryStatus::ValuePresent,
            ),
            (types::WALK_TARGET.id(), MemoryStatus::ValuePresent),
        ],
    )
}

fn init_dig_activity() -> ActivityData {
    ActivityData::with_pairs_and_conditions(
        Activity::Dig,
        vec![
            (0, Box::new(Timed::new(Digging))),
            (0, Box::new(Timed::new(FinishedDigging))),
        ],
        vec![
            (types::IS_PANICKING.id(), MemoryStatus::ValueAbsent),
            (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
            (types::SNIFFER_DIGGING.id(), MemoryStatus::ValuePresent),
        ],
    )
}

fn init_idle_activity() -> ActivityData {
    ActivityData::with_pairs_and_conditions(
        Activity::Idle,
        vec![
            (
                0,
                Box::new(Timed::new(ResetSniffingOnStart(AnimalMakeLove::new(
                    &EntityType::SNIFFER,
                    1.0,
                    2,
                )))),
            ),
            (
                1,
                Box::new(Timed::new(ResetSniffingOnStart(
                    FollowTemptation::new(|_| SPEED_MULTIPLIER_WHEN_TEMPTED)
                        .with_close_enough_distance_fn(|tick| {
                            if tick.mob.get_entity().is_baby() {
                                BABY_TEMPTED_CLOSE_ENOUGH_DIST
                            } else {
                                ADULT_TEMPTED_CLOSE_ENOUGH_DIST
                            }
                        }),
                ))),
            ),
            (2, Box::new(Timed::new(LookAtTargetSink::new(45, 90)))),
            (3, Box::new(Timed::new(FeelingHappy))),
            (
                4,
                Box::new(behavior::run_one(vec![
                    (
                        Box::new(behavior::set_walk_target_from_look_target(
                            |_| true,
                            SPEED_MULTIPLIER_WHEN_IDLING,
                            3,
                        )),
                        2,
                    ),
                    (Box::new(Timed::new(Scenting)), 1),
                    (Box::new(Timed::new(Sniffing)), 1),
                    (
                        Box::new(behavior::set_entity_look_target_of_type(
                            &EntityType::PLAYER,
                            MAX_LOOK_DISTANCE,
                        )),
                        1,
                    ),
                    (
                        Box::new(behavior::stroll(SPEED_MULTIPLIER_WHEN_IDLING, true)),
                        1,
                    ),
                    (Box::new(DoNothing::new(5, 20)), 2),
                ])),
            ),
        ],
        vec![(types::SNIFFER_DIGGING.id(), MemoryStatus::ValueAbsent)],
    )
}

/// Vanilla `SnifferAi.updateActivity`.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    tick.brain.set_active_activity_to_first_valid(&[
        Activity::Dig,
        Activity::Sniff,
        Activity::Idle,
    ]);
}

/// Vanilla `Sniffer.BRAIN_PROVIDER`'s sensors.
const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::HurtBy,
    SensorType::NearestPlayers,
    SensorType::FoodTemptations,
];

/// Vanilla `Sniffer.BRAIN_PROVIDER`, with `SnifferAi.getActivities`.
pub static SNIFFER_PROVIDER: BrainProvider = BrainProvider {
    memory_types: &[types::SNIFFER_EXPLORED_POSITIONS.id()],
    sensor_types: SENSOR_TYPES,
    activities: |_| {
        vec![
            init_core_activity(),
            init_idle_activity(),
            init_sniffing_activity(),
            init_dig_activity(),
        ]
    },
};
