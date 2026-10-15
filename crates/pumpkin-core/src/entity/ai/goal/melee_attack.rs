use super::{Controls, Goal};

use crate::entity::EntityBase;
use crate::entity::ai::goal::revenge::MobFilter;
use crate::entity::ai::pathfinder::path::Path;
use crate::entity::mob::Mob;
use crate::entity::predicate::EntityPredicate;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

const CAN_USE_CHECK_INTERVAL: i64 = 20;

/// A vanilla `checkAndPerformAttack` override: the hit that replaces swinging and hurting the
/// target once the attack is ready.
pub type MeleeAttackFn = fn(&dyn Mob, &dyn EntityBase);

pub struct MeleeAttackGoal {
    goal_control: Controls,
    speed: f64,
    pause_when_mob_idle: bool,
    path: Option<Path>,
    pathed_target: Vector3<f64>,
    update_countdown_ticks: i32,
    pub cooldown: i32,
    last_can_use_check: i64,
    gate: Option<MobFilter>,
    attack: Option<MeleeAttackFn>,
}

impl MeleeAttackGoal {
    #[must_use]
    pub fn new(speed: f64, pause_when_mob_idle: bool) -> Self {
        Self {
            goal_control: Controls::MOVE | Controls::LOOK,
            speed,
            pause_when_mob_idle,
            path: None,
            pathed_target: Vector3::new(0.0, 0.0, 0.0),
            update_countdown_ticks: 0,
            cooldown: 0,
            last_can_use_check: 0,
            gate: None,
            attack: None,
        }
    }

    /// Hits with `attack` instead of swinging and hurting the target, like vanilla's
    /// `FoxMeleeAttackGoal`.
    #[must_use]
    pub const fn attacking_with(mut self, attack: MeleeAttackFn) -> Self {
        self.attack = Some(attack);
        self
    }

    /// Extra condition for starting and for continuing, checked after the base goal's own.
    #[must_use]
    pub const fn gated_by(mut self, gate: MobFilter) -> Self {
        self.gate = Some(gate);
        self
    }

    #[must_use]
    pub fn get_max_cooldown(&self) -> i32 {
        self.get_tick_count(20)
    }

    fn base_should_continue(&self, mob: &dyn Mob) -> bool {
        let target = mob.get_mob_entity().get_target().clone();

        let Some(target) = target else {
            return false;
        };
        if !target.get_entity().is_alive() {
            return false;
        }

        if !self.pause_when_mob_idle {
            let is_idle = mob.is_navigator_idle();
            return !is_idle;
        }

        let is_valid_target = !target
            .get_player()
            .is_some_and(|p| p.is_spectator() || p.is_creative());

        let in_range = mob
            .get_mob_entity()
            .is_in_position_target_range_pos(&target.get_entity().block_pos.load());

        in_range && is_valid_target
    }
}

impl Goal for MeleeAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let time = mob.get_entity().world.load().get_world_age();
        if time - self.last_can_use_check < CAN_USE_CHECK_INTERVAL {
            return false;
        }
        self.last_can_use_check = time;

        let mob_entity = mob.get_mob_entity();
        let Some(target) = mob_entity.get_target() else {
            return false;
        };
        if !target.get_entity().is_alive() {
            return false;
        }

        self.path = mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .create_path(&mob_entity.living_entity, target.get_entity().pos.load(), 0);
        let can_use = self.path.is_some() || mob_entity.is_in_attack_range(target.as_ref());
        can_use && self.gate.is_none_or(|gate| gate(mob))
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.base_should_continue(mob) && self.gate.is_none_or(|gate| gate(mob))
    }

    fn start(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .move_to_path(self.path.take(), self.speed, &mob_entity.living_entity);
        mob_entity.set_attacking(true);
        self.update_countdown_ticks = 0;
        self.cooldown = 0;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        // Only clear target if they switched to creative/spectator
        let should_clear = mob
            .get_mob_entity()
            .get_target()
            .as_deref()
            .is_some_and(|entity| {
                !EntityPredicate::ExceptCreativeOrSpectator.test(entity.get_entity())
            });
        if should_clear {
            mob.set_mob_target(None);
        }

        mob.get_mob_entity().set_attacking(false);

        // Vanilla: this.mob.getNavigation().stop()
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let target = mob.get_mob_entity().get_target().clone();
        let Some(target) = target else {
            return;
        };

        mob.get_mob_entity()
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .look_at_entity_with_range(&target, 30.0, 30.0);

        self.update_countdown_ticks = (self.update_countdown_ticks - 1).max(0);

        let current_target_pos = target.get_entity().pos.load();
        let has_line_of_sight =
            self.pause_when_mob_idle || mob.has_line_of_sight(target.get_entity());
        // (0, 0, 0) doubles as "not pathed yet" and is never reset, as in vanilla.
        if has_line_of_sight
            && self.update_countdown_ticks <= 0
            && (self.pathed_target.x == 0.0
                && self.pathed_target.y == 0.0
                && self.pathed_target.z == 0.0
                || current_target_pos.squared_distance_to_vec(&self.pathed_target) >= 1.0
                || mob.get_random().random::<f32>() < 0.05)
        {
            self.pathed_target = current_target_pos;
            self.update_countdown_ticks = 4 + mob.get_random().random_range(0..7);
            let mob_entity = mob.get_mob_entity();
            let living = &mob_entity.living_entity;
            let dist_sq = living
                .entity
                .pos
                .load()
                .squared_distance_to_vec(&current_target_pos);
            if dist_sq > 1024.0 {
                self.update_countdown_ticks += 10;
            } else if dist_sq > 256.0 {
                self.update_countdown_ticks += 5;
            }

            let moved = {
                let mut navigator = mob_entity
                    .navigator
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                // Vanilla `moveTo(Entity, 0, speed)` keeps the old path when none is found.
                navigator
                    .create_path(living, current_target_pos, 0)
                    .is_some_and(|path| navigator.move_to_path(Some(path), self.speed, living))
            };
            if !moved {
                self.update_countdown_ticks += 15;
            }
            self.update_countdown_ticks = self.get_tick_count(self.update_countdown_ticks);
        }

        self.cooldown = (self.cooldown - 1).max(0);

        if self.cooldown <= 0
            && mob.get_mob_entity().is_in_attack_range(target.as_ref())
            && mob.has_line_of_sight(target.get_entity())
        {
            self.cooldown = self.get_max_cooldown();
            if let Some(attack) = self.attack {
                attack(mob, target.as_ref());
                return;
            }

            // Iron golem animation plays on every swing, even when blocked
            if let Some(golem) = mob.as_iron_golem() {
                golem.trigger_attack_animation();
            }

            mob.get_mob_entity().living_entity.swing_hand();
            let damaged = mob
                .get_mob_entity()
                .try_attack(mob.get_entity(), target.as_ref());
            if damaged {
                mob.on_attack(target.as_ref());
            }
        }
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
