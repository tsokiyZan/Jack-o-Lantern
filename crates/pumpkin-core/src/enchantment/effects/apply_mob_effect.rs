use std::sync::Arc;

use pumpkin_data::effect::StatusEffect;
use pumpkin_data::enchantment::LevelBasedValue;
use pumpkin_data::potion::Effect;
use pumpkin_util::math::vector3::Vector3;

use super::EnchantmentEntityEffectExt;
use crate::enchantment::EnchantedItemInUse;
use crate::entity::EntityBase;
use crate::world::World;

/// Enchantment entity effect that applies a status effect.
#[derive(Clone, Debug, PartialEq)]
pub struct ApplyMobEffect {
    pub to_apply: Vec<&'static StatusEffect>,
    pub min_duration: LevelBasedValue,
    pub max_duration: LevelBasedValue,
    pub min_amplifier: LevelBasedValue,
    pub max_amplifier: LevelBasedValue,
}

impl ApplyMobEffect {
    #[must_use]
    pub const fn new(
        to_apply: Vec<&'static StatusEffect>,
        min_duration: LevelBasedValue,
        max_duration: LevelBasedValue,
        min_amplifier: LevelBasedValue,
        max_amplifier: LevelBasedValue,
    ) -> Self {
        Self {
            to_apply,
            min_duration,
            max_duration,
            min_amplifier,
            max_amplifier,
        }
    }
}

/// Vanilla `Mth.randomBetween`.
fn random_between(min: f32, max: f32) -> f32 {
    rand::random::<f32>() * (max - min) + min
}

/// Java `Math.round(float)`.
fn java_round(value: f32) -> i32 {
    (value + 0.5).floor() as i32
}

impl EnchantmentEntityEffectExt for ApplyMobEffect {
    fn apply(
        &self,
        _world: &Arc<World>,
        enchantment_level: i32,
        _item: &EnchantedItemInUse<'_>,
        entity: &dyn EntityBase,
        _position: Vector3<f64>,
    ) {
        let Some(living) = entity.get_living_entity() else {
            return;
        };
        // `HolderSet.getRandomElement`.
        if self.to_apply.is_empty() {
            return;
        }
        let selected = self.to_apply[rand::random_range(0..self.to_apply.len())];
        let ticks = java_round(
            random_between(
                self.min_duration.calculate(enchantment_level),
                self.max_duration.calculate(enchantment_level),
            ) * 20.0,
        );
        let amplifier = java_round(random_between(
            self.min_amplifier.calculate(enchantment_level),
            self.max_amplifier.calculate(enchantment_level),
        ))
        .max(0);
        living.add_effect(Effect {
            effect_type: selected,
            duration: ticks,
            amplifier: amplifier as u8,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        });
    }
}
