//! Vanilla `HoglinBase`, the attack hoglins and zoglins share.

use std::sync::atomic::Ordering;

use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::enchantment::EnchantmentHelper;
use crate::entity::EntityBase;
use crate::entity::damage_source::DamageSource;
use crate::entity::mob::Mob;

/// Vanilla `HoglinBase.ATTACK_ANIMATION_DURATION`.
pub const ATTACK_ANIMATION_DURATION: i32 = 10;
/// Vanilla `HoglinBase.PROBABILITY_OF_SPAWNING_AS_BABY`.
pub const PROBABILITY_OF_SPAWNING_AS_BABY: f32 = 0.2;

/// Vanilla `HoglinBase.hurtAndThrowTarget`.
pub fn hurt_and_throw_target(body: &dyn Mob, target: &dyn EntityBase) -> bool {
    let attack_damage = body
        .get_mob_entity()
        .living_entity
        .get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32;
    let is_baby = body.get_entity().is_baby();
    let actual_damage = if !is_baby && attack_damage as i32 > 0 {
        // `level.getRandom().nextInt((int) attackDamage)`.
        let roll = body
            .get_entity()
            .world
            .load()
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_bounded_i32(attack_damage as i32);
        attack_damage / 2.0 + roll as f32
    } else {
        attack_damage
    };

    let attacker: &dyn EntityBase = body;
    let damage_source = DamageSource {
        damage_type: DamageType::MOB_ATTACK,
        direct: Some(attacker),
        causing: Some(attacker),
    };
    let was_hurt = target.damage_with_context(
        target,
        actual_damage,
        damage_source.damage_type,
        None,
        damage_source.direct,
        damage_source.causing,
    );
    if was_hurt {
        EnchantmentHelper::do_post_attack_effects(
            &body.get_entity().world.load_full(),
            target,
            &damage_source,
        );
        if !is_baby {
            throw_target(body, target);
        }
    }
    was_hurt
}

/// Vanilla `HoglinBase.throwTarget`.
pub fn throw_target(body: &dyn Mob, target: &dyn EntityBase) {
    let Some(target_living) = target.get_living_entity() else {
        return;
    };
    let knockback_power = body
        .get_mob_entity()
        .living_entity
        .get_attribute_value(&Attributes::ATTACK_KNOCKBACK);
    let knockback_resistance = target_living.get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE);
    let effective_knockback_power = knockback_power - knockback_resistance;
    if effective_knockback_power <= 0.0 {
        return;
    }

    let body_pos = body.get_entity().pos.load();
    let target_entity = target.get_entity();
    let target_pos = target_entity.pos.load();
    let xd = target_pos.x - body_pos.x;
    let zd = target_pos.z - body_pos.z;
    // `body.level().getRandom()`.
    let (angle_roll, horizontal_roll, vertical_roll) = {
        let world = body.get_entity().world.load();
        let mut random = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            random.next_bounded_i32(21),
            random.next_f32(),
            random.next_f32(),
        )
    };
    // Vanilla passes these degrees to `yRot` as radians.
    let horizontal_push_angle = (angle_roll - 10) as f32;
    let horizontal_scale = effective_knockback_power * f64::from(horizontal_roll * 0.5 + 0.2);
    let horizontal_push_vector =
        (Vector3::new(xd, 0.0, zd).normalize() * horizontal_scale).y_rot(horizontal_push_angle);
    let vertical_scale = effective_knockback_power * f64::from(vertical_roll) * 0.5;
    target.push_impulse(Vector3::new(
        horizontal_push_vector.x,
        vertical_scale,
        horizontal_push_vector.z,
    ));
    target_entity.sync_velocity.store(true, Ordering::SeqCst);
}
