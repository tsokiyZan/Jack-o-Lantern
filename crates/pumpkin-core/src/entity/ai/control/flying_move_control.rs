use crate::entity::ai::control::move_control::Operation;
use crate::entity::ai::control::{Control, MoveControlTrait, rotlerp};
use crate::entity::mob::Mob;
use pumpkin_data::attributes::Attributes;
use pumpkin_util::math;
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering;

pub struct FlyingMoveControl {
    pub wanted_x: f64,
    pub wanted_y: f64,
    pub wanted_z: f64,
    pub speed_modifier: f64,
    pub max_turn: i32,
    pub hovers_in_place: bool,
    pub operation: Operation,
}

impl FlyingMoveControl {
    #[must_use]
    pub const fn new(max_turn: i32, hovers_in_place: bool) -> Self {
        Self {
            wanted_x: 0.0,
            wanted_y: 0.0,
            wanted_z: 0.0,
            speed_modifier: 0.0,
            max_turn,
            hovers_in_place,
            operation: Operation::Wait,
        }
    }
}

impl Control for FlyingMoveControl {}

impl MoveControlTrait for FlyingMoveControl {
    /// Vanilla `FlyingMoveControl.tick`.
    fn tick(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        let living_entity = &mob_entity.living_entity;
        let entity = &living_entity.entity;
        let set_yya_zza = |yya: f32, zza: f32| {
            let input = living_entity.movement_input.load();
            living_entity.movement_input.store(Vector3::new(
                input.x,
                f64::from(yya),
                f64::from(zza),
            ));
        };

        if self.operation != Operation::MoveTo {
            if !self.hovers_in_place {
                entity.set_has_no_gravity(false);
            }
            set_yya_zza(0.0, 0.0);
            return;
        }
        self.operation = Operation::Wait;
        entity.set_has_no_gravity(true);
        let pos = entity.pos.load();
        let xd = self.wanted_x - pos.x;
        let yd = self.wanted_y - pos.y;
        let zd = self.wanted_z - pos.z;
        let dd = xd * xd + yd * yd + zd * zd;
        if dd < f64::from(2.500_000_3E-7f32) {
            set_yya_zza(0.0, 0.0);
            return;
        }

        let to_degrees =
            |radians: f64| radians * f64::from(180.0f32) / f64::from(std::f32::consts::PI);
        let y_rot_d = to_degrees(math::atan2(zd, xd)) as f32 - 90.0;
        entity.yaw.store(rotlerp(entity.yaw.load(), y_rot_d, 90.0));
        let speed_attribute = if entity.on_ground.load(Ordering::Relaxed) {
            &Attributes::MOVEMENT_SPEED
        } else {
            &Attributes::FLYING_SPEED
        };
        let speed =
            (self.speed_modifier * living_entity.get_attribute_value(speed_attribute)) as f32;
        mob_entity.set_speed(speed);
        #[expect(
            clippy::imprecise_flops,
            reason = "vanilla rounds Math.sqrt(xd * xd + zd * zd), not a hypot"
        )]
        let sd = (xd * xd + zd * zd).sqrt();
        let epsilon = f64::from(1.0E-5f32);
        if yd.abs() > epsilon || sd.abs() > epsilon {
            let x_rot_d = (-to_degrees(math::atan2(yd, sd))) as f32;
            entity.set_pitch(rotlerp(entity.pitch.load(), x_rot_d, self.max_turn as f32));
            let input = living_entity.movement_input.load();
            living_entity.movement_input.store(Vector3::new(
                input.x,
                f64::from(if yd > 0.0 { speed } else { -speed }),
                input.z,
            ));
        }
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
