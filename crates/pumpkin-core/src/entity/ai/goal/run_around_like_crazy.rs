use super::{Controls, Goal, to_goal_ticks};
use crate::entity::ai::pathfinder::NavigatorGoal;
use crate::entity::ai::util::default_random_pos;
use crate::entity::mob::Mob;
use pumpkin_data::entity::EntityStatus;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use std::sync::atomic::Ordering;

pub struct RunAroundLikeCrazyGoal {
    goal_control: Controls,
    speed_modifier: f64,
    pos_x: f64,
    pos_y: f64,
    pos_z: f64,
}

impl RunAroundLikeCrazyGoal {
    #[must_use]
    pub const fn new(speed_modifier: f64) -> Self {
        Self {
            goal_control: Controls::MOVE,
            speed_modifier,
            pos_x: 0.0,
            pos_y: 0.0,
            pos_z: 0.0,
        }
    }
}

impl Goal for RunAroundLikeCrazyGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(horse) = mob.as_abstract_horse() else {
            return false;
        };
        if horse.is_horse_tamed() || !mob.get_entity().has_passengers() {
            return false;
        }
        let Some(pos) = default_random_pos::get_pos(mob, 5, 4) else {
            return false;
        };
        self.pos_x = pos.x;
        self.pos_y = pos.y;
        self.pos_z = pos.z;
        true
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        mob.as_abstract_horse()
            .is_some_and(|horse| !horse.is_horse_tamed())
            && !mob.is_navigator_idle()
            && mob.get_entity().has_passengers()
    }

    fn start(&mut self, mob: &dyn Mob) {
        let mob_pos = mob.get_entity().pos.load();
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_progress(NavigatorGoal::new(
                mob_pos,
                Vector3::new(self.pos_x, self.pos_y, self.pos_z),
                self.speed_modifier,
            ));
    }

    /// Vanilla's taming roll: now and then the rider either tames the horse (more likely the
    /// higher its temper) or gets thrown off, which raises the temper for next time.
    fn tick(&mut self, mob: &dyn Mob) {
        let Some(horse) = mob.as_abstract_horse() else {
            return;
        };
        if horse.is_horse_tamed()
            || mob
                .get_entity()
                .random()
                .next_bounded_i32(to_goal_ticks(50))
                != 0
        {
            return;
        }
        let entity = mob.get_entity();
        let Some(passenger) = entity.get_first_passenger() else {
            return;
        };
        if let Some(player) = passenger.get_player() {
            let temper = horse.horse_temper().load(Ordering::Relaxed);
            let max_temper = horse.max_temper();
            if max_temper > 0 && mob.get_entity().random().next_bounded_i32(max_temper) < temper {
                horse.tame_with_name(player.gameprofile.id);
                return;
            }
            horse.modify_temper(5);
        }
        for passenger in entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            entity.remove_passenger(passenger.get_entity().entity_id);
        }
        horse.make_mad();
        entity
            .world
            .load()
            .send_entity_status(entity, EntityStatus::TamingFailed, None);
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
