use pumpkin_data::damage::DamageType;

use crate::entity::effect::{MobEffect, interval_tick};
use crate::entity::living::LivingEntity;

/// Vanilla `PoisonMobEffect.DAMAGE_INTERVAL`.
const DAMAGE_INTERVAL: i32 = 25;

pub struct PoisonMobEffect;

impl MobEffect for PoisonMobEffect {
    fn should_apply_effect_tick(&self, tick_count: i32, amplifier: u8) -> bool {
        interval_tick(DAMAGE_INTERVAL, tick_count, amplifier)
    }

    fn apply_effect_tick(&self, living: &LivingEntity, _amplifier: u8) -> bool {
        if living.health.load() > 1.0
            && let Some(dyn_self) = living
                .entity
                .world
                .load()
                .get_entity_by_id(living.entity.entity_id)
        {
            dyn_self.damage(&*dyn_self, 1.0, DamageType::MAGIC);
        }
        true
    }
}
