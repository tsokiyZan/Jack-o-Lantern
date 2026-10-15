//! Vanilla `LongJumpUtil`.

use pumpkin_data::entity::EntityPose;
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::vector3::Vector3;

use crate::entity::EntityBase;
use crate::entity::mob::Mob;

/// Vanilla `LongJumpUtil.calculateJumpVectorForAngle`: the launch velocity that lands
/// `body` on `target_pos` at `angle` degrees, if it stays under `max_jump_velocity` and,
/// with `check_collision`, the arc is clear.
#[must_use]
pub fn calculate_jump_vector_for_angle(
    body: &dyn Mob,
    target_pos: Vector3<f64>,
    max_jump_velocity: f32,
    angle: i32,
    check_collision: bool,
) -> Option<Vector3<f64>> {
    let mob_pos = body.get_entity().pos.load();
    let direction_vector_plane =
        Vector3::new(target_pos.x - mob_pos.x, 0.0, target_pos.z - mob_pos.z).normalize() * 0.5;
    let target_position = target_pos - direction_vector_plane;
    let direction_vector = target_position - mob_pos;
    let angrad = angle as f32 * std::f32::consts::PI / 180.0;
    let xz_ang = direction_vector.z.atan2(direction_vector.x);
    let r2 = Vector3::new(direction_vector.x, 0.0, direction_vector.z).length_squared();
    let r = r2.sqrt();
    let y = direction_vector.y;
    let g = body.current_gravity();
    let sin2ang = f64::from(2.0 * angrad).sin();
    let cosangsqr = f64::from(angrad).cos().powi(2);
    let sinangrad = f64::from(angrad).sin();
    let cosangrad = f64::from(angrad).cos();
    let sinxz_ang = xz_ang.sin();
    let cosxz_ang = xz_ang.cos();
    let v0sqr = r2 * g / (r * sin2ang - 2.0 * y * cosangsqr);
    if v0sqr < 0.0 {
        return None;
    }

    let v0 = v0sqr.sqrt();
    if v0 > f64::from(max_jump_velocity) {
        return None;
    }

    let v0r = v0 * cosangrad;
    let v0y = v0 * sinangrad;
    if check_collision {
        let samples = (r / v0r).ceil() as i32 * 2;
        let mut ri = 0.0;
        let mut previous_pos: Option<Vector3<f64>> = None;
        let mob_dimensions = body.dimensions_for_pose(EntityPose::LongJumping);

        for _ in 0..samples - 1 {
            ri += r / f64::from(samples);
            let yi =
                sinangrad / cosangrad * ri - ri.powi(2) * g / (2.0 * v0sqr * cosangrad.powi(2));
            let xi = ri * cosxz_ang;
            let zi = ri * sinxz_ang;
            let sample_pos = Vector3::new(mob_pos.x + xi, mob_pos.y + yi, mob_pos.z + zi);
            if let Some(previous_pos) = previous_pos
                && !is_clear_transition(body, &mob_dimensions, previous_pos, sample_pos)
            {
                return None;
            }

            previous_pos = Some(sample_pos);
        }
    }

    Some(Vector3::new(v0r * cosxz_ang, v0y, v0r * sinxz_ang) * f64::from(0.95f32))
}

/// Vanilla `LongJumpUtil.isClearTransition`.
fn is_clear_transition(
    body: &dyn Mob,
    entity_dimensions: &EntityDimensions,
    position1: Vector3<f64>,
    position2: Vector3<f64>,
) -> bool {
    let direction = position2 - position1;
    let min_dimension = f64::from(entity_dimensions.width.min(entity_dimensions.height));
    let checks = (direction.length() / min_dimension).ceil() as i32;
    let normalized_direction = direction.normalize();
    let mut next_point_to_check = position1;

    for i in 0..checks {
        next_point_to_check = if i == checks - 1 {
            position2
        } else {
            next_point_to_check + normalized_direction * (min_dimension * f64::from(0.9f32))
        };
        let bounding_box = BoundingBox::new_from_pos(
            next_point_to_check.x,
            next_point_to_check.y,
            next_point_to_check.z,
            entity_dimensions,
        );
        if !no_collision(body, bounding_box) {
            return false;
        }
    }

    true
}

/// Vanilla `CollisionGetter.noCollision(entity, box)` over blocks; Pumpkin entities have no
/// hard entity collisions.
fn no_collision(body: &dyn Mob, bounding_box: BoundingBox) -> bool {
    let world = body.get_entity().world.load();
    world
        .get_block_collisions(bounding_box, body as &dyn EntityBase)
        .0
        .is_empty()
}
