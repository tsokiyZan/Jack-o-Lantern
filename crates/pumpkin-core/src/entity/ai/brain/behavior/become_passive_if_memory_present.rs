use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::one_shot::OneShot;

/// Vanilla `BecomePassiveIfMemoryPresent.create`: while `pacifying_memory` holds a value,
/// drops the attack target and stays pacified for `pacify_duration` ticks.
#[must_use]
pub fn become_passive_if_memory_present(
    pacifying_memory: MemoryModuleId,
    pacify_duration: i64,
) -> OneShot {
    OneShot::new(
        "BecomePassiveIfMemoryPresent",
        vec![
            (types::ATTACK_TARGET.id(), MemoryStatus::Registered),
            (types::PACIFIED.id(), MemoryStatus::ValueAbsent),
            (pacifying_memory, MemoryStatus::ValuePresent),
        ],
        move |tick| {
            tick.brain
                .set_with_expiry(types::PACIFIED, true, pacify_duration);
            tick.brain.erase(types::ATTACK_TARGET.id());
            true
        },
    )
}
