use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, MemoryModuleType, types};
use super::Sensor;
use crate::entity::EntityBase;

const REQUIRES: &[MemoryModuleId] = &[types::NEAREST_LIVING_ENTITIES.id()];

pub type MobTest = fn(&BrainTick<'_>, &dyn EntityBase) -> bool;
pub type ReadyTest = fn(&BrainTick<'_>) -> bool;

/// Vanilla `MobSensor`: while `ready_test` holds, remembers `to_set` for a while whenever
/// a nearby living entity passes `mob_test`; otherwise forgets it.
pub struct MobSensor {
    mob_test: MobTest,
    ready_test: ReadyTest,
    to_set: MemoryModuleType<bool>,
    memory_time_to_live: i32,
}

impl MobSensor {
    #[must_use]
    pub const fn new(
        mob_test: MobTest,
        ready_test: ReadyTest,
        to_set: MemoryModuleType<bool>,
        memory_time_to_live: i32,
    ) -> Self {
        Self {
            mob_test,
            ready_test,
            to_set,
            memory_time_to_live,
        }
    }

    fn check_for_mobs_nearby(&self, tick: &mut BrainTick<'_>) {
        let tick_ref: &BrainTick<'_> = tick;
        let mob_present = tick_ref
            .brain
            .get(types::NEAREST_LIVING_ENTITIES)
            .is_some_and(|entities| {
                entities
                    .iter()
                    .any(|entity| (self.mob_test)(tick_ref, entity.as_ref()))
            });
        if mob_present {
            self.mob_detected(tick);
        }
    }

    fn mob_detected(&self, tick: &mut BrainTick<'_>) {
        tick.brain
            .set_with_expiry(self.to_set, true, i64::from(self.memory_time_to_live));
    }

    fn clear_memory(&self, tick: &mut BrainTick<'_>) {
        tick.brain.erase(self.to_set.id());
    }
}

impl Sensor for MobSensor {
    fn requires(&self) -> &'static [MemoryModuleId] {
        REQUIRES
    }

    fn do_tick(&mut self, tick: &mut BrainTick<'_>) {
        if (self.ready_test)(tick) {
            self.check_for_mobs_nearby(tick);
        } else {
            self.clear_memory(tick);
        }
    }
}
