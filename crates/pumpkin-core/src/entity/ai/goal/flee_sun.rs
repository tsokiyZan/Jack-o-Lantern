use super::{Controls, Goal};
use crate::entity::ai::pathfinder::NavigatorGoal;
use crate::entity::mob::Mob;
use crate::entity::mob::sun_burn;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

pub struct FleeSunGoal {
    goal_control: Controls,
    speed_modifier: f64,
    wanted_x: f64,
    wanted_y: f64,
    wanted_z: f64,
}

impl FleeSunGoal {
    #[must_use]
    pub const fn new(speed_modifier: f64) -> Self {
        Self {
            goal_control: Controls::MOVE,
            speed_modifier,
            wanted_x: 0.0,
            wanted_y: 0.0,
            wanted_z: 0.0,
        }
    }

    /// Vanilla `setWantedPos`: picks a hiding spot, returning whether there was one.
    pub fn set_wanted_pos(&mut self, mob: &dyn Mob) -> bool {
        let Some(pos) = Self::get_hide_pos(mob) else {
            return false;
        };
        self.wanted_x = pos.x;
        self.wanted_y = pos.y;
        self.wanted_z = pos.z;
        true
    }

    /// Vanilla `getHidePos`: ten tries within 10/3/10 blocks for a spot out of the sky
    /// that the mob values below zero.
    fn get_hide_pos(mob: &dyn Mob) -> Option<Vector3<f64>> {
        let pos = mob.get_entity().block_pos.load();
        let world = mob.get_entity().world.load();

        for _ in 0..10 {
            let random_pos = {
                let mut random = mob.get_entity().random();
                let dx = random.next_bounded_i32(20) - 10;
                let dy = random.next_bounded_i32(6) - 3;
                let dz = random.next_bounded_i32(20) - 10;
                BlockPos::new(pos.0.x + dx, pos.0.y + dy, pos.0.z + dz)
            };
            if !world.can_see_sky(&random_pos) && mob.get_walk_target_value(&random_pos) < 0.0 {
                return Some(Vector3::new(
                    f64::from(random_pos.0.x) + 0.5,
                    f64::from(random_pos.0.y),
                    f64::from(random_pos.0.z) + 0.5,
                ));
            }
        }

        None
    }
}

impl Goal for FleeSunGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if mob.get_mob_entity().get_target().is_some() {
            return false;
        }
        let entity = mob.get_entity();
        let world = entity.world.load();
        if !world.is_bright_outside()
            || !entity.is_on_fire()
            || !world.can_see_sky(&entity.block_pos.load())
            || sun_burn::is_protected(mob)
        {
            return false;
        }

        self.set_wanted_pos(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let is_idle = mob.is_navigator_idle();
        !is_idle
    }

    fn start(&mut self, mob: &dyn Mob) {
        let mob_pos = mob.get_entity().pos.load();
        let mut navigator = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        navigator.set_progress(NavigatorGoal::new(
            mob_pos,
            Vector3::new(self.wanted_x, self.wanted_y, self.wanted_z),
            self.speed_modifier,
        ));
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
