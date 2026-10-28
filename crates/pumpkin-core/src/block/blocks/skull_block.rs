use crate::block::blocks::redstone::block_receives_redstone_power;
use crate::block::entities::skull::SkullBlockEntity;
use crate::block::{
    BlockBehaviour, BlockMetadata, OnNeighborUpdateArgs, OnPlaceArgs, PathComputationType,
    PlacedArgs, nearest_looking_directions, standing_and_wall_placement,
};
use crate::entity::EntityBase;
use pumpkin_data::block_properties::{
    SkeletonSkullLikeProperties, SkeletonWallSkullLikeProperties,
};
use pumpkin_data::{Block, BlockId, BlockState, BlockStateId, FacingExt};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

/// Vanilla `SkullBlock` and `WallSkullBlock` (`PlayerHeadBlock`, `PiglinheadBlock` and the
/// rest); the wither skeleton's are `WitherSkullBlock`.
pub struct SkullBlock;

impl BlockMetadata for SkullBlock {
    fn ids() -> Box<[BlockId]> {
        [
            BlockId::SKELETON_SKULL,
            BlockId::SKELETON_WALL_SKULL,
            BlockId::PLAYER_HEAD,
            BlockId::PLAYER_WALL_HEAD,
            BlockId::ZOMBIE_HEAD,
            BlockId::ZOMBIE_WALL_HEAD,
            BlockId::CREEPER_HEAD,
            BlockId::CREEPER_WALL_HEAD,
            BlockId::PIGLIN_HEAD,
            BlockId::PIGLIN_WALL_HEAD,
            BlockId::DRAGON_HEAD,
            BlockId::DRAGON_WALL_HEAD,
        ]
        .into()
    }
}

/// Whether `block` is one of the wall skulls or heads.
fn is_wall(block: &Block) -> bool {
    block.name.contains("_wall_")
}

/// The wall block a skull item hangs as (`StandingAndWallBlockItem.wallBlock`).
fn wall_block(block: &Block) -> Option<&'static Block> {
    let name = block
        .name
        .strip_suffix("_skull")
        .map(|base| format!("{base}_wall_skull"))
        .or_else(|| {
            block
                .name
                .strip_suffix("_head")
                .map(|base| format!("{base}_wall_head"))
        })?;
    Block::from_name(&name)
}

/// Vanilla `StandingAndWallBlockItem.getPlacementState` for a skull.
///
/// `SkullBlock` stands with the player's rotation; `WallSkullBlock` hangs against the first
/// horizontal looking direction whose block can't be replaced. Skulls survive anywhere.
#[must_use]
pub fn skull_placement(args: &OnPlaceArgs<'_>) -> BlockStateId {
    // `AbstractSkullBlock.getStateForPlacement`: `hasNeighborSignal`.
    let powered = block_receives_redstone_power(args.world, args.position);
    let mut standing = SkeletonSkullLikeProperties::default(args.block);
    standing.rotation = args.player.get_entity().get_rotation_16();
    standing.powered = powered;
    let standing = standing.to_state_id(args.block);
    let wall = wall_block(args.block).and_then(|wall_block| {
        nearest_looking_directions(args)
            .into_iter()
            .find_map(|direction| {
                let facing = direction.opposite().to_horizontal_facing()?;
                let behind = args
                    .position
                    .offset(direction.to_block_direction().to_offset());
                (!args.world.get_block_state(&behind).replaceable()).then_some(facing)
            })
            .map(|facing| {
                let mut props = SkeletonWallSkullLikeProperties::default(wall_block);
                props.facing = facing;
                props.powered = powered;
                props.to_state_id(wall_block)
            })
    });
    standing_and_wall_placement(args, standing, wall, |_| true)
}

/// Vanilla `AbstractSkullBlock.neighborChanged`: `powered` follows the neighbour signal.
pub fn update_skull_power(args: &OnNeighborUpdateArgs<'_>) {
    let state_id = args.world.get_block_state_id(args.position);
    let powered = block_receives_redstone_power(args.world, args.position);
    let new_state = if is_wall(args.block) {
        let mut props = SkeletonWallSkullLikeProperties::from_state_id(state_id);
        if props.powered == powered {
            return;
        }
        props.powered = powered;
        props.to_state_id(args.block)
    } else {
        let mut props = SkeletonSkullLikeProperties::from_state_id(state_id);
        if props.powered == powered {
            return;
        }
        props.powered = powered;
        props.to_state_id(args.block)
    };
    args.world
        .set_block_state(args.position, new_state, BlockFlags::NOTIFY_LISTENERS);
}

impl BlockBehaviour for SkullBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.is_new_block() {
            return;
        }
        args.world
            .add_block_entity(Arc::new(SkullBlockEntity::new(*args.position)));
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        skull_placement(&args)
    }

    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        update_skull_power(&args);
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
