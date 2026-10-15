use pumpkin_data::tag::{self, Taggable};

use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, types};
use super::Sensor;

const REQUIRES: &[MemoryModuleId] = &[
    types::NEAREST_VISIBLE_ADULT.id(),
    types::NEAREST_VISIBLE_LIVING_ENTITIES.id(),
];

/// Vanilla `AdultSensor`: the nearest visible adult of the mob's own type.
pub struct AdultSensor;

impl Sensor for AdultSensor {
    fn requires(&self) -> &'static [MemoryModuleId] {
        REQUIRES
    }

    fn do_tick(&mut self, tick: &mut BrainTick<'_>) {
        if !tick
            .brain
            .has_memory_value(types::NEAREST_VISIBLE_LIVING_ENTITIES.id())
        {
            return;
        }
        let own_type = tick.mob.get_entity().entity_type;
        let adult = {
            let ctx = tick.visibility();
            ctx.brain
                .get(types::NEAREST_VISIBLE_LIVING_ENTITIES)
                .and_then(|visible| {
                    visible.find_closest(&ctx, |entity| {
                        entity.get_entity().entity_type == own_type
                            && !entity.get_entity().is_baby()
                    })
                })
        };
        tick.brain.set_optional(types::NEAREST_VISIBLE_ADULT, adult);
    }
}

/// Vanilla `AdultSensorAnyType`: the nearest visible adult of any followable friendly type.
pub struct AdultSensorAnyType;

impl Sensor for AdultSensorAnyType {
    fn requires(&self) -> &'static [MemoryModuleId] {
        REQUIRES
    }

    fn do_tick(&mut self, tick: &mut BrainTick<'_>) {
        if !tick
            .brain
            .has_memory_value(types::NEAREST_VISIBLE_LIVING_ENTITIES.id())
        {
            return;
        }
        let adult = {
            let ctx = tick.visibility();
            ctx.brain
                .get(types::NEAREST_VISIBLE_LIVING_ENTITIES)
                .and_then(|visible| {
                    visible.find_closest(&ctx, |entity| {
                        let entity = entity.get_entity();
                        entity
                            .entity_type
                            .has_tag(&tag::EntityType::MINECRAFT_FOLLOWABLE_FRIENDLY_MOBS)
                            && !entity.is_baby()
                    })
                })
        };
        tick.brain.set_optional(types::NEAREST_VISIBLE_ADULT, adult);
    }
}
