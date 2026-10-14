use super::{Controls, Goal, to_goal_ticks};
use crate::entity::ai::util::{default_random_pos, land_random_pos};
use crate::entity::{ai::pathfinder::NavigatorGoal, mob::Mob};
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

pub const DEFAULT_INTERVAL: i32 = 120;
/// The `noActionTime` at which vanilla's `RandomStrollGoal` stops starting.
const NO_ACTION_TIME_LIMIT: i32 = 100;
/// Vanilla `WaterAvoidingRandomStrollGoal.PROBABILITY`: the chance of an ordinary stroll.
const WATER_AVOIDING_PROBABILITY: f32 = 0.001;

pub struct WanderAroundGoal {
    goal_control: Controls,
    speed: f64,
    target: Option<Vector3<f64>>,
    interval: i32,
    force_trigger: bool,
    /// Vanilla `RandomStrollGoal.checkNoActionTime`.
    check_no_action_time: bool,
    position: fn(&dyn Mob) -> Option<Vector3<f64>>,
}

impl WanderAroundGoal {
    #[must_use]
    pub const fn new(speed: f64) -> Self {
        Self::with_interval(speed, DEFAULT_INTERVAL)
    }

    #[must_use]
    pub const fn with_interval(speed: f64, interval: i32) -> Self {
        Self {
            goal_control: Controls::MOVE,
            speed,
            target: None,
            interval,
            force_trigger: false,
            check_no_action_time: true,
            position: land_position,
        }
    }

    /// Vanilla's `RandomSwimmingGoal`: the same stroll, to random spots in water.
    #[must_use]
    pub const fn swimming(speed: f64, interval: i32) -> Self {
        Self {
            position: swimmable_position,
            ..Self::with_interval(speed, interval)
        }
    }

    /// Vanilla's `WaterAvoidingRandomStrollGoal`: strolls to spots on land, and out of
    /// water when in it.
    #[must_use]
    pub const fn avoiding_water(speed: f64) -> Self {
        Self {
            position: water_avoiding_position,
            ..Self::new(speed)
        }
    }

    /// Keeps wandering however long the mob has gone without a player nearby, like
    /// vanilla's brain-driven strolls and `GolemRandomStrollInVillageGoal`.
    #[must_use]
    pub const fn ignoring_no_action_time(mut self) -> Self {
        self.check_no_action_time = false;
        self
    }

    /// Run once on the next check, ignoring the interval.
    pub const fn trigger(&mut self) {
        self.force_trigger = true;
    }

    pub const fn set_interval(&mut self, interval: i32) {
        self.interval = interval;
    }
}

fn land_position(mob: &dyn Mob) -> Option<Vector3<f64>> {
    default_random_pos::get_pos(mob, 10, 7)
}

fn water_avoiding_position(mob: &dyn Mob) -> Option<Vector3<f64>> {
    if mob.get_mob_entity().living_entity.is_in_water() {
        return land_random_pos::get_pos(mob, 15, 7).or_else(|| land_position(mob));
    }
    if mob.get_random().random::<f32>() >= WATER_AVOIDING_PROBABILITY {
        land_random_pos::get_pos(mob, 10, 7)
    } else {
        land_position(mob)
    }
}

fn swimmable_position(mob: &dyn Mob) -> Option<Vector3<f64>> {
    default_random_pos::get_swimmable_pos(mob, 10, 7)
}

impl Goal for WanderAroundGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        // Every mob that carries a passenger is controlled by it.
        if mob.get_entity().has_passengers() {
            return false;
        }

        if !self.force_trigger {
            if self.check_no_action_time
                && mob
                    .get_mob_entity()
                    .living_entity
                    .no_action_time
                    .load(std::sync::atomic::Ordering::Relaxed)
                    >= NO_ACTION_TIME_LIMIT
            {
                return false;
            }
            if mob
                .get_random()
                .random_range(0..to_goal_ticks(self.interval))
                != 0
            {
                return false;
            }
        }

        self.target = (self.position)(mob);
        if self.target.is_none() {
            return false;
        }
        self.force_trigger = false;
        true
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let idle = mob.is_navigator_idle();
        !idle && !mob.get_entity().has_passengers()
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(target) = self.target {
            let pos = mob.get_mob_entity().living_entity.entity.pos.load();
            mob.get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .set_progress(NavigatorGoal::new(pos, target, self.speed));
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.target = None;
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
