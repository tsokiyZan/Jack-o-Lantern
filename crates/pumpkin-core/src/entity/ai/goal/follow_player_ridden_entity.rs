use std::sync::Arc;

use pumpkin_data::HorizontalFacingExt;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::Goal;
use crate::entity::EntityBase;
use crate::entity::mob::Mob;
use crate::entity::player::Player;

/// Returns the player steering `vehicle`, or `None` when `vehicle` is not the kind of
/// entity being followed or nobody steers it: vanilla's `getControllingPassenger`
/// narrowed to the followed class.
pub type ControllingPlayer = fn(&dyn EntityBase) -> Option<Arc<Player>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FollowEntityGoal {
    GoToEntity,
    GoInEntityDirection,
}

/// Vanilla `FollowPlayerRiddenEntityGoal`: swims along behind, then ahead of, a
/// vehicle a player is steering. It claims no controls, so it runs beside the
/// mob's other goals.
pub struct FollowPlayerRiddenEntityGoal {
    controlling_player: ControllingPlayer,
    time_to_recalc_path: i32,
    following: Option<Arc<Player>>,
    current_goal: FollowEntityGoal,
}

impl FollowPlayerRiddenEntityGoal {
    #[must_use]
    pub const fn new(controlling_player: ControllingPlayer) -> Self {
        Self {
            controlling_player,
            time_to_recalc_path: 0,
            following: None,
            current_goal: FollowEntityGoal::GoToEntity,
        }
    }

    fn nearby_controlling_players(&self, mob: &dyn Mob) -> Vec<Arc<Player>> {
        let entity = mob.get_entity();
        let area = entity.bounding_box.load().expand_all(5.0);
        entity
            .world
            .load()
            .get_entities_at_box(&area)
            .iter()
            .filter_map(|vehicle| (self.controlling_player)(vehicle.as_ref()))
            .collect()
    }

    fn move_to(mob: &dyn Mob, pos: BlockPos) {
        let mob_entity = mob.get_mob_entity();
        mob_entity.own_navigation().move_to_coords(
            f64::from(pos.0.x),
            f64::from(pos.0.y),
            f64::from(pos.0.z),
            1.0,
            &mob_entity.living_entity,
        );
    }
}

impl Goal for FollowPlayerRiddenEntityGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if self
            .following
            .as_ref()
            .is_some_and(|player| player.get_entity().has_moved_horizontally_recently())
        {
            return true;
        }
        self.nearby_controlling_players(mob)
            .iter()
            .any(|player| player.get_entity().has_moved_horizontally_recently())
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        self.following.as_ref().is_some_and(|player| {
            let entity = player.get_entity();
            entity.has_vehicle() && entity.has_moved_horizontally_recently()
        })
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(player) = self.nearby_controlling_players(mob).into_iter().next() {
            self.following = Some(player);
        }
        self.time_to_recalc_path = 0;
        self.current_goal = FollowEntityGoal::GoToEntity;
    }

    fn stop(&mut self, _mob: &dyn Mob) {
        self.following = None;
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(following) = self.following.clone() else {
            return;
        };
        let speed = if self.current_goal == FollowEntityGoal::GoInEntityDirection {
            0.01
        } else {
            0.015
        };
        let living = &mob.get_mob_entity().living_entity;
        living
            .entity
            .update_velocity_from_input(living.movement_input.load(), speed);
        living.make_move(mob);

        self.time_to_recalc_path -= 1;
        if self.time_to_recalc_path > 0 {
            return;
        }
        self.time_to_recalc_path = self.get_tick_count(10);

        let player = following.get_entity();
        let player_pos = player.block_pos.load();
        let distance = mob.get_entity().pos.load().sub(&player.pos.load()).length();
        // `getMotionDirection` is `getDirection` for a player.
        let direction = player.get_horizontal_facing().to_block_direction();
        match self.current_goal {
            FollowEntityGoal::GoToEntity => {
                let behind = player_pos.offset(direction.opposite().to_offset());
                Self::move_to(mob, behind.offset(Vector3::new(0, -1, 0)));
                if distance < 4.0 {
                    self.time_to_recalc_path = 0;
                    self.current_goal = FollowEntityGoal::GoInEntityDirection;
                }
            }
            FollowEntityGoal::GoInEntityDirection => {
                let go_to = player_pos.offset(direction.to_offset() * 10);
                Self::move_to(mob, go_to.offset(Vector3::new(0, -1, 0)));
                if distance > 12.0 {
                    self.time_to_recalc_path = 0;
                    self.current_goal = FollowEntityGoal::GoToEntity;
                }
            }
        }
    }
}
