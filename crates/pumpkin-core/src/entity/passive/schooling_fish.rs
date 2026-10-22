//! What cod, salmon and tropical fish share: vanilla's `AbstractSchoolingFish`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use pumpkin_util::random::RandomImpl;

use crate::entity::EntityBase;
use crate::entity::ai::goal::follow_flock_leader::FollowFlockLeaderGoal;
use crate::entity::ai::pathfinder::NavigatorGoal;
use crate::entity::mob::{Mob, MobEntity};

/// Vanilla `AbstractFish.getMaxSpawnClusterSize`, the default school size.
pub const DEFAULT_MAX_SCHOOL_SIZE: i32 = 8;
/// How far a fish looks for others of its kind, on every axis.
pub const SCHOOL_SEARCH_RANGE: f64 = 8.0;
const MAX_DISTANCE_TO_LEADER_SQ: f64 = 121.0;

/// The goal vanilla's `AbstractSchoolingFish.registerGoals` adds on top of the
/// plain fish goals.
pub fn init(mob_entity: &MobEntity) {
    mob_entity
        .goals_selector
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .add_goal(5, Box::new(FollowFlockLeaderGoal::for_mob(mob_entity)));
}

/// The school a fish belongs to. Other fish change `school_size` from their own
/// ticks, so it is atomic; the leader is only ever written by its owner.
pub struct SchoolingFish {
    leader: Mutex<Option<Arc<dyn EntityBase>>>,
    school_size: AtomicI32,
    max_school_size: i32,
}

impl SchoolingFish {
    #[must_use]
    pub const fn new(max_school_size: i32) -> Self {
        Self {
            leader: Mutex::new(None),
            school_size: AtomicI32::new(1),
            max_school_size,
        }
    }

    fn leader(&self) -> Option<Arc<dyn EntityBase>> {
        self.leader
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub const fn max_school_size(&self) -> i32 {
        self.max_school_size
    }

    #[must_use]
    pub fn school_size(&self) -> i32 {
        self.school_size.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_follower(&self) -> bool {
        self.leader()
            .is_some_and(|leader| leader.get_entity().is_alive())
    }

    pub fn start_following(&self, leader: &Arc<dyn EntityBase>) {
        let Some(leader_school) = school_of(leader) else {
            return;
        };
        *self.leader.lock().unwrap_or_else(PoisonError::into_inner) = Some(leader.clone());
        leader_school.school_size.fetch_add(1, Ordering::Relaxed);
    }

    pub fn stop_following(&self) {
        let leader = self
            .leader
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(leader_school) = leader.as_ref().and_then(school_of) {
            leader_school.school_size.fetch_sub(1, Ordering::Relaxed);
        }
    }

    #[must_use]
    pub fn has_followers(&self) -> bool {
        self.school_size() > 1
    }

    #[must_use]
    pub fn can_be_followed(&self) -> bool {
        self.has_followers() && self.school_size() < self.max_school_size
    }

    #[must_use]
    pub fn in_range_of_leader(&self, mob: &dyn Mob) -> bool {
        self.leader().is_some_and(|leader| {
            let pos = mob.get_entity().pos.load();
            pos.squared_distance_to_vec(&leader.get_entity().pos.load())
                <= MAX_DISTANCE_TO_LEADER_SQ
        })
    }

    pub fn path_to_leader(&self, mob: &dyn Mob) {
        if !self.is_follower() {
            return;
        }
        let Some(leader) = self.leader() else {
            return;
        };
        let mob_entity = mob.get_mob_entity();
        let from = mob_entity.living_entity.entity.pos.load();
        let to = leader.get_entity().pos.load();
        mob_entity
            .own_navigation()
            .set_progress(NavigatorGoal::new(from, to, 1.0));
    }

    /// The school part of vanilla's `AbstractSchoolingFish.tick`: a leader that
    /// has lost sight of every other fish of its kind forgets its school.
    pub fn tick(&self, mob: &dyn Mob) {
        // `this.level().getRandom().nextInt(200) == 1`, drawn only with followers; the
        // guard is dropped before the entity search.
        let roll = || {
            mob.get_entity()
                .world
                .load()
                .random
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .next_bounded_i32(200)
        };
        if self.has_followers() && roll() == 1 && others_of_same_kind(mob).is_empty() {
            self.school_size.store(1, Ordering::Relaxed);
        }
    }
}

/// The school state of `fish`, if it is a schooling fish.
#[must_use]
pub fn school_of(fish: &Arc<dyn EntityBase>) -> Option<&SchoolingFish> {
    fish.get_mob().and_then(Mob::as_schooling_fish)
}

/// Every other living fish of `mob`'s type whose box touches `mob`'s box grown
/// by [`SCHOOL_SEARCH_RANGE`]. Vanilla's `getEntitiesOfClass` also returns the
/// fish itself; callers here add it back where that matters.
#[must_use]
pub fn others_of_same_kind(mob: &dyn Mob) -> Vec<Arc<dyn EntityBase>> {
    let entity = mob.get_entity();
    let search_box = entity.bounding_box.load().expand(
        SCHOOL_SEARCH_RANGE,
        SCHOOL_SEARCH_RANGE,
        SCHOOL_SEARCH_RANGE,
    );
    entity
        .world
        .load()
        .entities
        .load()
        .iter()
        .filter(|other| {
            let other_entity = other.get_entity();
            other_entity.entity_id != entity.entity_id
                && other_entity.entity_type == entity.entity_type
                && other_entity.is_alive()
                && other_entity.bounding_box.load().intersects(&search_box)
        })
        .cloned()
        .collect()
}
