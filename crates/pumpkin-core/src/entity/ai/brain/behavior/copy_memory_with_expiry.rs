use pumpkin_util::random::RandomImpl;

use super::super::BrainTick;
use super::super::memory::{MemoryModuleType, MemoryStatus, value::MemoryValue};
use super::one_shot::OneShot;

#[must_use]
pub fn copy_memory_with_expiry<T: MemoryValue + Clone>(
    name: &'static str,
    mut copy_if: impl FnMut(&BrainTick<'_>) -> bool + Send + Sync + 'static,
    source: MemoryModuleType<T>,
    target: MemoryModuleType<T>,
    min_duration: i32,
    max_duration: i32,
) -> OneShot {
    OneShot::new(
        name,
        vec![
            (source.id(), MemoryStatus::ValuePresent),
            (target.id(), MemoryStatus::ValueAbsent),
        ],
        move |tick| {
            if !copy_if(tick) {
                return false;
            }
            let Some(value) = tick.brain.get(source).cloned() else {
                return false;
            };
            // `durationOfCopy.sample(level.getRandom())` (CopyMemoryWithExpiry.java:18), UniformInt.
            let duration = tick
                .world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .next_int_between_inclusive(min_duration, max_duration);
            tick.brain
                .set_with_expiry(target, value, i64::from(duration));
            true
        },
    )
}
