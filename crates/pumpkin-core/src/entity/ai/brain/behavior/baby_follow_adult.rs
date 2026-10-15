use std::sync::Arc;

use super::super::BrainTick;
use super::super::memory::position_tracker::{EntityTracker, PositionTracker};
use super::super::memory::walk_target::WalkTarget;
use super::super::memory::{MemoryModuleType, MemoryStatus, MemoryValue, types};
use super::one_shot::OneShot;
use crate::entity::EntityBase;

/// Vanilla `BabyFollowAdult.create`: a baby walks to the nearest visible adult while
/// it is between `min_range` and `max_range + 1` blocks away.
#[must_use]
pub fn baby_follow_adult(
    min_range: i32,
    max_range: i32,
    speed_modifier: impl Fn(&BrainTick<'_>) -> f32 + Send + Sync + 'static,
    target_eye: bool,
) -> OneShot {
    baby_follow_nearest(
        min_range,
        max_range,
        speed_modifier,
        types::NEAREST_VISIBLE_ADULT,
        Arc::clone,
        target_eye,
    )
}

/// Vanilla `BabyFollowAdult.create` with its `nearestVisibleType`: follows whatever that
/// memory holds, which `as_entity` turns into an entity.
#[must_use]
pub fn baby_follow_nearest<T: MemoryValue>(
    min_range: i32,
    max_range: i32,
    speed_modifier: impl Fn(&BrainTick<'_>) -> f32 + Send + Sync + 'static,
    nearest_visible: MemoryModuleType<T>,
    as_entity: fn(&T) -> Arc<dyn EntityBase>,
    target_eye: bool,
) -> OneShot {
    OneShot::with_required(
        "BabyFollowAdult",
        vec![
            (nearest_visible.id(), MemoryStatus::ValuePresent),
            (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
        ],
        vec![types::LOOK_TARGET.id()],
        move |tick| {
            if !tick.mob.get_entity().is_baby() {
                return false;
            }
            let Some(adult) = tick.brain.get(nearest_visible).map(as_entity) else {
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
                Arc::new(EntityTracker::with_target_eye_height(
                    Arc::clone(&adult),
                    true,
                    target_eye,
                )) as Arc<dyn PositionTracker>,
            );
            tick.brain.set(
                types::WALK_TARGET,
                WalkTarget::new(
                    Arc::new(EntityTracker::with_target_eye_height(
                        adult, target_eye, target_eye,
                    )),
                    speed_modifier,
                    min_range - 1,
                ),
            );
            true
        },
    )
}
