use crate::entity::ai::control::move_control::Operation;
use crate::entity::ai::control::{Control, MoveControlTrait};
use crate::entity::mob::Mob;
use pumpkin_data::attributes::Attributes;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::wrap_degrees;

pub struct SmoothSwimmingMoveControl {
    pub wanted_x: f64,
    pub wanted_y: f64,
    pub wanted_z: f64,
    pub speed_modifier: f64,
    pub max_turn_x: i32,
    pub max_turn_y: i32,
    pub in_water_speed_modifier: f32,
    pub outside_water_speed_modifier: f32,
    pub apply_gravity: bool,
    pub operation: Operation,
}

impl SmoothSwimmingMoveControl {
    #[must_use]
    pub const fn new(
        max_turn_x: i32,
        max_turn_y: i32,
        in_water_speed_modifier: f32,
        outside_water_speed_modifier: f32,
        apply_gravity: bool,
    ) -> Self {
        Self {
            wanted_x: 0.0,
            wanted_y: 0.0,
            wanted_z: 0.0,
            speed_modifier: 0.0,
            max_turn_x,
            max_turn_y,
            in_water_speed_modifier,
            outside_water_speed_modifier,
            apply_gravity,
            operation: Operation::Wait,
        }
    }

    /// Vanilla `MoveControl.rotlerp`, which keeps the result within 0 to 360.
    fn rotlerp(a: f32, b: f32, max: f32) -> f32 {
        let result = a + wrap_degrees(b - a).clamp(-max, max);
        if result < 0.0 {
            result + 360.0
        } else if result > 360.0 {
            result - 360.0
        } else {
            result
        }
    }

    fn get_turning_speed_factor(left_to_turn: f32) -> f32 {
        1.0 - ((left_to_turn - 10.0) / 50.0).clamp(0.0, 1.0)
    }
}

impl Control for SmoothSwimmingMoveControl {}

impl MoveControlTrait for SmoothSwimmingMoveControl {
    /// Vanilla `SmoothSwimmingMoveControl.tick`. Pumpkin has no separate mob speed:
    /// in water the input carries vanilla's `zza`/`yya`, whose length is the speed
    /// before `inWaterSpeedModifier`; out of water its forward part is the speed.
    fn tick(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        let living_entity = &mob_entity.living_entity;
        let entity = &living_entity.entity;

        if self.apply_gravity && entity.is_in_water() {
            // A plain store: the tracker syncs motion, as vanilla's setDeltaMovement
            // leaves to it.
            let vel = entity.velocity.load();
            entity
                .velocity
                .store(Vector3::new(vel.x, vel.y + 0.005, vel.z));
        }

        let navigation_done = mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_done();
        if self.operation != Operation::MoveTo || navigation_done {
            living_entity.speed.store(0.0);
            living_entity
                .movement_input
                .store(Vector3::new(0.0, 0.0, 0.0));
            return;
        }

        let pos = entity.pos.load();
        let xd = self.wanted_x - pos.x;
        let yd = self.wanted_y - pos.y;
        let zd = self.wanted_z - pos.z;
        let dd = xd * xd + yd * yd + zd * zd;
        if dd < f64::from(2.500_000_3E-7f32) {
            let input = living_entity.movement_input.load();
            living_entity
                .movement_input
                .store(Vector3::new(input.x, input.y, 0.0));
            return;
        }

        let y_rot_d = (zd.atan2(xd).to_degrees() as f32) - 90.0;
        let new_yaw = Self::rotlerp(entity.yaw.load(), y_rot_d, self.max_turn_y as f32);
        entity.yaw.store(new_yaw);
        entity.body_yaw.store(new_yaw);
        entity.head_yaw.store(new_yaw);

        let movement_speed = living_entity.get_attribute_value(&Attributes::MOVEMENT_SPEED);
        let speed = (self.speed_modifier * movement_speed) as f32;

        if entity.is_in_water() {
            let sqrt = xd.hypot(zd);
            if yd.abs() > 1.0E-5 || sqrt.abs() > 1.0E-5 {
                let x_rot_d = wrap_degrees(-(yd.atan2(sqrt).to_degrees() as f32))
                    .clamp(-(self.max_turn_x as f32), self.max_turn_x as f32);
                entity
                    .pitch
                    .store(self.change_angle(entity.pitch.load(), x_rot_d, 5.0));
            }

            living_entity
                .speed
                .store(speed * self.in_water_speed_modifier);
            let pitch = entity.pitch.load().to_radians();
            living_entity.movement_input.store(Vector3::new(
                0.0,
                f64::from(-pitch.sin() * speed),
                f64::from(pitch.cos() * speed),
            ));
        } else {
            let left_to_turn = wrap_degrees(entity.yaw.load() - y_rot_d).abs();
            let factor = Self::get_turning_speed_factor(left_to_turn);
            mob_entity.set_speed(speed * self.outside_water_speed_modifier * factor);
        }
    }

    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed_modifier: f64) {
        self.wanted_x = x;
        self.wanted_y = y;
        self.wanted_z = z;
        self.speed_modifier = speed_modifier;
        self.operation = Operation::MoveTo;
    }

    fn has_wanted(&self) -> bool {
        self.operation == Operation::MoveTo
    }
}
