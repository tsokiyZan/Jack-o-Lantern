use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::world::World;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_nbt::compound::NbtCompound;
use std::sync::Arc;

use crate::entity::{
    Entity,
    mob::{Mob, MobEntity, skeleton::SkeletonEntityBase},
};

pub struct WitherSkeletonEntity {
    entity: Arc<SkeletonEntityBase>,
}

impl WitherSkeletonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = SkeletonEntityBase::new(entity);
        // Vanilla `WitherSkeleton`'s constructor.
        entity.mob_entity.set_pathfinding_malus(PathType::Lava, 8.0);
        let skeleton = Self { entity };
        Arc::new(skeleton)
    }
}

impl Mob for WitherSkeletonEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn on_equip_item(&self, _slot: &EquipmentSlot) {
        super::reassess_weapon_goal(self);
    }

    fn mob_read_nbt(&self, _nbt: &NbtCompound) {
        super::reassess_weapon_goal(self);
    }

    fn finalize_spawn(
        &self,
        _world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.get_mob_entity().finalize_spawn_base();
        // Vanilla `WitherSkeleton.finalizeSpawn`.
        self.get_mob_entity()
            .living_entity
            .set_attribute_base(&Attributes::ATTACK_DAMAGE, 4.0);
        super::reassess_weapon_goal(self);
        group_data
    }
}
