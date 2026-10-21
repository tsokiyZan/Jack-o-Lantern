use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;

/// Vanilla `AbsorptionMobEffect`.
pub struct AbsorptionMobEffect;

impl MobEffect for AbsorptionMobEffect {
    fn should_apply_effect_tick(&self, _tick_count: i32, _amplifier: u8) -> bool {
        true
    }

    /// The effect ends once the absorption hearts are used up.
    fn apply_effect_tick(&self, living: &LivingEntity, _amplifier: u8) -> bool {
        living.get_absorption() > 0.0
    }

    fn on_effect_started(&self, living: &LivingEntity, amplifier: u8) {
        living.set_absorption(
            living
                .get_absorption()
                .max((4 * (1 + i32::from(amplifier))) as f32),
        );
    }
}
