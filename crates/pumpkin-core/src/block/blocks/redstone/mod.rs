/**
 * This implementation is heavily based on <https://github.com/MCHPR/MCHPRS>
 * Updated to fit pumpkin by 4lve
 */
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockDirection, BlockState};
use pumpkin_util::math::position::BlockPos;

use crate::world::World;

pub mod bell;
pub mod buttons;
pub mod comparator;
pub mod copper_bulb;
pub mod crafter;
pub mod daylight_detector;
pub mod dropper;
pub mod lever;
pub mod lightning_rod;
pub mod observer;
pub mod pressure_plate;
pub mod rails;
pub mod redstone_block;
pub mod redstone_lamp;
pub mod redstone_ore;
pub mod redstone_torch;
pub mod redstone_wire;
pub mod repeater;
pub mod sculk_sensor;
pub mod target_block;
pub mod tripwire;
pub mod tripwire_hook;

// abstract
pub mod abstract_redstone_gate;
pub mod dispenser;

pub fn is_emitting_redstone_power(
    block: &Block,
    state: &BlockState,
    world: &World,
    pos: &BlockPos,
    facing: BlockDirection,
) -> bool {
    get_redstone_power(block, state, world, pos, facing) > 0
}

pub fn get_redstone_power(
    block: &Block,
    state: &BlockState,
    world: &World,
    pos: &BlockPos,
    facing: BlockDirection,
) -> u8 {
    if state.is_solid_block() {
        return std::cmp::max(
            get_max_strong_power(world, pos, true),
            get_weak_power(block, state, world, pos, facing, true),
        );
    }
    get_weak_power(block, state, world, pos, facing, true)
}

pub fn get_redstone_power_no_dust(
    block: &Block,
    state: &BlockState,
    world: &World,
    pos: BlockPos,
    facing: BlockDirection,
) -> u8 {
    if state.is_solid_block() {
        return std::cmp::max(
            get_max_strong_power(world, &pos, false),
            get_weak_power(block, state, world, &pos, facing, false),
        );
    }
    get_weak_power(block, state, world, &pos, facing, false)
}

pub fn get_max_strong_power(world: &World, pos: &BlockPos, dust_power: bool) -> u8 {
    let mut max_power = 0;
    for side in BlockDirection::all() {
        let (block, state) = world.get_block_and_state(&pos.offset(side.to_offset()));
        max_power = max_power.max(get_strong_power(
            block,
            state,
            world,
            &pos.offset(side.to_offset()),
            side,
            dust_power,
        ));
    }
    max_power
}

pub fn get_max_weak_power(world: &World, pos: &BlockPos, dust_power: bool) -> u8 {
    let mut max_power = 0;
    for side in BlockDirection::all() {
        let (block, state) = world.get_block_and_state(&pos.offset(side.to_offset()));
        max_power = max_power.max(get_weak_power(
            block,
            state,
            world,
            &pos.offset(side.to_offset()),
            side,
            dust_power,
        ));
    }
    max_power
}

fn get_weak_power(
    block: &Block,
    state: &BlockState,
    world: &World,
    pos: &BlockPos,
    side: BlockDirection,
    dust_power: bool,
) -> u8 {
    if !dust_power && block == &Block::REDSTONE_WIRE {
        return 0;
    }
    world
        .block_registry
        .get_weak_redstone_power(block, world, pos, state, side)
}

fn get_strong_power(
    block: &Block,
    state: &BlockState,
    world: &World,
    pos: &BlockPos,
    side: BlockDirection,
    dust_power: bool,
) -> u8 {
    if !dust_power && block == &Block::REDSTONE_WIRE {
        return 0;
    }
    world
        .block_registry
        .get_strong_redstone_power(block, world, pos, state, side)
}

/// Vanilla `SignalGetter.getBestOwnOrNeighbourSignal`.
pub fn get_best_own_or_neighbour_signal(world: &World, pos: &BlockPos) -> u8 {
    let mut best = 0;
    for facing in BlockDirection::all() {
        let neighbor_pos = pos.offset(facing.to_offset());
        let (block, state) = world.get_block_and_state(&neighbor_pos);
        best = best.max(get_redstone_power(
            block,
            state,
            world,
            &neighbor_pos,
            facing,
        ));
        if best >= 15 {
            return 15;
        }
    }
    best.max(get_own_signal(world, pos))
}

/// Vanilla `BlockState.getOwnSignal`. The blocks that override `ownSignal` give the same
/// signal as their strongest side, and every other block gives none.
fn get_own_signal(world: &World, pos: &BlockPos) -> u8 {
    let (block, state) = world.get_block_and_state(pos);
    let has_own_signal = block == &Block::LEVER
        || block == &Block::REDSTONE_TORCH
        || block == &Block::REDSTONE_WALL_TORCH
        || block == &Block::TARGET
        || block == &Block::JUKEBOX
        || block == &Block::SCULK_SENSOR
        || block == &Block::CALIBRATED_SCULK_SENSOR
        || block == &Block::DETECTOR_RAIL
        || block.has_tag(&tag::Block::MINECRAFT_BUTTONS)
        || block.has_tag(&tag::Block::MINECRAFT_PRESSURE_PLATES)
        || block.has_tag(&tag::Block::MINECRAFT_LIGHTNING_RODS);
    if !has_own_signal {
        return 0;
    }
    BlockDirection::all()
        .into_iter()
        .map(|side| get_weak_power(block, state, world, pos, side, true))
        .max()
        .unwrap_or(0)
}

pub fn block_receives_redstone_power(world: &World, pos: &BlockPos) -> bool {
    for facing in BlockDirection::all() {
        let neighbor_pos = pos.offset(facing.to_offset());
        let (block, state) = world.get_block_and_state(&neighbor_pos);
        if is_emitting_redstone_power(block, state, world, &neighbor_pos, facing) {
            return true;
        }
    }
    false
}

#[must_use]
pub fn is_diode(block: &Block) -> bool {
    block == &Block::REPEATER || block == &Block::COMPARATOR
}
