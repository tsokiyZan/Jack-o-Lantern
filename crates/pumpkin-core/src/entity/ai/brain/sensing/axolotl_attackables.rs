use pumpkin_data::tag::{self, Taggable};

use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, types};
use super::{Sensor, is_entity_attackable};

const REQUIRES: &[MemoryModuleId] = &[
    types::NEAREST_ATTACKABLE.id(),
    types::NEAREST_VISIBLE_LIVING_ENTITIES.id(),
    types::HAS_HUNTING_COOLDOWN.id(),
];

/// Vanilla `AxolotlAttackablesSensor.TARGET_DETECTION_DISTANCE`, squared.
const TARGET_DETECTION_DISTANCE_SQ: f64 = 64.0;

/// Vanilla `AxolotlAttackablesSensor`: the nearest visible mob in water within eight
/// blocks that axolotls always attack, or hunt when not on a hunting cooldown.
pub struct AxolotlAttackablesSensor;

impl Sensor for AxolotlAttackablesSensor {
    fn requires(&self) -> &'static [MemoryModuleId] {
        REQUIRES
    }

    fn do_tick(&mut self, tick: &mut BrainTick<'_>) {
        let body_pos = tick.mob.get_entity().pos.load();
        let hunting_cooldown = tick
            .brain
            .has_memory_value(types::HAS_HUNTING_COOLDOWN.id());
        let target = {
            let ctx = tick.visibility();
            ctx.brain
                .get(types::NEAREST_VISIBLE_LIVING_ENTITIES)
                .and_then(|visible| {
                    visible.find_closest(&ctx, |mob| {
                        let entity = mob.get_entity();
                        let entity_type = entity.entity_type;
                        entity.pos.load().squared_distance_to_vec(&body_pos)
                            <= TARGET_DETECTION_DISTANCE_SQ
                            && entity.is_in_water()
                            && (entity_type
                                .has_tag(&tag::EntityType::MINECRAFT_AXOLOTL_ALWAYS_HOSTILES)
                                || !hunting_cooldown
                                    && entity_type
                                        .has_tag(&tag::EntityType::MINECRAFT_AXOLOTL_HUNT_TARGETS))
                            && is_entity_attackable(&ctx, mob.as_ref())
                    })
                })
        };
        tick.brain.set_optional(types::NEAREST_ATTACKABLE, target);
    }
}
