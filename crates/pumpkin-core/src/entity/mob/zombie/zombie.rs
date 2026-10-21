use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::zombie::ZombieEntityBase;
use crate::entity::mob::{Mob, MobEntity};
use crate::entity::{Entity, EntityBase};
use crate::world::World;
use pumpkin_nbt::compound::NbtCompound;
use std::sync::Arc;

pub struct ZombieEntity {
    entity: Arc<ZombieEntityBase>,
}

impl ZombieEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = ZombieEntityBase::new(entity);
        let zombie = Self { entity };
        Arc::new(zombie)
    }

    #[must_use]
    pub fn with_can_break_doors(entity: Entity, can_break_doors: bool) -> Arc<Self> {
        let entity =
            ZombieEntityBase::with_can_break_doors(entity, can_break_doors, super::BABY_DIMENSIONS);
        let zombie = Self { entity };
        Arc::new(zombie)
    }
}

impl Mob for ZombieEntity {
    crate::impl_zombie_mob!(entity);

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.entity
            .finalize_zombie_spawn(self, world, group_data, false)
    }

    fn populate_default_equipment_slots(
        &self,
        world: &Arc<World>,
        difficulty: &RegionalDifficulty,
    ) {
        self.entity
            .populate_default_equipment_slots(world, difficulty);
    }

    fn on_attack(&self, target: &dyn EntityBase) {
        self.entity.on_attack(self, target);
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.entity.write_zombie_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.entity.read_zombie_nbt(self, nbt);
    }
}

impl ZombieEntity {
    #[must_use]
    pub fn can_break_doors(&self) -> bool {
        self.entity.can_break_doors()
    }

    pub fn set_can_break_doors(&self, can_break: bool) {
        self.entity.set_can_break_doors(can_break, self);
    }

    #[must_use]
    pub fn is_baby(&self) -> bool {
        self.entity.is_baby()
    }

    pub fn set_baby(&self, baby: bool) {
        self.entity.set_baby(baby);
    }
}
