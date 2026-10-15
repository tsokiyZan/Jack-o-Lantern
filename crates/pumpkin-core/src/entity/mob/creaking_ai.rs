//! Vanilla `CreakingAi`.

use std::sync::Arc;

use pumpkin_data::environment_attribute::Activity;

use crate::entity::EntityBase;
use crate::entity::ai::brain::activity::ActivityData;
use crate::entity::ai::brain::behavior::timed::Behavior;
use crate::entity::ai::brain::behavior::{
    self, BehaviorControl, DoNothing, LookAtTargetSink, MoveToTargetSink, Swim, Timed,
};
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, types};
use crate::entity::ai::brain::sensing::SensorType;
use crate::entity::ai::brain::{BrainProvider, BrainTick};

use super::creaking::{ATTACK_INTERVAL, CreakingEntity, SPEED_MULTIPLIER_WHEN_IDLING};

/// Vanilla `SetEntityLookTargetSometimes.create(8.0F, UniformInt.of(30, 60))`.
const LOOK_DISTANCE: f32 = 8.0;
const LOOK_INTERVAL: (i32, i32) = (30, 60);

fn creaking<'a>(tick: &'a BrainTick<'_>) -> Option<&'a CreakingEntity> {
    tick.mob.cast_any().downcast_ref::<CreakingEntity>()
}

fn can_move(tick: &BrainTick<'_>) -> bool {
    creaking(tick).is_some_and(CreakingEntity::can_move)
}

/// `initCoreActivity`'s `Swim` subclass: no swimming while frozen.
struct CreakingSwim(Swim);

impl Behavior for CreakingSwim {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        self.0.entry_conditions()
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        can_move(tick) && self.0.check_extra_start_conditions(tick)
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        self.0.can_still_use(tick)
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        self.0.tick(tick);
    }

    fn debug_name(&self) -> &'static str {
        self.0.debug_name()
    }
}

fn init_core_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Core,
        0,
        vec![
            Box::new(Timed::new(CreakingSwim(Swim::new(0.8)))),
            Box::new(Timed::new(LookAtTargetSink::new(45, 90))),
            Box::new(Timed::new(MoveToTargetSink::default())),
        ],
    )
}

fn init_idle_activity() -> ActivityData {
    ActivityData::with_priority_start(
        Activity::Idle,
        10,
        vec![
            Box::new(behavior::start_attacking(
                |tick| creaking(tick).is_some_and(CreakingEntity::is_active),
                |tick| {
                    tick.brain
                        .get(types::NEAREST_VISIBLE_ATTACKABLE_PLAYER)
                        .map(|player| Arc::clone(player) as Arc<dyn EntityBase>)
                },
            )),
            Box::new(behavior::set_entity_look_target_sometimes(
                |_| true,
                LOOK_DISTANCE,
                LOOK_INTERVAL.0,
                LOOK_INTERVAL.1,
            )),
            Box::new(behavior::run_one(vec![
                (
                    Box::new(behavior::stroll(SPEED_MULTIPLIER_WHEN_IDLING, true))
                        as Box<dyn BehaviorControl>,
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

fn init_fight_activity() -> ActivityData {
    ActivityData::with_priority_start_and_conditions(
        Activity::Fight,
        10,
        vec![
            Box::new(behavior::set_walk_target_from_attack_target_if_target_out_of_reach(1.0)),
            Box::new(behavior::melee_attack(can_move, ATTACK_INTERVAL)),
            Box::new(behavior::stop_attacking_if_target_invalid(
                |tick, target| !is_attack_target_still_reachable(tick, target),
                |_, _| {},
                true,
            )),
        ],
        vec![(types::ATTACK_TARGET.id(), MemoryStatus::ValuePresent)],
    )
}

/// Vanilla `CreakingAi.isAttackTargetStillReachable`.
fn is_attack_target_still_reachable(tick: &BrainTick<'_>, target: &Arc<dyn EntityBase>) -> bool {
    let target_id = target.get_entity().entity_id;
    tick.brain
        .get(types::NEAREST_VISIBLE_ATTACKABLE_PLAYERS)
        .is_some_and(|players| {
            players
                .iter()
                .any(|player| player.get_entity().entity_id == target_id)
        })
}

/// Vanilla `CreakingAi.updateActivity`.
pub fn update_activity(tick: &mut BrainTick<'_>) {
    if can_move(tick) {
        tick.brain
            .set_active_activity_to_first_valid(&[Activity::Fight, Activity::Idle]);
    } else {
        tick.brain.use_default_activity();
    }
}

/// Vanilla `Creaking.BRAIN_PROVIDER`'s sensors.
const SENSOR_TYPES: &[SensorType] = &[
    SensorType::NearestLivingEntities,
    SensorType::NearestPlayers,
];

pub static CREAKING_PROVIDER: BrainProvider = BrainProvider {
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
