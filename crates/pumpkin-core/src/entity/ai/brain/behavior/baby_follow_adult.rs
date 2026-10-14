use std::sync::Arc;

use super::super::BrainTick;
use super::super::memory::position_tracker::{EntityTracker, PositionTracker};
use super::super::memory::walk_target::WalkTarget;
use super::super::memory::{MemoryStatus, types};
use super::one_shot::OneShot;

/// Vanilla `BabyFollowAdult.create`: a baby walks to the nearest visible adult while
/// it is between `min_range` and `max_range + 1` blocks away.
#[must_use]
pub fn baby_follow_adult(
    min_range: i32,
    max_range: i32,
    speed_modifier: impl Fn(&BrainTick<'_>) -> f32 + Send + Sync + 'static,
    target_eye: bool,
) -> OneShot {
    OneShot::with_required(
        "BabyFollowAdult",
        vec![
            (
                types::NEAREST_VISIBLE_ADULT.id(),
                MemoryStatus::ValuePresent,
            ),
            (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
        ],
        vec![types::LOOK_TARGET.id()],
        move |tick| {
            if !tick.mob.get_entity().is_baby() {
                return false;
            }
            let Some(adult) = tick.brain.get(types::NEAREST_VISIBLE_ADULT).map(Arc::clone) else {
                return false;
            };
            let distance_sq = tick
                .mob
                .get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&adult.get_entity().pos.load());
            let closer_than = |range: i32| distance_sq < f64::from(range) * f64::from(range);
            if !closer_than(max_range + 1) || closer_than(min_range) {
                return false;
            }
            let speed_modifier = speed_modifier(tick);
            tick.brain.set(
                types::LOOK_TARGET,
                Arc::new(EntityTracker::new(Arc::clone(&adult), true)) as Arc<dyn PositionTracker>,
            );
            tick.brain.set(
                types::WALK_TARGET,
                WalkTarget::new(
                    Arc::new(EntityTracker::new(adult, target_eye)),
                    speed_modifier,
                    min_range - 1,
                ),
            );
            true
        },
    )
}
