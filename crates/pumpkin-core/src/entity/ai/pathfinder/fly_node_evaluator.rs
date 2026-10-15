use pumpkin_data::Block;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use rand::RngExt;

use crate::entity::ai::pathfinder::{
    node::{Node, PathType, Target},
    node_evaluator::{MobData, NodeEvaluator},
    pathfinding_context::PathfindingContext,
    walk_node_evaluator::WalkNodeEvaluator,
};

const SMALL_MOB_SIZE: f64 = 1.0;
const SMALL_MOB_INFLATED_START_NODE_BOUNDING_BOX: f32 = 1.1;
const MAX_START_NODE_CANDIDATES: usize = 10;

pub struct FlyNodeEvaluator {
    pub walk: WalkNodeEvaluator,
}

impl FlyNodeEvaluator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            walk: WalkNodeEvaluator::with_path_type(Self::get_path_type_static),
        }
    }

    pub fn get_cached_path_type(&mut self, pos: Vector3<i32>) -> PathType {
        self.walk.get_cached_path_type(pos)
    }

    pub fn can_start_at(&mut self, pos: Vector3<i32>) -> bool {
        let path_type = self.get_cached_path_type(pos);
        self.walk.get_mob_penalty(path_type) >= 0.0
    }

    fn find_accepted_node(&mut self, x: i32, y: i32, z: i32) -> Option<Node> {
        let path_type = self.get_cached_path_type(Vector3::new(x, y, z));
        let path_cost = self.walk.get_mob_penalty(path_type);
        (path_cost >= 0.0).then(|| {
            let best = self.walk.base.node_mut(BlockPos::new(x, y, z));
            best.path_type = path_type;
            best.cost_malus = best.cost_malus.max(path_cost);
            if path_type == PathType::Walkable {
                best.cost_malus += 1.0;
            }
            *best
        })
    }

    const fn has_malus(node: Option<&Node>) -> bool {
        if let Some(n) = node {
            n.cost_malus >= 0.0
        } else {
            false
        }
    }

    const fn is_open(node: Option<&Node>) -> bool {
        if let Some(n) = node { !n.closed } else { false }
    }

    /// Vanilla `FlyNodeEvaluator.iteratePathfindingStartNodeCandidatePositions`.
    fn start_node_candidate_positions(mob_data: &MobData) -> Vec<Vector3<i32>> {
        let half_width = f64::from(mob_data.width) / 2.0;
        let min_x = mob_data.position.x - half_width;
        let max_x = mob_data.position.x + half_width;
        let min_y = mob_data.position.y;
        let max_y = mob_data.position.y + f64::from(mob_data.height);
        let min_z = mob_data.position.z - half_width;
        let max_z = mob_data.position.z + half_width;
        let x_size = max_x - min_x;
        let y_size = max_y - min_y;
        let z_size = max_z - min_z;

        // `AABB.getSize`: the average side length.
        let is_small_mob = (x_size + y_size + z_size) / 3.0 < SMALL_MOB_SIZE;
        if !is_small_mob {
            let block_y = mob_data.position.y.floor() as i32;
            return vec![
                Vector3::new(min_x.floor() as i32, block_y, min_z.floor() as i32),
                Vector3::new(min_x.floor() as i32, block_y, max_z.floor() as i32),
                Vector3::new(max_x.floor() as i32, block_y, min_z.floor() as i32),
                Vector3::new(max_x.floor() as i32, block_y, max_z.floor() as i32),
            ];
        }

        let inflated = f64::from(SMALL_MOB_INFLATED_START_NODE_BOUNDING_BOX);
        let z_padding = (inflated - z_size).max(0.0);
        let x_padding = (inflated - x_size).max(0.0);
        let y_padding = (inflated - y_size).max(0.0);
        let from_x = (min_x - x_padding).floor() as i32;
        let from_y = (min_y - y_padding).floor() as i32;
        let from_z = (min_z - z_padding).floor() as i32;
        let to_x = (max_x + x_padding).floor() as i32;
        let to_y = (max_y + y_padding).floor() as i32;
        let to_z = (max_z + z_padding).floor() as i32;

        // Vanilla `BlockPos.randomBetweenClosed` draws from the mob's random; Pumpkin
        // mobs have no random of their own, so this is the thread RNG `Mob::get_random` uses.
        let mut random = rand::rng();
        (0..MAX_START_NODE_CANDIDATES)
            .map(|_| {
                Vector3::new(
                    from_x + random.random_range(0..=to_x - from_x),
                    from_y + random.random_range(0..=to_y - from_y),
                    from_z + random.random_range(0..=to_z - from_z),
                )
            })
            .collect()
    }

    /// Vanilla `FlyNodeEvaluator.getPathType`.
    pub fn get_path_type_static(context: &mut PathfindingContext, pos: Vector3<i32>) -> PathType {
        let mut block_path_type = context.get_path_type_from_state(pos);
        if block_path_type == PathType::Open && pos.y > context.min_y() {
            let below_pos = Vector3::new(pos.x, pos.y - 1, pos.z);
            let below_type = context.get_path_type_from_state(below_pos);
            if below_type == PathType::DamageFire || below_type == PathType::Lava {
                block_path_type = PathType::DamageFire;
            } else if below_type == PathType::DamageOther {
                block_path_type = PathType::DamageOther;
            } else if below_type == PathType::Cocoa {
                block_path_type = PathType::Cocoa;
            } else if below_type == PathType::Fence {
                if below_pos != context.mob_position() {
                    block_path_type = PathType::Fence;
                }
            } else {
                block_path_type = if below_type != PathType::Walkable
                    && below_type != PathType::Open
                    && below_type != PathType::Water
                {
                    PathType::Walkable
                } else {
                    PathType::Open
                };
            }
        }

        if block_path_type == PathType::Walkable || block_path_type == PathType::Open {
            block_path_type = context.get_node_type_from_neighbors(pos, block_path_type);
        }

        block_path_type
    }
}

impl NodeEvaluator for FlyNodeEvaluator {
    fn prepare(&mut self, context: PathfindingContext, mob_data: MobData) {
        self.walk.prepare(context, mob_data);
    }

    fn done(&mut self) {
        self.walk.done();
    }

    fn get_start(&mut self) -> Option<Node> {
        let mob_data = self.walk.base.mob_data?;
        let block_x = mob_data.position.x.floor() as i32;
        let block_z = mob_data.position.z.floor() as i32;

        let start_y = if self.walk.base.can_float && mob_data.is_in_water {
            let ctx = self.walk.base.context.as_ref()?;
            let mut y = mob_data.position.y.floor() as i32;
            // Vanilla stops one block above the water: at the first block that is
            // not `Blocks.WATER`, which waterlogged blocks are not either.
            while ctx.world().get_block(&BlockPos::new(block_x, y, block_z)) == &Block::WATER {
                y += 1;
            }
            y
        } else {
            (mob_data.position.y + 0.5).floor() as i32
        };

        let start_pos = Vector3::new(block_x, start_y, block_z);
        if !self.can_start_at(start_pos) {
            for candidate in Self::start_node_candidate_positions(&mob_data) {
                if self.can_start_at(candidate) {
                    return Some(self.walk.get_start_node(candidate));
                }
            }
        }

        Some(self.walk.get_start_node(start_pos))
    }

    fn get_target(&mut self, pos: BlockPos) -> Target {
        Target::new(self.walk.base.get_node(pos))
    }

    #[expect(clippy::too_many_lines)]
    fn get_neighbors(&mut self, current: &Node, out: &mut Vec<Node>) {
        let x = current.pos.0.x;
        let y = current.pos.0.y;
        let z = current.pos.0.z;

        let south = self.find_accepted_node(x, y, z + 1);
        if Self::is_open(south.as_ref())
            && let Some(n) = south
        {
            out.push(n);
        }

        let west = self.find_accepted_node(x - 1, y, z);
        if Self::is_open(west.as_ref())
            && let Some(n) = west
        {
            out.push(n);
        }

        let east = self.find_accepted_node(x + 1, y, z);
        if Self::is_open(east.as_ref())
            && let Some(n) = east
        {
            out.push(n);
        }

        let north = self.find_accepted_node(x, y, z - 1);
        if Self::is_open(north.as_ref())
            && let Some(n) = north
        {
            out.push(n);
        }

        let up = self.find_accepted_node(x, y + 1, z);
        if Self::is_open(up.as_ref())
            && let Some(n) = up
        {
            out.push(n);
        }

        let down = self.find_accepted_node(x, y - 1, z);
        if Self::is_open(down.as_ref())
            && let Some(n) = down
        {
            out.push(n);
        }

        let south_up = self.find_accepted_node(x, y + 1, z + 1);
        if Self::is_open(south_up.as_ref())
            && Self::has_malus(south.as_ref())
            && Self::has_malus(up.as_ref())
            && let Some(n) = south_up
        {
            out.push(n);
        }

        let west_up = self.find_accepted_node(x - 1, y + 1, z);
        if Self::is_open(west_up.as_ref())
            && Self::has_malus(west.as_ref())
            && Self::has_malus(up.as_ref())
            && let Some(n) = west_up
        {
            out.push(n);
        }

        let east_up = self.find_accepted_node(x + 1, y + 1, z);
        if Self::is_open(east_up.as_ref())
            && Self::has_malus(east.as_ref())
            && Self::has_malus(up.as_ref())
            && let Some(n) = east_up
        {
            out.push(n);
        }

        let north_up = self.find_accepted_node(x, y + 1, z - 1);
        if Self::is_open(north_up.as_ref())
            && Self::has_malus(north.as_ref())
            && Self::has_malus(up.as_ref())
            && let Some(n) = north_up
        {
            out.push(n);
        }

        let south_down = self.find_accepted_node(x, y - 1, z + 1);
        if Self::is_open(south_down.as_ref())
            && Self::has_malus(south.as_ref())
            && Self::has_malus(down.as_ref())
            && let Some(n) = south_down
        {
            out.push(n);
        }

        let west_down = self.find_accepted_node(x - 1, y - 1, z);
        if Self::is_open(west_down.as_ref())
            && Self::has_malus(west.as_ref())
            && Self::has_malus(down.as_ref())
            && let Some(n) = west_down
        {
            out.push(n);
        }

        let east_down = self.find_accepted_node(x + 1, y - 1, z);
        if Self::is_open(east_down.as_ref())
            && Self::has_malus(east.as_ref())
            && Self::has_malus(down.as_ref())
            && let Some(n) = east_down
        {
            out.push(n);
        }

        let north_down = self.find_accepted_node(x, y - 1, z - 1);
        if Self::is_open(north_down.as_ref())
            && Self::has_malus(north.as_ref())
            && Self::has_malus(down.as_ref())
            && let Some(n) = north_down
        {
            out.push(n);
        }

        let north_east = self.find_accepted_node(x + 1, y, z - 1);
        if Self::is_open(north_east.as_ref())
            && Self::has_malus(north.as_ref())
            && Self::has_malus(east.as_ref())
            && let Some(n) = north_east
        {
            out.push(n);
        }

        let south_east = self.find_accepted_node(x + 1, y, z + 1);
        if Self::is_open(south_east.as_ref())
            && Self::has_malus(south.as_ref())
            && Self::has_malus(east.as_ref())
            && let Some(n) = south_east
        {
            out.push(n);
        }

        let north_west = self.find_accepted_node(x - 1, y, z - 1);
        if Self::is_open(north_west.as_ref())
            && Self::has_malus(north.as_ref())
            && Self::has_malus(west.as_ref())
            && let Some(n) = north_west
        {
            out.push(n);
        }

        let south_west = self.find_accepted_node(x - 1, y, z + 1);
        if Self::is_open(south_west.as_ref())
            && Self::has_malus(south.as_ref())
            && Self::has_malus(west.as_ref())
            && let Some(n) = south_west
        {
            out.push(n);
        }

        let north_east_up = self.find_accepted_node(x + 1, y + 1, z - 1);
        if Self::is_open(north_east_up.as_ref())
            && Self::has_malus(north_east.as_ref())
            && Self::has_malus(north.as_ref())
            && Self::has_malus(east.as_ref())
            && Self::has_malus(up.as_ref())
            && Self::has_malus(north_up.as_ref())
            && Self::has_malus(east_up.as_ref())
            && let Some(n) = north_east_up
        {
            out.push(n);
        }

        let south_east_up = self.find_accepted_node(x + 1, y + 1, z + 1);
        if Self::is_open(south_east_up.as_ref())
            && Self::has_malus(south_east.as_ref())
            && Self::has_malus(south.as_ref())
            && Self::has_malus(east.as_ref())
            && Self::has_malus(up.as_ref())
            && Self::has_malus(south_up.as_ref())
            && Self::has_malus(east_up.as_ref())
            && let Some(n) = south_east_up
        {
            out.push(n);
        }

        let north_west_up = self.find_accepted_node(x - 1, y + 1, z - 1);
        if Self::is_open(north_west_up.as_ref())
            && Self::has_malus(north_west.as_ref())
            && Self::has_malus(north.as_ref())
            && Self::has_malus(west.as_ref())
            && Self::has_malus(up.as_ref())
            && Self::has_malus(north_up.as_ref())
            && Self::has_malus(west_up.as_ref())
            && let Some(n) = north_west_up
        {
            out.push(n);
        }

        let south_west_up = self.find_accepted_node(x - 1, y + 1, z + 1);
        if Self::is_open(south_west_up.as_ref())
            && Self::has_malus(south_west.as_ref())
            && Self::has_malus(south.as_ref())
            && Self::has_malus(west.as_ref())
            && Self::has_malus(up.as_ref())
            && Self::has_malus(south_up.as_ref())
            && Self::has_malus(west_up.as_ref())
            && let Some(n) = south_west_up
        {
            out.push(n);
        }

        let north_east_down = self.find_accepted_node(x + 1, y - 1, z - 1);
        if Self::is_open(north_east_down.as_ref())
            && Self::has_malus(north_east.as_ref())
            && Self::has_malus(north.as_ref())
            && Self::has_malus(east.as_ref())
            && Self::has_malus(down.as_ref())
            && Self::has_malus(north_down.as_ref())
            && Self::has_malus(east_down.as_ref())
            && let Some(n) = north_east_down
        {
            out.push(n);
        }

        let south_east_down = self.find_accepted_node(x + 1, y - 1, z + 1);
        if Self::is_open(south_east_down.as_ref())
            && Self::has_malus(south_east.as_ref())
            && Self::has_malus(south.as_ref())
            && Self::has_malus(east.as_ref())
            && Self::has_malus(down.as_ref())
            && Self::has_malus(south_down.as_ref())
            && Self::has_malus(east_down.as_ref())
            && let Some(n) = south_east_down
        {
            out.push(n);
        }

        let north_west_down = self.find_accepted_node(x - 1, y - 1, z - 1);
        if Self::is_open(north_west_down.as_ref())
            && Self::has_malus(north_west.as_ref())
            && Self::has_malus(north.as_ref())
            && Self::has_malus(west.as_ref())
            && Self::has_malus(down.as_ref())
            && Self::has_malus(north_down.as_ref())
            && Self::has_malus(west_down.as_ref())
            && let Some(n) = north_west_down
        {
            out.push(n);
        }

        let south_west_down = self.find_accepted_node(x - 1, y - 1, z + 1);
        if Self::is_open(south_west_down.as_ref())
            && Self::has_malus(south_west.as_ref())
            && Self::has_malus(south.as_ref())
            && Self::has_malus(west.as_ref())
            && Self::has_malus(down.as_ref())
            && Self::has_malus(south_down.as_ref())
            && Self::has_malus(west_down.as_ref())
            && let Some(n) = south_west_down
        {
            out.push(n);
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

impl Default for FlyNodeEvaluator {
    fn default() -> Self {
        Self::new()
    }
}
