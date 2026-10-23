use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::block_properties::{
    Axis, PistonHeadLikeProperties, PistonType, StickyPistonLikeProperties,
};
use pumpkin_data::block_state::PistonBehavior;
use pumpkin_data::{Block, BlockDirection, BlockState};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};

use crate::entity::EntityBase;
use crate::world::{BlockFlags, World};

use super::BlockEntity;

thread_local! {
    /// Vanilla `PistonMovingBlockEntity.NOCLIP`: the direction a piston is moving an entity in.
    static NOCLIP: std::cell::Cell<Option<BlockDirection>> = const { std::cell::Cell::new(None) };
}

pub struct PistonBlockEntity {
    pub position: BlockPos,
    pub pushed_block_state: &'static BlockState,
    pub facing: BlockDirection,
    /// Vanilla `progress`.
    pub current_progress: AtomicCell<f32>,
    /// Vanilla `progressO`.
    pub last_progress: AtomicCell<f32>,
    pub extending: bool,
    pub source: bool,
    /// Vanilla `lastTicked`: the game time of the last tick. Not saved.
    last_ticked: AtomicI64,
    /// Vanilla `BlockEntity.remove`.
    removed: AtomicBool,
}

impl PistonBlockEntity {
    pub const ID: &'static str = "minecraft:piston";

    /// Vanilla `PistonMovingBlockEntity(pos, state, movedState, direction, extending, isSourcePiston)`.
    #[must_use]
    pub const fn new(
        position: BlockPos,
        pushed_block_state: &'static BlockState,
        facing: BlockDirection,
        extending: bool,
        source: bool,
    ) -> Self {
        Self {
            position,
            pushed_block_state,
            facing,
            current_progress: AtomicCell::new(0.0),
            last_progress: AtomicCell::new(0.0),
            extending,
            source,
            last_ticked: AtomicI64::new(0),
            removed: AtomicBool::new(false),
        }
    }

    /// Vanilla `getLastTicked`.
    pub fn get_last_ticked(&self) -> i64 {
        self.last_ticked.load(Ordering::Relaxed)
    }

    /// Vanilla `getMovementDirection`.
    pub const fn get_movement_direction(&self) -> BlockDirection {
        if self.extending {
            self.facing
        } else {
            self.facing.opposite()
        }
    }

    /// Vanilla `getExtendedProgress`: how far back from the block's final position
    /// the visual is at a given animation progress. Negative for extending.
    fn get_extended_progress(&self, progress: f32) -> f32 {
        if self.extending {
            progress - 1.0
        } else {
            1.0 - progress
        }
    }

    /// Vanilla `getCollisionShape`: block-local boxes.
    pub fn get_collision_shape(&self) -> Vec<BoundingBox> {
        let progress = self.current_progress.load();
        // `pistonHeadShape`: a retracting piston base keeps its extended shape where it stands.
        let base: Vec<BoundingBox> =
            if !self.extending && self.source && is_piston_base(self.pushed_block_state) {
                extended_base_state(self.pushed_block_state)
                    .get_block_collision_shapes_at(&self.position)
                    .collect()
            } else {
                Vec::new()
            };
        if f64::from(progress) < 1.0 && NOCLIP.get() == Some(self.get_movement_direction()) {
            return base;
        }
        let state = if self.source {
            // `SHORT`: `extending != (1.0F - progress < 0.25F)`.
            piston_head_state(self.facing, self.extending != (1.0 - progress < 0.25))
        } else {
            self.pushed_block_state
        };
        let extended = self.get_extended_progress(progress);
        let off = self.facing.to_offset();
        // `direction.getStep*() * extendedProgress`: int times float, widened to double.
        let offset = Vector3::new(
            f64::from(off.x as f32 * extended),
            f64::from(off.y as f32 * extended),
            f64::from(off.z as f32 * extended),
        );
        base.into_iter()
            .chain(
                state
                    .get_block_collision_shapes_at(&self.position)
                    .map(|b| b.shift(offset)),
            )
            .collect()
    }

    fn dir_vec(dir: BlockDirection, scale: f64) -> Vector3<f64> {
        let off = dir.to_offset();
        Vector3::new(
            f64::from(off.x) * scale,
            f64::from(off.y) * scale,
            f64::from(off.z) * scale,
        )
    }

    /// Vanilla `getCollisionRelatedBlockState`: a retracting source piston base pushes
    /// entities with its head.
    fn get_collision_related_block_state(&self) -> &'static BlockState {
        if !self.extending && self.source && is_piston_base(self.pushed_block_state) {
            let base = StickyPistonLikeProperties::from_state_id(self.pushed_block_state.id);
            let mut props = PistonHeadLikeProperties::default(&Block::PISTON_HEAD);
            props.short = self.current_progress.load() > 0.25;
            props.r#type =
                if Block::from_state_id(self.pushed_block_state.id) == &Block::STICKY_PISTON {
                    PistonType::Sticky
                } else {
                    PistonType::Normal
                };
            props.facing = base.facing;
            BlockState::from_id(props.to_state_id(&Block::PISTON_HEAD))
        } else {
            self.pushed_block_state
        }
    }

    /// Vanilla `moveCollidedEntities`.
    fn move_collided_entities(&self, world: &Arc<World>, new_progress: f32) {
        let movement = self.get_movement_direction();
        let delta_progress = f64::from(new_progress - self.current_progress.load());
        let shape: Vec<BoundingBox> = self
            .get_collision_related_block_state()
            .get_block_collision_shapes_at(&self.position)
            .collect();
        let Some(bounds) = shape.iter().copied().reduce(|a, b| a.minmax(&b)) else {
            return;
        };
        let aabb = self.move_by_position_and_progress(bounds);
        // `getEntities(null, ..)` uses `NO_SPECTATORS`.
        let entities: Vec<_> = world
            .get_all_at_box(&get_movement_area(aabb, movement, delta_progress).minmax(&aabb))
            .into_iter()
            .filter(|entity| !entity.is_spectator())
            .collect();
        if entities.is_empty() {
            return;
        }
        let cause_bounce = Block::from_state_id(self.pushed_block_state.id) == &Block::SLIME_BLOCK;

        for entity in entities {
            if entity.piston_push_reaction() == PistonBehavior::Ignore {
                continue;
            }
            if cause_bounce && entity.can_simulate_movement() {
                let base = entity.get_entity();
                let mut velocity = base.velocity.load();
                let step = movement.to_offset();
                match movement.to_axis() {
                    Axis::X => velocity.x = f64::from(step.x),
                    Axis::Y => velocity.y = f64::from(step.y),
                    Axis::Z => velocity.z = f64::from(step.z),
                }
                // `setDeltaMovement`: stored, nothing sent.
                base.velocity.store(velocity);
            }

            let mut delta = 0.0f64;
            for shape_aabb in &shape {
                let moving_aabb = get_movement_area(
                    self.move_by_position_and_progress(*shape_aabb),
                    movement,
                    delta_progress,
                );
                let entity_aabb = entity.get_entity().bounding_box.load();
                if moving_aabb.intersects(&entity_aabb) {
                    delta = java_max(
                        delta,
                        Self::get_movement(moving_aabb, movement, entity_aabb),
                    );
                    if delta >= delta_progress {
                        break;
                    }
                }
            }

            // `!(delta <= 0.0)`: NaN moves too.
            if !matches!(
                delta.partial_cmp(&0.0),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            ) {
                delta = java_min(delta, delta_progress) + 0.01;
                Self::move_entity_by_piston(movement, entity.as_ref(), delta, movement);
                if !self.extending && self.source {
                    Self::fix_entity_within_piston_base(
                        self.position,
                        entity.as_ref(),
                        movement,
                        delta_progress,
                    );
                }
            }
        }
    }

    /// Vanilla `moveStuckEntities`: a honey block carries the entities standing on it
    /// sideways.
    fn move_stuck_entities(&self, world: &Arc<World>, new_progress: f32) {
        if !self.is_sticky_for_entities() {
            return;
        }
        let movement = self.get_movement_direction();
        if movement.to_axis() == Axis::Y {
            return;
        }
        // `shape.max(Y)`: negative infinity for an empty shape.
        let sticky_top = self
            .pushed_block_state
            .get_block_collision_shapes_at(&self.position)
            .fold(f64::NEG_INFINITY, |top, b| top.max(b.max.y));
        let aabb = self.move_by_position_and_progress(BoundingBox::new(
            Vector3::new(0.0, sticky_top, 0.0),
            Vector3::new(1.0, 1.500_001_000_000_000_1, 1.0),
        ));
        let delta_progress = f64::from(new_progress - self.current_progress.load());
        // `getEntities(null, aabb, predicate)`: only the predicate, no spectator filter.
        for entity in world.get_all_at_box(&aabb) {
            if Self::matches_sticky_criteria(aabb, entity.as_ref(), self.position) {
                Self::move_entity_by_piston(movement, entity.as_ref(), delta_progress, movement);
            }
        }
    }

    /// Vanilla `matchesStickyCritera`.
    fn matches_sticky_criteria(aabb: BoundingBox, entity: &dyn EntityBase, pos: BlockPos) -> bool {
        let base = entity.get_entity();
        let p = base.pos.load();
        entity.piston_push_reaction() == PistonBehavior::Normal
            && base.on_ground.load(Ordering::Relaxed)
            && (base.is_supported_by(pos)
                || p.x >= aabb.min.x && p.x <= aabb.max.x && p.z >= aabb.min.z && p.z <= aabb.max.z)
    }

    /// Vanilla `isStickyForEntities`.
    fn is_sticky_for_entities(&self) -> bool {
        Block::from_state_id(self.pushed_block_state.id) == &Block::HONEY_BLOCK
    }

    /// Vanilla `moveEntityByPiston`: `NOCLIP` lets the entity pass through the moving block
    /// pushing it while it moves.
    fn move_entity_by_piston(
        piston_direction: BlockDirection,
        entity: &dyn EntityBase,
        delta: f64,
        movement: BlockDirection,
    ) {
        NOCLIP.set(Some(piston_direction));
        // `move(PISTON)`, `applyEffectsFromBlocks`, `removeLatestMovementRecording`.
        entity.move_by_piston(entity, Self::dir_vec(movement, delta));
        NOCLIP.set(None);
    }

    /// Vanilla `getMovement`: how far `aabb` must move along `movement` to be outside of
    /// `aabb_to_be_outside_of`.
    fn get_movement(
        aabb_to_be_outside_of: BoundingBox,
        movement: BlockDirection,
        aabb: BoundingBox,
    ) -> f64 {
        match movement {
            BlockDirection::East => aabb_to_be_outside_of.max.x - aabb.min.x,
            BlockDirection::West => aabb.max.x - aabb_to_be_outside_of.min.x,
            BlockDirection::Up => aabb_to_be_outside_of.max.y - aabb.min.y,
            BlockDirection::Down => aabb.max.y - aabb_to_be_outside_of.min.y,
            BlockDirection::South => aabb_to_be_outside_of.max.z - aabb.min.z,
            BlockDirection::North => aabb.max.z - aabb_to_be_outside_of.min.z,
        }
    }

    /// Vanilla `moveByPositionAndProgress`: a block-local box at the block's current
    /// position (`direction`, not the movement direction).
    fn move_by_position_and_progress(&self, aabb: BoundingBox) -> BoundingBox {
        let current = f64::from(self.get_extended_progress(self.current_progress.load()));
        let step = self.facing.to_offset();
        let pos = self.position.0;
        aabb.shift(Vector3::new(
            f64::from(pos.x) + current * f64::from(step.x),
            f64::from(pos.y) + current * f64::from(step.y),
            f64::from(pos.z) + current * f64::from(step.z),
        ))
    }

    /// Vanilla `fixEntityWithinPistonBase`: a retracting head pushes an entity inside the
    /// piston base back out the way the head came.
    fn fix_entity_within_piston_base(
        pos: BlockPos,
        entity: &dyn EntityBase,
        direction: BlockDirection,
        delta_progress: f64,
    ) {
        let entity_aabb = entity.get_entity().bounding_box.load();
        let block = BoundingBox::from_block(&pos);
        if entity_aabb.intersects(&block) {
            let opposite = direction.opposite();
            let delta = Self::get_movement(block, opposite, entity_aabb) + 0.01;
            let delta_intersected =
                Self::get_movement(block, opposite, entity_aabb.intersect(&block)) + 0.01;
            if (delta - delta_intersected).abs() < 0.01 {
                let delta = java_min(delta, delta_progress) + 0.01;
                Self::move_entity_by_piston(direction, entity, delta, opposite);
            }
        }
    }

    /// Vanilla `finalTick`: jumps the move to its end and places the block now.
    pub fn final_tick(&self, world: &Arc<World>) {
        if self.last_progress.load() < 1.0 {
            self.current_progress.store(1.0);
            self.last_progress.store(1.0);
            let pos = self.position;
            world.remove_block_entity(&pos);
            self.removed.store(true, Ordering::Relaxed);
            if world.get_block(&pos) == &Block::MOVING_PISTON {
                let new_state = if self.source {
                    Block::AIR.default_state.id
                } else {
                    world.update_from_neighbor_shapes(self.pushed_block_state.id, &pos)
                };
                // `setBlockAndUpdate`, then `neighborChanged` at the block's own position.
                world.set_block_state(&pos, new_state, BlockFlags::NOTIFY_ALL);
                world.update_neighbor(&pos, Block::from_state_id(new_state));
            }
        }
    }
}

/// Vanilla `PistonMath.getMovementArea`: the slab `amount` deep beyond `aabb`'s face
/// in `direction`.
#[must_use]
pub fn get_movement_area(aabb: BoundingBox, direction: BlockDirection, amount: f64) -> BoundingBox {
    let delta = amount * if direction.positive() { 1.0 } else { -1.0 };
    let min = java_min(delta, 0.0);
    let max = java_max(delta, 0.0);
    let (lo, hi) = (aabb.min, aabb.max);
    let (min_corner, max_corner) = match direction {
        BlockDirection::West => (
            Vector3::new(lo.x + min, lo.y, lo.z),
            Vector3::new(lo.x + max, hi.y, hi.z),
        ),
        BlockDirection::East => (
            Vector3::new(hi.x + min, lo.y, lo.z),
            Vector3::new(hi.x + max, hi.y, hi.z),
        ),
        BlockDirection::Down => (
            Vector3::new(lo.x, lo.y + min, lo.z),
            Vector3::new(hi.x, lo.y + max, hi.z),
        ),
        BlockDirection::Up => (
            Vector3::new(lo.x, hi.y + min, lo.z),
            Vector3::new(hi.x, hi.y + max, hi.z),
        ),
        BlockDirection::North => (
            Vector3::new(lo.x, lo.y, lo.z + min),
            Vector3::new(hi.x, hi.y, lo.z + max),
        ),
        BlockDirection::South => (
            Vector3::new(lo.x, lo.y, hi.z + min),
            Vector3::new(hi.x, hi.y, hi.z + max),
        ),
    };
    BoundingBox::new(min_corner, max_corner)
}

/// Java `Math.max`: NaN wins, `0.0` beats `-0.0`.
fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        a
    } else if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        b
    } else if a >= b {
        a
    } else {
        b
    }
}

/// Java `Math.min`: NaN wins, `-0.0` beats `0.0`.
fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        a
    } else if a == 0.0 && b == 0.0 && b.is_sign_negative() {
        b
    } else if a <= b {
        a
    } else {
        b
    }
}

/// Vanilla `movedState.getBlock() instanceof PistonBaseBlock`.
fn is_piston_base(state: &BlockState) -> bool {
    let block = Block::from_state_id(state.id);
    block == &Block::PISTON || block == &Block::STICKY_PISTON
}

/// Vanilla `movedState.setValue(PistonBaseBlock.EXTENDED, true)`.
fn extended_base_state(state: &BlockState) -> &'static BlockState {
    let mut props = StickyPistonLikeProperties::from_state_id(state.id);
    props.extended = true;
    BlockState::from_id(props.to_state_id(Block::from_state_id(state.id)))
}

/// Vanilla `PISTON_HEAD.defaultBlockState()` with `FACING` and `SHORT` set; `TYPE` stays default.
fn piston_head_state(facing: BlockDirection, short: bool) -> &'static BlockState {
    let mut props = PistonHeadLikeProperties::default(&Block::PISTON_HEAD);
    props.facing = facing.to_facing();
    props.short = short;
    BlockState::from_id(props.to_state_id(&Block::PISTON_HEAD))
}

const BLOCK_STATE: &str = "blockState";
const FACING: &str = "facing";
const LAST_PROGRESS: &str = "progress";
const EXTENDING: &str = "extending";
const SOURCE: &str = "source";

impl BlockEntity for PistonBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn tick(&self, world: &Arc<World>) {
        // Vanilla `BoundTickingBlockEntity.tick` skips a removed block entity; Pumpkin's
        // tick list is taken before the tick, so a `finalTick` earlier in it must stop this one.
        if self.removed.load(Ordering::Relaxed) {
            return;
        }
        self.last_ticked
            .store(world.get_world_age(), Ordering::Relaxed);
        let progress_o = self.current_progress.load();
        self.last_progress.store(progress_o);
        if progress_o >= 1.0 {
            let pos = self.position;
            world.remove_block_entity(&pos);
            self.removed.store(true, Ordering::Relaxed);
            if world.get_block(&pos) == &Block::MOVING_PISTON {
                let moved = self.pushed_block_state.id;
                let mut new_state = world.update_from_neighbor_shapes(moved, &pos);
                if new_state.to_state().is_air() {
                    // `setBlock(movedState, 340)`: `UPDATE_INVISIBLE` has no Pumpkin flag.
                    world.set_block_state(
                        &pos,
                        moved,
                        BlockFlags::MOVED
                            | BlockFlags::UPDATE_KNOWN_SHAPE
                            | BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK,
                    );
                    // `Block.updateOrDestroy(movedState, newState, level, pos, 3)`: the new
                    // state is air, so `destroyBlock(pos, true, null, 512)`.
                    if new_state != moved {
                        world.break_block(&pos, None, BlockFlags::NOTIFY_ALL);
                    }
                } else {
                    if new_state.is_waterlogged() {
                        new_state = new_state
                            .to_block()
                            .set_waterlogged(new_state, false)
                            .unwrap_or(new_state);
                    }
                    world.set_block_state(
                        &pos,
                        new_state,
                        BlockFlags::NOTIFY_ALL | BlockFlags::MOVED,
                    );
                    world.update_neighbor(&pos, Block::from_state_id(new_state));
                }
            }
        } else {
            let new_progress = progress_o + 0.5;
            self.move_collided_entities(world, new_progress);
            self.move_stuck_entities(world, new_progress);
            self.current_progress.store(if new_progress >= 1.0 {
                1.0
            } else {
                new_progress
            });
        }
    }

    /// Vanilla `preRemoveSideEffects`.
    fn on_block_replaced(self: Arc<Self>, world: &Arc<World>, _position: &BlockPos) {
        self.final_tick(world);
    }

    /// Vanilla `setRemoved`.
    fn set_removed(&self, _world: &World) {
        self.removed.store(true, Ordering::Relaxed);
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        // Vanilla `PistonMovingBlockEntity.loadAdditional`: air when missing.
        let pushed_block_state = nbt
            .get(BLOCK_STATE)
            .and_then(Block::state_from_nbt)
            .map_or(Block::AIR.default_state, BlockState::from_id);
        let facing = nbt.get_byte(FACING).unwrap_or(0);
        let last_progress = nbt.get_float(LAST_PROGRESS).unwrap_or(0.0);
        let extending = nbt.get_bool(EXTENDING).unwrap_or(false);
        let source = nbt.get_bool(SOURCE).unwrap_or(false);
        let be = Self::new(
            position,
            pushed_block_state,
            BlockDirection::from_index(facing as u8).unwrap_or(BlockDirection::Down),
            extending,
            source,
        );
        be.current_progress.store(last_progress);
        be.last_progress.store(last_progress);
        be
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put(BLOCK_STATE, Block::state_to_nbt(self.pushed_block_state.id));
        nbt.put_byte(FACING, self.facing.to_index() as i8);
        nbt.put_float(LAST_PROGRESS, self.last_progress.load());
        nbt.put_bool(EXTENDING, self.extending);
        nbt.put_bool(SOURCE, self.source);
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        nbt.put(BLOCK_STATE, Block::state_to_nbt(self.pushed_block_state.id));
        nbt.put_byte(FACING, self.facing.to_index() as i8);
        nbt.put_float(LAST_PROGRESS, self.last_progress.load());
        nbt.put_bool(EXTENDING, self.extending);
        nbt.put_bool(SOURCE, self.source);
        // TODO: duplicated code because of async :c
        Some(nbt)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::block_properties::Facing;

    fn piston_be(
        pushed_block_state: &'static BlockState,
        facing: BlockDirection,
        progress: f32,
        extending: bool,
        source: bool,
    ) -> PistonBlockEntity {
        let be = PistonBlockEntity::new(
            BlockPos::new(0, 0, 0),
            pushed_block_state,
            facing,
            extending,
            source,
        );
        be.current_progress.store(progress);
        be.last_progress.store(progress);
        be
    }

    fn corners(boxes: &[BoundingBox]) -> Vec<[f64; 6]> {
        boxes
            .iter()
            .map(|b| [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z])
            .collect()
    }

    fn with_noclip<T>(direction: BlockDirection, f: impl FnOnce() -> T) -> T {
        NOCLIP.set(Some(direction));
        let result = f();
        NOCLIP.set(None);
        result
    }

    #[test]
    fn pushed_block_shape_trails_by_progress_and_noclip_hides_it() {
        let be = piston_be(
            Block::STONE.default_state,
            BlockDirection::East,
            0.5,
            true,
            false,
        );
        let expected = vec![[-0.5, 0.0, 0.0, 0.5, 1.0, 1.0]];
        assert_eq!(corners(&be.get_collision_shape()), expected);
        assert!(with_noclip(BlockDirection::East, || be.get_collision_shape()).is_empty());
        // NOCLIP for another piston's direction does not hide this block.
        assert_eq!(
            corners(&with_noclip(BlockDirection::West, || be.get_collision_shape())),
            expected
        );
    }

    #[test]
    fn retracting_source_piston_keeps_extended_base_under_noclip() {
        let mut props = StickyPistonLikeProperties::default(&Block::PISTON);
        props.facing = Facing::East;
        props.extended = false;
        let moved = BlockState::from_id(props.to_state_id(&Block::PISTON));
        let be = piston_be(moved, BlockDirection::East, 0.5, false, true);
        assert_eq!(
            corners(&with_noclip(BlockDirection::West, || be.get_collision_shape())),
            vec![[0.0, 0.0, 0.0, 0.75, 1.0, 1.0]]
        );
    }

    #[test]
    fn movement_area_is_slab_beyond_leading_face() {
        let aabb = BoundingBox::new(Vector3::new(0.0, 0.0, 0.0), Vector3::new(1.0, 1.0, 1.0));
        let area = |d| corners(&[get_movement_area(aabb, d, 0.5)])[0];
        assert_eq!(area(BlockDirection::East), [1.0, 0.0, 0.0, 1.5, 1.0, 1.0]);
        assert_eq!(area(BlockDirection::West), [-0.5, 0.0, 0.0, 0.0, 1.0, 1.0]);
        assert_eq!(area(BlockDirection::Up), [0.0, 1.0, 0.0, 1.0, 1.5, 1.0]);
        assert_eq!(area(BlockDirection::Down), [0.0, -0.5, 0.0, 1.0, 0.0, 1.0]);
        assert_eq!(area(BlockDirection::South), [0.0, 0.0, 1.0, 1.0, 1.0, 1.5]);
        assert_eq!(area(BlockDirection::North), [0.0, 0.0, -0.5, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn retracting_sticky_base_collides_as_its_head() {
        let mut props = StickyPistonLikeProperties::default(&Block::STICKY_PISTON);
        props.facing = Facing::Up;
        let moved = BlockState::from_id(props.to_state_id(&Block::STICKY_PISTON));
        let head = |progress| {
            let state = piston_be(moved, BlockDirection::Up, progress, false, true)
                .get_collision_related_block_state();
            assert_eq!(Block::from_state_id(state.id), &Block::PISTON_HEAD);
            PistonHeadLikeProperties::from_state_id(state.id)
        };
        let early = head(0.0);
        assert!(!early.short);
        assert_eq!(early.r#type, PistonType::Sticky);
        assert_eq!(early.facing, Facing::Up);
        assert!(head(0.5).short);
        // Extending or not the source: the moved state itself.
        let be = piston_be(moved, BlockDirection::Up, 0.5, true, true);
        assert_eq!(be.get_collision_related_block_state().id, moved.id);
    }

    #[test]
    fn java_min_max_keep_nan_and_signed_zero() {
        assert!(java_max(f64::NAN, 1.0).is_nan());
        assert!(java_max(1.0, f64::NAN).is_nan());
        assert!(java_min(f64::NAN, 1.0).is_nan());
        assert!(java_min(-0.0, 0.0).is_sign_negative());
        assert!(java_max(-0.0, 0.0).is_sign_positive());
    }
}
