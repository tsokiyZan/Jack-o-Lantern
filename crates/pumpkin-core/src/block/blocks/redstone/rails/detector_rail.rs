use std::sync::Arc;

use pumpkin_data::block_properties::DetectorRailProperties;
use pumpkin_data::entity::EntityType;
use pumpkin_data::{Block, BlockDirection, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

use crate::block::BlockBehaviour;
use crate::block::CanPlaceAtArgs;
use crate::block::EmitsRedstonePowerArgs;
use crate::block::GetComparatorOutputArgs;
use crate::block::GetRedstonePowerArgs;
use crate::block::GetStateForNeighborUpdateArgs;
use crate::block::OnEntityCollisionArgs;
use crate::block::OnNeighborUpdateArgs;
use crate::block::OnPlaceArgs;
use crate::block::OnScheduledTickArgs;
use crate::block::OnStateReplacedArgs;
use crate::block::PlacedArgs;
use crate::entity::vehicle::minecart::MinecartEntity;
use crate::world::World;

use super::RailState;
use super::common::{
    affect_neighbors_after_removal, can_place_rail_at, on_place, rail_state_for_placement,
    schedule_water_tick, survives_neighbor_change,
};

/// Vanilla `DetectorRailBlock.PRESSED_CHECK_PERIOD`.
const PRESSED_CHECK_PERIOD: i32 = 20;

#[pumpkin_block("minecraft:detector_rail")]
pub struct DetectorRailBlock;

fn is_minecart(entity_type: &EntityType) -> bool {
    entity_type == &EntityType::MINECART
        || entity_type == &EntityType::CHEST_MINECART
        || entity_type == &EntityType::COMMAND_BLOCK_MINECART
        || entity_type == &EntityType::FURNACE_MINECART
        || entity_type == &EntityType::HOPPER_MINECART
        || entity_type == &EntityType::SPAWNER_MINECART
        || entity_type == &EntityType::TNT_MINECART
}

/// Vanilla `DetectorRailBlock.updatePowerToConnected`: a neighbour update at each position
/// the rail's shape connects to.
fn update_power_to_connected(world: &Arc<World>, pos: &BlockPos, state_id: BlockStateId) {
    let rail = RailState::new(*pos, state_id);
    for connection in rail.connections() {
        world.update_neighbor(connection, world.get_block(connection));
    }
}

impl DetectorRailBlock {
    /// Vanilla `DetectorRailBlock.checkPressed`.
    pub fn check_pressed(
        &self,
        world: &Arc<World>,
        pos: &BlockPos,
        block: &Block,
        state_id: BlockStateId,
    ) {
        if !can_place_rail_at(world.as_ref(), pos) || world.get_block(pos) != block {
            return;
        }

        let was_pressed = DetectorRailProperties::from_state_id(state_id).powered;
        let should_be_pressed = world
            .get_entities_at_box(&search_box(pos))
            .iter()
            .any(|e| is_minecart(e.get_entity().entity_type));

        if should_be_pressed != was_pressed {
            let mut props = DetectorRailProperties::from_state_id(state_id);
            props.powered = should_be_pressed;
            let new_state_id = props.to_state_id(block);
            world.set_block_state(pos, new_state_id, BlockFlags::NOTIFY_ALL);
            update_power_to_connected(world, pos, new_state_id);
            world.update_neighbors_at(pos, block, None);
            world.update_neighbors_at(&pos.down(), block, None);
        }

        if should_be_pressed {
            world.schedule_block_tick(block, *pos, PRESSED_CHECK_PERIOD, TickPriority::Normal);
        }

        world.update_neighbour_for_output_signal(pos, block);
    }
}

/// Vanilla `DetectorRailBlock.getSearchBB`.
fn search_box(pos: &BlockPos) -> BoundingBox {
    BoundingBox::new_array([0.2, 0.0, 0.2], [0.8, 0.8, 0.8]).at_pos(*pos)
}

impl BlockBehaviour for DetectorRailBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        rail_state_for_placement(&args)
    }

    /// Vanilla `DetectorRailBlock.onPlace`.
    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.is_new_block() {
            return;
        }
        let state_id = on_place(args.world, args.position, args.block, args.state_id);
        self.check_pressed(args.world, args.position, args.block, state_id);
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        schedule_water_tick(args.world, args.position, args.block, args.state_id);
        args.state_id
    }

    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        survives_neighbor_change(args.world, args.position, args.block);
    }

    fn on_entity_collision(&self, args: OnEntityCollisionArgs<'_>) {
        let props = DetectorRailProperties::from_state_id(args.state.id);
        if !props.powered {
            self.check_pressed(args.world, args.position, args.block, args.state.id);
        }
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        let props = DetectorRailProperties::from_state_id(state_id);
        if props.powered {
            self.check_pressed(args.world, args.position, args.block, state_id);
        }
    }

    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        affect_neighbors_after_removal(
            args.world,
            args.position,
            args.block,
            args.old_state_id,
            args.moved,
        );
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        can_place_rail_at(args.block_accessor, args.position)
    }

    fn emits_redstone_power(&self, _args: EmitsRedstonePowerArgs<'_>) -> bool {
        true
    }

    fn get_weak_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        let props = DetectorRailProperties::from_state_id(args.state.id);
        if props.powered { 15 } else { 0 }
    }

    fn get_strong_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        let props = DetectorRailProperties::from_state_id(args.state.id);
        if props.powered && args.direction == BlockDirection::Up {
            15
        } else {
            0
        }
    }

    /// Vanilla `DetectorRailBlock.getAnalogOutputSignal`.
    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        let props = DetectorRailProperties::from_state_id(args.state.id);
        if !props.powered {
            return Some(0);
        }
        let entities = args.world.get_entities_at_box(&search_box(args.position));
        // A command block minecart reports its success count, which stays 0 because the
        // fork's command block minecart never runs its command.
        if entities
            .iter()
            .any(|e| e.get_entity().entity_type == &EntityType::COMMAND_BLOCK_MINECART)
        {
            return Some(0);
        }
        let output = entities.iter().find_map(|entity| {
            let minecart = entity.cast_any().downcast_ref::<MinecartEntity>()?;
            minecart.comparator_output()
        });
        Some(output.unwrap_or(0))
    }
}
