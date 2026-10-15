use std::sync::Arc;

use pumpkin_util::math::vector3::Vector3;

use super::super::BrainTick;
use super::super::memory::position_tracker::PositionTracker;
use super::super::memory::{MemoryModuleId, MemoryModuleType, MemoryStatus, types};
use super::utils::set_walk_and_look_target_memories;
use super::{BehaviorControl, Status, required_memories_of};

/// Vanilla `GoAndGiveItemsToTarget.CLOSE_ENOUGH_DISTANCE_TO_TARGET`.
const CLOSE_ENOUGH_DISTANCE_TO_TARGET: i32 = 3;

pub type TargetPositionGetter = fn(&mut BrainTick<'_>) -> Option<Arc<dyn PositionTracker>>;
pub type ItemThrower = fn(&mut BrainTick<'_>, Vector3<f64>);

/// Vanilla `GoAndGiveItemsToTarget`: flies to a target while holding items and throws
/// them at it once close.
///
/// It drives `BehaviorControl` itself rather than going through `Timed`, because the
/// target getter may erase memories and vanilla calls it from `canStillUse` too.
pub struct GoAndGiveItemsToTarget {
    target_position_getter: TargetPositionGetter,
    speed_modifier: f32,
    timeout_duration: i32,
    item_thrower: ItemThrower,
    cooldown_memory: MemoryModuleType<i32>,
    cooldown_duration: i32,
    has_item_predicate: fn(&BrainTick<'_>) -> bool,
    conditions: [(MemoryModuleId, MemoryStatus); 3],
    required: Box<[MemoryModuleId]>,
    status: Status,
    end_timestamp: i64,
}

impl GoAndGiveItemsToTarget {
    #[must_use]
    pub fn new(
        target_position_getter: TargetPositionGetter,
        speed_modifier: f32,
        timeout_duration: i32,
        item_thrower: ItemThrower,
        cooldown_memory: MemoryModuleType<i32>,
        cooldown_duration: i32,
        has_item_predicate: fn(&BrainTick<'_>) -> bool,
    ) -> Self {
        let conditions = [
            (types::LOOK_TARGET.id(), MemoryStatus::Registered),
            (types::WALK_TARGET.id(), MemoryStatus::Registered),
            (cooldown_memory.id(), MemoryStatus::Registered),
        ];
        Self {
            target_position_getter,
            speed_modifier,
            timeout_duration,
            item_thrower,
            cooldown_memory,
            cooldown_duration,
            has_item_predicate,
            required: required_memories_of(&conditions),
            conditions,
            status: Status::Stopped,
            end_timestamp: 0,
        }
    }

    fn can_throw_item_to_target(&self, tick: &mut BrainTick<'_>) -> bool {
        (self.has_item_predicate)(tick) && (self.target_position_getter)(tick).is_some()
    }

    fn start(&self, tick: &mut BrainTick<'_>) {
        if let Some(position_tracker) = (self.target_position_getter)(tick) {
            set_walk_and_look_target_memories(
                tick.brain,
                position_tracker,
                self.speed_modifier,
                CLOSE_ENOUGH_DISTANCE_TO_TARGET,
            );
        }
    }

    fn tick(&self, tick: &mut BrainTick<'_>) {
        let Some(deposit_target) = (self.target_position_getter)(tick) else {
            return;
        };
        let deposit_position = deposit_target.current_position();
        let distance_to_target = deposit_position
            .squared_distance_to_vec(&tick.mob.get_entity().get_eye_pos())
            .sqrt();
        if distance_to_target < f64::from(CLOSE_ENOUGH_DISTANCE_TO_TARGET) {
            (self.item_thrower)(tick, deposit_target.current_position());
            tick.brain.set(self.cooldown_memory, self.cooldown_duration);
        }
    }
}

impl BehaviorControl for GoAndGiveItemsToTarget {
    fn status(&self) -> Status {
        self.status
    }

    fn required_memories(&self) -> &[MemoryModuleId] {
        &self.required
    }

    /// Vanilla `Behavior.tryStart`, with `timeoutDuration` as both the minimum and maximum.
    fn try_start(&mut self, tick: &mut BrainTick<'_>) -> bool {
        if !tick.brain.check_all(&self.conditions) || !self.can_throw_item_to_target(tick) {
            return false;
        }
        self.status = Status::Running;
        self.end_timestamp = tick.time + i64::from(self.timeout_duration);
        self.start(tick);
        true
    }

    /// Vanilla `Behavior.tickOrStop`.
    fn tick_or_stop(&mut self, tick: &mut BrainTick<'_>) {
        if tick.time <= self.end_timestamp && self.can_throw_item_to_target(tick) {
            self.tick(tick);
        } else {
            self.do_stop(tick);
        }
    }

    fn do_stop(&mut self, _tick: &mut BrainTick<'_>) {
        self.status = Status::Stopped;
    }

    fn debug_string(&self) -> String {
        "GoAndGiveItemsToTarget".to_owned()
    }
}
