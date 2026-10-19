use std::sync::Arc;

use pumpkin_util::random::RandomImpl;
use rand::RngExt;

use super::{Goal, to_goal_ticks};
use crate::entity::EntityBase;
use crate::entity::mob::{Mob, MobEntity};
use crate::entity::passive::schooling_fish::{SchoolingFish, others_of_same_kind, school_of};

const INTERVAL_TICKS: i32 = 200;

/// Vanilla's `FollowFlockLeaderGoal`: a schooling fish with no school joins
/// one nearby, or starts its own, and then keeps swimming after the leader.
pub struct FollowFlockLeaderGoal {
    time_to_recalc_path: i32,
    next_start_tick: i32,
}

impl FollowFlockLeaderGoal {
    /// Draws the first start delay from a thread RNG; [`Self::for_mob`] is the vanilla one.
    #[must_use]
    pub fn new() -> Self {
        Self {
            time_to_recalc_path: 0,
            next_start_tick: to_goal_ticks(
                INTERVAL_TICKS + rand::rng().random_range(0..INTERVAL_TICKS) % 20,
            ),
        }
    }

    /// Vanilla's constructor: the first start delay comes from the fish's own random.
    #[must_use]
    pub fn for_mob(mob_entity: &MobEntity) -> Self {
        Self {
            time_to_recalc_path: 0,
            next_start_tick: Self::next_start_tick(&mob_entity.living_entity.entity),
        }
    }

    /// Vanilla `nextStartTick`.
    fn next_start_tick(entity: &crate::entity::Entity) -> i32 {
        to_goal_ticks(INTERVAL_TICKS + entity.random().next_bounded_i32(INTERVAL_TICKS) % 20)
    }

    /// Vanilla's `AbstractSchoolingFish.addFollowers`. The limit is applied
    /// before the leader is filtered out, as in vanilla.
    fn add_followers(
        leader: &Arc<dyn EntityBase>,
        leader_school: &SchoolingFish,
        candidates: Vec<Arc<dyn EntityBase>>,
    ) {
        let free = usize::try_from(leader_school.max_school_size() - leader_school.school_size())
            .unwrap_or(0);
        let leader_id = leader.get_entity().entity_id;
        for fish in candidates
            .into_iter()
            .take(free)
            .filter(|fish| fish.get_entity().entity_id != leader_id)
        {
            if let Some(school) = school_of(&fish) {
                school.start_following(leader);
            }
        }
    }
}

impl Default for FollowFlockLeaderGoal {
    fn default() -> Self {
        Self::new()
    }
}

impl Goal for FollowFlockLeaderGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(school) = mob.as_schooling_fish() else {
            return false;
        };
        if school.has_followers() {
            return false;
        }
        if school.is_follower() {
            return true;
        }
        if self.next_start_tick > 0 {
            self.next_start_tick -= 1;
            return false;
        }
        self.next_start_tick = Self::next_start_tick(mob.get_entity());

        let entity = mob.get_entity();
        let Some(this) = entity.world.load().get_entity_by_id(entity.entity_id) else {
            return false;
        };

        // This fish first: it is neither leading nor following, so it always
        // passes vanilla's predicate.
        let mut leaders_with_space_or_not_followers = vec![this.clone()];
        leaders_with_space_or_not_followers.extend(others_of_same_kind(mob).into_iter().filter(
            |fish| {
                school_of(fish).is_some_and(|other| other.can_be_followed() || !other.is_follower())
            },
        ));

        let leader = leaders_with_space_or_not_followers
            .iter()
            .find(|fish| school_of(fish).is_some_and(SchoolingFish::can_be_followed))
            .cloned()
            .unwrap_or(this);
        let Some(leader_school) = school_of(&leader) else {
            return false;
        };
        let not_followers = leaders_with_space_or_not_followers
            .into_iter()
            .filter(|fish| school_of(fish).is_some_and(|other| !other.is_follower()))
            .collect();
        Self::add_followers(&leader, leader_school, not_followers);

        school.is_follower()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        mob.as_schooling_fish()
            .is_some_and(|school| school.is_follower() && school.in_range_of_leader(mob))
    }

    fn start(&mut self, _mob: &dyn Mob) {
        self.time_to_recalc_path = 0;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(school) = mob.as_schooling_fish() {
            school.stop_following();
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.time_to_recalc_path -= 1;
        if self.time_to_recalc_path <= 0 {
            self.time_to_recalc_path = self.get_tick_count(10);
            if let Some(school) = mob.as_schooling_fish() {
                school.path_to_leader(mob);
            }
        }
    }
}
