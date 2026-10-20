use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::chunk::ChunkHeightmapType;

use super::{Controls, Goal, to_goal_ticks};
use crate::entity::ai::util::land_random_pos;
use crate::entity::mob::Mob;
use crate::world::World;

/// Vanilla `StrollThroughVillageGoal.DISTANCE_THRESHOLD`.
const DISTANCE_THRESHOLD: f64 = 10.0;

/// Vanilla `StrollThroughVillageGoal`: at night, near a village, walk towards its centre.
pub struct StrollThroughVillageGoal {
    interval: i32,
    wanted_pos: Option<BlockPos>,
}

impl StrollThroughVillageGoal {
    #[must_use]
    pub const fn new(interval: i32) -> Self {
        Self {
            interval: to_goal_ticks(interval),
            wanted_pos: None,
        }
    }

    /// Vanilla `getHeightmapPos(MOTION_BLOCKING_NO_LEAVES, pos)`.
    fn motion_blocking_no_leaves_pos(world: &World, pos: BlockPos) -> BlockPos {
        BlockPos::new(
            pos.0.x,
            world.get_heightmap_height(
                ChunkHeightmapType::MotionBlockingNoLeaves,
                pos.0.x,
                pos.0.z,
            ) + 1,
            pos.0.z,
        )
    }

    /// Vanilla `navigation.moveTo(x, y, z, 1.0)` on a block position, returning whether
    /// a path was found.
    fn move_to(mob: &dyn Mob, pos: BlockPos) -> bool {
        let mob_entity = mob.get_mob_entity();
        let living = &mob_entity.living_entity;
        let mut navigator = mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        navigator
            .create_path(living, pos.to_f64(), 1)
            .is_some_and(|path| navigator.move_to_path(Some(path), 1.0, living))
    }

    /// Vanilla `moveRandomly`.
    fn move_randomly(mob: &dyn Mob) {
        let pos = mob.get_entity().block_pos.load();
        let (dx, dz) = {
            let mut random = mob.get_entity().random();
            (random.next_bounded_i32(16), random.next_bounded_i32(16))
        };
        let offset = BlockPos::new(pos.0.x - 8 + dx, pos.0.y, pos.0.z - 8 + dz);
        let path_target =
            Self::motion_blocking_no_leaves_pos(&mob.get_entity().world.load(), offset);
        Self::move_to(mob, path_target);
    }
}

impl Goal for StrollThroughVillageGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if mob.get_controlling_passenger().is_some() {
            return false;
        }
        let entity = mob.get_entity();
        let world = entity.world.load_full();
        if world.is_bright_outside() {
            return false;
        }
        if mob.get_entity().random().next_bounded_i32(self.interval) != 0 {
            return false;
        }
        if !world.is_close_to_village(&entity.block_pos.load(), 6) {
            return false;
        }

        let land_pos = land_random_pos::get_pos_scored(mob, 15, 7, |pos| {
            f64::from(-world.sections_to_village(pos))
        });
        self.wanted_pos = land_pos.map(|pos| BlockPos::floored(pos.x, pos.y, pos.z));
        self.wanted_pos.is_some()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let Some(wanted_pos) = self.wanted_pos else {
            return false;
        };
        let navigator = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !navigator.is_done() && navigator.get_target_pos() == Some(wanted_pos)
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(wanted_pos) = self.wanted_pos else {
            return;
        };
        let entity = mob.get_entity();
        let position = entity.pos.load();
        let wanted_center = wanted_pos.to_centered_f64();
        if !mob.is_navigator_idle()
            || wanted_center.squared_distance_to_vec(&position)
                < DISTANCE_THRESHOLD * DISTANCE_THRESHOLD
        {
            return;
        }

        let long_distance_target = wanted_pos.to_f64();
        let long_distance_target = (position - long_distance_target) * 0.4 + long_distance_target;
        let direction = long_distance_target - position;
        // Vanilla `Vec3.normalize`, which leaves a near-zero vector at zero.
        let length = direction.length();
        let direction = if length < f64::from(1.0E-5f32) {
            Vector3::new(0.0, 0.0, 0.0)
        } else {
            Vector3::new(
                direction.x / length,
                direction.y / length,
                direction.z / length,
            )
        };
        let move_target = direction * DISTANCE_THRESHOLD + position;
        let path_target = Self::motion_blocking_no_leaves_pos(
            &entity.world.load(),
            BlockPos::floored(move_target.x, move_target.y, move_target.z),
        );
        if !Self::move_to(mob, path_target) {
            Self::move_randomly(mob);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}
