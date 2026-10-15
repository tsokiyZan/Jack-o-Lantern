//! Vanilla `Slide`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use pumpkin_data::entity::EntityPose;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use super::breeze_ai::{
    JUMP_CIRCLE_INNER_RADIUS, JUMP_CIRCLE_MIDDLE_RADIUS, SPEED_MULTIPLIER_WHEN_SLIDING,
};
use super::breeze_util::{has_line_of_sight, random_point_behind_target};
use super::{pose, within_inner_circle_range};
use crate::entity::EntityBase;
use crate::entity::ai::brain::BrainTick;
use crate::entity::ai::brain::behavior::Behavior;
use crate::entity::ai::brain::memory::walk_target::WalkTarget;
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, types};
use crate::entity::ai::util::default_random_pos;
use crate::entity::mob::Mob;

const ENTRY_CONDITIONS: [(MemoryModuleId, MemoryStatus); 4] = [
    (types::ATTACK_TARGET.id(), MemoryStatus::ValuePresent),
    (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
    (types::BREEZE_JUMP_COOLDOWN.id(), MemoryStatus::ValueAbsent),
    (types::BREEZE_SHOOT.id(), MemoryStatus::ValueAbsent),
];

/// Vanilla `Slide`: picks a spot to slide to, away from a close target or around it.
pub struct Slide;

impl Slide {
    /// Vanilla `Slide.randomPointInMiddleCircle`.
    fn random_point_in_middle_circle(breeze: &dyn Mob, enemy: &dyn EntityBase) -> Vector3<f64> {
        let breeze_pos = breeze.get_entity().pos.load();
        let direction = enemy.get_entity().pos.load() - breeze_pos;
        let distance = direction.length()
            - pumpkin_util::math::lerp(
                breeze.get_random().random::<f64>(),
                f64::from(JUMP_CIRCLE_MIDDLE_RADIUS),
                f64::from(JUMP_CIRCLE_INNER_RADIUS),
            );
        breeze_pos + direction.normalize() * distance
    }
}

impl Behavior for Slide {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &ENTRY_CONDITIONS
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        let entity = tick.mob.get_entity();
        entity.on_ground.load(Ordering::Relaxed)
            && !entity.is_in_water()
            && pose(tick) == EntityPose::Standing
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        let Some(enemy) = tick.brain.get(types::ATTACK_TARGET).map(Arc::clone) else {
            return;
        };
        let breeze = tick.mob;
        let breeze_pos = breeze.get_entity().pos.load();
        let enemy_pos = enemy.get_entity().pos.load();
        let position = if within_inner_circle_range(breeze.get_entity(), enemy_pos) {
            default_random_pos::get_pos_away(breeze, 5, 5, enemy_pos).filter(|away| {
                has_line_of_sight(breeze, *away)
                    && enemy_pos.squared_distance_to_vec(away)
                        > enemy_pos.squared_distance_to_vec(&breeze_pos)
            })
        } else {
            None
        };
        let position = position.unwrap_or_else(|| {
            if breeze.get_random().random::<bool>() {
                random_point_behind_target(enemy.as_ref(), &mut breeze.get_random())
            } else {
                Self::random_point_in_middle_circle(breeze, enemy.as_ref())
            }
        });
        tick.brain.set(
            types::WALK_TARGET,
            WalkTarget::from_vec(position, SPEED_MULTIPLIER_WHEN_SLIDING, 1),
        );
    }

    fn debug_name(&self) -> &'static str {
        "Slide"
    }
}
