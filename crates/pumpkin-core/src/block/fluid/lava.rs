use super::flowing_trait::FlowingFluid;
use crate::{
    block::{FluidMetadata, blocks::fire::FireBlockBase, fluid::FluidBehaviour},
    entity::{
        Entity, EntityBase,
        inside_block_effect::{InsideBlockEffectApplier, InsideBlockEffectType},
    },
    world::World,
};
use pumpkin_data::{
    Block, BlockDirection, BlockState, BlockStateId,
    fluid::{Falling, Fluid, FluidProperties, Level},
    tag::{self, Taggable},
    world::WorldEvent,
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::{tick::TickPriority, world::BlockFlags};
use std::sync::Arc;
type FlowingFluidProperties = pumpkin_data::fluid::FlowingWaterLikeFluidProperties;

pub struct FlowingLava;

impl FluidMetadata for FlowingLava {
    fn ids() -> Box<[u16]> {
        [Fluid::FLOWING_LAVA.id].into()
    }
}

impl FlowingLava {
    fn can_spread_fire_around(world: &Arc<World>, pos: &BlockPos) -> bool {
        let spread_radius = world
            .level_info
            .load()
            .game_rules
            .fire_spread_radius_around_player;

        if spread_radius == 0 {
            return false;
        }
        if spread_radius == -1 {
            return true;
        }

        world
            .get_closest_player(pos.to_centered_f64(), spread_radius as f64)
            .is_some()
    }

    fn is_flammable_state(block_state: &BlockState) -> bool {
        let block = Block::from_state_id(block_state.id);
        if block.is_waterlogged(block_state.id) {
            return false;
        }
        block
            .flammable
            .as_ref()
            .is_some_and(|flammable| flammable.burn_chance > 0)
    }

    fn is_flammable(world: &Arc<World>, pos: &BlockPos) -> bool {
        world
            .get_block_state_if_loaded(pos)
            .is_some_and(Self::is_flammable_state)
    }

    fn has_flammable_neighbours(world: &Arc<World>, pos: &BlockPos) -> bool {
        BlockDirection::all()
            .iter()
            .any(|dir| Self::is_flammable(world, &pos.offset(dir.to_offset())))
    }

    fn receive_neighbor_fluids(world: &Arc<World>, block_pos: &BlockPos) -> bool {
        // Logic to determine if we should replace the fluid with any of (cobble, obsidian, stone, etc.)
        let below_is_soul_soil = world
            .get_block(&block_pos.offset(BlockDirection::Down.to_offset()))
            == &Block::SOUL_SOIL;
        let is_still = world.get_block_state_id(block_pos) == Block::LAVA.default_state.id;

        for dir in BlockDirection::all() {
            // Vanilla tests every direction but Down here. Lava landing in water
            // below is turned to stone by `spread_to`, not by this scan, so Down
            // is skipped rather than ending it - a lava block with water beneath
            // *and* water beside it still forms cobblestone.
            if dir == BlockDirection::Down {
                continue;
            }

            let neighbor_pos = block_pos.offset(dir.to_offset());
            if world
                .get_fluid(&neighbor_pos)
                .has_tag(&pumpkin_data::tag::Fluid::MINECRAFT_WATER)
            {
                let block = if is_still {
                    Block::OBSIDIAN
                } else {
                    Block::COBBLESTONE
                };
                world.set_block_state(block_pos, block.default_state.id, BlockFlags::NOTIFY_ALL);
                world.sync_world_event(WorldEvent::LavaFizz, *block_pos, 0);
                return false;
            }
            if below_is_soul_soil && world.get_block(&neighbor_pos) == &Block::BLUE_ICE {
                if let Some(server) = world.server.upgrade() {
                    let mut event =
                        crate::plugin::api::events::block::block_form::BlockFormEvent::new(
                            *block_pos,
                            &Block::BASALT,
                        );
                    server.plugin_manager.fire_blocking(&server, &mut event);
                    if event.cancelled {
                        return false;
                    }
                }
                world.set_block_state(
                    block_pos,
                    Block::BASALT.default_state.id,
                    BlockFlags::NOTIFY_ALL,
                );
                world.sync_world_event(WorldEvent::LavaFizz, *block_pos, 0);
                return false;
            }
        }
        true
    }
}

const LAVA_FLOW_SPEED_NETHER: i32 = 10;
const LAVA_FLOW_SPEED_SLOW: i32 = 30;

impl FluidBehaviour for FlowingLava {
    fn placed(
        &self,
        world: &Arc<World>,
        fluid: &Fluid,
        state_id: BlockStateId,
        block_pos: &BlockPos,
        old_state_id: BlockStateId,
        _notify: bool,
    ) {
        if old_state_id != state_id && Self::receive_neighbor_fluids(world, block_pos) {
            let flow_speed = self.get_flow_speed(world);
            world.schedule_fluid_tick(fluid, *block_pos, flow_speed, TickPriority::Normal);
        }
    }

    fn on_scheduled_tick(&self, world: &Arc<World>, _fluid: &Fluid, block_pos: &BlockPos) {
        Self.on_scheduled_tick_internal(world, &Fluid::FLOWING_LAVA, block_pos);
    }

    fn on_neighbor_update(
        &self,
        world: &Arc<World>,
        fluid: &Fluid,
        block_pos: &BlockPos,
        _notify: bool,
    ) {
        if Self::receive_neighbor_fluids(world, block_pos) {
            let flow_speed = self.get_flow_speed(world);
            world.schedule_fluid_tick(fluid, *block_pos, flow_speed, TickPriority::Normal);
        }
    }

    /// Vanilla `LavaFluid.entityInside`.
    fn on_entity_collision(
        &self,
        _entity: &dyn EntityBase,
        effect_applier: &InsideBlockEffectApplier,
    ) {
        effect_applier.apply(InsideBlockEffectType::ClearFreeze);
        effect_applier.apply(InsideBlockEffectType::LavaIgnite);
        effect_applier.run_after(InsideBlockEffectType::LavaIgnite, Entity::lava_hurt);
    }

    /// Vanilla `LavaFluid.randomTick`, drawing from the level random like the chunk tick.
    fn random_tick(&self, _fluid: &Fluid, world: &Arc<World>, block_pos: &BlockPos) {
        if !Self::can_spread_fire_around(world, block_pos) {
            return;
        }

        let random = || {
            world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        };
        let passes = random().next_bounded_i32(3);
        if passes > 0 {
            let mut test_pos = *block_pos;

            for _ in 0..passes {
                let x = random().next_bounded_i32(3) - 1;
                let z = random().next_bounded_i32(3) - 1;
                test_pos = test_pos.offset(Vector3::new(x, 1, z));
                if !world.is_loaded(&test_pos) {
                    return;
                }

                let state = world.get_block_state(&test_pos);
                if state.is_air() {
                    if Self::has_flammable_neighbours(world, &test_pos) {
                        world.set_block_state(
                            &test_pos,
                            FireBlockBase::get_state(world, &test_pos),
                            BlockFlags::NOTIFY_ALL,
                        );
                        return;
                    }
                } else if Block::from_state_id(state.id)
                    .has_tag(&tag::Block::MINECRAFT_BLOCKS_LAVA_FIRE_SPREAD)
                {
                    return;
                }
            }
        } else {
            for _ in 0..3 {
                let x = random().next_bounded_i32(3) - 1;
                let z = random().next_bounded_i32(3) - 1;
                let test_pos = block_pos.offset(Vector3::new(x, 0, z));
                if !world.is_loaded(&test_pos) {
                    return;
                }

                // `BaseFireBlock.getState(level, testPos)`: the state for `testPos`, not the
                // fire's own position above it.
                if world.get_block_state(&test_pos.up()).is_air()
                    && Self::is_flammable(world, &test_pos)
                {
                    world.set_block_state(
                        &test_pos.up(),
                        FireBlockBase::get_state(world, &test_pos),
                        BlockFlags::NOTIFY_ALL,
                    );
                }
            }
        }
    }
}

impl FlowingFluid for FlowingLava {
    fn get_level_decrease_per_block(&self, world: &World) -> i32 {
        if world.dimension.fast_lava { 1 } else { 2 }
    }

    fn get_flow_speed(&self, world: &World) -> i32 {
        // EnvironmentAttributes.FAST_LAVA
        if world.dimension.fast_lava {
            LAVA_FLOW_SPEED_NETHER
        } else {
            LAVA_FLOW_SPEED_SLOW
        }
    }

    fn get_max_flow_distance(&self, world: &World) -> i32 {
        if world.dimension.fast_lava { 5 } else { 3 }
    }

    /// Determines if lava can convert to source blocks based on game rules.
    fn can_convert_to_source(&self, world: &Arc<World>) -> bool {
        world.level_info.load().game_rules.lava_source_conversion
    }

    fn spread_to(&self, world: &Arc<World>, fluid: &Fluid, pos: &BlockPos, state_id: BlockStateId) {
        let new_props = FlowingFluidProperties::from_state_id(state_id, fluid);
        let current_state_id = world.get_block_state_id(pos);
        let block = Block::from_state_id(current_state_id);

        if new_props.level == Level::L8 && new_props.falling == Falling::True {
            // Stone creation when lava meets water
            if block == &Block::WATER {
                world.set_block_state(pos, Block::STONE.default_state.id, BlockFlags::NOTIFY_ALL);
                world.sync_world_event(WorldEvent::LavaFizz, *pos, 0);
                return;
            }
        }

        // Don't flow into waterlogged blocks
        if block.is_waterlogged(current_state_id) {
            return;
        }

        // Delegate quiescence, replacement and scheduling to the shared helper
        self.apply_spread(world, fluid, pos, state_id, new_props);
    }
}
