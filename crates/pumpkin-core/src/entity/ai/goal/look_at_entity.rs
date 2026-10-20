use super::{Controls, Goal};

use crate::entity::EntityBase;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::ai::util::goal_utils;
use crate::entity::mob::Mob;
use crate::entity::predicate::EntityPredicate;
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use std::sync::{Arc, Weak};

pub struct LookAtEntityGoal {
    goal_control: Controls,
    target: Option<Arc<dyn EntityBase>>,
    range: f32,
    look_time: i32,
    chance: f32,
    look_forward: bool,
    /// `None` is vanilla's `Mob.class`: any mob.
    target_type: Option<&'static EntityType>,
    target_predicate: TargetPredicate,
}

impl LookAtEntityGoal {
    #[must_use]
    pub fn new(
        mob_weak: Weak<dyn Mob>,
        target_type: &'static EntityType,
        range: f32,
        chance: f32,
        look_forward: bool,
    ) -> Self {
        let target_predicate = Self::create_target_predicate(mob_weak, target_type, range);
        Self {
            goal_control: Controls::LOOK,
            target: None,
            range,
            look_time: 0,
            chance,
            look_forward,
            target_type: Some(target_type),
            target_predicate,
        }
    }

    /// Vanilla `LookAtPlayerGoal(mob, Mob.class, range)`: looks at any nearby mob.
    #[must_use]
    pub fn at_mobs(range: f32) -> Box<Self> {
        let mut target_predicate = TargetPredicate::create_non_attackable();
        target_predicate.base_max_distance = f64::from(range);
        Box::new(Self {
            goal_control: Controls::LOOK,
            target: None,
            range,
            look_time: 0,
            chance: 0.02,
            look_forward: false,
            target_type: None,
            target_predicate,
        })
    }

    #[must_use]
    pub fn with_default(
        mob_weak: Weak<dyn Mob>,
        target_type: &'static EntityType,
        range: f32,
    ) -> Box<Self> {
        Box::new(Self::new(mob_weak, target_type, range, 0.02, false))
    }

    /// Vanilla `lookAtContext`: in range and, for players, not riding the mob.
    pub(crate) fn create_target_predicate(
        mob_weak: Weak<dyn Mob>,
        target_type: &'static EntityType,
        range: f32,
    ) -> TargetPredicate {
        let mut target_predicate = TargetPredicate::create_non_attackable();
        target_predicate.base_max_distance = f64::from(range);
        if target_type == &EntityType::PLAYER {
            target_predicate.set_predicate(move |living_entity, _world| {
                mob_weak.upgrade().is_some_and(|mob_arc| {
                    let predicate = EntityPredicate::Rides(mob_arc.get_entity());
                    predicate.test(&living_entity.entity)
                })
            });
        }
        target_predicate
    }
}

impl Goal for LookAtEntityGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if mob.get_entity().random().next_f32() >= self.chance {
            return false;
        }

        let mob_entity = mob.get_mob_entity();

        {
            let mob_target = mob_entity
                .target
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if mob_target.is_some() {
                self.target.clone_from(&mob_target);
            }
        }

        let entity = &mob_entity.living_entity.entity;
        let world = entity.world.load();
        let pos = entity.pos.load();
        let search_pos = Vector3::new(pos.x, entity.get_eye_y(), pos.z);

        self.target = if self.target_type == Some(&EntityType::PLAYER) {
            self.target_predicate
                .get_nearest_player(&world, Some(mob), search_pos)
        } else {
            let range = f64::from(self.range);
            let area = entity.bounding_box.load().expand(range, 3.0, range);
            self.target_predicate.get_nearest_entity(
                &world,
                goal_utils::get_entities_of_class(&world, self.target_type, &area),
                Some(mob),
                search_pos,
            )
        };

        self.target.is_some()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let mob_entity = mob.get_mob_entity();
        if let Some(target) = &self.target {
            if !target.get_entity().is_alive() {
                return false;
            }
            let mob_pos = mob_entity.living_entity.entity.pos.load();
            let target_pos = target.get_entity().pos.load();
            if mob_pos.squared_distance_to_vec(&target_pos) > f64::from(self.range * self.range) {
                return false;
            }
            return self.look_time > 0;
        }
        false
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.look_time = self.get_tick_count(40 + mob.get_entity().random().next_bounded_i32(40));
    }

    fn stop(&mut self, _mob: &dyn Mob) {
        self.target = None;
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        if let Some(target) = &self.target
            && target.get_entity().is_alive()
        {
            let target_entity = target.get_entity();
            let target_pos = target_entity.pos.load();
            let look_y = if self.look_forward {
                mob_entity.living_entity.entity.get_eye_y()
            } else {
                target_entity.get_eye_y()
            };
            mob_entity
                .look_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .look_at(mob, target_pos.x, look_y, target_pos.z);
            self.look_time -= 1;
        }
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
