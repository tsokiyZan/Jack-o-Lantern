use crate::block::blocks::redstone::block_receives_redstone_power;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, GetStateForNeighborUpdateArgs, NormalUseArgs, OnNeighborUpdateArgs,
    OnPlaceArgs, PathComputationType,
};
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::world::World;
use pumpkin_data::BlockDirection;
use pumpkin_data::block_properties::Half;
use pumpkin_data::fluid::Fluid;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockState, BlockStateId, tag};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

type TrapDoorProperties = pumpkin_data::block_properties::OakTrapdoorLikeProperties;

/// Vanilla `TrapDoorBlock.toggle`: write with flags 2, the waterlogged fluid tick, then
/// `playSound` (sound and `BLOCK_OPEN`/`BLOCK_CLOSE` from the player).
fn toggle_trapdoor(player: &Player, world: &Arc<World>, block_pos: &BlockPos) {
    let (block, block_state) = world.get_block_and_state_id(block_pos);
    let mut trapdoor_props = TrapDoorProperties::from_state_id(block_state);
    trapdoor_props.open = !trapdoor_props.open;

    world.set_block_state(
        block_pos,
        trapdoor_props.to_state_id(block),
        BlockFlags::NOTIFY_LISTENERS,
    );
    if trapdoor_props.waterlogged {
        schedule_water_tick(world, block_pos);
    }

    world.play_block_sound_expect(
        player,
        get_sound(block, trapdoor_props.open),
        SoundCategory::Blocks,
        *block_pos,
    );
    world.game_event_from(
        Some(player),
        open_close_event(trapdoor_props.open),
        block_pos.to_centered_f64(),
    );
}

const fn open_close_event(open: bool) -> GameEvent {
    if open {
        GameEvent::BlockOpen
    } else {
        GameEvent::BlockClose
    }
}

fn schedule_water_tick(world: &World, pos: &BlockPos) {
    world.schedule_fluid_tick(
        &Fluid::FLOWING_WATER,
        *pos,
        Fluid::FLOWING_WATER.flow_speed as i32,
        TickPriority::Normal,
    );
}

fn can_open_trapdoor(block: &Block) -> bool {
    if block == &Block::IRON_TRAPDOOR {
        return false;
    }
    true
}

fn get_sound(block: &Block, open: bool) -> Sound {
    if open {
        if block.has_tag(&tag::Block::MINECRAFT_WOODEN_TRAPDOORS) {
            Sound::BlockWoodenTrapdoorOpen
        } else if block == &Block::IRON_TRAPDOOR {
            Sound::BlockIronTrapdoorOpen
        } else {
            Sound::BlockCopperTrapdoorOpen
        }
    } else if block.has_tag(&tag::Block::MINECRAFT_WOODEN_TRAPDOORS) {
        Sound::BlockWoodenTrapdoorClose
    } else if block == &Block::IRON_TRAPDOOR {
        Sound::BlockIronTrapdoorClose
    } else {
        Sound::BlockCopperTrapdoorClose
    }
}

#[pumpkin_block_from_tag("minecraft:trapdoors")]
pub struct TrapDoorBlock;

impl BlockBehaviour for TrapDoorBlock {
    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        {
            if !can_open_trapdoor(args.block) {
                return BlockActionResult::Pass;
            }

            toggle_trapdoor(args.player, args.world, args.position);

            BlockActionResult::Success
        }
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut trapdoor_props = TrapDoorProperties::default(args.block);
        trapdoor_props.waterlogged = args.replacing.water_source();

        let powered = block_receives_redstone_power(args.world, args.position);

        let player_facing = args.player.get_entity().get_horizontal_facing();

        // Correct facing logic using Option unwrap
        let facing = args
            .direction
            .to_horizontal_facing()
            .unwrap_or(player_facing);

        trapdoor_props.facing = facing;

        trapdoor_props.half = match args.direction {
            BlockDirection::Up => Half::Top,
            BlockDirection::Down => Half::Bottom,
            _ => {
                if args.use_item_on.cursor_pos.y > 0.5 {
                    Half::Top
                } else {
                    Half::Bottom
                }
            }
        };

        trapdoor_props.powered = powered;
        trapdoor_props.open = powered;

        trapdoor_props.to_state_id(args.block)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if TrapDoorProperties::from_state_id(args.state_id).waterlogged {
            schedule_water_tick(args.world, args.position);
        }
        args.state_id
    }

    /// Vanilla `TrapDoorBlock.neighborChanged`: only a change of the neighbour signal against
    /// `POWERED` does anything; `OPEN` follows it with `playSound` (no source entity).
    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        let block_state = args.world.get_block_state(args.position);
        let mut trapdoor_props = TrapDoorProperties::from_state_id(block_state.id);
        let powered = block_receives_redstone_power(args.world, args.position);

        if powered != trapdoor_props.powered {
            if powered != trapdoor_props.open {
                trapdoor_props.open = powered;

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

            trapdoor_props.powered = powered;

            args.world.set_block_state(
                args.position,
                trapdoor_props.to_state_id(args.block),
                BlockFlags::NOTIFY_LISTENERS,
            );
            if trapdoor_props.waterlogged {
                schedule_water_tick(args.world, args.position);
            }
        }
    }

    fn is_pathfindable(&self, state: &BlockState, computation_type: PathComputationType) -> bool {
        let props = TrapDoorProperties::from_state_id(state.id);
        match computation_type {
            PathComputationType::Land | PathComputationType::Air => props.open,
            PathComputationType::Water => props.waterlogged,
        }
    }
}
