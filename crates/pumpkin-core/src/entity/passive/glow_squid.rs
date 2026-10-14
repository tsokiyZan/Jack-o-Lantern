use crate::entity::breath::BreathManager;
use pumpkin_data::entity::EntityPose;
use pumpkin_util::math::boundingbox::EntityDimensions;
use std::sync::Arc;

use pumpkin_data::particle::Particle;
use pumpkin_data::sound::Sound;

use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
    passive::squid::SquidMovement,
};

/// Represents a Glow Squid, a passive aquatic mob that emits a glowing particle effect.
///
/// Wiki: <https://minecraft.wiki/w/Glow_Squid>
pub struct GlowSquidEntity {
    pub mob_entity: MobEntity,
    pub movement: SquidMovement,
}

impl GlowSquidEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        Arc::new(Self {
            mob_entity: MobEntity::new(entity),
            movement: SquidMovement::new(),
        })
    }
}

/// Vanilla `Squid.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    EntityDimensions::scalable(0.5, 0.5).with_eye_height(0.37);

impl Mob for GlowSquidEntity {
    /// Vanilla `Squid.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    /// Vanilla `AgeableWaterCreature.handleAirSupply`.
    fn handle_air_supply(&self, pre_tick_air: i32) {
        BreathManager::water_animal_handle_air_supply(self, pre_tick_air);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.movement.tick(self);
    }

    fn on_damage(
        &self,
        _damage_type: pumpkin_data::damage::DamageType,
        _source: Option<&dyn EntityBase>,
    ) {
        self.movement.spawn_ink_if_hurt_by_mob(
            self,
            Particle::GlowSquidInk,
            Sound::EntityGlowSquidSquirt,
        );
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        SquidMovement::travel(self, caller)
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }
}
