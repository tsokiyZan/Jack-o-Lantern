use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, types};
use super::{Sensor, is_entity_attackable};
use crate::entity::passive::frog;

const REQUIRES: &[MemoryModuleId] = &[
    types::NEAREST_ATTACKABLE.id(),
    types::NEAREST_VISIBLE_LIVING_ENTITIES.id(),
    types::UNREACHABLE_TONGUE_TARGETS.id(),
];

/// Vanilla `FrogAttackablesSensor.TARGET_DETECTION_DISTANCE`.
const TARGET_DETECTION_DISTANCE: f64 = 10.0;

/// Vanilla `FrogAttackablesSensor`: the nearest visible mob a frog can eat within ten
/// blocks that it has not recently failed to reach.
pub struct FrogAttackablesSensor;

impl Sensor for FrogAttackablesSensor {
    fn requires(&self) -> &'static [MemoryModuleId] {
        REQUIRES
    }

    fn do_tick(&mut self, tick: &mut BrainTick<'_>) {
        let body_pos = tick.mob.get_entity().pos.load();
        let target = {
            let ctx = tick.visibility();
            let unreachable = ctx.brain.get(types::UNREACHABLE_TONGUE_TARGETS);
            ctx.brain
                .get(types::NEAREST_VISIBLE_LIVING_ENTITIES)
                .and_then(|visible| {
                    visible.find_closest(&ctx, |mob| {
                        is_entity_attackable(&ctx, mob.as_ref())
                            && frog::can_eat(mob.as_ref())
                            && !unreachable.is_some_and(|targets| {
                                targets.contains(&mob.get_entity().entity_uuid)
                            })
                            && mob.get_entity().pos.load().squared_distance_to_vec(&body_pos)
                                < TARGET_DETECTION_DISTANCE * TARGET_DETECTION_DISTANCE
                    })
                })
        };
        tick.brain.set_optional(types::NEAREST_ATTACKABLE, target);
    }
}
