use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, MemoryModuleType, MemoryStatus};
use super::timed::Behavior;

/// Vanilla `CountDownCooldownTicks`: counts an integer memory down to zero, then
/// erases it.
pub struct CountDownCooldownTicks {
    cooldown_ticks: MemoryModuleType<i32>,
    conditions: [(MemoryModuleId, MemoryStatus); 1],
}

impl CountDownCooldownTicks {
    #[must_use]
    pub const fn new(cooldown_ticks: MemoryModuleType<i32>) -> Self {
        Self {
            cooldown_ticks,
            conditions: [(cooldown_ticks.id(), MemoryStatus::ValuePresent)],
        }
    }
}

impl Behavior for CountDownCooldownTicks {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn times_out(&self) -> bool {
        false
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        tick.brain
            .get(self.cooldown_ticks)
            .is_some_and(|ticks| *ticks > 0)
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(ticks) = tick.brain.get(self.cooldown_ticks).copied() {
            tick.brain.set(self.cooldown_ticks, ticks - 1);
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        tick.brain.erase(self.cooldown_ticks.id());
    }

    fn debug_name(&self) -> &'static str {
        "CountDownCooldownTicks"
    }
}
