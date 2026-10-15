use std::sync::Arc;

use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Effect;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity, equipment::RegionalDifficulty, skeleton::SkeletonEntityBase},
};
use crate::world::World;

/// Vanilla `WitherSkeleton`.
pub struct WitherSkeletonEntity {
    entity: Arc<SkeletonEntityBase>,
}

impl WitherSkeletonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = SkeletonEntityBase::new(entity);
        entity.mob_entity.set_pathfinding_malus(PathType::Lava, 8.0);
        Arc::new(Self { entity })
    }
}

impl Mob for WitherSkeletonEntity {
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
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        let group_data = self.entity.finalize_spawn(self, world, group_data);
        self.get_mob_entity()
            .living_entity
            .set_attribute_base(&Attributes::ATTACK_DAMAGE, 4.0);
        self.entity.reassess_weapon_goal(self);
        group_data
    }

    fn populate_default_equipment_slots(
        &self,
        _world: &Arc<World>,
        _difficulty: &RegionalDifficulty,
    ) {
        self.get_mob_entity()
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .put(
                &EquipmentSlot::MAIN_HAND,
                ItemStack::new(1, &Item::STONE_SWORD),
            );
    }

    fn populate_default_equipment_enchantments(&self, _difficulty: &RegionalDifficulty) {}

    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        self.entity.pick_up_loot(self);
    }

    /// Vanilla `WitherSkeleton.doHurtTarget`: a hit that lands withers the target.
    fn on_attack(&self, target: &dyn EntityBase) {
        if let Some(living) = target.get_living_entity() {
            living.add_effect(Effect {
                effect_type: &StatusEffect::WITHER,
                duration: 200,
                amplifier: 0,
                ambient: false,
                show_particles: true,
                show_icon: true,
                blend: false,
            });
        }
    }
}
