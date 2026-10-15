use pumpkin_data::BlockDirection;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

use crate::entity::ai::pathfinder::{
    node::{Node, PathType, Target},
    node_evaluator::{MobData, NodeEvaluator},
    pathfinding_context::PathfindingContext,
    walk_node_evaluator::WalkNodeEvaluator,
};

pub struct AmphibiousNodeEvaluator {
    pub walk: WalkNodeEvaluator,
    prefers_shallow_swimming: bool,
    /// Vanilla `Frog.FrogNodeEvaluator`'s `getStart` in water.
    starts_at_bounding_box_min: bool,
    old_walkable_cost: f32,
    old_water_border_cost: f32,
}

impl AmphibiousNodeEvaluator {
    #[must_use]
    pub fn new(prefers_shallow_swimming: bool) -> Self {
        let mut walk = WalkNodeEvaluator::with_path_type(Self::get_path_type_static);
        walk.is_amphibious = true;
        Self {
            walk,
            prefers_shallow_swimming,
            starts_at_bounding_box_min: false,
            old_walkable_cost: 0.0,
            old_water_border_cost: 0.0,
        }
    }

    /// Vanilla `Frog.FrogNodeEvaluator`, which always prefers shallow swimming.
    #[must_use]
    pub fn frog() -> Self {
        let mut evaluator = Self::new(true);
        evaluator.walk = WalkNodeEvaluator::with_path_type(Self::frog_get_path_type);
        evaluator.walk.is_amphibious = true;
        evaluator.starts_at_bounding_box_min = true;
        evaluator
    }

    /// Vanilla `Frog.FrogNodeEvaluator.getPathType`: open above a block frogs prefer to
    /// jump to.
    fn frog_get_path_type(context: &mut PathfindingContext, pos: Vector3<i32>) -> PathType {
        let below = BlockPos(pos.add_raw(0, -1, 0));
        if pumpkin_data::Block::from_state_id(context.get_block_state(&below).id)
            .has_tag(&tag::Block::MINECRAFT_FROG_PREFER_JUMP_TO)
        {
            PathType::Open
        } else {
            Self::get_path_type_static(context, pos)
        }
    }

    /// Vanilla `AmphibiousNodeEvaluator.getPathType`.
    pub fn get_path_type_static(context: &mut PathfindingContext, pos: Vector3<i32>) -> PathType {
        let block_path_type = context.get_path_type_from_state(pos);
        if block_path_type == PathType::Water {
            for dir in BlockDirection::all() {
                let neighbor_pos = pos + dir.to_offset();
                if context.get_path_type_from_state(neighbor_pos) == PathType::Blocked {
                    return PathType::WaterBorder;
                }
            }
            PathType::Water
        } else {
            WalkNodeEvaluator::get_path_type_static(context, pos)
        }
    }

    fn is_vertical_neighbor_valid(vertical_node: Option<&Node>, pos: &Node) -> bool {
        WalkNodeEvaluator::is_neighbor_valid(vertical_node, pos)
            && vertical_node.is_some_and(|n| n.path_type == PathType::Water)
    }
}

impl NodeEvaluator for AmphibiousNodeEvaluator {
    fn prepare(&mut self, context: PathfindingContext, mut mob_data: MobData) {
        mob_data.set_pathfinding_malus(PathType::Water, 0.0);
        self.old_walkable_cost = mob_data.get_pathfinding_malus(PathType::Walkable);
        mob_data.set_pathfinding_malus(PathType::Walkable, 6.0);
        self.old_water_border_cost = mob_data.get_pathfinding_malus(PathType::WaterBorder);
        mob_data.set_pathfinding_malus(PathType::WaterBorder, 4.0);
        self.walk.prepare(context, mob_data);
    }

    fn done(&mut self) {
        if let Some(ref mut mob) = self.walk.base.mob_data {
            mob.set_pathfinding_malus(PathType::Walkable, self.old_walkable_cost);
            mob.set_pathfinding_malus(PathType::WaterBorder, self.old_water_border_cost);
        }
        self.walk.done();
    }

    fn get_start(&mut self) -> Option<Node> {
        let mob_data = self.walk.base.mob_data?;
        if mob_data.is_in_water {
            let half_width = f64::from(mob_data.width) / 2.0;
            let min_x = (mob_data.position.x - half_width).floor() as i32;
            let min_y = if self.starts_at_bounding_box_min {
                mob_data.position.y.floor() as i32
            } else {
                (mob_data.position.y + 0.5).floor() as i32
            };
            let min_z = (mob_data.position.z - half_width).floor() as i32;
            Some(self.walk.get_start_node(Vector3::new(min_x, min_y, min_z)))
        } else {
            self.walk.get_start()
        }
    }

    fn get_target(&mut self, pos: BlockPos) -> Target {
        Target::new(self.walk.base.get_node(pos))
    }

    fn get_neighbors(&mut self, current: &Node, out: &mut Vec<Node>) {
        self.walk.get_neighbors(current, out);
        let block_path_type_above = self
            .walk
            .get_cached_path_type(current.pos.0.add_raw(0, 1, 0));
        let block_path_type_current = self.walk.get_cached_path_type(current.pos.0);

        let jump_size = if self.walk.get_mob_penalty(block_path_type_above) >= 0.0
            && block_path_type_current != PathType::StickyHoney
        {
            self.walk.get_mob_jump_height().floor() as i32
        } else {
            0
        };

        let pos_height = self.walk.get_floor_level(&current.pos);
        let up_node = self.walk.find_accepted_node(
            current.pos.0.x,
            current.pos.0.y + 1,
            current.pos.0.z,
            (jump_size - 1).max(0),
            pos_height,
            BlockDirection::Up,
            block_path_type_current,
        );
        let down_node = self.walk.find_accepted_node(
            current.pos.0.x,
            current.pos.0.y - 1,
            current.pos.0.z,
            jump_size,
            pos_height,
            BlockDirection::Down,
            block_path_type_current,
        );

        // Looking up and down can retype the current node itself, which vanilla
        // sees through its shared `Node`.
        let current = &self.walk.base.get_node(current.pos);
        if Self::is_vertical_neighbor_valid(up_node.as_ref(), current)
            && let Some(n) = up_node
        {
            out.push(n);
        }

        if Self::is_vertical_neighbor_valid(down_node.as_ref(), current)
            && block_path_type_current != PathType::Trapdoor
            && let Some(n) = down_node
        {
            out.push(n);
        }

        let Some(sea_level) = self
            .walk
            .base
            .context
            .as_ref()
            .map(PathfindingContext::sea_level)
        else {
            return;
        };
        for neighbor in out.iter_mut() {
            if neighbor.path_type == PathType::Water
                && self.prefers_shallow_swimming
                && neighbor.pos.0.y < sea_level - 10
            {
                let node = self.walk.base.node_mut(neighbor.pos);
                node.cost_malus += 1.0;
                *neighbor = *node;
            }
        }
    }

    fn get_path_type_of_mob(
        &mut self,
        context: &mut PathfindingContext,
        pos: Vector3<i32>,
        mob_data: &MobData,
    ) -> PathType {
        self.walk.get_path_type_of_mob(context, pos, mob_data)
    }

    fn get_path_type(&mut self, context: &mut PathfindingContext, pos: Vector3<i32>) -> PathType {
        self.walk.get_path_type(context, pos)
    }

    fn set_can_pass_doors(&mut self, can_pass: bool) {
        self.walk.set_can_pass_doors(can_pass);
    }

    fn set_can_open_doors(&mut self, can_open: bool) {
        self.walk.set_can_open_doors(can_open);
    }

    fn set_can_float(&mut self, can_float: bool) {
        self.walk.set_can_float(can_float);
    }

    fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        self.walk.set_can_walk_over_fences(can_walk);
    }

    fn can_pass_doors(&self) -> bool {
        self.walk.can_pass_doors()
    }

    fn can_open_doors(&self) -> bool {
        self.walk.can_open_doors()
    }

    fn can_float(&self) -> bool {
        self.walk.can_float()
    }

    fn can_walk_over_fences(&self) -> bool {
        self.walk.can_walk_over_fences()
    }
}
