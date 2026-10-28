use crate::block::registry::BlockActionResult;
use crate::entity::player::Player;
use crate::item::items::block_transform::transform_block;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::block_transformer::AXE;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::{Block, BlockDirection, tag};
use pumpkin_util::Hand;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct AxeItem;

impl ItemMetadata for AxeItem {
    fn ids() -> Box<[u16]> {
        tag::Item::MINECRAFT_AXES.1.into()
    }
}

impl ItemBehaviour for AxeItem {
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        hand: Hand,
        location: BlockPos,
        face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        transform_block(&AXE, item, player, hand, location, face, block)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
