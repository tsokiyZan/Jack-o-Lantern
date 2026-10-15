use std::sync::Arc;

use super::revenge::MobFilter;
use super::{Controls, Goal};
use crate::entity::ai::pathfinder::path::Path;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::ai::util::{default_random_pos, goal_utils};
use crate::entity::living::LivingEntity;
use crate::entity::predicate::EntityPredicate;
use crate::entity::{EntityBase, mob::Mob};
use pumpkin_data::entity::EntityType;

const FAST_DISTANCE_SQ: f64 = 49.0;
const HORIZONTAL_RANGE: i32 = 16;
const VERTICAL_RANGE: i32 = 7;

pub struct AvoidEntityGoal {
    goal_control: Controls,
    /// `None` is any mob, narrowed by the avoid predicate (e.g. vanilla's `Monster.class`).
    avoid_class: Option<&'static EntityType>,
    max_dist: f64,
    walk_speed_modifier: f64,
    sprint_speed_modifier: f64,
    avoid_entity_targeting: TargetPredicate,
    to_avoid: Option<Arc<dyn EntityBase>>,
    path: Option<Path>,
    /// What vanilla subclasses (cat, ocelot, wolf, rabbit) add to `canUse` and
    /// `canContinueToUse`.
    gate: Option<MobFilter>,
}

impl AvoidEntityGoal {
    /// Avoids `avoid_class` and its vanilla subclasses (a `Guardian` goal also flees elder guardians).
    #[must_use]
    pub fn new(
        avoid_class: &'static EntityType,
        max_dist: f64,
        walk_speed_modifier: f64,
        sprint_speed_modifier: f64,
    ) -> Self {
        let mut avoid_entity_targeting =
            TargetPredicate::create_attackable().set_base_max_distance(max_dist);
        avoid_entity_targeting.set_predicate(|target, _world| {
            EntityPredicate::ExceptCreativeOrSpectator.test(&target.entity)
        });
        Self {
            goal_control: Controls::MOVE,
            avoid_class: Some(avoid_class),
            max_dist,
            walk_speed_modifier,
            sprint_speed_modifier,
            avoid_entity_targeting,
            to_avoid: None,
            path: None,
            gate: None,
        }
    }

    /// Flees any mob passing the avoid predicate, for vanilla classes with no entity type of
    /// their own (`Monster`).
    #[must_use]
    pub fn any_mob(max_dist: f64, walk_speed_modifier: f64, sprint_speed_modifier: f64) -> Self {
        let mut goal = Self::new(
            &EntityType::PLAYER,
            max_dist,
            walk_speed_modifier,
            sprint_speed_modifier,
        );
        goal.avoid_class = None;
        goal
    }

    /// Vanilla's `avoidPredicate`: only flee entities passing `predicate`, alongside the
    /// no-creative-or-spectator rule.
    #[must_use]
    pub fn avoiding_only(
        mut self,
        predicate: impl Fn(&LivingEntity) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.avoid_entity_targeting
            .set_predicate(move |target, _world| {
                predicate(target) && EntityPredicate::ExceptCreativeOrSpectator.test(&target.entity)
            });
        self
    }

    /// A condition the mob must meet to start and keep fleeing.
    #[must_use]
    pub const fn gated_by(mut self, gate: MobFilter) -> Self {
        self.gate = Some(gate);
        self
    }
}

impl Goal for AvoidEntityGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if self.gate.is_some_and(|gate| !gate(mob)) {
            return false;
        }
        let entity = &mob.get_mob_entity().living_entity.entity;
        let world = entity.world.load();
        let mob_pos = entity.pos.load();
        let area = entity
            .bounding_box
            .load()
            .expand(self.max_dist, 3.0, self.max_dist);
        self.to_avoid = self.avoid_entity_targeting.get_nearest_entity(
            &world,
            goal_utils::get_entities_of_class(&world, self.avoid_class, &area),
            Some(mob),
            mob_pos,
        );
        let Some(to_avoid) = &self.to_avoid else {
            return false;
        };

        let threat_pos = to_avoid.get_entity().pos.load();
        let Some(pos) =
            default_random_pos::get_pos_away(mob, HORIZONTAL_RANGE, VERTICAL_RANGE, threat_pos)
        else {
            return false;
        };

        if threat_pos.squared_distance_to_vec(&pos) < threat_pos.squared_distance_to_vec(&mob_pos) {
            return false;
        }

        self.path = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .create_path(&mob.get_mob_entity().living_entity, pos, 0);
        self.path.is_some()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.gate.is_none_or(|gate| gate(mob)) && !mob.is_navigator_idle()
    }

    fn start(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .move_to_path(
                self.path.clone(),
                self.walk_speed_modifier,
                &mob_entity.living_entity,
            );
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(to_avoid) = &self.to_avoid else {
            return;
        };
        let mob_entity = mob.get_mob_entity();
        let mob_pos = mob_entity.living_entity.entity.pos.load();
        let speed = if mob_pos.squared_distance_to_vec(&to_avoid.get_entity().pos.load())
            < FAST_DISTANCE_SQ
        {
            self.sprint_speed_modifier
        } else {
            self.walk_speed_modifier
        };
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_speed(speed);
    }

    fn stop(&mut self, _mob: &dyn Mob) {
        self.to_avoid = None;
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
