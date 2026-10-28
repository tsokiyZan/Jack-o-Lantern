use std::sync::Arc;

use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::{Block, BlockDirection, BlockStateId, block_properties::HorizontalFacing};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use pumpkin_world::{tick::TickPriority, world::BlockFlags};

use crate::{
    block::{
        BlockBehaviour, GetInsideCollisionShapeArgs, GetStateForNeighborUpdateArgs,
        OnEntityCollisionArgs, OnPlaceArgs, OnScheduledTickArgs, OnStateReplacedArgs, PlacedArgs,
        PlayerWillDestroyArgs,
    },
    world::World,
};

use super::tripwire_hook::TripwireHookBlock;

type TripwireProperties = pumpkin_data::block_properties::TripwireLikeProperties;
type TripwireHookProperties = pumpkin_data::block_properties::TripwireHookLikeProperties;

#[pumpkin_block("minecraft:tripwire")]
pub struct TripwireBlock;

/// Vanilla `TripWireBlock.SHAPE_ATTACHED` / `SHAPE_NOT_ATTACHED`, block-local.
fn shape(state_id: BlockStateId) -> BoundingBox {
    if TripwireProperties::from_state_id(state_id).attached {
        BoundingBox::new(
            Vector3::new(0.0, 0.0625, 0.0),
            Vector3::new(1.0, 0.15625, 1.0),
        )
    } else {
        BoundingBox::new(Vector3::new(0.0, 0.0, 0.0), Vector3::new(1.0, 0.5, 1.0))
    }
}

/// Vanilla `TripWireBlock.RECHECK_PERIOD`.
const RECHECK_PERIOD: i32 = 10;

impl BlockBehaviour for TripwireBlock {
    /// Vanilla `TripWireBlock.getEntityInsideCollisionShape`: the wire's own shape.
    fn get_inside_collision_shape(&self, args: GetInsideCollisionShapeArgs<'_>) -> BoundingBox {
        shape(args.state.id)
    }

    /// Vanilla `TripWireBlock.entityInside`.
    fn on_entity_collision(&self, args: OnEntityCollisionArgs<'_>) {
        if !TripwireProperties::from_state_id(args.state.id).powered
            && !args
                .world
                .is_block_tick_scheduled(args.position, args.block)
        {
            Self::check_pressed(
                args.world,
                args.position,
                args.block,
                !args.entity.is_ignoring_block_triggers(),
            );
        }
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let [connect_north, connect_east, connect_south, connect_west] = [
            BlockDirection::North,
            BlockDirection::East,
            BlockDirection::South,
            BlockDirection::West,
        ]
        .map(|dir| {
            let current_pos = args.position.offset(dir.to_offset());
            let state_id = args.world.get_block_state_id(&current_pos);
            Self::should_connect_to(state_id, dir)
        });

        let mut props = TripwireProperties::from_state_id(args.block.default_state.id);

        props.north = connect_north;
        props.south = connect_south;
        props.west = connect_west;
        props.east = connect_east;

        props.to_state_id(args.block)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.is_new_block() {
            return;
        }

        Self::update(args.world, args.position, args.state_id);
    }

    /// Vanilla `TripWireBlock.playerWillDestroy`: shears disarm the wire first, so the hooks
    /// don't click when it goes.
    fn player_will_destroy(&self, args: PlayerWillDestroyArgs<'_>) {
        if args.player.inventory().held_item().get_item() != &Item::SHEARS {
            return;
        }
        let mut props = TripwireProperties::from_state_id(args.state.id);
        props.disarmed = true;
        // Flags 260: no neighbour or client update.
        args.world.set_block_state(
            args.position,
            props.to_state_id(args.block),
            BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK,
        );
        args.world.game_event_from(
            Some(args.player.as_ref()),
            GameEvent::Shear,
            args.position.to_centered_f64(),
        );
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        args.direction
            .to_horizontal_facing()
            .map_or(args.state_id, |facing| {
                let mut props = TripwireProperties::from_state_id(args.state_id);
                *match facing {
                    HorizontalFacing::North => &mut props.north,
                    HorizontalFacing::South => &mut props.south,
                    HorizontalFacing::West => &mut props.west,
                    HorizontalFacing::East => &mut props.east,
                } = Self::should_connect_to(args.neighbor_state_id, args.direction);
                props.to_state_id(args.block)
            })
    }

    /// Vanilla `TripWireBlock.tick`.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        if !TripwireProperties::from_state_id(state_id).powered {
            return;
        }
        // `level.getEntities(null, bounds)`: no spectators.
        let aabb = shape(state_id).at_pos(*args.position);
        let pressed = args
            .world
            .get_entities_at_box(&aabb)
            .iter()
            .any(|e| !e.is_spectator() && !e.is_ignoring_block_triggers())
            || args
                .world
                .get_players_at_box(&aabb)
                .iter()
                .any(|p| !p.is_spectator());
        Self::check_pressed(args.world, args.position, args.block, pressed);
    }

    /// Vanilla `TripWireBlock.affectNeighborsAfterRemoval`: the hooks see the removed wire
    /// as powered.
    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        if args.moved {
            return;
        }
        let mut props = TripwireProperties::from_state_id(args.old_state_id);
        props.powered = true;
        Self::update(args.world, args.position, props.to_state_id(args.block));
    }
}

impl TripwireBlock {
    /// Vanilla `TripWireBlock.checkPressed`, given whether an entity that doesn't ignore
    /// block triggers is on the wire.
    fn check_pressed(world: &Arc<World>, pos: &BlockPos, block: &Block, should_be_pressed: bool) {
        let state_id = world.get_block_state_id(pos);
        let mut props = TripwireProperties::from_state_id(state_id);
        let was_pressed = props.powered;
        if should_be_pressed != was_pressed {
            props.powered = should_be_pressed;
            let new_state_id = props.to_state_id(block);
            world.set_block_state(pos, new_state_id, BlockFlags::NOTIFY_ALL);
            Self::update(world, pos, new_state_id);
        }

        if should_be_pressed {
            world.schedule_block_tick(block, *pos, RECHECK_PERIOD, TickPriority::Normal);
        } else if was_pressed {
            world.schedule_block_tick(block, *pos, 1, TickPriority::Normal);
        }
    }

    /// Vanilla `TripWireBlock.updateSource`.
    fn update(world: &Arc<World>, pos: &BlockPos, state_id: BlockStateId) {
        for dir in [BlockDirection::South, BlockDirection::West] {
            for i in 1..42 {
                let current_pos = pos.offset_dir(dir.to_offset(), i);
                let (current_block, current_state) = world.get_block_and_state_id(&current_pos);
                if current_block == &Block::TRIPWIRE_HOOK {
                    let current_props = TripwireHookProperties::from_state_id(current_state);
                    if dir
                        .opposite()
                        .to_horizontal_facing()
                        .is_some_and(|f| current_props.facing == f)
                    {
                        TripwireHookBlock::update(
                            world,
                            current_pos,
                            current_state,
                            false,
                            true,
                            i,
                            Some(state_id),
                        );
                    }
                    break;
                }
                if current_block != &Block::TRIPWIRE {
                    break;
                }
            }
        }
    }

    #[must_use]
    pub fn should_connect_to(state_id: BlockStateId, facing: BlockDirection) -> bool {
        let block = Block::from_state_id(state_id);
        if block == &Block::TRIPWIRE_HOOK {
            let props = TripwireHookProperties::from_state_id(state_id);
            Some(props.facing) == facing.opposite().to_horizontal_facing()
        } else {
            block == &Block::TRIPWIRE
        }
    }
}
