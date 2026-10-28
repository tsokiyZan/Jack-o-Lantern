use pumpkin_data::block_properties::{AcaciaShelfLikeProperties, SideChainPart};
use pumpkin_data::data_component_impl::{EquipmentSlot, UseEffectsImpl};
use pumpkin_data::fluid::Fluid;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId, HorizontalFacingExt};
use pumpkin_inventory::Inventory;
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

use crate::block::blocks::redstone::block_receives_redstone_power;
use crate::block::entities::BlockEntity;
use crate::block::entities::shelf::ShelfBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, BlockHitResult, GetComparatorOutputArgs, GetStateForNeighborUpdateArgs,
    OnNeighborUpdateArgs, OnPlaceArgs, OnStateReplacedArgs, PathComputationType, PlacedArgs,
    UseWithItemArgs,
};
use crate::entity::EntityBase;
use crate::world::World;
use crate::world::game_event::GameEventContext;
use std::sync::Arc;

#[pumpkin_block_from_tag("minecraft:wooden_shelves")]
pub struct ShelfBlock;

/// Vanilla `ShelfBlock.getMaxChainLength`.
const MAX_CHAIN_LENGTH: i32 = 3;
/// Vanilla `ShelfBlock.getColumns` (one row).
const COLUMNS: i32 = 3;

impl BlockBehaviour for ShelfBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut properties = AcaciaShelfLikeProperties::default(args.block);

        properties.facing = args.player.get_entity().get_horizontal_facing().opposite();
        properties.powered = block_receives_redstone_power(args.world, args.position);
        properties.waterlogged = args.replacing.water_source();

        properties.to_state_id(args.block)
    }

    /// Vanilla `ShelfBlock.updateShape`: the water tick.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let props = AcaciaShelfLikeProperties::from_state_id(args.state_id);
        if props.waterlogged {
            args.world.schedule_fluid_tick(
                &Fluid::WATER,
                *args.position,
                Fluid::WATER.flow_speed as i32,
                TickPriority::Normal,
            );
        }
        args.state_id
    }

    /// Vanilla `ShelfBlock.neighborChanged`: power follows the neighbours' signal, and losing
    /// it unlinks the shelf.
    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        let mut props = AcaciaShelfLikeProperties::from_state_id(state_id);
        let signal = block_receives_redstone_power(args.world, args.position);
        if props.powered == signal {
            return;
        }
        props.powered = signal;
        if !signal {
            props.side_chain = SideChainPart::Unconnected;
        }
        let new_state = props.to_state_id(args.block);
        args.world
            .set_block_state(args.position, new_state, BlockFlags::NOTIFY_ALL);
        play_sound(
            args.world,
            args.position,
            if signal {
                Sound::BlockShelfActivate
            } else {
                Sound::BlockShelfDeactivate
            },
        );
        args.world.game_event(
            if signal {
                GameEvent::BlockActivate
            } else {
                GameEvent::BlockDeactivate
            },
            args.position.to_centered_f64(),
            GameEventContext::of_state(BlockState::from_id(new_state)),
        );
    }

    /// Vanilla `ShelfBlock.onPlace`: a powered shelf links to its neighbours, an unpowered one
    /// unlinks them.
    fn placed(&self, args: PlacedArgs<'_>) {
        if args.is_new_block() {
            let entity = ShelfBlockEntity::new(*args.position);
            args.world.add_block_entity(Arc::new(entity));
        }
        let props = AcaciaShelfLikeProperties::from_state_id(args.state_id);
        if props.powered {
            update_self_and_neighbors_on_powering_up(
                args.world,
                args.position,
                args.state_id,
                args.old_state_id,
            );
        } else {
            update_neighbors_after_powering_down(args.world, args.position, args.state_id);
        }
    }

    /// Vanilla `ShelfBlock.affectNeighborsAfterRemoval`.
    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        args.world
            .update_neighbour_for_output_signal(args.position, args.block);
        update_neighbors_after_powering_down(args.world, args.position, args.old_state_id);
    }

    /// Vanilla `ShelfBlock.useItemOn`: unpowered, the held stack swaps with the slot hit;
    /// powered, the end of the hotbar swaps with the whole linked row.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        if *args.equipment_slot == EquipmentSlot::OFF_HAND {
            return BlockActionResult::Pass;
        }
        let Some(block_entity) = args.world.get_block_entity(args.position) else {
            return BlockActionResult::Pass;
        };
        let Some(shelf) = block_entity.as_any().downcast_ref::<ShelfBlockEntity>() else {
            return BlockActionResult::Pass;
        };
        let state_id = args.world.get_block_state_id(args.position);
        let props = AcaciaShelfLikeProperties::from_state_id(state_id);
        let Some(slot) = hit_slot(args.hit, props.facing.to_block_direction()) else {
            return BlockActionResult::Pass;
        };
        let inventory = args.player.inventory();
        if !props.powered {
            // `swapSingleItem`.
            let held = std::mem::replace(args.item_stack, ItemStack::EMPTY.clone());
            let removed = shelf.swap_item_no_update(slot, held.clone());
            let new_inventory_item = if args.player.has_infinite_materials() && removed.is_empty() {
                held.clone()
            } else {
                removed.clone()
            };
            let interacts = new_inventory_item
                .get_data_component::<UseEffectsImpl>()
                .is_none_or(|effects| effects.interact_vibrations);
            *args.item_stack = new_inventory_item;
            set_changed(
                args.world,
                &block_entity,
                interacts.then_some(GameEvent::ItemInteractFinish),
            );
            if removed.is_empty() {
                if held.is_empty() {
                    return BlockActionResult::Pass;
                }
                play_sound(args.world, args.position, Sound::BlockShelfPlaceItem);
            } else {
                play_sound(
                    args.world,
                    args.position,
                    if held.is_empty() {
                        Sound::BlockShelfTakeItem
                    } else {
                        Sound::BlockShelfSingleSwap
                    },
                );
            }
            return BlockActionResult::SuccessServer;
        }
        // `swapHotbar`: the selected slot is written back from `item_stack`, so it goes
        // through there.
        let selected = usize::from(inventory.get_selected_slot());
        inventory.set_stack(selected, args.item_stack.clone());
        let any_swapped = swap_hotbar(args.world, args.position, inventory.as_ref());
        *args.item_stack = inventory.get_stack(selected);
        if !any_swapped {
            return BlockActionResult::Consume;
        }
        play_sound(args.world, args.position, Sound::BlockShelfMultiSwap);
        BlockActionResult::SuccessServer
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        // Vanilla reads a shelf only through its back face.
        let props = AcaciaShelfLikeProperties::from_state_id(args.state.id);
        if args.direction != props.facing.to_block_direction().opposite() {
            return Some(0);
        }

        if let Some(block_entity) = args.world.get_block_entity(args.position)
            && let Some(shelf) = block_entity.as_any().downcast_ref::<ShelfBlockEntity>()
        {
            let b0 = u8::from(!shelf.get_stack(0).is_empty());
            let b1 = u8::from(!shelf.get_stack(1).is_empty());
            let b2 = u8::from(!shelf.get_stack(2).is_empty());
            Some(b0 | (b1 << 1) | (b2 << 2))
        } else {
            None
        }
    }

    fn is_pathfindable(&self, state: &BlockState, computation_type: PathComputationType) -> bool {
        computation_type == PathComputationType::Water && state.is_waterlogged()
    }
}

fn play_sound(world: &World, pos: &BlockPos, sound: Sound) {
    world.play_sound(sound, SoundCategory::Blocks, &pos.to_centered_f64());
}

/// Vanilla `ShelfBlockEntity.setChanged(event)`: the game event (if any) and a block update to
/// the clients.
fn set_changed(world: &World, block_entity: &Arc<dyn BlockEntity>, event: Option<GameEvent>) {
    let pos = block_entity.get_position();
    if let Some(event) = event {
        world.game_event(
            event,
            pos.to_centered_f64(),
            GameEventContext::of_state(world.get_block_state(&pos)),
        );
    }
    world.update_block_entity(block_entity);
}

/// Vanilla `SelectableSlotContainer.getHitSlot` for one row of three.
fn hit_slot(hit: &BlockHitResult<'_>, facing: BlockDirection) -> Option<usize> {
    if *hit.face != facing {
        return None;
    }
    let x = match facing {
        BlockDirection::North => 1.0 - hit.cursor_pos.x,
        BlockDirection::South => hit.cursor_pos.x,
        BlockDirection::West => hit.cursor_pos.z,
        BlockDirection::East => 1.0 - hit.cursor_pos.z,
        BlockDirection::Up | BlockDirection::Down => return None,
    };
    // `getSection`.
    let section_size = 16.0 / COLUMNS as f32;
    let column = ((x * 16.0) / section_size).floor() as i32;
    Some(column.clamp(0, COLUMNS - 1) as usize)
}

/// Vanilla `ShelfBlock.swapHotbar`: the last hotbar slots, three per linked shelf, swap with
/// the shelves' slots in row order.
fn swap_hotbar(world: &Arc<World>, pos: &BlockPos, inventory: &dyn Inventory) -> bool {
    let connected = all_blocks_connected_to(world, pos);
    if connected.is_empty() {
        return false;
    }
    let mut any_swapped = false;
    let count = connected.len() as i32;
    for (index, part_pos) in connected.iter().enumerate() {
        let Some(block_entity) = world.get_block_entity(part_pos) else {
            continue;
        };
        let Some(shelf) = block_entity.as_any().downcast_ref::<ShelfBlockEntity>() else {
            continue;
        };
        let size = ShelfBlockEntity::INVENTORY_SIZE as i32;
        for slot in 0..size {
            let inventory_slot = 9 - (count - index as i32) * size + slot;
            if inventory_slot < 0 {
                continue;
            }
            let inventory_slot = inventory_slot as usize;
            let placed = inventory.remove_stack(inventory_slot);
            let removed = shelf.swap_item_no_update(slot as usize, placed.clone());
            if !placed.is_empty() || !removed.is_empty() {
                inventory.set_stack(inventory_slot, removed);
                any_swapped = true;
            }
        }
        set_changed(world, &block_entity, Some(GameEvent::EntityInteract));
    }
    any_swapped
}

/// Vanilla `ShelfBlock.isConnectable`: a powered wooden shelf.
fn is_connectable(state_id: BlockStateId) -> bool {
    Block::from_state_id(state_id).has_tag(&tag::Block::MINECRAFT_WOODEN_SHELVES)
        && AcaciaShelfLikeProperties::from_state_id(state_id).powered
}

fn facing_of(state_id: BlockStateId) -> BlockDirection {
    AcaciaShelfLikeProperties::from_state_id(state_id)
        .facing
        .to_block_direction()
}

/// Vanilla `SideChainPartBlock.Neighbor`: a linkable shelf facing the same way, with its part,
/// or nothing.
#[derive(Clone, Copy)]
struct Neighbor {
    pos: BlockPos,
    part: Option<SideChainPart>,
}

impl Neighbor {
    fn at(world: &World, pos: BlockPos, facing: BlockDirection) -> Self {
        let state_id = world.get_block_state_id(&pos);
        let part = (is_connectable(state_id) && facing_of(state_id) == facing)
            .then(|| AcaciaShelfLikeProperties::from_state_id(state_id).side_chain);
        Self { pos, part }
    }

    const fn is_connectable(self) -> bool {
        self.part.is_some()
    }

    /// `isUnconnectableOrChainEnd`.
    fn is_chain_end(self) -> bool {
        self.part.is_none_or(|part| part != SideChainPart::Center)
    }

    /// `connectsTowards`: `isConnectionTowards`.
    fn connects_towards(self, end: SideChainPart) -> bool {
        self.part
            .is_some_and(|part| part == SideChainPart::Center || part == end)
    }

    fn update(self, world: &Arc<World>, change: fn(SideChainPart) -> SideChainPart) {
        if let Some(part) = self.part {
            set_part(world, &self.pos, change(part));
        }
    }
}

/// `SideChainPart.whenConnectedToTheRight`.
const fn connected_right(part: SideChainPart) -> SideChainPart {
    match part {
        SideChainPart::Unconnected | SideChainPart::Left => SideChainPart::Left,
        SideChainPart::Right | SideChainPart::Center => SideChainPart::Center,
    }
}

/// `SideChainPart.whenConnectedToTheLeft`.
const fn connected_left(part: SideChainPart) -> SideChainPart {
    match part {
        SideChainPart::Unconnected | SideChainPart::Right => SideChainPart::Right,
        SideChainPart::Center | SideChainPart::Left => SideChainPart::Center,
    }
}

/// `SideChainPart.whenDisconnectedFromTheRight`.
const fn disconnected_right(part: SideChainPart) -> SideChainPart {
    match part {
        SideChainPart::Unconnected | SideChainPart::Left => SideChainPart::Unconnected,
        SideChainPart::Right | SideChainPart::Center => SideChainPart::Right,
    }
}

/// `SideChainPart.whenDisconnectedFromTheLeft`.
const fn disconnected_left(part: SideChainPart) -> SideChainPart {
    match part {
        SideChainPart::Unconnected | SideChainPart::Right => SideChainPart::Unconnected,
        SideChainPart::Center | SideChainPart::Left => SideChainPart::Left,
    }
}

/// `Neighbors.left(steps)`: clockwise of the facing.
fn left(world: &World, center: &BlockPos, facing: BlockDirection, steps: i32) -> Neighbor {
    let dir = facing.rotate_clockwise().to_offset();
    Neighbor::at(
        world,
        center.offset(dir.multiply(steps, steps, steps)),
        facing,
    )
}

/// `Neighbors.right(steps)`: counter-clockwise of the facing.
fn right(world: &World, center: &BlockPos, facing: BlockDirection, steps: i32) -> Neighbor {
    let dir = facing.rotate_counter_clockwise().to_offset();
    Neighbor::at(
        world,
        center.offset(dir.multiply(steps, steps, steps)),
        facing,
    )
}

/// Vanilla `SideChainPartBlock.setPart`: `setBlockAndUpdate` when the part changes.
fn set_part(world: &Arc<World>, pos: &BlockPos, part: SideChainPart) {
    let state_id = world.get_block_state_id(pos);
    let mut props = AcaciaShelfLikeProperties::from_state_id(state_id);
    if props.side_chain != part {
        props.side_chain = part;
        world.set_block_state(
            pos,
            props.to_state_id(Block::from_state_id(state_id)),
            BlockFlags::NOTIFY_ALL,
        );
    }
}

/// Vanilla `SideChainPartBlock.getAllBlocksConnectedTo`, left to right.
fn all_blocks_connected_to(world: &World, pos: &BlockPos) -> Vec<BlockPos> {
    let state_id = world.get_block_state_id(pos);
    if !is_connectable(state_id) {
        return Vec::new();
    }
    let facing = facing_of(state_id);
    let mut results = std::collections::VecDeque::from([*pos]);
    // `addBlocksConnectingTowards`.
    for steps in 1..MAX_CHAIN_LENGTH {
        let neighbor = left(world, pos, facing, steps);
        if neighbor.connects_towards(SideChainPart::Left) {
            results.push_front(neighbor.pos);
        }
        if neighbor.is_chain_end() {
            break;
        }
    }
    for steps in 1..MAX_CHAIN_LENGTH {
        let neighbor = right(world, pos, facing, steps);
        if neighbor.connects_towards(SideChainPart::Right) {
            results.push_back(neighbor.pos);
        }
        if neighbor.is_chain_end() {
            break;
        }
    }
    results.into()
}

/// Vanilla `SideChainPartBlock.updateNeighborsAfterPoweringDown`.
fn update_neighbors_after_powering_down(
    world: &Arc<World>,
    pos: &BlockPos,
    state_id: BlockStateId,
) {
    let facing = facing_of(state_id);
    let left_neighbor = left(world, pos, facing, 1);
    let right_neighbor = right(world, pos, facing, 1);
    left_neighbor.update(world, disconnected_right);
    right_neighbor.update(world, disconnected_left);
}

/// Vanilla `SideChainPartBlock.updateSelfAndNeighborsOnPoweringUp`.
fn update_self_and_neighbors_on_powering_up(
    world: &Arc<World>,
    pos: &BlockPos,
    state_id: BlockStateId,
    old_state_id: BlockStateId,
) {
    if !is_connectable(state_id) {
        return;
    }
    // `isBeingUpdatedByNeighbor`.
    let getting_connected =
        AcaciaShelfLikeProperties::from_state_id(state_id).side_chain != SideChainPart::Unconnected;
    let connected_before = is_connectable(old_state_id)
        && AcaciaShelfLikeProperties::from_state_id(old_state_id).side_chain
            != SideChainPart::Unconnected;
    if getting_connected || connected_before {
        return;
    }
    let facing = facing_of(state_id);
    let left_neighbor = left(world, pos, facing, 1);
    let right_neighbor = right(world, pos, facing, 1);
    let mut new_part = SideChainPart::Unconnected;
    let chain_left = if left_neighbor.is_connectable() {
        all_blocks_connected_to(world, &left_neighbor.pos).len() as i32
    } else {
        0
    };
    let chain_right = if right_neighbor.is_connectable() {
        all_blocks_connected_to(world, &right_neighbor.pos).len() as i32
    } else {
        0
    };
    let can_connect =
        |new_blocks: i32, current: i32| new_blocks > 0 && current + new_blocks <= MAX_CHAIN_LENGTH;
    let mut current = 1;
    if can_connect(chain_left, current) {
        new_part = connected_left(new_part);
        left_neighbor.update(world, connected_right);
        current += chain_left;
    }
    if can_connect(chain_right, current) {
        new_part = connected_right(new_part);
        right_neighbor.update(world, connected_left);
    }
    set_part(world, pos, new_part);
}
