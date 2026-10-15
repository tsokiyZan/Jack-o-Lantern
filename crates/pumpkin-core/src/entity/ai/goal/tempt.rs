use std::sync::Arc;

use super::{Controls, Goal, to_goal_ticks};
use crate::entity::EntityBase;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::{ai::pathfinder::NavigatorGoal, mob::Mob, player::Player};
use pumpkin_data::attributes::Attributes;
use pumpkin_data::item::Item;
use pumpkin_util::math::vector3::Vector3;

const DEFAULT_STOP_DISTANCE: f64 = 2.5;
/// Following stops once the player moves further than this while being watched.
const SCARE_RANGE_SQ: f64 = 36.0;
const SCARE_MOVE_SQ: f64 = 0.010_000_000_000_000_002;
const SCARE_ROTATION: f64 = 5.0;

pub struct TemptGoal {
    goal_control: Controls,
    speed: f64,
    tempt_items: &'static [&'static Item],
    can_scare: bool,
    stop_distance: f64,
    targeting_conditions: TargetPredicate,
    target_player: Option<Arc<dyn EntityBase>>,
    watched_pos: Vector3<f64>,
    watched_yaw: f64,
    watched_pitch: f64,
    cooldown: i32,
    running: bool,
}

impl TemptGoal {
    #[must_use]
    pub fn new(speed: f64, tempt_items: &'static [&'static Item], can_scare: bool) -> Self {
        Self::with_stop_distance(speed, tempt_items, can_scare, DEFAULT_STOP_DISTANCE)
    }

    #[must_use]
    pub fn with_stop_distance(
        speed: f64,
        tempt_items: &'static [&'static Item],
        can_scare: bool,
        stop_distance: f64,
    ) -> Self {
        Self {
            goal_control: Controls::MOVE | Controls::LOOK,
            speed,
            tempt_items,
            can_scare,
            stop_distance,
            targeting_conditions: TargetPredicate::create_non_attackable().ignore_visibility(),
            target_player: None,
            watched_pos: Vector3::new(0.0, 0.0, 0.0),
            watched_yaw: 0.0,
            watched_pitch: 0.0,
            cooldown: 0,
            running: false,
        }
    }

    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.running
    }

    fn is_tempt_item(&self, stack: &pumpkin_data::item_stack::ItemStack) -> bool {
        stack.item_count > 0 && self.tempt_items.iter().any(|i| i.id == stack.item.id)
    }

    fn is_holding_tempt_item(&self, player: &Player) -> bool {
        self.is_tempt_item(&player.inventory().held_item())
            || self.is_tempt_item(&player.inventory().off_hand_item())
    }

    /// Vanilla `TEMPT_TARGETING` ranged by `tempt_range`, with the held-item selector.
    fn find_tempting_player(&mut self, mob: &dyn Mob) -> Option<Arc<dyn EntityBase>> {
        let mob_entity = mob.get_mob_entity();
        self.targeting_conditions.base_max_distance = mob_entity
            .living_entity
            .get_attribute_value(&Attributes::TEMPT_RANGE);
        let world = mob_entity.living_entity.entity.world.load();
        let players = world.players.load();
        // The selector has no side effects, so filtering first picks the same player.
        self.targeting_conditions.get_nearest_entity(
            &world,
            players
                .iter()
                .filter(|player| self.is_holding_tempt_item(player))
                .map(|player| Arc::clone(player) as Arc<dyn EntityBase>),
            Some(mob),
            mob_entity.living_entity.entity.pos.load(),
        )
    }
}

impl Goal for TemptGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return false;
        }
        self.target_player = self.find_tempting_player(mob);
        self.target_player.is_some()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        if self.can_scare {
            let Some(player) = self.target_player.clone() else {
                return false;
            };
            let entity = player.get_entity();
            let player_pos = entity.pos.load();
            let mob_pos = mob.get_mob_entity().living_entity.entity.pos.load();

            if mob_pos.squared_distance_to_vec(&player_pos) < SCARE_RANGE_SQ {
                if player_pos.squared_distance_to_vec(&self.watched_pos) > SCARE_MOVE_SQ {
                    return false;
                }
                if (f64::from(entity.pitch.load()) - self.watched_pitch).abs() > SCARE_ROTATION
                    || (f64::from(entity.yaw.load()) - self.watched_yaw).abs() > SCARE_ROTATION
                {
                    return false;
                }
            } else {
                self.watched_pos = player_pos;
            }

            self.watched_pitch = f64::from(entity.pitch.load());
            self.watched_yaw = f64::from(entity.yaw.load());
        }

        self.can_start(mob)
    }

    fn start(&mut self, _mob: &dyn Mob) {
        // Vanilla stores only the position; the rotations carry over from the last run.
        if let Some(player) = &self.target_player {
            self.watched_pos = player.get_entity().pos.load();
        }
        self.running = true;
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(player) = &self.target_player else {
            return;
        };
        let mob_entity = mob.get_mob_entity();
        let player_pos = player.get_entity().pos.load();

        mob_entity
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .look_at_with_range(
                player_pos.x,
                player.get_entity().get_eye_y(),
                player_pos.z,
                mob.get_max_head_rotation() + 20.0,
                mob.get_max_look_pitch_change(),
            );

        let mob_pos = mob_entity.living_entity.entity.pos.load();
        let mut navigator = mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if mob_pos.squared_distance_to_vec(&player_pos) < self.stop_distance * self.stop_distance {
            navigator.stop();
        } else {
            navigator.set_progress(NavigatorGoal::new(mob_pos, player_pos, self.speed));
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.target_player = None;
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
        self.cooldown = to_goal_ticks(100);
        self.running = false;
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
