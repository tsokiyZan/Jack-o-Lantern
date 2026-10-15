use std::sync::Arc;

use super::super::BrainTick;
use super::super::memory::walk_target::WalkTarget;
use super::super::memory::{MemoryStatus, types};
use super::go_and_give_items_to_target::TargetPositionGetter;
use super::one_shot::OneShot;

/// Vanilla `StayCloseToTarget`: walks back towards a target once it is `too_far` away.
#[must_use]
pub fn stay_close_to_target(
    target_position_getter: TargetPositionGetter,
    should_run_predicate: fn(&BrainTick<'_>) -> bool,
    close_enough: i32,
    too_far: i32,
    speed_modifier: f32,
) -> OneShot {
    OneShot::new(
        "StayCloseToTarget",
        vec![
            (types::LOOK_TARGET.id(), MemoryStatus::Registered),
            (types::WALK_TARGET.id(), MemoryStatus::Registered),
        ],
        move |tick| {
            let Some(target) = target_position_getter(tick) else {
                return false;
            };
            if !should_run_predicate(tick) {
                return false;
            }
            let too_far = f64::from(too_far);
            if tick
                .mob
                .get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&target.current_position())
                < too_far * too_far
            {
                return false;
            }
            tick.brain.set(types::LOOK_TARGET, Arc::clone(&target));
            tick.brain.set(
                types::WALK_TARGET,
                WalkTarget::new(target, speed_modifier, close_enough),
            );
            true
        },
    )
}
