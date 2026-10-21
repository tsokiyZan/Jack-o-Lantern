use crate::entity::effect::{MobEffect, interval_tick};
use crate::entity::living::LivingEntity;

pub struct RegenerationMobEffect;

impl MobEffect for RegenerationMobEffect {
    fn should_apply_effect_tick(&self, tick_count: i32, amplifier: u8) -> bool {
        interval_tick(50, tick_count, amplifier)
    }

    fn apply_effect_tick(&self, living: &LivingEntity, _amplifier: u8) -> bool {
        if living.health.load() < living.get_max_health() {
            living.heal(1.0);
        }
        true
    }
}
