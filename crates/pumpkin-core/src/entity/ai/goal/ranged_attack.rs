use std::sync::{Arc, Weak};

use super::revenge::MobFilter;
use crate::entity::EntityBase;
use crate::entity::ai::goal::{Controls, Goal};
use crate::entity::mob::{Mob, RangedAttackMob};

/// Generic ranged attack goal for mobs implementing [`RangedAttackMob`].
///
/// Mirrors vanilla `RangedAttackGoal`. The mob maintains distance from its target,
/// points towards it, and fires ranged attacks periodically with calculated power.
pub struct RangedAttackGoal {
    mob: Weak<dyn RangedAttackMob>,
    speed_modifier: f64,
    attack_interval_min: i32,
    attack_interval_max: i32,
    attack_radius: f32,
    attack_radius_sqr: f32,
    see_time: i32,
    attack_time: i32,
    target: Option<Arc<dyn EntityBase>>,
    gate: Option<MobFilter>,
}

impl RangedAttackGoal {
    #[must_use]
    pub const fn new(
        mob: Weak<dyn RangedAttackMob>,
        speed_modifier: f64,
        attack_interval: i32,
        attack_radius: f32,
    ) -> Self {
        Self::new_with_range(
            mob,
            speed_modifier,
            attack_interval,
            attack_interval,
            attack_radius,
        )
    }

    #[must_use]
    pub const fn new_with_range(
        mob: Weak<dyn RangedAttackMob>,
        speed_modifier: f64,
        attack_interval_min: i32,
        attack_interval_max: i32,
        attack_radius: f32,
    ) -> Self {
        Self {
            mob,
            speed_modifier,
            attack_interval_min,
            attack_interval_max,
            attack_radius,
            attack_radius_sqr: attack_radius * attack_radius,
            see_time: 0,
            attack_time: -1,
            target: None,
            gate: None,
        }
    }

    /// Extra condition on `canUse`, like vanilla's `DrownedTridentAttackGoal` wanting a
    /// trident. `canContinueToUse` calls the overridden `canUse`, so it applies there too.
    #[must_use]
    pub const fn gated_by(mut self, gate: MobFilter) -> Self {
        self.gate = Some(gate);
        self
    }
}

impl Goal for RangedAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(target) = mob
            .get_target()
            .filter(|target| target.get_entity().is_alive())
        else {
            return false;
        };
        self.target = Some(target);
        self.gate.is_none_or(|gate| gate(mob))
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_start(mob)
            || self
                .target
                .as_ref()
                .is_some_and(|target| target.get_entity().is_alive())
                && !mob.is_navigator_idle()
    }

    fn stop(&mut self, _mob: &dyn Mob) {
        self.target = None;
        self.see_time = 0;
        self.attack_time = -1;
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(target) = self.target.clone() else {
            return;
        };

        let mob_entity = mob.get_mob_entity();
        let living = &mob_entity.living_entity;
        let target_pos = target.get_entity().pos.load();
        let target_dist_sqr = living
            .entity
            .pos
            .load()
            .squared_distance_to_vec(&target_pos);

        let has_line_of_sight = mob.has_line_of_sight(target.get_entity());
        if has_line_of_sight {
            self.see_time += 1;
        } else {
            self.see_time = 0;
        }

        mob_entity.with_navigation(|navigation, owner| {
            let owner = &owner.living_entity;
            if target_dist_sqr <= f64::from(self.attack_radius_sqr) && self.see_time >= 5 {
                navigation.stop();
            } else if let Some(path) = navigation.create_path(owner, target_pos, 1) {
                // Vanilla `moveTo(Entity, speed)` keeps the old path when none is found.
                navigation.move_to_path(Some(path), self.speed_modifier, owner);
            }
        });

        mob_entity
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .look_at_entity_with_range(&target, 30.0, 30.0);

        self.attack_time -= 1;
        if self.attack_time == 0 {
            if !has_line_of_sight {
                return;
            }

            let dist = target_dist_sqr.sqrt() as f32 / self.attack_radius;
            let power = dist.clamp(0.1, 1.0);
            if let Some(ranged) = self.mob.upgrade() {
                ranged.perform_ranged_attack(&target, power);
            }
            self.attack_time = (dist * (self.attack_interval_max - self.attack_interval_min) as f32
                + self.attack_interval_min as f32)
                .floor() as i32;
        } else if self.attack_time < 0 {
            // `Mth.lerp` in doubles, unlike the float maths after an attack.
            let delta = target_dist_sqr.sqrt() / f64::from(self.attack_radius);
            let min = f64::from(self.attack_interval_min);
            self.attack_time =
                (min + delta * (f64::from(self.attack_interval_max) - min)).floor() as i32;
        }
    }
}
