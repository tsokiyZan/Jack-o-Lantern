use pumpkin_data::{BlockDirection, BlockId, BlockState, BlockStateId, entity::EntityType};
use pumpkin_util::{Difficulty, math::position::BlockPos};
use std::sync::Arc;

use crate::block::blocks::carved_pumpkin::{
    BlockPattern, Cell, clear_pattern_blocks, update_pattern_blocks,
};
use crate::block::blocks::skull_block::{skull_placement, update_skull_power};
use crate::block::entities::skull::SkullBlockEntity;
use crate::{
    block::{
        BlockBehaviour, BlockMetadata, OnNeighborUpdateArgs, OnPlaceArgs, PathComputationType,
        PlacedArgs, PlayerPlacedArgs,
    },
    entity::{Entity, boss::wither::WitherEntity},
    world::World,
};

/// Vanilla `WitherSkullBlock.getOrCreateWitherFull`: `"^^^", "###", "~#~"`.
const WITHER_FULL: BlockPattern = BlockPattern(&[
    &[Cell::WitherSkull, Cell::WitherSkull, Cell::WitherSkull],
    &[Cell::WitherBase, Cell::WitherBase, Cell::WitherBase],
    &[Cell::Air, Cell::WitherBase, Cell::Air],
]);
/// Vanilla `WitherSkullBlock.getOrCreateWitherBase`: `"   ", "###", "~#~"`.
const WITHER_BASE: BlockPattern = BlockPattern(&[
    &[Cell::Any, Cell::Any, Cell::Any],
    &[Cell::WitherBase, Cell::WitherBase, Cell::WitherBase],
    &[Cell::Air, Cell::WitherBase, Cell::Air],
]);

/// Vanilla `WitherSkullBlock.canSpawnMob`, for a dispensed wither skeleton skull.
#[must_use]
pub fn can_spawn_mob(world: &World, pos: &BlockPos) -> bool {
    pos.0.y >= world.min_y + 2
        && world.level_info.load().difficulty != Difficulty::Peaceful
        && WITHER_BASE.find(world, pos).is_some()
}

/// Vanilla `WitherSkullBlock.checkSpawn`: a skull completing the pattern clears it and
/// summons an invulnerable wither at its bottom middle, facing across the pattern.
// TODO: `CriteriaTriggers.SUMMONED_ENTITY` for players within 50 blocks.
pub fn check_spawn(world: &Arc<World>, pos: &BlockPos) {
    let block = world.get_block(pos);
    let correct_block = block.id == BlockId::WITHER_SKELETON_SKULL
        || block.id == BlockId::WITHER_SKELETON_WALL_SKULL;
    if !correct_block
        || pos.0.y < world.min_y
        || world.level_info.load().difficulty == Difficulty::Peaceful
    {
        return;
    }
    let Some(found) = WITHER_FULL.find(world, pos) else {
        return;
    };
    clear_pattern_blocks(world, &found);
    let (spawn_pos, _) = found.get_block(1, 2);
    let yaw = if matches!(found.forwards, BlockDirection::East | BlockDirection::West) {
        0.0
    } else {
        90.0
    };
    let entity = Entity::new(
        world.clone(),
        pumpkin_util::math::vector3::Vector3::new(
            f64::from(spawn_pos.0.x) + 0.5,
            f64::from(spawn_pos.0.y) + 0.55,
            f64::from(spawn_pos.0.z) + 0.5,
        ),
        &EntityType::WITHER,
    );
    entity.set_rotation(yaw, 0.0);
    entity.body_yaw.store(yaw);
    let wither = WitherEntity::new(entity);
    wither.make_invulnerable();
    world.spawn_entity(wither);
    update_pattern_blocks(world, &found);
}

/// Vanilla `WitherSkullBlock` and `WitherWallSkullBlock`.
pub struct WitherSkeletonSkullBlock;

impl BlockMetadata for WitherSkeletonSkullBlock {
    fn ids() -> Box<[BlockId]> {
        [
            BlockId::WITHER_SKELETON_SKULL,
            BlockId::WITHER_SKELETON_WALL_SKULL,
        ]
        .into()
    }
}

impl BlockBehaviour for WitherSkeletonSkullBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        skull_placement(&args)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.is_new_block() {
            return;
        }
        args.world
            .add_block_entity(Arc::new(SkullBlockEntity::new(*args.position)));
    }

    /// Vanilla `WitherSkullBlock.setPlacedBy` (and the wall skull's): `checkSpawn`.
    fn player_placed(&self, args: PlayerPlacedArgs<'_>) {
        check_spawn(args.world, args.position);
    }

    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        update_skull_power(&args);
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
