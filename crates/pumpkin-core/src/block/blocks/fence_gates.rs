use std::sync::Arc;

use crate::block::blocks::redstone::block_receives_redstone_power;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, GetStateForNeighborUpdateArgs, NormalUseArgs, OnNeighborUpdateArgs,
    OnPlaceArgs, PathComputationType,
};
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::world::World;
use pumpkin_data::block_properties::HorizontalFacing;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

type FenceGateProperties = pumpkin_data::block_properties::OakFenceGateLikeProperties;

fn get_sound(block: &Block, open: bool) -> Sound {
    match (block, open) {
        (b, true) if b == &Block::BAMBOO_FENCE_GATE => Sound::BlockBambooWoodFenceGateOpen,
        (b, false) if b == &Block::BAMBOO_FENCE_GATE => Sound::BlockBambooWoodFenceGateClose,
        (b, true) if b == &Block::CHERRY_FENCE_GATE => Sound::BlockCherryWoodFenceGateOpen,
        (b, false) if b == &Block::CHERRY_FENCE_GATE => Sound::BlockCherryWoodFenceGateClose,
        (b, true) if b == &Block::CRIMSON_FENCE_GATE || b == &Block::WARPED_FENCE_GATE => {
            Sound::BlockNetherWoodFenceGateOpen
        }
        (b, false) if b == &Block::CRIMSON_FENCE_GATE || b == &Block::WARPED_FENCE_GATE => {
            Sound::BlockNetherWoodFenceGateClose
        }
        (_, true) => Sound::BlockFenceGateOpen,
        (_, false) => Sound::BlockFenceGateClose,
    }
}

pub fn toggle_fence_gate(
    world: &Arc<World>,
    block_pos: &BlockPos,
    player: &Player,
) -> BlockStateId {
    let (block, state) = world.get_block_and_state_id(block_pos);

    let mut fence_gate_props = FenceGateProperties::from_state_id(state);
    if fence_gate_props.open {
        fence_gate_props.open = false;
    } else {
        if fence_gate_props.facing
            == player
                .living_entity
                .entity
                .get_horizontal_facing()
                .opposite()
        {
            fence_gate_props.facing = player.get_entity().get_horizontal_facing();
        }
        fence_gate_props.open = true;
    }

    world.set_block_state(
        block_pos,
        fence_gate_props.to_state_id(block),
        BlockFlags::NOTIFY_LISTENERS,
    );
    world.play_block_sound_expect(
        player,
        get_sound(block, fence_gate_props.open),
        SoundCategory::Blocks,
        *block_pos,
    );
    world.game_event_from(
        Some(player),
        open_close_event(fence_gate_props.open),
        block_pos.to_centered_f64(),
    );
    fence_gate_props.to_state_id(block)
}

const fn open_close_event(open: bool) -> GameEvent {
    if open {
        GameEvent::BlockOpen
    } else {
        GameEvent::BlockClose
    }
}

#[pumpkin_block_from_tag("minecraft:fence_gates")]
pub struct FenceGateBlock;

impl BlockBehaviour for FenceGateBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut fence_gate_props = FenceGateProperties::default(args.block);
        fence_gate_props.facing = args.player.get_entity().get_horizontal_facing();

        let powered = block_receives_redstone_power(args.world, args.position);
        fence_gate_props.powered = powered;
        fence_gate_props.open = powered;
        fence_gate_props.in_wall = is_in_wall(args.world, args.position, fence_gate_props.facing);

        fence_gate_props.to_state_id(args.block)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let mut fence_props = FenceGateProperties::from_state_id(args.state_id);
        let side_offset_left = args
            .position
            .offset(fence_props.facing.rotate_clockwise().to_offset());
        let side_offset_right = args
            .position
            .offset(fence_props.facing.rotate_counter_clockwise().to_offset());
        if args.neighbor_position == &side_offset_left
            || args.neighbor_position == &side_offset_right
        {
            fence_props.in_wall = is_in_wall(args.world, args.position, fence_props.facing);
        }
        fence_props.to_state_id(args.block)
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        {
            toggle_fence_gate(args.world, args.position, args.player);

            BlockActionResult::Success
        }
    }

    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        {
            let block_state = args.world.get_block_state(args.position);
            let mut fence_gate_props = FenceGateProperties::from_state_id(block_state.id);
            let powered = block_receives_redstone_power(args.world, args.position);

            if powered == fence_gate_props.powered {
                return;
            }

            // Vanilla `FenceGateBlock.neighborChanged`: write first, then the sound and
            // `BLOCK_OPEN`/`BLOCK_CLOSE` (no source entity) when `OPEN` changes.
            let was_open = fence_gate_props.open;
            fence_gate_props.powered = powered;
            fence_gate_props.open = powered;
            args.world.set_block_state(
                args.position,
                fence_gate_props.to_state_id(args.block),
                BlockFlags::NOTIFY_LISTENERS,
            );

            if was_open != powered {
                args.world.play_block_sound(
                    get_sound(args.block, powered),
                    SoundCategory::Blocks,
                    *args.position,
                );
                args.world.game_event_from(
                    None,
                    open_close_event(powered),
                    args.position.to_centered_f64(),
                );
            }
        }
    }

    fn is_pathfindable(&self, state: &BlockState, computation_type: PathComputationType) -> bool {
        match computation_type {
            PathComputationType::Land | PathComputationType::Air => {
                FenceGateProperties::from_state_id(state.id).open
            }
            PathComputationType::Water => false,
        }
    }
}

fn is_in_wall(world: &World, pos: &BlockPos, facing: HorizontalFacing) -> bool {
    let side_offset_left = pos.offset(facing.rotate_clockwise().to_offset());
    let side_offset_right = pos.offset(facing.rotate_counter_clockwise().to_offset());

    world
        .get_block(&side_offset_left)
        .has_tag(&tag::Block::MINECRAFT_WALLS)
        || world
            .get_block(&side_offset_right)
            .has_tag(&tag::Block::MINECRAFT_WALLS)
}
