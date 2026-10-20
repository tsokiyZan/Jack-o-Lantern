use super::{Controls, Goal, to_goal_ticks};
use crate::entity::EntityBase;
use crate::entity::mob::Mob;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub struct LeapAtTargetGoal {
    goal_control: Controls,
    yd: f32,
    target: Option<Arc<dyn EntityBase>>,
}

impl LeapAtTargetGoal {
    #[must_use]
    pub const fn new(yd: f32) -> Self {
        Self {
            goal_control: Controls::MOVE.union(Controls::JUMP),
            yd,
            target: None,
        }
    }
}

impl Goal for LeapAtTargetGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let mob_entity = mob.get_mob_entity();
        let entity = &mob_entity.living_entity.entity;

        // `hasControllingPassenger`: a ridden spider (a jockey) doesn't leap.
        if !mob_entity.is_no_ai()
            && entity.get_first_passenger().is_some_and(|passenger| {
                passenger.get_mob().is_some()
                    && !passenger
                        .get_entity()
                        .entity_type
                        .has_tag(&tag::EntityType::MINECRAFT_NON_CONTROLLING_RIDER)
            })
        {
            return false;
        }

        // Vanilla keeps the target even when the leap is refused.
        self.target = mob_entity.get_target();
        let Some(target) = &self.target else {
            return false;
        };

        let mob_pos = entity.pos.load();
        let target_pos = target.get_entity().pos.load();
        let dist_sq = mob_pos.squared_distance_to_vec(&target_pos);
        if !(4.0..=16.0).contains(&dist_sq) {
            return false;
        }

        entity.on_ground.load(Ordering::Relaxed)
            && mob.get_entity().random().next_bounded_i32(to_goal_ticks(5)) == 0
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let entity = &mob.get_mob_entity().living_entity.entity;
        !entity.on_ground.load(Ordering::Relaxed)
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(target) = &self.target else {
            return;
        };

        let mob_entity = mob.get_mob_entity();
        let entity = &mob_entity.living_entity.entity;
        let mob_pos = entity.pos.load();
        let target_pos = target.get_entity().pos.load();
        let movement = entity.velocity.load();

        let dx = target_pos.x - mob_pos.x;
        let dz = target_pos.z - mob_pos.z;
        let dist_xz_sq = dx * dx + dz * dz;

        // `delta.normalize().scale(0.4).add(movement.scale(0.2))`; a tiny delta stays as is.
        let (x, z) = if dist_xz_sq > 1.0E-7 {
            let dist_xz = dist_xz_sq.sqrt();
            (
                dx / dist_xz * 0.4 + movement.x * 0.2,
                dz / dist_xz * 0.4 + movement.z * 0.2,
            )
        } else {
            (dx, dz)
        };

        entity.set_velocity(Vector3::new(x, f64::from(self.yd), z));
    }

    fn stop(&mut self, _mob: &dyn Mob) {
        self.target = None;
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
