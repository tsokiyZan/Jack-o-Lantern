use crate::entity::breath::BreathManager;
use pumpkin_util::random::RandomImpl;
use std::sync::Arc;

use pumpkin_data::sound::Sound;

use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
    passive::{
        fish,
        schooling_fish::{self, SchoolingFish},
    },
};

/// Represents a Salmon, a passive fish of rivers and cold oceans.
///
/// Wiki: <https://minecraft.wiki/w/Salmon>
/// Vanilla `Salmon.getMaxSchoolSize`.
const MAX_SCHOOL_SIZE: i32 = 5;

pub struct SalmonEntity {
    pub mob_entity: MobEntity,
    pub school: SchoolingFish,
}

impl SalmonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        fish::init(&mob_entity);
        schooling_fish::init(&mob_entity);
        Arc::new(Self {
            mob_entity,
            school: SchoolingFish::new(MAX_SCHOOL_SIZE),
        })
    }
}

impl Mob for SalmonEntity {
    /// Vanilla `WaterAnimal.checkSpawnObstruction`: water doesn't obstruct it.
    fn check_spawn_obstruction(&self, world: &crate::world::World) -> bool {
        let entity = self.get_entity();
        world.is_unobstructed(Some(entity), &entity.bounding_box.load())
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
            &pumpkin_data::item::Item::SALMON_BUCKET,
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
        fish::flop(self, Sound::EntitySalmonFlop);
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
