use super::{Controls, Goal, to_goal_ticks};

use crate::entity::ageable::AgeableMob;
use crate::entity::ai::goal::revenge::MobFilter;
use crate::entity::ai::goal::track_target::TrackTargetGoal;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::ai::util::goal_utils;
use crate::entity::living::LivingEntity;
use crate::entity::mob::Mob;
use crate::entity::{EntityBase, mob::MobEntity};
use crate::world::World;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use std::sync::Arc;

const DEFAULT_RECIPROCAL_CHANCE: i32 = 10;

/// Extra gate on top of the target predicate, for mobs that pick targets conditionally.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum TargetCondition {
    #[default]
    Always,
    /// Only what the mob holds a grudge against. Neutral mobs.
    AngryAt,
    /// Nothing at all in daylight. Spiders.
    NoDaylight,
    /// Nothing at all while the mob is a baby. (Polar bears hunting foxes)
    Adult,
}

impl TargetCondition {
    /// Checked once per search attempt.
    fn allows_search(self, mob: &dyn Mob) -> bool {
        match self {
            Self::NoDaylight => !mob.get_mob_entity().is_in_daylight(),
            // A calm mob matches nobody: skip the search. Grudge check first, it is lock-free.
            Self::AngryAt => mob.as_neutral().is_some_and(|neutral| {
                neutral.get_persistent_anger_target().is_some() || neutral.is_angry()
            }),
            Self::Adult => !mob.as_ageable().is_some_and(AgeableMob::is_baby),
            Self::Always => true,
        }
    }

    /// Checked per candidate during the search.
    fn allows_target(self, mob: &dyn Mob, target: &dyn EntityBase, world: &World) -> bool {
        match self {
            Self::AngryAt => mob
                .as_neutral()
                .is_some_and(|neutral| neutral.is_angry_at(target, world)),
            Self::Always | Self::NoDaylight | Self::Adult => true,
        }
    }
}

pub struct ActiveTargetGoal {
    track_target_goal: TrackTargetGoal,
    target: Option<Arc<dyn EntityBase>>,
    reciprocal_chance: i32,
    target_type: Option<&'static EntityType>,
    target_predicate: TargetPredicate,
    condition: TargetCondition,
    gate: Option<MobFilter>,
    /// A subclass `canUse` check that, unlike `gate`, leaves `canContinueToUse` alone.
    start_gate: Option<MobFilter>,
    /// A subclass `canUse` check made after `super.canUse()` found a target.
    confirm: Option<MobFilter>,
    /// Vanilla `getTargetSearchArea` overrides: the box searched for non-player targets.
    search_area: Option<fn(&dyn Mob, f64) -> BoundingBox>,
}

impl ActiveTargetGoal {
    pub fn new<F>(
        mob: &MobEntity,
        target_type: &'static EntityType,
        reciprocal_chance: i32,
        check_visibility: bool,
        check_can_navigate: bool,
        predicate: Option<F>,
    ) -> Self
    where
        F: Fn(&LivingEntity, &World) -> bool + Send + Sync + 'static,
    {
        let track_target_goal = TrackTargetGoal::new(check_visibility, check_can_navigate);
        let mut target_predicate = TargetPredicate::create_attackable();
        target_predicate.base_max_distance = mob
            .living_entity
            .get_attribute_value(&Attributes::FOLLOW_RANGE);

        if let Some(predicate) = predicate {
            target_predicate.set_predicate(predicate);
        }

        Self {
            track_target_goal,
            target: None,
            reciprocal_chance: to_goal_ticks(reciprocal_chance),
            target_type: Some(target_type),
            target_predicate,
            condition: TargetCondition::Always,
            gate: None,
            start_gate: None,
            confirm: None,
            search_area: None,
        }
    }

    /// Chains onto any of the constructors, including the boxed ones.
    #[must_use]
    pub fn when(mut self: Box<Self>, condition: TargetCondition) -> Box<Self> {
        self.condition = condition;
        self
    }

    /// Vanilla `TargetGoal.setUnseenMemoryTicks`.
    #[must_use]
    pub const fn with_unseen_memory_ticks(mut self: Box<Self>, ticks: i32) -> Box<Self> {
        self.track_target_goal.max_time_without_visibility = ticks;
        self
    }

    /// Extra condition for starting and for continuing, e.g. an angry, unspent bee.
    #[must_use]
    pub fn gated_by(mut self: Box<Self>, gate: MobFilter) -> Box<Self> {
        self.gate = Some(gate);
        self
    }

    /// Vanilla `mustReach`: skip targets the navigation can't get within reach of.
    #[must_use]
    pub fn must_reach(mut self: Box<Self>) -> Box<Self> {
        self.track_target_goal.set_check_can_navigate(true);
        self
    }

    /// An extra condition in `canUse` only, like a subclass that checks before `super.canUse()`.
    #[must_use]
    pub fn starting_only_when(mut self: Box<Self>, gate: MobFilter) -> Box<Self> {
        self.start_gate = Some(gate);
        self
    }

    /// A subclass `canUse` check made only once `super.canUse()` succeeded.
    #[must_use]
    pub fn confirmed_by(mut self: Box<Self>, confirm: MobFilter) -> Box<Self> {
        self.confirm = Some(confirm);
        self
    }

    /// A vanilla `getFollowDistance` override, as a factor on the follow range.
    #[must_use]
    pub const fn with_follow_distance_scale(mut self: Box<Self>, scale: f64) -> Box<Self> {
        self.track_target_goal.follow_distance_scale = scale;
        self
    }

    /// Vanilla `getTargetSearchArea`, given the follow distance.
    #[must_use]
    pub fn with_search_area(
        mut self: Box<Self>,
        area: fn(&dyn Mob, f64) -> BoundingBox,
    ) -> Box<Self> {
        self.search_area = Some(area);
        self
    }

    #[must_use]
    pub fn with_default(
        mob: &MobEntity,
        target_type: &'static EntityType,
        check_visibility: bool,
    ) -> Box<Self> {
        let track_target_goal = TrackTargetGoal::with_default(check_visibility);
        let mut target_predicate = TargetPredicate::create_attackable();
        target_predicate.base_max_distance = mob
            .living_entity
            .get_attribute_value(&Attributes::FOLLOW_RANGE);

        Box::new(Self {
            track_target_goal,
            target: None,
            reciprocal_chance: to_goal_ticks(DEFAULT_RECIPROCAL_CHANCE),
            target_type: Some(target_type),
            target_predicate,
            condition: TargetCondition::Always,
            gate: None,
            start_gate: None,
            confirm: None,
            search_area: None,
        })
    }

    /// Targets the closest entity of any type passing `predicate`, like vanilla's
    /// class-agnostic `NearestAttackableTargetGoal` (e.g. iron golems targeting
    /// every `Enemy` but creepers). All candidates are tested, so an invalid
    /// entity nearest to the mob cannot block a valid one farther away.
    pub fn predicated(
        mob: &MobEntity,
        reciprocal_chance: i32,
        check_visibility: bool,
        predicate: impl Fn(&LivingEntity, &World) -> bool + Send + Sync + 'static,
    ) -> Box<Self> {
        let track_target_goal = TrackTargetGoal::new(check_visibility, false);
        let mut target_predicate = TargetPredicate::create_attackable();
        target_predicate.base_max_distance = mob
            .living_entity
            .get_attribute_value(&Attributes::FOLLOW_RANGE);
        target_predicate.set_predicate(predicate);

        Box::new(Self {
            track_target_goal,
            target: None,
            reciprocal_chance: to_goal_ticks(reciprocal_chance),
            target_type: None,
            target_predicate,
            condition: TargetCondition::Always,
            gate: None,
            start_gate: None,
            confirm: None,
            search_area: None,
        })
    }

    pub fn set_target(&mut self, target: Option<Arc<dyn EntityBase>>) {
        self.target = target;
    }

    /// Vanilla `NearestAttackableTargetGoal.findTarget`, for subclasses that skip the random
    /// interval; returns whether a target was found.
    pub fn find_target(&mut self, mob: &dyn Mob) -> bool {
        self.find_closest_target(mob);
        self.target.is_some()
    }

    fn find_closest_target(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        let entity = &mob_entity.living_entity.entity;
        let follow_range = mob_entity
            .living_entity
            .get_attribute_value(&Attributes::FOLLOW_RANGE)
            * self.track_target_goal.follow_distance_scale;

        // Vanilla updates the target conditions with the current follow distance on every search
        self.target_predicate.base_max_distance = follow_range;

        let world = entity.world.load();
        let pos = entity.pos.load();
        let search_pos = Vector3::new(pos.x, entity.get_eye_y(), pos.z);

        let candidates = if self.target_type == Some(&EntityType::PLAYER) {
            world
                .players
                .load()
                .iter()
                .map(|player| Arc::clone(player) as Arc<dyn EntityBase>)
                .collect()
        } else {
            let area = self.search_area.map_or_else(
                || {
                    entity
                        .bounding_box
                        .load()
                        .expand(follow_range, follow_range, follow_range)
                },
                |area| area(mob, follow_range),
            );
            goal_utils::get_entities_of_class(&world, self.target_type, &area)
        };
        let condition = self.condition;
        self.target = self.target_predicate.get_nearest_entity(
            &world,
            candidates
                .into_iter()
                .filter(|candidate| condition.allows_target(mob, candidate.as_ref(), &world)),
            Some(mob),
            search_pos,
        );
    }
}

impl Goal for ActiveTargetGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if self.gate.is_some_and(|gate| !gate(mob))
            || self.start_gate.is_some_and(|gate| !gate(mob))
        {
            return false;
        }
        // Vanilla `SpiderTargetGoal` checks the light before the random interval roll.
        let checked_before_roll = self.condition == TargetCondition::NoDaylight;
        if checked_before_roll && !self.condition.allows_search(mob) {
            return false;
        }
        if self.reciprocal_chance > 0
            && mob
                .get_entity()
                .random()
                .next_bounded_i32(self.reciprocal_chance)
                != 0
        {
            return false;
        }
        if !checked_before_roll && !self.condition.allows_search(mob) {
            return false;
        }
        self.find_closest_target(mob);
        self.target.is_some() && self.confirm.is_none_or(|confirm| confirm(mob))
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        if self.gate.is_some_and(|gate| !gate(mob)) {
            self.target = None;
            return false;
        }
        // Like vanilla TargetGoal.canContinueToUse, the condition only gates the search.
        self.track_target_goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.set_mob_target(self.target.clone());
        self.track_target_goal.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.track_target_goal.stop(mob);
    }

    fn controls(&self) -> Controls {
        self.track_target_goal.controls()
    }
}

/// Vanilla `NonTameRandomTargetGoal`: hunts only while untamed, and keeps chasing while the
/// prey still passes the targeting conditions (range, sight and the selector).
pub struct NonTameRandomTargetGoal {
    inner: Box<ActiveTargetGoal>,
}

impl NonTameRandomTargetGoal {
    #[must_use]
    pub fn new(inner: Box<ActiveTargetGoal>) -> Box<Self> {
        Box::new(Self { inner })
    }
}

impl Goal for NonTameRandomTargetGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        !mob.is_tamed() && self.inner.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let world = mob.get_entity().world.load();
        self.inner.target.as_ref().is_some_and(|target| {
            self.inner
                .target_predicate
                .test(&world, Some(mob), target.as_ref())
        })
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.inner.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.inner.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.inner.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.inner.controls()
    }
}
