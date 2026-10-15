use std::sync::Arc;

use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::position::BlockPos;

use crate::entity::EntityBase;
use crate::entity::ai::util::goal_utils::within_box_by_manhattan_distance;
use crate::entity::mob::hoglin::HoglinEntity;
use crate::entity::mob::hoglin_ai::{
    REPELLENT_DETECTION_RANGE_HORIZONTAL, REPELLENT_DETECTION_RANGE_VERTICAL,
};
use crate::world::World;

use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, NearestVisibleLivingEntities, types};
use super::Sensor;

const REQUIRES: &[MemoryModuleId] = &[
    types::NEAREST_VISIBLE_LIVING_ENTITIES.id(),
    types::NEAREST_REPELLENT.id(),
    types::NEAREST_VISIBLE_ADULT_PIGLIN.id(),
    types::NEAREST_VISIBLE_ADULT_HOGLINS.id(),
    types::VISIBLE_ADULT_PIGLIN_COUNT.id(),
    types::VISIBLE_ADULT_HOGLIN_COUNT.id(),
];

/// Vanilla `HoglinSpecificSensor`.
pub struct HoglinSpecificSensor;

impl Sensor for HoglinSpecificSensor {
    fn requires(&self) -> &'static [MemoryModuleId] {
        REQUIRES
    }

    fn do_tick(&mut self, tick: &mut BrainTick<'_>) {
        let repellent = find_nearest_repellent(tick.world, tick.mob.get_entity().block_pos.load());
        if let Some(hoglin) = tick.mob.cast_any().downcast_ref::<HoglinEntity>() {
            hoglin.set_nearest_repellent(repellent);
        }
        tick.brain.set_optional(types::NEAREST_REPELLENT, repellent);

        let mut adult_piglin: Option<Arc<dyn EntityBase>> = None;
        let mut adult_piglin_count = 0;
        let mut adult_hoglins: Vec<Arc<dyn EntityBase>> = Vec::new();
        {
            let ctx = tick.visibility();
            let empty = NearestVisibleLivingEntities::empty();
            let visible = ctx
                .brain
                .get(types::NEAREST_VISIBLE_LIVING_ENTITIES)
                .unwrap_or(&empty);
            for entity in visible.find_all(&ctx, |entity| {
                let entity = entity.get_entity();
                !entity.is_baby()
                    && (entity.entity_type == &EntityType::PIGLIN
                        || entity.entity_type == &EntityType::HOGLIN)
            }) {
                if entity.get_entity().entity_type == &EntityType::PIGLIN {
                    adult_piglin_count += 1;
                    if adult_piglin.is_none() {
                        adult_piglin = Some(Arc::clone(entity));
                    }
                } else {
                    adult_hoglins.push(Arc::clone(entity));
                }
            }
        }

        let adult_hoglin_count = i32::try_from(adult_hoglins.len()).unwrap_or(i32::MAX);
        tick.brain
            .set_optional(types::NEAREST_VISIBLE_ADULT_PIGLIN, adult_piglin);
        tick.brain
            .set(types::NEAREST_VISIBLE_ADULT_HOGLINS, adult_hoglins);
        tick.brain
            .set(types::VISIBLE_ADULT_PIGLIN_COUNT, adult_piglin_count);
        tick.brain
            .set(types::VISIBLE_ADULT_HOGLIN_COUNT, adult_hoglin_count);
    }
}

/// Vanilla `HoglinSpecificSensor.findNearestRepellent`.
fn find_nearest_repellent(world: &World, center: BlockPos) -> Option<BlockPos> {
    within_box_by_manhattan_distance(
        center,
        REPELLENT_DETECTION_RANGE_HORIZONTAL,
        REPELLENT_DETECTION_RANGE_VERTICAL,
        REPELLENT_DETECTION_RANGE_HORIZONTAL,
    )
    .into_iter()
    .find(|pos| {
        world
            .get_block(pos)
            .has_tag(&tag::Block::MINECRAFT_HOGLIN_REPELLENTS)
    })
}
