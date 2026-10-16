use std::sync::Arc;

use crate::entity::{
    Entity,
    mob::{Mob, MobEntity},
};

pub struct GiantEntity {
    pub mob_entity: MobEntity,
}

impl GiantEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        // Vanilla `Giant.createAttributes` (100 health, 0.5 speed, 50 damage) is the type's
        // attribute supplier.
        let mob_entity = MobEntity::new(entity);
        mob_entity.living_entity.health.store(100.0);

        // Vanilla `Giant` registers no goals: it stands where it spawned.
        Arc::new(Self { mob_entity })
    }
}

impl Mob for GiantEntity {
    /// Vanilla `Giant.getWalkTargetValue`: `getPathfindingCostFromLightLevels`.
    fn get_walk_target_value(&self, pos: &pumpkin_util::math::position::BlockPos) -> f32 {
        self.mob_entity
            .living_entity
            .entity
            .world
            .load()
            .get_light_level_dependent_magic_value(pos)
            - 0.5
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }
}
