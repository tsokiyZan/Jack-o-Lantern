use std::sync::Arc;

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity, equipment::RegionalDifficulty, skeleton::SkeletonEntityBase},
};
use crate::world::World;

/// Vanilla `Stray`.
pub struct StraySkeletonEntity {
    entity: Arc<SkeletonEntityBase>,
}

impl StraySkeletonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = SkeletonEntityBase::new(entity);
        Arc::new(Self { entity })
    }
}

impl Mob for StraySkeletonEntity {
    /// Vanilla `rideTick`.
    fn mob_after_ride_tick(&self) {
        crate::entity::mob::turn_body_with_controlled_vehicle(self);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn on_equip_item(&self, _slot: &EquipmentSlot) {
        self.entity.reassess_weapon_goal(self);
    }

    fn mob_read_nbt(&self, _nbt: &NbtCompound) {
        self.entity.reassess_weapon_goal(self);
    }

    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.entity.finalize_spawn(self, world, group_data)
    }

    fn populate_default_equipment_slots(
        &self,
        world: &Arc<World>,
        difficulty: &RegionalDifficulty,
    ) {
        self.entity
            .populate_default_equipment_slots(world, difficulty);
    }

    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        self.entity.pick_up_loot(self);
    }

    fn get_preferred_weapon_type(&self) -> Option<&'static pumpkin_data::tag::Tag> {
        Some(&pumpkin_data::tag::Item::MINECRAFT_SKELETON_PREFERRED_WEAPONS)
    }
}
