//! What every fish shares: vanilla's `AbstractFish`.

use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::Sound;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crate::entity::EntityBase;
use crate::entity::ai::control::fish_move_control::FishMoveControl;
use crate::entity::ai::goal::{
    Controls, Goal, avoid_entity::AvoidEntityGoal, escape_danger::EscapeDangerGoal,
    wander_around::WanderAroundGoal,
};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::pathfinder::{Navigator, WaterBoundPathNavigation};
use crate::entity::mob::{Mob, MobEntity};

/// Gives a new fish vanilla's swimming AI in place of the walking defaults every
/// mob starts with: water navigation, panic when hurt, keeping clear of players,
/// and swimming to random spots.
pub fn init(mob_entity: &MobEntity) {
    let mut navigator =
        Navigator::new(WaterBoundPathNavigation::new(false).steered_by_move_control());
    navigator.set_pathfinding_malus(PathType::Water, 0.0);
    *mob_entity
        .navigator
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = navigator;
    *mob_entity
        .move_control
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Box::new(FishMoveControl::default());

    let mut goals = mob_entity
        .goals_selector
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    goals.add_goal(0, EscapeDangerGoal::new(1.25));
    goals.add_goal(
        2,
        Box::new(AvoidEntityGoal::new(&EntityType::PLAYER, 8.0, 1.6, 1.4)),
    );
    goals.add_goal(4, Box::new(FishSwimGoal::new()));
}

/// Vanilla's `AbstractFish.FishSwimGoal`: random swimming, unless the fish is
/// following a school leader (`canRandomSwim`).
pub struct FishSwimGoal {
    swim: WanderAroundGoal,
}

impl FishSwimGoal {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            swim: WanderAroundGoal::swimming(1.0, 40),
        }
    }
}

impl Default for FishSwimGoal {
    fn default() -> Self {
        Self::new()
    }
}

impl Goal for FishSwimGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let can_random_swim = mob
            .as_schooling_fish()
            .is_none_or(|school| !school.is_follower());
        can_random_swim && self.swim.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.swim.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.swim.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.swim.stop(mob);
    }

    fn controls(&self) -> Controls {
        self.swim.controls()
    }
}

/// Vanilla's `AbstractFish.travel`: in water a fish swims at a fraction of the
/// usual pace with no gravity, sinking slowly while it has nothing to chase. Out
/// of water it falls like anything else.
pub fn travel(mob: &dyn Mob, caller: &dyn EntityBase) -> bool {
    let mob_entity = mob.get_mob_entity();
    let living = &mob_entity.living_entity;
    let entity = &living.entity;
    if !entity.touching_water.load(Ordering::Relaxed) {
        return false;
    }

    entity.update_velocity_from_input(living.movement_input.load(), 0.01);
    living.make_move(caller);

    let mut velocity = entity.velocity.load() * 0.9;
    let chasing = mob_entity
        .target
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some();
    if !chasing {
        velocity.y -= 0.005;
    }
    entity.velocity.store(velocity);
    true
}

/// Vanilla's `AbstractFish.aiStep` on dry land, where a stranded fish hops about.
pub fn flop(mob: &dyn Mob, sound: Sound) {
    let entity = mob.get_entity();
    // `verticalCollision` holds whenever the last move left the fish on the ground.
    if entity.touching_water.load(Ordering::Relaxed) || !entity.on_ground.load(Ordering::Relaxed) {
        return;
    }
    let mut rng = mob.get_random();
    let velocity = entity.velocity.load();
    let x = (rng.random::<f32>() * 2.0 - 1.0) * 0.05;
    let z = (rng.random::<f32>() * 2.0 - 1.0) * 0.05;
    entity.velocity.store(Vector3::new(
        velocity.x + f64::from(x),
        velocity.y + f64::from(0.4f32),
        velocity.z + f64::from(z),
    ));
    entity.on_ground.store(false, Ordering::Relaxed);
    // Vanilla `needsSync`: sent with the tick's changes, after travel.
    entity.velocity_dirty.store(true, Ordering::SeqCst);
    entity.play_sound(sound);
}
