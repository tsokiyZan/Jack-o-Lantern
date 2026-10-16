use std::sync::Arc;

use pumpkin_data::enchantment::LevelBasedValue;
use pumpkin_util::math::vector3::Vector3;

use super::EnchantmentEntityEffectExt;
use crate::enchantment::EnchantedItemInUse;
use crate::entity::EntityBase;
use crate::world::World;

/// Enchantment entity effect that sets an entity on fire for a duration calculated from the level.
#[derive(Clone, Debug, PartialEq)]
pub struct Ignite {
    pub duration: LevelBasedValue,
}

impl Ignite {
    #[must_use]
    pub const fn new(duration: LevelBasedValue) -> Self {
        Self { duration }
    }
}

impl EnchantmentEntityEffectExt for Ignite {
    fn apply(
        &self,
        _world: &Arc<World>,
        enchantment_level: i32,
        _item: &EnchantedItemInUse<'_>,
        entity: &dyn EntityBase,
        _position: Vector3<f64>,
    ) {
        // `Entity.igniteForSeconds`.
        let seconds = self.duration.calculate(enchantment_level);
        entity.set_on_fire_for_ticks((seconds * 20.0).floor() as u32);
    }
}
