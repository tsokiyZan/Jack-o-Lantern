use crate::entity::ai::control::move_control::Operation;
use crate::entity::ai::control::{Control, MoveControlTrait, rotlerp};
use crate::entity::mob::Mob;
use pumpkin_data::attributes::Attributes;
use pumpkin_util::math::{self, vector3::Vector3};

/// Vanilla `AbstractFish.FishMoveControl`: eases the speed in, swims up or down towards the
/// wanted position, and keeps `MOVE_TO` until the navigation is done.
#[derive(Default)]
pub struct FishMoveControl {
    wanted_x: f64,
    wanted_y: f64,
    wanted_z: f64,
    speed_modifier: f64,
    operation: Operation,
}

impl Control for FishMoveControl {}

impl MoveControlTrait for FishMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        let living = &mob_entity.living_entity;
        let entity = &living.entity;
        if entity.is_submerged_in_water() {
            let velocity = entity.velocity.load();
            entity
                .velocity
                .store(Vector3::new(velocity.x, velocity.y + 0.005, velocity.z));
        }

        let navigation_done = mob_entity.own_navigation().is_done();
        if self.operation == Operation::MoveTo && !navigation_done {
            let target_speed = (self.speed_modifier
                * living.get_attribute_value(&Attributes::MOVEMENT_SPEED))
                as f32;
            let speed = living.speed.load();
            mob_entity.set_speed(speed + 0.125 * (target_speed - speed));
            let pos = entity.pos.load();
            let xd = self.wanted_x - pos.x;
            let yd = self.wanted_y - pos.y;
            let zd = self.wanted_z - pos.z;
            if yd != 0.0 {
                let dd = (xd * xd + yd * yd + zd * zd).sqrt();
                let velocity = entity.velocity.load();
                entity.velocity.store(Vector3::new(
                    velocity.x,
                    velocity.y + f64::from(living.speed.load()) * (yd / dd) * 0.1,
                    velocity.z,
                ));
            }
            if xd != 0.0 || zd != 0.0 {
                let y_rot_d = (math::atan2(zd, xd) * f64::from(180.0f32)
                    / f64::from(std::f32::consts::PI)) as f32
                    - 90.0;
                let yaw = rotlerp(entity.yaw.load(), y_rot_d, 90.0);
                entity.yaw.store(yaw);
                entity.body_yaw.store(yaw);
            }
        } else {
            mob_entity.set_speed(0.0);
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
}
