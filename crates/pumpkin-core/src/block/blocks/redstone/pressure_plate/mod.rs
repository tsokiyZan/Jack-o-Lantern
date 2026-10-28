use std::sync::Arc;

use pumpkin_data::game_event::GameEvent;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos};
use pumpkin_world::{
    tick::TickPriority,
    world::{BlockAccessor, BlockFlags},
};

use crate::{
    block::{GetStateForNeighborUpdateArgs, OnEntityCollisionArgs, OnStateReplacedArgs},
    entity::EntityBase,
    world::World,
};

pub mod plate;
pub mod weighted;

#[cfg(test)]
mod tests;

// Vanilla pressure plates detect entities in a centered 14x4x14-pixel volume.
const PRESSURE_PLATE_DETECTION_BOX: BoundingBox = BoundingBox::new_array(
    [1.0 / 16.0, 0.0, 1.0 / 16.0],
    [15.0 / 16.0, 4.0 / 16.0, 15.0 / 16.0],
);

fn detection_box_at(pos: &BlockPos) -> BoundingBox {
    PRESSURE_PLATE_DETECTION_BOX.at_pos(*pos)
}

/// Vanilla `BasePressurePlateBlock.getEntityCount`: entities in the box that are not
/// spectators and don't ignore block triggers, players included. `living_only` is the
/// `MOBS` sensitivity.
fn entity_count(world: &World, pos: &BlockPos, living_only: bool) -> usize {
    let aabb = detection_box_at(pos);
    let entities = world
        .get_entities_at_box(&aabb)
        .into_iter()
        .filter(|entity| {
            !entity.is_spectator()
                && !entity.is_ignoring_block_triggers()
                && (!living_only || entity.get_living_entity().is_some())
        })
        .count();
    let players = world
        .get_players_at_box(&aabb)
        .into_iter()
        .filter(|player| !player.is_spectator())
        .count();
    entities + players
}

pub(crate) trait PressurePlate {
    /// Vanilla `BasePressurePlateBlock.entityInside`.
    fn on_entity_collision_pp(&self, args: OnEntityCollisionArgs<'_>) {
        let output = self.get_redstone_output(args.block, args.state.id);
        if output == 0 {
            self.update_plate_state(
                Some(args.entity),
                args.world,
                args.position,
                args.block,
                args.state,
                output,
            );
        }
    }

    /// Vanilla `BasePressurePlateBlock.affectNeighborsAfterRemoval`.
    fn on_state_replaced_pp(&self, args: OnStateReplacedArgs<'_>) {
        if !args.moved && self.get_redstone_output(args.block, args.old_state_id) > 0 {
            args.world
                .update_neighbors_at(args.position, args.block, None);
            args.world
                .update_neighbors_at(&args.position.down(), args.block, None);
        }
    }

    /// Vanilla `BasePressurePlateBlock.updateShape`: the plate goes when its support below
    /// does.
    fn get_state_for_neighbor_update_pp(
        &self,
        args: &GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if args.direction == BlockDirection::Down
            && !Self::can_pressure_plate_place_at(args.world, args.position)
        {
            return Block::AIR.default_state.id;
        }
        args.state_id
    }

    /// Vanilla `BasePressurePlateBlock.checkPressed`. `source` is the entity that stepped on
    /// the plate, `None` from the scheduled tick.
    fn update_plate_state(
        &self,
        source: Option<&dyn EntityBase>,
        world: &Arc<World>,
        pos: &BlockPos,
        block: &Block,
        state: &BlockState,
        output: u8,
    ) {
        let calc_output = self.calculate_redstone_output(world, block, pos);
        let has_output = calc_output > 0;
        if calc_output != output {
            let next_output = if let Some(server) = world.server.upgrade() {
                let mut event = crate::plugin::block::block_redstone::BlockRedstoneEvent::new(
                    world.clone(),
                    state.id,
                    *pos,
                    i32::from(output),
                    i32::from(calc_output),
                );
                server.plugin_manager.fire_blocking(&server, &mut event);
                if event.cancelled {
                    return;
                }
                event.new_current.clamp(0, 15) as u8
            } else {
                calc_output
            };
            let state = self.set_redstone_output(block, state, next_output);
            world.set_block_state(pos, state, BlockFlags::NOTIFY_LISTENERS);
            world.update_neighbors_at(pos, block, None);
            world.update_neighbors_at(&pos.down(), block, None);
        }

        let was_pressed = output > 0;
        let (click_on, click_off) = self.click_sounds(block);
        if !has_output && was_pressed {
            world.play_block_sound(click_off, SoundCategory::Blocks, *pos);
            world.game_event_from(source, GameEvent::BlockDeactivate, pos.to_centered_f64());
        } else if has_output && !was_pressed {
            world.play_block_sound(click_on, SoundCategory::Blocks, *pos);
            world.game_event_from(source, GameEvent::BlockActivate, pos.to_centered_f64());
        }

        if has_output {
            world.schedule_block_tick(block, *pos, self.tick_rate(), TickPriority::Normal);
        }
    }

    /// Vanilla `BasePressurePlateBlock.canSurvive`: `canSupportRigidBlock` or
    /// `canSupportCenter` below. The fork has no rigid support shape, so the first is the
    /// full-face check.
    fn can_pressure_plate_place_at(world: &dyn BlockAccessor, block_pos: &BlockPos) -> bool {
        let below = block_pos.down();
        world
            .get_block_state(&below)
            .is_side_solid(BlockDirection::Up)
            || crate::block::can_support_center(world, &below, BlockDirection::Up)
    }

    fn get_redstone_output(&self, block: &Block, state: BlockStateId) -> u8;

    fn set_redstone_output(&self, block: &Block, state: &BlockState, output: u8) -> BlockStateId;

    fn calculate_redstone_output(&self, world: &World, block: &Block, pos: &BlockPos) -> u8;

    /// Vanilla `BlockSetType.pressurePlateClickOn` / `pressurePlateClickOff`.
    fn click_sounds(&self, block: &Block) -> (Sound, Sound);

    fn tick_rate(&self) -> i32 {
        20
    }
}
