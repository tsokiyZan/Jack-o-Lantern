use std::sync::Arc;

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::tracked_data;
use pumpkin_data::world::WorldEvent;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::mob::conversion::{ConversionTracker, convert_single};
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity, equipment::RegionalDifficulty, skeleton::SkeletonEntityBase},
};
use crate::world::World;

pub struct SkeletonEntity {
    entity: Arc<SkeletonEntityBase>,
    /// Vanilla `freezingTracker`: powder snow turns a skeleton into a stray.
    freezing_tracker: ConversionTracker,
}

impl SkeletonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = SkeletonEntityBase::new(entity);
        let skeleton = Self {
            entity,
            freezing_tracker: ConversionTracker::new(
                tracked_data::skeleton::DATA_STRAY_CONVERSION_ID,
                "FreezingTime",
                140,
                "StrayConversionTime",
                300,
            ),
        };
        Arc::new(skeleton)
    }

    fn convert_to_stray(&self) {
        let world = self.get_entity().world.load();
        convert_single(self, &EntityType::STRAY, true, true, |converted| {
            let converted = converted.get_entity();
            if !converted.is_silent() {
                world.sync_world_event(
                    WorldEvent::SoundSkeletonToStray,
                    converted.block_pos.load(),
                    0,
                );
            }
        });
    }
}

impl Mob for SkeletonEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    /// Vanilla `Skeleton.tick` runs the tracker before `super.tick()`.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        let in_powder_snow = self.get_entity().is_in_powder_snow();
        self.freezing_tracker
            .tick(self, in_powder_snow, || self.convert_to_stray());
    }

    fn can_freeze(&self) -> bool {
        false
    }

    fn on_equip_item(&self, _slot: &EquipmentSlot) {
        self.entity.reassess_weapon_goal(self);
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.freezing_tracker
            .write_nbt(nbt, self.get_entity().is_in_powder_snow());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.entity.reassess_weapon_goal(self);
        self.freezing_tracker.read_nbt(self, nbt);
    }

    fn finalize_spawn(
        &self,
        world: &Arc<World>,
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
