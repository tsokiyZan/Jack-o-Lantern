use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::position::BlockPos;
use std::sync::atomic::Ordering;

use super::{Controls, Goal};
use crate::block::blocks::doors::DoorBlock;
use crate::entity::ai::util::goal_utils::has_ground_path_navigation;
use crate::entity::mob::Mob;

pub struct DoorInteractGoal {
    pub door_pos: BlockPos,
    pub has_door: bool,
    pub passed: bool,
    pub door_open_dir_x: f32,
    pub door_open_dir_z: f32,
}

impl Default for DoorInteractGoal {
    fn default() -> Self {
        Self::new()
    }
}

impl DoorInteractGoal {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            door_pos: BlockPos::ZERO,
            has_door: false,
            passed: false,
            door_open_dir_x: 0.0,
            door_open_dir_z: 0.0,
        }
    }

    pub fn is_open(&mut self, mob: &dyn Mob) -> bool {
        if !self.has_door {
            return false;
        }
        let world = mob.get_entity().world.load();
        let (block, _) = world.get_block_and_state_id(&self.door_pos);
        if !block.has_tag(&tag::Block::MINECRAFT_DOORS) {
            self.has_door = false;
            return false;
        }
        DoorBlock::is_open(&world, &self.door_pos)
    }

    pub fn set_open(&mut self, mob: &dyn Mob, open: bool) {
        if self.has_door {
            let world = mob.get_entity().world.load_full();
            let (block, _) = world.get_block_and_state_id(&self.door_pos);
            if block.has_tag(&tag::Block::MINECRAFT_DOORS) {
                DoorBlock::set_open(&world, &self.door_pos, open);
            }
        }
    }

    pub fn can_use(&mut self, mob: &dyn Mob) -> bool {
        if !has_ground_path_navigation(mob) {
            return false;
        }

        if !mob
            .get_entity()
            .horizontal_collision
            .load(Ordering::Relaxed)
        {
            return false;
        }

        // The nodes up to two past the next one, read from `getNavigation().getPath()`.
        let node_positions = mob.get_mob_entity().with_navigation(|navigation, _| {
            let path = navigation.get_path().filter(|path| !path.is_done())?;
            let limit = (path.get_next_node_index() + 2).min(path.get_node_count());
            Some(
                (0..limit)
                    .filter_map(|i| path.get_node(i).map(|node| node.pos))
                    .collect::<Vec<_>>(),
            )
        });
        let Some(node_positions) = node_positions else {
            return false;
        };

        let world = mob.get_entity().world.load();
        let mob_pos = mob.get_entity().pos.load();

        for node_pos in node_positions {
            let door_pos = BlockPos::new(node_pos.0.x, node_pos.0.y + 1, node_pos.0.z);
            let dx = mob_pos.x - f64::from(door_pos.0.x);
            let dz = mob_pos.z - f64::from(door_pos.0.z);
            let dist_sqr = dx * dx + dz * dz;

            if dist_sqr <= 2.25 {
                self.door_pos = door_pos;
                self.has_door = DoorBlock::is_wooden_door(&world, &self.door_pos);
                if self.has_door {
                    return true;
                }
            }
        }

        let above_pos = mob.get_entity().block_pos.load().up();
        self.door_pos = above_pos;
        self.has_door = DoorBlock::is_wooden_door(&world, &self.door_pos);
        self.has_door
    }

    #[must_use]
    pub const fn can_continue_to_use(&self) -> bool {
        !self.passed
    }

    pub fn start_interaction(&mut self, mob: &dyn Mob) {
        self.passed = false;
        let mob_pos = mob.get_entity().pos.load();
        self.door_open_dir_x = (self.door_pos.0.x as f32 + 0.5) - mob_pos.x as f32;
        self.door_open_dir_z = (self.door_pos.0.z as f32 + 0.5) - mob_pos.z as f32;
    }

    pub fn tick_interaction(&mut self, mob: &dyn Mob) {
        let mob_pos = mob.get_entity().pos.load();
        let new_door_dir_x = (self.door_pos.0.x as f32 + 0.5) - mob_pos.x as f32;
        let new_door_dir_z = (self.door_pos.0.z as f32 + 0.5) - mob_pos.z as f32;
        let dot = self.door_open_dir_x * new_door_dir_x + self.door_open_dir_z * new_door_dir_z;
        if dot < 0.0 {
            self.passed = true;
        }
    }
}

impl Goal for DoorInteractGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.can_use(mob)
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        self.can_continue_to_use()
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.start_interaction(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.tick_interaction(mob);
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        Controls::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::DoorInteractGoal;
    use crate::entity::ai::pathfinder::Navigator;
    use crate::entity::ai::pathfinder::node::Node;
    use crate::entity::ai::pathfinder::path::Path;
    use pumpkin_data::Block;
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::position::BlockPos;
    use pumpkin_util::math::vector2::Vector2;
    use pumpkin_util::math::vector3::Vector3;
    use pumpkin_world::chunk::ChunkData;
    use std::sync::atomic::Ordering;

    /// Vanilla `DoorInteractGoal.canUse` starts with `GoalUtils.hasGroundPathNavigation`: a
    /// zombie bumping into a door on its path is refused once its navigation isn't a ground one
    /// (a flying vehicle's, say).
    #[tokio::test]
    async fn can_use_needs_ground_navigation() {
        let (_folder, world) = crate::world::test_support::test_world();
        let chunk = ChunkData::empty_sync(0, 0);
        chunk.set_block_absolute_y(0, 101, 0, Block::OAK_DOOR.default_state.id);
        world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
        for (navigation, expected) in [(Navigator::ground(), true), (Navigator::flying(), false)] {
            let zombie = crate::entity::r#type::from_type(
                &EntityType::ZOMBIE,
                Vector3::new(0.5, 100.0, 0.5),
                &world,
                uuid::Uuid::new_v4(),
            );
            let mob = zombie.get_mob().unwrap();
            let mob_entity = mob.get_mob_entity();
            mob_entity.set_own_navigation(navigation);
            let nodes = vec![
                Node::new(BlockPos::new(0, 100, 0)),
                Node::new(BlockPos::new(0, 100, 5)),
            ];
            let path = Path::new(nodes, BlockPos::new(0, 100, 5), true);
            mob_entity
                .own_navigation()
                .move_to_path(Some(path), 1.0, &mob_entity.living_entity);
            mob.get_entity()
                .horizontal_collision
                .store(true, Ordering::Relaxed);
            assert_eq!(DoorInteractGoal::new().can_use(mob), expected);
        }
    }
}
