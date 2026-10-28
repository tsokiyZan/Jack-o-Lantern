use crate::block::blocks::falling::FallingBlock;
use crate::block::registry::BlockActionResult;
use crate::block::{
    AttackArgs, BlockBehaviour, GetStateForNeighborUpdateArgs, NormalUseArgs, OnScheduledTickArgs,
    PathComputationType, PlacedArgs,
};
use crate::world::World;
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

#[pumpkin_block("minecraft:dragon_egg")]
pub struct DragonEggBlock;

impl DragonEggBlock {
    // DragonEggBlock.getDelayAfterPlace
    const DELAY_AFTER_PLACE: i32 = 5;
    const TELEPORT_RADIUS_XZ: i32 = 16;
    const TELEPORT_RADIUS_Y: i32 = 8;

    /// Vanilla `DragonEggBlock.teleport`: up to 1000 tries, from the level random, for an air
    /// block over a non-air one inside the world border and the build height.
    fn teleport(world: &Arc<World>, pos: &BlockPos) {
        let state_id = world.get_block_state_id(pos);
        for _ in 0..1000 {
            let test_pos = {
                let mut random = world
                    .random
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut offset =
                    |radius: i32| random.next_bounded_i32(radius) - random.next_bounded_i32(radius);
                let x = offset(Self::TELEPORT_RADIUS_XZ);
                let y = offset(Self::TELEPORT_RADIUS_Y);
                let z = offset(Self::TELEPORT_RADIUS_XZ);
                pos.add(x, y, z)
            };

            if world.get_block_state(&test_pos).is_air()
                && !world.get_block_state(&test_pos.down()).is_air()
                && world
                    .worldborder
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .contains_block(test_pos.0.x, test_pos.0.z)
                && world.is_in_build_limit(test_pos)
            {
                let packed_diff = pos.pack_difference_in_position(
                    &test_pos,
                    Self::TELEPORT_RADIUS_XZ,
                    Self::TELEPORT_RADIUS_Y,
                    Self::TELEPORT_RADIUS_XZ,
                );
                world.sync_world_event(WorldEvent::ParticlesDragonEggTeleport, *pos, packed_diff);
                world.set_block_state(&test_pos, state_id, BlockFlags::NOTIFY_LISTENERS);
                world.set_block_state(
                    pos,
                    pumpkin_data::Block::AIR.default_state.id,
                    BlockFlags::NOTIFY_ALL,
                );
                return;
            }
        }
    }
}

impl BlockBehaviour for DragonEggBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        FallingBlock::placed_with_delay(&args, Self::DELAY_AFTER_PLACE);
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        FallingBlock::get_state_for_neighbor_update_with_delay(&args, Self::DELAY_AFTER_PLACE)
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        Self::teleport(args.world, args.position);
        BlockActionResult::Success
    }

    fn attack(&self, args: AttackArgs<'_>) {
        Self::teleport(args.world, args.position);
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        FallingBlock::on_scheduled_tick(&FallingBlock, args);
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
