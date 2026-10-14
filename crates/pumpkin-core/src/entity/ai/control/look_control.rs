use crate::entity::EntityBase;
use crate::entity::ai::control::Control;
use crate::entity::mob::{Mob, MobEntity};
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::{clamp_angle, wrap_degrees};
use std::sync::Arc;

/// Vanilla `SmoothSwimmingLookControl.HEAD_TILT_X`.
const SMOOTH_SWIMMING_HEAD_TILT_X: f32 = 10.0;
/// Vanilla `SmoothSwimmingLookControl.HEAD_TILT_Y`.
const SMOOTH_SWIMMING_HEAD_TILT_Y: f32 = 20.0;

// Please keep the atomic values out of here!!!
#[derive(Default)]
pub struct LookControl {
    max_yaw_change: f32,
    max_pitch_change: f32,
    look_at_timer: i32,
    position: Vector3<f64>,
    /// Set for vanilla's `SmoothSwimmingLookControl`: its `maxYRotFromCenter`.
    smooth_swimming_max_y_rot_from_center: Option<f32>,
}

impl Control for LookControl {}

impl LookControl {
    /// Vanilla `SmoothSwimmingLookControl`: the head leads the body while the pitch
    /// is left to the swimming, easing back level once the mob stops.
    #[must_use]
    pub fn smooth_swimming(max_y_rot_from_center: f32) -> Self {
        Self {
            smooth_swimming_max_y_rot_from_center: Some(max_y_rot_from_center),
            ..Self::default()
        }
    }

    pub fn look_at_position(&mut self, mob: &dyn Mob, position: Vector3<f64>) {
        self.look_at(mob, position.x, position.y, position.z);
    }

    pub fn look_at_entity(&mut self, mob: &dyn Mob, entity: &Arc<dyn EntityBase>) {
        let entity = entity.get_entity();
        let pos = entity.get_eye_pos();
        self.look_at(mob, pos.x, pos.y, pos.z);
    }

    pub fn look_at_entity_with_range(
        &mut self,
        entity: &Arc<dyn EntityBase>,
        max_yaw_change: f32,
        max_pitch_change: f32,
    ) {
        let entity = entity.get_entity();
        let pos = entity.pos.load();
        self.look_at_with_range(
            pos.x,
            entity.get_eye_y(),
            pos.z,
            max_yaw_change,
            max_pitch_change,
        );
    }

    pub fn look_at(&mut self, mob: &dyn Mob, x: f64, y: f64, z: f64) {
        self.look_at_with_range(
            x,
            y,
            z,
            mob.get_max_look_yaw_change(),
            mob.get_max_look_pitch_change(),
        );
    }

    pub const fn look_at_with_range(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        max_yaw_change: f32,
        max_pitch_change: f32,
    ) {
        self.position = Vector3::new(x, y, z);
        self.max_yaw_change = max_yaw_change;
        self.max_pitch_change = max_pitch_change;
        self.look_at_timer = 2;
    }

    pub fn tick(&mut self, mob: &dyn Mob) {
        if let Some(max_y_rot_from_center) = self.smooth_swimming_max_y_rot_from_center {
            self.tick_smooth_swimming(mob, max_y_rot_from_center);
            return;
        }
        let entity = mob.get_entity();
        entity.set_pitch(0.0);

        if self.look_at_timer > 0 {
            self.look_at_timer -= 1;
            if let Some(yaw) = self.get_target_yaw(mob.get_mob_entity()) {
                entity.head_yaw.store(self.change_angle(
                    entity.head_yaw.load(),
                    yaw,
                    self.max_yaw_change,
                ));
            }
            if let Some(pitch) = self.get_target_pitch(mob.get_mob_entity()) {
                entity.set_pitch(self.change_angle(
                    entity.pitch.load(),
                    pitch,
                    self.max_pitch_change,
                ));
            }
        } else {
            entity.head_yaw.store(self.change_angle(
                entity.head_yaw.load(),
                entity.body_yaw.load(),
                10.0,
            ));
        }

        Self::clamp_head_yaw(mob);
    }

    /// Vanilla `SmoothSwimmingLookControl.tick`.
    fn tick_smooth_swimming(&mut self, mob: &dyn Mob, max_y_rot_from_center: f32) {
        let mob_entity = mob.get_mob_entity();
        let entity = &mob_entity.living_entity.entity;
        if self.look_at_timer > 0 {
            self.look_at_timer -= 1;
            if let Some(yaw) = self.get_target_yaw(mob_entity) {
                entity.head_yaw.store(self.change_angle(
                    entity.head_yaw.load(),
                    yaw + SMOOTH_SWIMMING_HEAD_TILT_Y,
                    self.max_yaw_change,
                ));
            }
            if let Some(pitch) = self.get_target_pitch(mob_entity) {
                entity.set_pitch(self.change_angle(
                    entity.pitch.load(),
                    pitch + SMOOTH_SWIMMING_HEAD_TILT_X,
                    self.max_pitch_change,
                ));
            }
        } else {
            let navigation_done = mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_done();
            if navigation_done {
                entity.set_pitch(self.change_angle(entity.pitch.load(), 0.0, 5.0));
            }
            entity.head_yaw.store(self.change_angle(
                entity.head_yaw.load(),
                entity.body_yaw.load(),
                self.max_yaw_change,
            ));
        }

        let head_diff_body = wrap_degrees(entity.head_yaw.load() - entity.body_yaw.load());
        if head_diff_body < -max_y_rot_from_center {
            entity.body_yaw.store(entity.body_yaw.load() - 4.0);
        } else if head_diff_body > max_y_rot_from_center {
            entity.body_yaw.store(entity.body_yaw.load() + 4.0);
        }
    }

    fn clamp_head_yaw(mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        if let Ok(navigator) = mob_entity.navigator.try_lock()
            && navigator.is_in_progress()
        {
            let entity = &mob_entity.living_entity.entity;
            let max_head_rotation = mob.get_max_head_rotation();
            entity.head_yaw.store(clamp_angle(
                entity.head_yaw.load(),
                entity.body_yaw.load(),
                max_head_rotation,
            ));
        }
    }

    fn get_target_pitch(&self, mob: &MobEntity) -> Option<f32> {
        let position = self.position;
        let mob_position = mob.living_entity.entity.pos.load();
        let d = position.x - mob_position.x;
        let e = position.y - mob.living_entity.entity.get_eye_y();
        let f = position.z - mob_position.z;
        let g = d.hypot(f);
        if e.abs() <= 1.0E-5 && g.abs() <= 1.0E-5 {
            None
        } else {
            Some(-(e.atan2(g) as f32).to_degrees())
        }
    }

    fn get_target_yaw(&self, mob: &MobEntity) -> Option<f32> {
        let position = self.position;
        let mob_position = mob.living_entity.entity.pos.load();
        let d = position.x - mob_position.x;
        let e = position.z - mob_position.z;
        if e.abs() <= 1.0E-5 && d.abs() <= 1.0E-5 {
            None
        } else {
            Some((e.atan2(d) as f32).to_degrees() - 90.0)
        }
    }
}
