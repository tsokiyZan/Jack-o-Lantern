use crate::entity::mob::Mob;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::{subtract_angles, wrap_degrees};
use std::any::Any;

pub mod body_rotation_control;
pub mod fish_move_control;
pub mod flying_move_control;
pub mod guardian_move_control;
pub mod jump_control;
pub mod look_control;
pub mod move_control;
pub mod smooth_swimming_look_control;
pub mod smooth_swimming_move_control;

/// Vanilla `MoveControl.rotlerp`: turns `a` towards `b` by at most `max`, wrapped into 0..360.
#[must_use]
pub fn rotlerp(a: f32, b: f32, max: f32) -> f32 {
    let result = a + wrap_degrees(b - a).clamp(-max, max);
    if result < 0.0 {
        result + 360.0
    } else if result > 360.0 {
        result - 360.0
    } else {
        result
    }
}

pub trait Control: Send + Sync {
    fn change_angle(&self, start: f32, end: f32, max_change: f32) -> f32 {
        let i = subtract_angles(start, end);
        let j = i.clamp(-max_change, max_change);
        start + j
    }
}

/// `Any` lets a mob reach its own control's extra state, as vanilla casts `mob.moveControl`.
pub trait MoveControlTrait: Control + Any {
    fn tick(&mut self, mob: &dyn Mob);

    fn set_wanted_position(&mut self, _x: f64, _y: f64, _z: f64, _speed_modifier: f64) {}

    fn strafe(&mut self, _forward: f32, _right: f32) {}

    fn has_wanted(&self) -> bool {
        false
    }

    /// Vanilla `getWantedX`/`Y`/`Z`; only the controls whose callers read it keep it.
    fn wanted_position(&self) -> Vector3<f64> {
        Vector3::new(0.0, 0.0, 0.0)
    }

    /// Vanilla `MoveControl.setWait`. Left empty only by a control that never reads its
    /// `operation` (the phantom's).
    fn set_wait(&mut self) {}
}
