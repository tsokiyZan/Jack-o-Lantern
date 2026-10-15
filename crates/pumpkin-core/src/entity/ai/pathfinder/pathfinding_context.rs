use pumpkin_data::{
    Block, BlockState,
    block_properties::{OakDoorLikeProperties, OakFenceGateLikeProperties},
    fluid::Fluid,
    tag::{self, Taggable},
};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};

use crate::{
    block::blocks::doors::can_open_door,
    entity::ai::pathfinder::{
        node::{Coordinate, PathComputationType, PathType},
        node_evaluator::is_burning_block,
        path_type_cache::PathTypeCache,
    },
    world::World,
};

use rustc_hash::FxHashMap;
use std::sync::Arc;

/// The margin vanilla's `BlockCollisions` widens the scanned block range by.
const COLLISION_EPSILON: f64 = 1.0E-7;

/// Vanilla `BlockState.hasLargeCollisionShape`: the collision shape leaves its block.
fn has_large_collision_shape(state: &BlockState) -> bool {
    state
        .get_block_collision_shapes_at(&BlockPos::ZERO)
        .any(|shape| {
            shape.min.x < 0.0
                || shape.min.y < 0.0
                || shape.min.z < 0.0
                || shape.max.x > 1.0
                || shape.max.y > 1.0
                || shape.max.z > 1.0
        })
}

pub struct PathfindingContext {
    path_type_cache: Option<PathTypeCache>,
    mob_position: Vector3<i32>,
    world: Arc<World>,
    collision_cache: FxHashMap<[u64; 6], bool>,
}

impl PathfindingContext {
    #[must_use]
    pub fn new(mob_position: Vector3<i32>, world: Arc<World>) -> Self {
        Self {
            path_type_cache: Some(PathTypeCache::new()),
            mob_position,
            world,
            collision_cache: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn with_cache(mob_position: Vector3<i32>, world: Arc<World>, cache: PathTypeCache) -> Self {
        Self {
            path_type_cache: Some(cache),
            mob_position,
            world,
            collision_cache: FxHashMap::default(),
        }
    }

    #[must_use]
    pub const fn mob_position(&self) -> Vector3<i32> {
        self.mob_position
    }

    #[must_use]
    pub const fn world(&self) -> &Arc<World> {
        &self.world
    }

    /// Vanilla `level.getMinY()`.
    #[must_use]
    pub fn min_y(&self) -> i32 {
        self.world.dimension.min_y
    }

    /// Vanilla `level.getSeaLevel()`.
    #[must_use]
    pub fn sea_level(&self) -> i32 {
        self.world.sea_level
    }

    pub fn get_path_type_from_state(&mut self, pos: Vector3<i32>) -> PathType {
        if let Some(ref cache) = self.path_type_cache
            && let Some(pt) = cache.get(pos)
        {
            return pt;
        }

        let pt = self.compute_path_type_from_state(pos);

        if let Some(ref mut cache) = self.path_type_cache {
            cache.insert(pos, pt);
        }

        pt
    }

    /// Vanilla `WalkNodeEvaluator.getPathTypeFromState`.
    #[must_use]
    pub fn compute_path_type_from_state(&self, pos: Vector3<i32>) -> PathType {
        let block_pos = pos.as_blockpos();
        let (block, state) = self.world.get_block_and_state(&block_pos);

        if state.is_air() {
            return PathType::Open;
        }

        if block.has_tag(&tag::Block::MINECRAFT_TRAPDOORS)
            || block == &Block::LILY_PAD
            || block == &Block::BIG_DRIPLEAF
        {
            return PathType::Trapdoor;
        }

        if block == &Block::POWDER_SNOW {
            return PathType::PowderSnow;
        }

        if block == &Block::CACTUS || block == &Block::SWEET_BERRY_BUSH {
            return PathType::DamageOther;
        }

        if block == &Block::HONEY_BLOCK {
            return PathType::StickyHoney;
        }

        if block == &Block::COCOA {
            return PathType::Cocoa;
        }

        if block == &Block::WITHER_ROSE || block.has_tag(&tag::Block::MINECRAFT_SPELEOTHEMS) {
            return PathType::DamageCautious;
        }

        let fluid = World::fluid_type_from_state_id(state.id);
        if fluid.has_tag(&tag::Fluid::MINECRAFT_LAVA) {
            return PathType::Lava;
        }

        if is_burning_block(block, state) {
            return PathType::DamageFire;
        }

        if block.has_tag(&tag::Block::MINECRAFT_DOORS) {
            return if OakDoorLikeProperties::from_state_id(state.id).open {
                PathType::DoorOpen
            } else if can_open_door(block) {
                PathType::DoorWoodClosed
            } else {
                PathType::DoorIronClosed
            };
        }

        if block.has_tag(&tag::Block::MINECRAFT_RAILS) {
            return PathType::Rail;
        }

        if block.has_tag(&tag::Block::MINECRAFT_LEAVES) {
            return PathType::Leaves;
        }

        if block.has_tag(&tag::Block::MINECRAFT_FENCES)
            || block.has_tag(&tag::Block::MINECRAFT_WALLS)
            || (block.has_tag(&tag::Block::MINECRAFT_FENCE_GATES)
                && !OakFenceGateLikeProperties::from_state_id(state.id).open)
        {
            return PathType::Fence;
        }

        if !self
            .world
            .block_registry
            .is_pathfindable(block, state, PathComputationType::Land)
        {
            return PathType::Blocked;
        }

        if fluid.has_tag(&tag::Fluid::MINECRAFT_WATER) {
            PathType::Water
        } else {
            PathType::Open
        }
    }

    pub fn get_land_node_type(&mut self, pos: Vector3<i32>) -> PathType {
        let raw_type = self.get_path_type_from_state(pos);

        if raw_type == PathType::Open && pos.y > self.min_y() {
            let below_type = self.get_path_type_from_state(Vector3::new(pos.x, pos.y - 1, pos.z));
            return match below_type {
                PathType::Open | PathType::Water | PathType::Lava | PathType::Walkable => {
                    PathType::Open
                }
                PathType::DamageFire => PathType::DamageFire,
                PathType::DamageOther => PathType::DamageOther,
                PathType::StickyHoney => PathType::StickyHoney,
                PathType::PowderSnow => PathType::DangerPowderSnow,
                PathType::DamageCautious => PathType::DamageCautious,
                PathType::Trapdoor => PathType::DangerTrapdoor,
                _ => self.get_node_type_from_neighbors(pos, PathType::Walkable),
            };
        }

        raw_type
    }

    pub fn get_node_type_from_neighbors(
        &mut self,
        pos: Vector3<i32>,
        fallback: PathType,
    ) -> PathType {
        for dx in -1..=1i32 {
            for dy in -1..=1i32 {
                for dz in -1..=1i32 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }

                    let neighbor_type = self.get_path_type_from_state(Vector3::new(
                        pos.x + dx,
                        pos.y + dy,
                        pos.z + dz,
                    ));

                    if neighbor_type == PathType::DamageOther {
                        return PathType::DangerOther;
                    }
                    if neighbor_type == PathType::DamageFire || neighbor_type == PathType::Lava {
                        return PathType::DangerFire;
                    }
                    if neighbor_type == PathType::Water {
                        return PathType::WaterBorder;
                    }
                    if neighbor_type == PathType::DamageCautious {
                        return PathType::DamageCautious;
                    }
                }
            }
        }

        fallback
    }

    #[must_use]
    pub fn get_floor_level(&self, pos: &BlockPos) -> f64 {
        let target = pos.down();
        let state = self.world.get_block_state(&target);
        let max_y = state
            .get_block_collision_shapes_at(&target)
            .map(|s| s.max.y)
            .fold(0.0f64, f64::max);
        f64::from(target.0.y) + max_y
    }

    /// Whether `bb` hits a block, vanilla's `!level.noCollision(mob, bb)` over a
    /// `PathNavigationRegion`, which has no entity collisions.
    pub fn has_collision(&mut self, bb: &BoundingBox) -> bool {
        let key = [
            bb.min.x.to_bits(),
            bb.min.y.to_bits(),
            bb.min.z.to_bits(),
            bb.max.x.to_bits(),
            bb.max.y.to_bits(),
            bb.max.z.to_bits(),
        ];

        if let Some(&cached) = self.collision_cache.get(&key) {
            return cached;
        }

        let collided = self.has_block_collision(bb);
        self.collision_cache.insert(key, collided);
        collided
    }

    /// Vanilla `BlockCollisions`: scans one block past the box so shapes taller or
    /// wider than a block (fences, walls) reach into it from outside.
    fn has_block_collision(&self, bb: &BoundingBox) -> bool {
        let x0 = (bb.min.x - COLLISION_EPSILON).floor() as i32 - 1;
        let x1 = (bb.max.x + COLLISION_EPSILON).floor() as i32 + 1;
        let y0 = (bb.min.y - COLLISION_EPSILON).floor() as i32 - 1;
        let y1 = (bb.max.y + COLLISION_EPSILON).floor() as i32 + 1;
        let z0 = (bb.min.z - COLLISION_EPSILON).floor() as i32 - 1;
        let z1 = (bb.max.z + COLLISION_EPSILON).floor() as i32 + 1;

        for z in z0..=z1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    // `Cursor3D.getNextType`: how many axes sit on the scan's outer shell.
                    let shell_axes = u8::from(x == x0 || x == x1)
                        + u8::from(y == y0 || y == y1)
                        + u8::from(z == z0 || z == z1);
                    if shell_axes == 3 {
                        continue;
                    }

                    let pos = BlockPos::new(x, y, z);
                    let (block, state) = self.world.get_block_and_state(&pos);
                    if (shell_axes == 1 && !has_large_collision_shape(state))
                        || (shell_axes == 2 && block != &Block::MOVING_PISTON)
                    {
                        continue;
                    }

                    if state
                        .get_block_collision_shapes_at(&pos)
                        .any(|shape| shape.at_pos(pos).intersects(bb))
                    {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Vanilla `getFluidState(pos).getType()`, waterlogged blocks included.
    #[must_use]
    pub fn get_fluid(&self, pos: &BlockPos) -> &'static Fluid {
        World::fluid_type_from_state_id(self.world.get_block_state_id(pos))
    }

    #[must_use]
    pub fn is_water(&self, pos: &BlockPos) -> bool {
        self.get_fluid(pos).has_tag(&tag::Fluid::MINECRAFT_WATER)
    }

    /// Whether the fluid at `pos` is in `#entity_floatable`.
    #[must_use]
    pub fn is_floatable(&self, pos: &BlockPos) -> bool {
        self.get_fluid(pos)
            .has_tag(&tag::Fluid::MINECRAFT_ENTITY_FLOATABLE)
    }

    #[must_use]
    pub fn is_air(&self, pos: &BlockPos) -> bool {
        self.world.get_block_state(pos).is_air()
    }

    #[must_use]
    pub fn get_block_state(&self, pos: &BlockPos) -> &'static BlockState {
        self.world.get_block_state(pos)
    }

    #[must_use]
    pub fn is_pathfindable(&self, pos: &BlockPos, computation_type: PathComputationType) -> bool {
        let (block, state) = self.world.get_block_and_state(pos);
        self.world
            .block_registry
            .is_pathfindable(block, state, computation_type)
    }

    pub fn clear_caches(&mut self) {
        if let Some(ref mut cache) = self.path_type_cache {
            cache.clear();
        }
        self.collision_cache.clear();
    }
}
