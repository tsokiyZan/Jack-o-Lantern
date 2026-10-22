use crate::entity::ai::control::move_control::Operation;
use crate::entity::ai::control::{Control, MoveControlTrait, rotlerp};
use crate::entity::mob::Mob;
use crate::entity::mob::guardian::guardian_data;
use pumpkin_data::attributes::Attributes;
use pumpkin_util::math::{self, vector3::Vector3};

/// Vanilla `Guardian.GuardianMoveControl`: swims straight at the wanted position with a
/// wobble, eases the head towards where it is going, and flags the guardian as moving.
#[derive(Default)]
pub struct GuardianMoveControl {
    wanted_x: f64,
    wanted_y: f64,
    wanted_z: f64,
    speed_modifier: f64,
    operation: Operation,
}

impl Control for GuardianMoveControl {}

impl MoveControlTrait for GuardianMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let Some(guardian) = guardian_data(mob) else {
            return;
        };
        let mob_entity = mob.get_mob_entity();
        let living = &mob_entity.living_entity;
        let entity = &living.entity;
        let navigation_done = mob_entity.with_navigation(|navigation, _| navigation.is_done());
        if self.operation != Operation::MoveTo || navigation_done {
            mob_entity.set_speed(0.0);
            guardian.set_moving(entity, false);
            return;
        }

        let pos = entity.pos.load();
        let delta = Vector3::new(
            self.wanted_x - pos.x,
            self.wanted_y - pos.y,
            self.wanted_z - pos.z,
        );
        let length = delta.length();
        let xd = delta.x / length;
        let yd = delta.y / length;
        let zd = delta.z / length;
        let y_rot_d = (math::atan2(delta.z, delta.x) * f64::from(180.0f32)
            / f64::from(std::f32::consts::PI)) as f32
            - 90.0;
        let yaw = rotlerp(entity.yaw.load(), y_rot_d, 90.0);
        entity.yaw.store(yaw);
        entity.body_yaw.store(yaw);

        let target_speed =
            (self.speed_modifier * living.get_attribute_value(&Attributes::MOVEMENT_SPEED)) as f32;
        let speed = living.speed.load();
        let new_speed = speed + 0.125 * (target_speed - speed);
        mob_entity.set_speed(new_speed);

        let wobble = f64::from(
            entity
                .tick_count
                .load(std::sync::atomic::Ordering::Relaxed)
                .wrapping_add(entity.entity_id),
        );
        let push = (wobble * 0.5).sin() * 0.05;
        let yaw_radians = f64::from(yaw * (std::f64::consts::PI / 180.0) as f32);
        let cos = yaw_radians.cos();
        let sin = yaw_radians.sin();
        let y_push = (wobble * 0.75).sin() * 0.05;
        let velocity = entity.velocity.load();
        entity.velocity.store(Vector3::new(
            velocity.x + push * cos,
            velocity.y + (y_push * (sin + cos) * 0.25 + f64::from(new_speed) * yd * 0.1),
            velocity.z + push * sin,
        ));

        let new_look = Vector3::new(
            pos.x + xd * 2.0,
            entity.get_eye_y() + yd / length,
            pos.z + zd * 2.0,
        );
        let mut look_control = mob_entity
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let old_look = if look_control.is_looking_at_target() {
            look_control.wanted_position()
        } else {
            new_look
        };
        look_control.look_at_with_range(
            old_look.x + 0.125 * (new_look.x - old_look.x),
            old_look.y + 0.125 * (new_look.y - old_look.y),
            old_look.z + 0.125 * (new_look.z - old_look.z),
            10.0,
            40.0,
        );
        drop(look_control);
        guardian.set_moving(entity, true);
    }

    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed_modifier: f64) {
        self.wanted_x = x;
        self.wanted_y = y;
        self.wanted_z = z;
        self.speed_modifier = speed_modifier;
        if self.operation != Operation::Jumping {
            self.operation = Operation::MoveTo;
        }
    }

    fn has_wanted(&self) -> bool {
        self.operation == Operation::MoveTo
    }

    /// Vanilla `MoveControl.setWait`.
    fn set_wait(&mut self) {
        self.operation = Operation::Wait;
    }
}
