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

/// Represents a Tropical Fish, a passive fish of warm oceans.
///
/// Wiki: <https://minecraft.wiki/w/Tropical_Fish>
pub struct TropicalFishEntity {
    pub mob_entity: MobEntity,
    pub school: SchoolingFish,
}

impl TropicalFishEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        fish::init(&mob_entity);
        schooling_fish::init(&mob_entity);
        Arc::new(Self {
            mob_entity,
            school: SchoolingFish::new(DEFAULT_MAX_SCHOOL_SIZE),
        })
    }
}

impl Mob for TropicalFishEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        fish::flop(self, Sound::EntityTropicalFishFlop);
        self.school.tick(self);
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
