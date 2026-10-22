use std::sync::Arc;
use std::sync::atomic::Ordering;

use pumpkin_data::enchantment::LevelBasedValue;
use pumpkin_util::math::vector3::Vector3;

use super::EnchantmentEntityEffectExt;
use crate::enchantment::EnchantedItemInUse;
use crate::entity::EntityBase;
use crate::net::ClientPlatform;
use crate::world::World;

/// Enchantment entity effect that applies a velocity impulse to an entity.
#[derive(Clone, Debug, PartialEq)]
pub struct ApplyEntityImpulse {
    pub direction: Vector3<f64>,
    pub coordinate_scale: Vector3<f64>,
    pub magnitude: LevelBasedValue,
}

impl ApplyEntityImpulse {
    #[must_use]
    pub const fn new(
        direction: Vector3<f64>,
        coordinate_scale: Vector3<f64>,
        magnitude: LevelBasedValue,
    ) -> Self {
        Self {
            direction,
            coordinate_scale,
            magnitude,
        }
    }
}

impl EnchantmentEntityEffectExt for ApplyEntityImpulse {
    /// Vanilla `ApplyEntityImpulse.apply`.
    fn apply(
        &self,
        _world: &Arc<World>,
        enchantment_level: i32,
        _item: &EnchantedItemInUse<'_>,
        entity: &dyn EntityBase,
        _position: Vector3<f64>,
    ) {
        let base = entity.get_entity();
        let scale = self.coordinate_scale;
        let magnitude = f64::from(self.magnitude.calculate(enchantment_level));
        let rotated = base.get_look_quaternion().transform(self.direction);
        let impulse = Vector3::new(
            rotated.x * scale.x * magnitude,
            rotated.y * scale.y * magnitude,
            rotated.z * scale.z * magnitude,
        );
        // `addDeltaMovement`.
        if impulse.x.is_finite() && impulse.y.is_finite() && impulse.z.is_finite() {
            base.velocity.store(base.velocity.load() + impulse);
        }
        // Bedrock players keep `sync_velocity` for the tracker to send.
        if let Some(player) = entity.get_player()
            && matches!(player.client.as_ref(), ClientPlatform::Java(_))
        {
            player.send_own_motion();
            base.sync_velocity.store(false, Ordering::SeqCst);
        } else {
            base.sync_velocity.store(true, Ordering::SeqCst);
        }
        if let Some(living) = entity.get_living_entity() {
            living.apply_post_impulse_grace_time(10);
        }
    }
}
