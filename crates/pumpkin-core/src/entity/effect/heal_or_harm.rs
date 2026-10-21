use pumpkin_data::damage::DamageType;

use crate::entity::EntityBase;
use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;

/// Vanilla `HealOrHarmMobEffect`: instant health and instant damage, swapped for entities in
/// `#inverted_healing_and_harm`.
pub struct HealOrHarmMobEffect {
    pub is_harm: bool,
}

/// Java's `base << amplifier`: the shift count is masked to five bits and the result wraps.
const fn shifted(base: i32, amplifier: u8) -> i32 {
    base.wrapping_shl(amplifier as u32)
}

impl MobEffect for HealOrHarmMobEffect {
    fn is_instantaneous(&self) -> bool {
        true
    }

    /// Vanilla `InstantaneousMobEffect.shouldApplyEffectTickThisTick`.
    fn should_apply_effect_tick(&self, tick_count: i32, _amplifier: u8) -> bool {
        tick_count >= 1
    }

    fn apply_effect_tick(&self, living: &LivingEntity, amplifier: u8) -> bool {
        if self.is_harm == living.is_inverted_heal_and_harm() {
            living.heal(shifted(4, amplifier).max(0) as f32);
        } else if let Some(dyn_self) = living
            .entity
            .world
            .load()
            .get_entity_by_id(living.entity.entity_id)
        {
            dyn_self.damage(&*dyn_self, shifted(6, amplifier) as f32, DamageType::MAGIC);
        }
        true
    }

    fn apply_instantaneous_effect(
        &self,
        source: Option<&dyn EntityBase>,
        owner: Option<&dyn EntityBase>,
        living: &LivingEntity,
        amplifier: u8,
        scale: f64,
    ) {
        if self.is_harm == living.is_inverted_heal_and_harm() {
            let amount = (scale * f64::from(shifted(4, amplifier)) + 0.5) as i32;
            living.heal(amount as f32);
        } else {
            let amount = (scale * f64::from(shifted(6, amplifier)) + 0.5) as i32;
            let Some(dyn_self) = living
                .entity
                .world
                .load()
                .get_entity_by_id(living.entity.entity_id)
            else {
                return;
            };
            if source.is_none() {
                dyn_self.damage(&*dyn_self, amount as f32, DamageType::MAGIC);
            } else {
                // `damageSources().indirectMagic(source, owner)`.
                dyn_self.damage_with_context(
                    &*dyn_self,
                    amount as f32,
                    DamageType::INDIRECT_MAGIC,
                    None,
                    source,
                    owner,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::shifted;

    #[test]
    fn amplifier_shift_wraps_like_java() {
        assert_eq!(shifted(4, 0), 4);
        assert_eq!(shifted(4, 1), 8);
        // `4 << 29` overflows into the sign bit, `4 << 30` to zero, `4 << 32` masks to `4 << 0`.
        assert_eq!(shifted(4, 29), i32::MIN);
        assert_eq!(shifted(4, 30), 0);
        assert_eq!(shifted(4, 32), 4);
        assert_eq!(shifted(4, 31).max(0), 0);
    }
}
