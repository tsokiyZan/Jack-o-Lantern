//! Vanilla `BreezeUtil`.

use pumpkin_data::attributes::Attributes;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::clip::{ClipBlock, ClipContext, ClipFluid};
use rand::{Rng, RngExt};

use super::breeze_ai::{JUMP_CIRCLE_INNER_RADIUS, JUMP_CIRCLE_MIDDLE_RADIUS};
use crate::entity::mob::Mob;
use crate::entity::{Entity, EntityBase};

/// Vanilla `BreezeUtil.MAX_LINE_OF_SIGHT_TEST_RANGE`.
const MAX_LINE_OF_SIGHT_TEST_RANGE: f64 = 50.0;
/// The `spreadDegrees` of vanilla `BreezeUtil.randomPointBehindTarget`.
const SPREAD_DEGREES: f32 = 90.0;

/// Vanilla `RandomSource.nextGaussian`, the polar method of `MarsagliaPolarGaussian`.
fn next_gaussian(random: &mut impl Rng) -> f64 {
    loop {
        let x = 2.0 * random.random::<f64>() - 1.0;
        let y = 2.0 * random.random::<f64>() - 1.0;
        let s = x * x + y * y;
        if s < 1.0 && s != 0.0 {
            return x * (-2.0 * s.ln() / s).sqrt();
        }
    }
}

/// Vanilla `BreezeUtil.randomPointBehindTarget`: four to eight blocks behind where `enemy`
/// looks, spread around its back.
#[must_use]
pub fn random_point_behind_target(enemy: &dyn EntityBase, random: &mut impl Rng) -> Vector3<f64> {
    let entity = enemy.get_entity();
    let view_angle =
        entity.head_yaw.load() + 180.0 + next_gaussian(random) as f32 * SPREAD_DEGREES / 2.0;
    let r = pumpkin_util::math::lerp(
        random.random::<f32>(),
        JUMP_CIRCLE_INNER_RADIUS,
        JUMP_CIRCLE_MIDDLE_RADIUS,
    );
    entity.pos.load() + Entity::direction_from_rotation(0.0, view_angle) * f64::from(r)
}

/// Vanilla `BreezeUtil.hasLineOfSight`: no block collider between the breeze's feet and
/// `target`, which must be within the test range.
#[must_use]
pub fn has_line_of_sight(breeze: &dyn Mob, target: Vector3<f64>) -> bool {
    let from = breeze.get_entity().pos.load();
    if target.sub(&from).length() > get_max_line_of_sight_test_range(breeze) {
        return false;
    }
    breeze
        .get_entity()
        .world
        .load()
        .clip(&ClipContext::new(
            from,
            target,
            ClipBlock::Collider,
            ClipFluid::None,
        ))
        .is_miss
}

/// Vanilla `BreezeUtil.getMaxLineOfSightTestRange`.
fn get_max_line_of_sight_test_range(breeze: &dyn Mob) -> f64 {
    MAX_LINE_OF_SIGHT_TEST_RANGE.max(
        breeze
            .get_mob_entity()
            .living_entity
            .get_attribute_value(&Attributes::FOLLOW_RANGE),
    )
}
