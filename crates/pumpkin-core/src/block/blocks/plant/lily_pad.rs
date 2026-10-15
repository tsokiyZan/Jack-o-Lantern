use crate::entity::vehicle::boat::BoatEntity;
use pumpkin_data::BlockStateId;
use pumpkin_data::fluid::Fluid;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::{BlockAccessor, BlockFlags};

use crate::block::{GetStateForNeighborUpdateArgs, blocks::plant::PlantBlockBase};

use crate::block::{BlockBehaviour, CanPlaceAtArgs, OnEntityCollisionArgs};
use crate::world::World;

#[pumpkin_block("minecraft:lily_pad")]
pub struct LilyPadBlock;

impl BlockBehaviour for LilyPadBlock {
    fn on_entity_collision(&self, args: OnEntityCollisionArgs<'_>) {
        // Vanilla `LilyPadBlock.entityInside`: any `AbstractBoat`, rafts and chest boats included.
        if args
            .entity
            .cast_any()
            .downcast_ref::<BoatEntity>()
            .is_some()
        {
            args.world
                .break_block(args.position, None, BlockFlags::NOTIFY_ALL);
        }
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        <Self as PlantBlockBase>::can_place_at(self, args.block_accessor, args.position)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        <Self as PlantBlockBase>::get_state_for_neighbor_update(
            self,
            args.world,
            args.position,
            args.state_id,
        )
    }
}

impl PlantBlockBase for LilyPadBlock {
    /// Vanilla `LilyPadBlock.mayPlaceOn`.
    fn can_plant_on_top(&self, block_accessor: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        let state_id = block_accessor.get_block_state_id(pos);
        let fluid = World::fluid_type_from_state_id(state_id);
        let fluid_above =
            World::fluid_type_from_state_id(block_accessor.get_block_state_id(&pos.up()));
        (fluid.has_tag(&tag::Fluid::MINECRAFT_SUPPORTS_LILY_PAD)
            || state_id
                .to_block()
                .has_tag(&tag::Block::MINECRAFT_SUPPORTS_LILY_PAD))
            && fluid_above == &Fluid::EMPTY
    }
}
