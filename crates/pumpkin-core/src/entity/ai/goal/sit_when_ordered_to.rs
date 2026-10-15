use super::{Controls, Goal};
use crate::entity::mob::Mob;
use std::sync::atomic::Ordering;

/// Vanilla `SitWhenOrderedToGoal`: holds a tamed mob in its sitting pose while it is
/// ordered to sit, unless its nearby owner was just hurt.
#[derive(Default)]
pub struct SitWhenOrderedToGoal {
    goal_control: Controls,
}

impl SitWhenOrderedToGoal {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            goal_control: Controls::MOVE.union(Controls::JUMP),
        }
    }
}

impl Goal for SitWhenOrderedToGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(tamable) = mob.as_tamable() else {
            return false;
        };
        let ordered_to_sit = tamable.is_ordered_to_sit();
        if !ordered_to_sit && !tamable.is_tame() {
            return false;
        }
        let entity = &mob.get_mob_entity().living_entity.entity;
        if entity.touching_water.load(Ordering::Relaxed)
            || !entity.on_ground.load(Ordering::Relaxed)
        {
            return false;
        }
        // An owner elsewhere (or offline) counts as absent, which keeps the mob sitting.
        let world = entity.world.load();
        let Some(owner) = tamable
            .get_owner()
            .and_then(|uuid| world.get_player_by_uuid(uuid))
        else {
            return true;
        };
        let owner_living = &owner.living_entity;
        if entity
            .pos
            .load()
            .squared_distance_to_vec(&owner_living.entity.pos.load())
            < 144.0
            && owner_living.last_attacker_id.load(Ordering::Relaxed) != 0
        {
            false
        } else {
            ordered_to_sit
        }
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_start(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
        if let Some(tamable) = mob.as_tamable() {
            tamable.set_in_sitting_pose(true);
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(tamable) = mob.as_tamable() {
            tamable.set_in_sitting_pose(false);
        }
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
