use pumpkin_data::data_component_impl::KineticWeaponImpl;
use pumpkin_util::Hand;
use pumpkin_util::math::vector3::Vector3;

use super::{Controls, Goal, to_goal_ticks};
use crate::entity::ai::util::land_random_pos;
use crate::entity::mob::Mob;

/// Vanilla `SpearUseGoal.MIN_REPOSITION_DISTANCE`.
const MIN_REPOSITION_DISTANCE: i32 = 6;
/// Vanilla `SpearUseGoal.MAX_REPOSITION_DISTANCE`.
const MAX_REPOSITION_DISTANCE: i32 = 7;
/// Vanilla `SpearUseGoal.MIN_COOLDOWN_DISTANCE`.
const MIN_COOLDOWN_DISTANCE: i32 = 9;
/// Vanilla `SpearUseGoal.MAX_COOLDOWN_DISTANCE`.
const MAX_COOLDOWN_DISTANCE: i32 = 11;
/// Vertical range of the spots the mob backs off to.
const AWAY_POS_VERTICAL_DIST: i32 = 7;

/// Vanilla `SpearUseGoal.MAX_FLEEING_TIME`.
fn max_fleeing_time() -> f64 {
    f64::from(to_goal_ticks(100))
}

/// Vanilla `SpearUseGoal`: a mob holding a kinetic weapon closes in, lowers it for the
/// weapon's damage window while charging the target, then backs off and comes round again.
pub struct SpearUseGoal {
    state: Option<SpearUseState>,
    speed_modifier_when_charging: f64,
    speed_modifier_when_repositioning: f64,
    approach_distance_sq: f32,
    target_in_range_radius_sq: f32,
}

impl SpearUseGoal {
    #[must_use]
    pub fn new(
        speed_modifier_when_charging: f64,
        speed_modifier_when_repositioning: f64,
        approach_distance: f32,
        target_in_range_radius: f32,
    ) -> Box<Self> {
        Box::new(Self {
            state: None,
            speed_modifier_when_charging,
            speed_modifier_when_repositioning,
            approach_distance_sq: approach_distance * approach_distance,
            target_in_range_radius_sq: target_in_range_radius * target_in_range_radius,
        })
    }

    /// Vanilla `ableToAttack`.
    fn able_to_attack(mob: &dyn Mob) -> bool {
        let mob_entity = mob.get_mob_entity();
        mob.get_target().is_some()
            && mob_entity
                .get_item_slot(&pumpkin_data::data_component_impl::EquipmentSlot::MAIN_HAND)
                .get_data_component::<KineticWeaponImpl>()
                .is_some()
    }

    /// Vanilla `getKineticWeaponUseDuration`: `KineticWeapon.computeDamageUseDuration`.
    fn get_kinetic_weapon_use_duration(mob: &dyn Mob) -> i32 {
        let duration = mob
            .get_mob_entity()
            .get_item_slot(&pumpkin_data::data_component_impl::EquipmentSlot::MAIN_HAND)
            .get_data_component::<KineticWeaponImpl>()
            .map_or(0, |weapon| {
                weapon.delay_ticks
                    + weapon
                        .damage_conditions
                        .as_ref()
                        .map_or(0, |condition| condition.max_duration_ticks)
            });
        to_goal_ticks(duration)
    }

    fn is_using_item(mob: &dyn Mob) -> bool {
        mob.get_mob_entity()
            .living_entity
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }
}

impl Goal for SpearUseGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        Self::able_to_attack(mob) && !Self::is_using_item(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.state.as_ref().is_some_and(|state| !state.done) && Self::able_to_attack(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity().set_attacking(true);
        self.state = Some(SpearUseState::default());
    }

    fn stop(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
        mob_entity.set_attacking(false);
        self.state = None;
        mob_entity.living_entity.clear_active_hand();
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let mob_entity = mob.get_mob_entity();
        let Some(target) = mob.get_target() else {
            return;
        };
        let Some(target_living) = target.get_living_entity() else {
            return;
        };
        let living = &mob_entity.living_entity;
        let entity = &living.entity;
        let target_pos = target.get_entity().pos.load();
        let target_dist_sqr = entity.pos.load().squared_distance_to_vec(&target_pos);
        // Vanilla `Mob.chargeSpeedModifier` of the root vehicle: 1 for every mob but the
        // camel husk, which Pumpkin does not have yet.
        let speed_modifier = 1.0f32;
        let mount_distance = if entity.has_vehicle() { 2 } else { 0 };
        mob_entity.look_at(target.as_ref(), 30.0, 30.0);
        mob_entity
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .look_at_entity_with_range(&target, 30.0, 30.0);

        let navigator = || {
            mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        };
        if state.not_engaged_yet() {
            if target_dist_sqr > f64::from(self.approach_distance_sq) {
                navigator().move_to_entity(
                    target_living,
                    f64::from(speed_modifier) * self.speed_modifier_when_repositioning,
                    living,
                );
                return;
            }

            state.start_engagement(Self::get_kinetic_weapon_use_duration(mob));
            mob_entity.start_using_item(Hand::Right);
        }

        if state.tick_and_check_engagement() {
            living.clear_active_hand();
            let distance = target_dist_sqr.sqrt();
            state.away_pos = land_random_pos::get_pos_away_between(
                mob,
                (f64::from(MIN_COOLDOWN_DISTANCE + mount_distance) - distance).max(0.0),
                (f64::from(MAX_COOLDOWN_DISTANCE + mount_distance) - distance).max(1.0),
                AWAY_POS_VERTICAL_DIST,
                target_pos,
            );
            state.fleeing_time = 1;
        }

        if state.tick_and_check_fleeing() {
            return;
        }
        if let Some(away_pos) = state.away_pos {
            let is_done = {
                let mut navigator = navigator();
                navigator.move_to_coords(
                    away_pos.x,
                    away_pos.y,
                    away_pos.z,
                    f64::from(speed_modifier) * self.speed_modifier_when_repositioning,
                    living,
                );
                navigator.is_done()
            };
            if is_done {
                if state.fleeing_time > 0 {
                    state.done = true;
                    return;
                }
                state.away_pos = None;
            }
        } else {
            let is_done = {
                let mut navigator = navigator();
                navigator.move_to_entity(
                    target_living,
                    f64::from(speed_modifier) * self.speed_modifier_when_charging,
                    living,
                );
                navigator.is_done()
            };
            if target_dist_sqr < f64::from(self.target_in_range_radius_sq) || is_done {
                let distance = target_dist_sqr.sqrt();
                state.away_pos = land_random_pos::get_pos_away_between(
                    mob,
                    f64::from(MIN_REPOSITION_DISTANCE + mount_distance) - distance,
                    f64::from(MAX_REPOSITION_DISTANCE + mount_distance) - distance,
                    AWAY_POS_VERTICAL_DIST,
                    target_pos,
                );
            }
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}

/// Vanilla `SpearUseGoal.SpearUseState`.
struct SpearUseState {
    engage_time: i32,
    fleeing_time: i32,
    away_pos: Option<Vector3<f64>>,
    done: bool,
}

impl Default for SpearUseState {
    fn default() -> Self {
        Self {
            engage_time: -1,
            fleeing_time: -1,
            away_pos: None,
            done: false,
        }
    }
}

impl SpearUseState {
    const fn not_engaged_yet(&self) -> bool {
        self.engage_time < 0
    }

    const fn start_engagement(&mut self, spear_down_time: i32) {
        self.engage_time = spear_down_time;
    }

    const fn tick_and_check_engagement(&mut self) -> bool {
        if self.engage_time > 0 {
            self.engage_time -= 1;
            if self.engage_time == 0 {
                return true;
            }
        }
        false
    }

    fn tick_and_check_fleeing(&mut self) -> bool {
        if self.fleeing_time > 0 {
            self.fleeing_time += 1;
            if f64::from(self.fleeing_time) > max_fleeing_time() {
                self.done = true;
                return true;
            }
        }
        false
    }
}
