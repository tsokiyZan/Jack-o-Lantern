use pumpkin_data::entity::EntityPose;
use pumpkin_util::math::boundingbox::EntityDimensions;
use std::sync::Arc;

use crate::entity::mob::zombie::ZombieEntityBase;
use crate::entity::{
    Entity,
    mob::{Mob, MobEntity},
};
use pumpkin_nbt::compound::NbtCompound;

pub struct HuskEntity {
    entity: Arc<ZombieEntityBase>,
}

impl HuskEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = ZombieEntityBase::with_can_break_doors(entity, false, BABY_DIMENSIONS);
        let zombie = Self { entity };
        Arc::new(zombie)
    }

    #[must_use]
    pub fn with_can_break_doors(entity: Entity, can_break_doors: bool) -> Arc<Self> {
        let entity =
            ZombieEntityBase::with_can_break_doors(entity, can_break_doors, BABY_DIMENSIONS);
        let zombie = Self { entity };
        Arc::new(zombie)
    }
}

/// Vanilla `Husk.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    EntityDimensions::scalable(0.49, 0.98).with_eye_height(0.825);

impl Mob for HuskEntity {
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        self.entity.default_dimensions(pose)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn spawn_as_baby(&self) -> bool {
        self.entity.spawn_as_baby()
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.entity.mob_write_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.entity.mob_read_nbt(nbt);
    }
}
