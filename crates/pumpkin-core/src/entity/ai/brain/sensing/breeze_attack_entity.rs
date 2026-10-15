use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, types};
use super::{NearestLivingEntitySensor, Sensor, is_entity_attackable};

const REQUIRES: &[MemoryModuleId] = &[
    types::NEAREST_LIVING_ENTITIES.id(),
    types::NEAREST_VISIBLE_LIVING_ENTITIES.id(),
    types::NEAREST_ATTACKABLE.id(),
];

/// Vanilla `BreezeAttackEntitySensor`: a `NearestLivingEntitySensor` that also remembers the
/// nearest attackable entity, seen or not, that is not a creative or spectator player.
pub struct BreezeAttackEntitySensor;

impl Sensor for BreezeAttackEntitySensor {
    fn requires(&self) -> &'static [MemoryModuleId] {
        REQUIRES
    }

    fn do_tick(&mut self, tick: &mut BrainTick<'_>) {
        NearestLivingEntitySensor.do_tick(tick);
        let target = {
            let ctx = tick.visibility();
            ctx.brain
                .get(types::NEAREST_LIVING_ENTITIES)
                .and_then(|entities| {
                    entities
                        .iter()
                        .find(|entity| {
                            // `EntitySelector.NO_CREATIVE_OR_SPECTATOR`.
                            entity.get_player().is_none_or(|player| {
                                !player.is_spectator() && !player.is_creative()
                            }) && is_entity_attackable(&ctx, entity.as_ref())
                        })
                        .cloned()
                })
        };
        tick.brain.set_optional(types::NEAREST_ATTACKABLE, target);
    }
}
