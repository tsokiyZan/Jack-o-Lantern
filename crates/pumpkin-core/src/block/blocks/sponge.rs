use pumpkin_macros::pumpkin_block;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use crate::block::{BlockBehaviour, OnNeighborUpdateArgs, PlacedArgs, drop_loot};
use crate::world::World;
use crate::world::loot::LootContextParameters;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::Taggable;
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

#[pumpkin_block("minecraft:sponge")]
pub struct SpongeBlock;

/// Vanilla `SpongeBlock.MAX_DEPTH`.
const MAX_DEPTH: u32 = 6;
/// Vanilla `SpongeBlock.MAX_COUNT`; the traversal counts the sponge itself, so it runs to
/// `MAX_COUNT + 1`.
const MAX_COUNT: usize = 64;

impl SpongeBlock {
    /// Vanilla `SpongeBlock.tryAbsorbWater`.
    pub fn absorb_water(world: &Arc<World>, position: &BlockPos) -> bool {
        // Pumpkin's plugin hook; fired only when there is water next to the sponge, which
        // vanilla needs to absorb anything.
        let touches_water = BlockDirection::all()
            .into_iter()
            .any(|dir| is_water(world, &position.offset(dir.to_offset())));
        if !touches_water {
            return false;
        }
        let mut event =
            crate::plugin::api::events::block::sponge_absorb::SpongeAbsorbEvent::new(*position);
        if let Some(server) = world.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled {
            return false;
        }

        if Self::remove_water_breadth_first_search(world, position) {
            world.set_block_state(
                position,
                Block::WET_SPONGE.default_state.id,
                BlockFlags::NOTIFY_LISTENERS,
            );
            world.play_block_sound(Sound::BlockSpongeAbsorb, SoundCategory::Blocks, *position);
            true
        } else {
            false
        }
    }

    /// Vanilla `SpongeBlock.removeWaterBreadthFirstSearch`: `BlockPos.breadthFirstTraversal`
    /// to depth 6 and 65 accepted nodes (the sponge included), removing water as it goes.
    fn remove_water_breadth_first_search(world: &Arc<World>, start: &BlockPos) -> bool {
        let mut nodes = VecDeque::new();
        let mut visited = HashSet::new();
        nodes.push_back((*start, 0));
        let mut count = 0;

        while let Some((pos, depth)) = nodes.pop_front() {
            if !visited.insert(pos) {
                continue;
            }
            if pos != *start && !Self::absorb_at(world, &pos) {
                continue;
            }
            count += 1;
            if count > MAX_COUNT {
                break;
            }
            if depth < MAX_DEPTH {
                for dir in BlockDirection::all() {
                    nodes.push_back((pos.offset(dir.to_offset()), depth + 1));
                }
            }
        }

        count > 1
    }

    /// The traversal's node processor: whether `pos` is accepted (its water removed).
    fn absorb_at(world: &Arc<World>, pos: &BlockPos) -> bool {
        if !is_water(world, pos) {
            return false;
        }
        let (block, state_id) = world.get_block_and_state_id(pos);
        // `BucketPickup.pickupBlock`: a water source or bubble column becomes air, a
        // waterlogged block dries.
        if block == &Block::BUBBLE_COLUMN
            || (block == &Block::WATER && state_id == Block::WATER.default_state.id)
        {
            world.set_block_state(pos, BlockStateId::AIR, BlockFlags::NOTIFY_ALL);
            return true;
        }
        if block.is_waterlogged(state_id) {
            let dry = block.set_waterlogged(state_id, false).unwrap_or(state_id);
            world.set_block_state(pos, dry, BlockFlags::NOTIFY_ALL);
            return true;
        }
        if block == &Block::WATER {
            world.set_block_state(pos, BlockStateId::AIR, BlockFlags::NOTIFY_ALL);
            return true;
        }
        if block == &Block::KELP
            || block == &Block::KELP_PLANT
            || block == &Block::SEAGRASS
            || block == &Block::TALL_SEAGRASS
        {
            // `dropResources` then `setBlockAndUpdate(AIR)`.
            let params = LootContextParameters {
                block_state: Some(BlockState::from_id(state_id)),
                position: Some(pos.to_f64()),
                ..Default::default()
            };
            drop_loot(world, block, pos, true, &params);
            world.set_block_state(pos, BlockStateId::AIR, BlockFlags::NOTIFY_ALL);
            return true;
        }
        false
    }
}

/// `getFluidState(pos).is(FluidTags.WATER)`.
fn is_water(world: &World, pos: &BlockPos) -> bool {
    world
        .get_fluid(pos)
        .has_tag(&pumpkin_data::tag::Fluid::MINECRAFT_WATER)
}

impl BlockBehaviour for SpongeBlock {
    /// Vanilla `SpongeBlock.onPlace`: only when the block becomes a sponge.
    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.is_new_block() {
            return;
        }
        Self::absorb_water(args.world, args.position);
    }

    /// Vanilla `SpongeBlock.neighborChanged`: any neighbour change tries to absorb.
    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        Self::absorb_water(args.world, args.position);
    }
}

#[pumpkin_block("minecraft:wet_sponge")]
pub struct WetSpongeBlock;

impl BlockBehaviour for WetSpongeBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        {
            // Check if placed where water evaporates (EnvironmentAttributes.WATER_EVAPORATES), if so, dry out
            if args.world.dimension.water_evaporates {
                args.world.set_block_state(
                    args.position,
                    Block::SPONGE.default_state.id,
                    BlockFlags::NOTIFY_ALL,
                );

                // `levelEvent(2009, pos, 0)`: the client draws the steam.
                args.world.sync_world_event(
                    WorldEvent::ParticlesWaterEvaporating,
                    *args.position,
                    0,
                );
                args.world.play_block_sound(
                    Sound::BlockWetSpongeDries,
                    SoundCategory::Blocks,
                    *args.position,
                );
            }
        }
    }
}
