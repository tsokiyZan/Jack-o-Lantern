use crate::block::{
    BlockBehaviour, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs, PlacedArgs,
    nearest_looking_directions, standing_and_wall_placement,
};
use crate::entity::EntityBase;
use pumpkin_data::block_properties::{WallTorchLikeProperties, WhiteBannerLikeProperties};
use pumpkin_data::{Block, BlockDirection, BlockStateId, FacingExt, HorizontalFacingExt};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockAccessor;

use crate::block::entities::banner::BannerBlockEntity;
use std::sync::Arc;

/// Vanilla `BannerBlock` and `WallBannerBlock`, every colour (`#banners` holds both).
#[pumpkin_block_from_tag("minecraft:banners")]
pub struct BannerBlock;

impl BannerBlock {
    fn is_wall(block: &Block) -> bool {
        block.name.ends_with("_wall_banner")
    }

    /// The `WallBannerBlock` a banner item hangs as (`StandingAndWallBlockItem.wallBlock`).
    fn wall_block(block: &Block) -> Option<&'static Block> {
        Block::from_name(&block.name.replace("_banner", "_wall_banner"))
    }

    /// Vanilla `BannerBlock.canSurvive` and `WallBannerBlock.canSurvive`.
    fn can_survive(world: &dyn BlockAccessor, pos: &BlockPos, state: BlockStateId) -> bool {
        let block = Block::from_state_id(state);
        let support = if Self::is_wall(block) {
            let facing = WallTorchLikeProperties::from_state_id(state).facing;
            pos.offset(facing.to_block_direction().opposite().to_offset())
        } else {
            pos.down()
        };
        world.get_block_state(&support).is_solid()
    }
}

impl BlockBehaviour for BannerBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.is_new_block() {
            return;
        }
        args.world
            .add_block_entity(Arc::new(BannerBlockEntity::new(*args.position)));
    }

    /// Vanilla `StandingAndWallBlockItem.getPlacementState` with `BannerBlock` standing
    /// (rotation `getRotation() + 180`) and `WallBannerBlock` on the wall.
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut standing = WhiteBannerLikeProperties::default(args.block);
        standing.rotation = args.player.get_entity().get_flipped_rotation_16();
        let standing = standing.to_state_id(args.block);
        // `WallBannerBlock.getStateForPlacement`: the first horizontal looking direction whose
        // opposite side holds it.
        let wall = Self::wall_block(args.block).and_then(|wall_block| {
            nearest_looking_directions(&args)
                .into_iter()
                .find_map(|direction| {
                    let facing = direction.opposite().to_horizontal_facing()?;
                    let mut props = WallTorchLikeProperties::default(wall_block);
                    props.facing = facing;
                    let state = props.to_state_id(wall_block);
                    Self::can_survive(args.world, args.position, state).then_some(state)
                })
        });
        standing_and_wall_placement(&args, standing, wall, |state| {
            Self::can_survive(args.world, args.position, state)
        })
    }

    /// The placement itself picks a surviving state (see `on_place`); a placed banner survives
    /// like vanilla's `canSurvive`.
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        args.use_item_on.is_some()
            || Self::can_survive(args.block_accessor, args.position, args.state.id)
    }

    /// Vanilla `BannerBlock.updateShape` and `WallBannerBlock.updateShape`: the block the banner
    /// hangs on or stands on going removes it.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let support = if Self::is_wall(args.block) {
            WallTorchLikeProperties::from_state_id(args.state_id)
                .facing
                .to_block_direction()
                .opposite()
        } else {
            BlockDirection::Down
        };
        if args.direction == support && !Self::can_survive(args.world, args.position, args.state_id)
        {
            return BlockStateId::AIR;
        }
        args.state_id
    }
}
