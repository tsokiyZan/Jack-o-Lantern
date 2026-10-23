use crate::entity::breath::BreathManager;
use pumpkin_util::random::RandomImpl;
use std::sync::Arc;

use pumpkin_data::sound::Sound;

use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
    passive::{
        fish,
        schooling_fish::{self, DEFAULT_MAX_SCHOOL_SIZE, SchoolingFish},
    },
};

/// Represents a Cod, a common passive aquatic mob.
///
/// Wiki: <https://minecraft.wiki/w/Cod>
pub struct CodEntity {
    pub mob_entity: MobEntity,
    pub school: SchoolingFish,
    pub from_bucket: fish::FromBucket,
}

impl CodEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        fish::init(&mob_entity);
        schooling_fish::init(&mob_entity);
        Arc::new(Self {
            mob_entity,
            school: SchoolingFish::new(DEFAULT_MAX_SCHOOL_SIZE),
            from_bucket: fish::FromBucket::default(),
        })
    }
}

impl Mob for CodEntity {
    /// Vanilla `AbstractSchoolingFish.finalizeSpawn`.
    fn finalize_spawn(
        &self,
        _world: &Arc<crate::world::World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        self.mob_entity.finalize_spawn_base();
        Some(schooling_fish::finalize_school_spawn(
            &self.school,
            group_data,
        ))
    }

    /// Vanilla `WaterAnimal.checkSpawnObstruction`: water doesn't obstruct it.
    fn check_spawn_obstruction(&self, world: &crate::world::World) -> bool {
        let entity = self.get_entity();
        world.is_unobstructed(Some(entity), &entity.bounding_box.load())
    }

    fn set_from_bucket(&self, from_bucket: bool) {
        self.from_bucket.set(self, from_bucket);
    }

    fn requires_custom_persistence(&self) -> bool {
        self.from_bucket.requires_custom_persistence(self)
    }

    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        self.from_bucket.remove_when_far_away(self)
    }

    fn mob_write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.from_bucket.write_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.from_bucket.read_nbt(self, nbt);
    }

    /// Vanilla `AbstractFish.loadFromBucketTag`.
    fn load_from_bucket_tag(&self, tag: &pumpkin_nbt::compound::NbtCompound) {
        crate::entity::bucketable::load_default_data_from_bucket_tag(self, tag);
    }

    fn mob_interact(
        &self,
        player: &Arc<crate::entity::player::Player>,
        _hand: pumpkin_util::Hand,
        item_stack: &mut pumpkin_data::item_stack::ItemStack,
    ) -> crate::item::interaction_result::InteractionResult {
        crate::entity::bucketable::fish_mob_interact(
            self,
            player,
            item_stack,
            &pumpkin_data::item::Item::COD_BUCKET,
            |bucket| crate::entity::bucketable::save_fish_to_bucket_tag(self, bucket),
        )
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `WaterAnimal.getBaseExperienceReward`.
    fn get_base_experience_reward(&self) -> u32 {
        1 + self.get_entity().random().next_bounded_i32(3) as u32
    }

    /// Vanilla `WaterAnimal.handleAirSupply`.
    fn handle_air_supply(&self, pre_tick_air: i32) {
        BreathManager::water_animal_handle_air_supply(self, pre_tick_air);
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.school.tick(self);
    }

    /// Vanilla `AbstractFish.aiStep`.
    fn ai_step(&self, _caller: &dyn EntityBase) {
        fish::flop(self, Sound::EntityCodFlop);
    }

    fn as_schooling_fish(&self) -> Option<&SchoolingFish> {
        Some(&self.school)
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        fish::travel(self, caller)
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }
}
