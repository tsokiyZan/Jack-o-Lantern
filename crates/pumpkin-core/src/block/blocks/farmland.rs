use std::sync::Arc;

use crate::block::{
    BlockBehaviour, CanPlaceAtArgs, FallOnArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs,
    OnScheduledTickArgs, PathComputationType, RandomTickArgs,
};
use crate::entity::EntityBase;
use crate::world::World;
use crate::world::game_event::GameEventContext;
use pumpkin_data::block_properties::FarmlandLikeProperties;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::tag;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockAccessor;
use pumpkin_world::world::BlockFlags;

type FarmlandProperties = FarmlandLikeProperties;

#[pumpkin_block("minecraft:farmland")]
pub struct FarmlandBlock;

impl BlockBehaviour for FarmlandBlock {
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        turn_to_base_block(None, args.world, args.position);
    }

    /// Vanilla `FarmlandBlock.fallOn`: a heavy enough landing tramples the farmland.
    fn fall_on(&self, args: FallOnArgs<'_>) {
        let roll = args
            .world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_f32();
        if f64::from(roll) < args.fall_distance - 0.5 && args.entity.get_living_entity().is_some() {
            let can_trample = args.entity.get_player().map_or_else(
                || args.world.level_info.load().game_rules.mob_griefing,
                |player| !args.world.is_in_spawn_protection(player, args.position),
            );
            let entity = args.entity.get_entity();
            if can_trample && entity.width() * entity.width() * entity.height() > 0.512 {
                turn_to_base_block(Some(args.entity), args.world, args.position);
            }
        }
        crate::block::default_fall_on(&args);
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        if !can_place_at(args.world, args.position) {
            return Block::DIRT.default_state.id;
        }
        args.block.default_state.id
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if args.direction == BlockDirection::Up && !can_place_at(args.world, args.position) {
            args.world
                .schedule_block_tick(args.block, *args.position, 1, TickPriority::Normal);
        }
        args.state_id
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        can_place_at(args.block_accessor, args.position)
    }

    fn random_tick(&self, args: RandomTickArgs<'_>) {
        if args.world.is_raining_at(&args.position.up())
            || is_water_nearby(args.world, args.position)
        {
            let mut props = FarmlandProperties::default(args.block);
            let mut new_moisture = 7;
            if let Some(server) = args.world.server.upgrade() {
                let mut event =
                    crate::plugin::api::events::block::moisture_change::MoistureChangeEvent::new(
                        *args.position,
                        args.world.clone(),
                        new_moisture,
                    );
                server.plugin_manager.fire_blocking(&server, &mut event);
                if event.cancelled {
                    return;
                }
                new_moisture = event.new_moisture;
            }
            props.moisture = new_moisture.clamp(0, 7) as u8;
            args.world.set_block_state(
                args.position,
                props.to_state_id(args.block),
                BlockFlags::NOTIFY_LISTENERS,
            );
        } else {
            let state_id = args.world.get_block_state_id(args.position);
            let mut props = FarmlandProperties::from_state_id(state_id);
            if props.moisture == 0 {
                if !args
                    .world
                    .get_block(&args.position.up())
                    .has_tag(&tag::Block::MINECRAFT_MAINTAINS_FARMLAND)
                {
                    turn_to_base_block(None, args.world, args.position);
                }
            } else {
                let mut new_moisture = (props.moisture as i32 - 1).clamp(0, 7);
                if let Some(server) = args.world.server.upgrade() {
                    let mut event = crate::plugin::api::events::block::moisture_change::MoistureChangeEvent::new(
                        *args.position,
                        args.world.clone(),
                        new_moisture,
                    );
                    server.plugin_manager.fire_blocking(&server, &mut event);
                    if event.cancelled {
                        return;
                    }
                    new_moisture = event.new_moisture;
                }
                props.moisture = new_moisture.clamp(0, 7) as u8;
                args.world.set_block_state(
                    args.position,
                    props.to_state_id(args.block),
                    BlockFlags::NOTIFY_LISTENERS,
                );
            }
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

/// Vanilla `FarmlandBlock.turnToBaseBlock`: back to dirt, lifting whatever stands in the
/// farmland's missing top sixteenth (`Block.pushEntitiesUp`).
pub fn turn_to_base_block(
    source_entity: Option<&dyn EntityBase>,
    world: &Arc<World>,
    pos: &BlockPos,
) {
    let (x, y, z) = (f64::from(pos.0.x), f64::from(pos.0.y), f64::from(pos.0.z));
    let added = BoundingBox::new(
        Vector3::new(x, y + 15.0 / 16.0, z),
        Vector3::new(x + 1.0, y + 1.0, z + 1.0),
    );
    let mut entities = world.get_entities_at_box(&added);
    entities.extend(
        world
            .get_players_at_box(&added)
            .into_iter()
            .map(|player| player as Arc<dyn EntityBase>),
    );
    for entity in entities {
        // `Shapes.collide(Y, box.move(0, 1, 0), added, -1)`: how far the box, raised a block,
        // drops back before it rests on the dirt.
        let raised_min_y = entity.get_entity().bounding_box.load().min.y + 1.0;
        let offset = if raised_min_y >= added.max.y - 1.0e-7 {
            (added.max.y - raised_min_y).max(-1.0)
        } else {
            -1.0
        };
        // `teleportRelative`.
        let entity_pos = entity.get_entity().pos.load();
        crate::entity::teleport_to(
            entity.as_ref(),
            Vector3::new(entity_pos.x, entity_pos.y + 1.0 + offset, entity_pos.z),
        );
    }
    world.set_block_state(pos, Block::DIRT.default_state.id, BlockFlags::NOTIFY_ALL);
    world.game_event(
        GameEvent::BlockChange,
        pos.to_centered_f64(),
        GameEventContext::of(source_entity, Some(Block::DIRT.default_state)),
    );
}

fn can_place_at(world: &dyn BlockAccessor, block_pos: &BlockPos) -> bool {
    let state = world.get_block_state(&block_pos.up());
    !state.is_solid() // TODO: add fence gate block
}

fn is_water_nearby(world: &Arc<World>, block_pos: &BlockPos) -> bool {
    for dx in -4..=4 {
        for dy in 0..=1 {
            for dz in -4..=4 {
                let check_pos = block_pos.offset(Vector3 {
                    x: dx,
                    y: dy,
                    z: dz,
                });
                //TODO this should use tag water. It does not seem to work rn.
                if world.get_block(&check_pos) == &Block::WATER {
                    return true;
                }
            }
        }
    }
    false
}
