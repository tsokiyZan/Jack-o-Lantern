use std::sync::Arc;

use pumpkin_data::damage::DamageType;
use pumpkin_data::enchantment::LevelBasedValue;
use pumpkin_util::math::vector3::Vector3;

use super::EnchantmentEntityEffectExt;
use crate::enchantment::EnchantedItemInUse;
use crate::entity::EntityBase;
use crate::world::World;

/// Enchantment entity effect that damages an entity.
#[derive(Clone, Debug, PartialEq)]
pub struct DamageEntity {
    pub min_damage: LevelBasedValue,
    pub max_damage: LevelBasedValue,
    pub damage_type: Option<&'static DamageType>,
}

impl DamageEntity {
    #[must_use]
    pub const fn new(
        min_damage: LevelBasedValue,
        max_damage: LevelBasedValue,
        damage_type: Option<&'static DamageType>,
    ) -> Self {
        Self {
            min_damage,
            max_damage,
            damage_type,
        }
    }
}

impl EnchantmentEntityEffectExt for DamageEntity {
    fn apply(
        &self,
        _world: &Arc<World>,
        enchantment_level: i32,
        item: &EnchantedItemInUse<'_>,
        entity: &dyn EntityBase,
        _position: Vector3<f64>,
    ) {
        let Some(damage_type) = self.damage_type else {
            return;
        };
        let min = self.min_damage.calculate(enchantment_level);
        let max = self.max_damage.calculate(enchantment_level);
        // `Mth.randomBetween(entity.getRandom(), ...)`.
        let damage = rand::random::<f32>() * (max - min) + min;
        // `new DamageSource(damageType, item.owner())`: the owner is both direct and causing.
        entity.damage_with_context(entity, damage, *damage_type, None, item.owner, item.owner);
    }
}
