use std::sync::Arc;

use crate::block::UseWithItemArgs;
use crate::block::blocks::chests::get_connected_direction;
use crate::block::blocks::weathering_copper::with_properties_of;
use crate::block::entities::BlockEntity;
use crate::block::registry::BlockActionResult;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::Taggable;
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockStateId, tag};
use pumpkin_data::{BlockDirection, BlockId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;

pub struct HoneyCombItem;

impl ItemMetadata for HoneyCombItem {
    fn ids() -> Box<[u16]> {
        [Item::HONEYCOMB.id].into()
    }
}

impl ItemBehaviour for HoneyCombItem {
    /// Vanilla `HoneycombItem.useOn`.
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        location: BlockPos,
        _face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        _block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        let world = player.world();
        let old_state = world.get_block_state(&location);
        let Some(waxed_state) = get_waxed(old_state.id) else {
            return BlockActionResult::Pass;
        };
        item.decrement_unless_creative(player.gamemode.load(), 1);
        world.set_block_state(&location, waxed_state, BlockFlags::NOTIFY_ALL);
        world.sync_world_event_from(player, WorldEvent::ParticlesWaxOn, location, 0);
        world.play_block_sound_expect(
            player,
            Sound::ItemHoneycombWaxOn,
            SoundCategory::Blocks,
            location,
        );
        if Block::from_state_id(old_state.id).has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS)
            && let Some(direction) = get_connected_direction(old_state.id)
        {
            let neighbor_pos = location.offset(direction.to_offset());
            world.sync_world_event_from(player, WorldEvent::ParticlesWaxOn, neighbor_pos, 0);
            world.play_block_sound_expect(
                player,
                Sound::ItemHoneycombWaxOn,
                SoundCategory::Blocks,
                location,
            );
        }
        BlockActionResult::Success
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Vanilla `HoneycombItem.getWaxed`: the waxed block, with the old state's properties.
pub(crate) fn get_waxed(state_id: BlockStateId) -> Option<BlockStateId> {
    let block = Block::from_state_id(state_id);
    let waxed = get_waxed_equivalent(block.id)?.to_block();
    Some(with_properties_of(block, state_id, waxed))
}

impl HoneyCombItem {
    pub fn apply_to_sign(
        &self,
        args: &UseWithItemArgs<'_>,
        block_entity: &Arc<dyn BlockEntity>,
        sign_entity: &crate::block::entities::sign::SignEntityRef<'_>,
    ) -> BlockActionResult {
        if sign_entity.is_waxed() {
            return BlockActionResult::PassToDefaultBlockAction;
        }
        sign_entity.set_waxed(true);

        // Vanilla `HoneycombItem.tryApplyToSign`.
        args.world.update_block_entity(block_entity);
        args.world
            .sync_world_event(WorldEvent::ParticlesWaxOn, *args.position, 0);
        args.world.play_block_sound(
            Sound::ItemHoneycombWaxOn,
            SoundCategory::Blocks,
            *args.position,
        );

        BlockActionResult::Success
    }
}

pub(crate) const fn get_waxed_equivalent(id: BlockId) -> Option<BlockId> {
    match id {
        BlockId::OXIDIZED_COPPER => Some(BlockId::WAXED_OXIDIZED_COPPER),
        BlockId::WEATHERED_COPPER => Some(BlockId::WAXED_WEATHERED_COPPER),
        BlockId::EXPOSED_COPPER => Some(BlockId::WAXED_EXPOSED_COPPER),
        BlockId::COPPER_BLOCK => Some(BlockId::WAXED_COPPER_BLOCK),
        BlockId::OXIDIZED_CHISELED_COPPER => Some(BlockId::WAXED_OXIDIZED_CHISELED_COPPER),
        BlockId::WEATHERED_CHISELED_COPPER => Some(BlockId::WAXED_WEATHERED_CHISELED_COPPER),
        BlockId::EXPOSED_CHISELED_COPPER => Some(BlockId::WAXED_EXPOSED_CHISELED_COPPER),
        BlockId::CHISELED_COPPER => Some(BlockId::WAXED_CHISELED_COPPER),
        BlockId::OXIDIZED_COPPER_GRATE => Some(BlockId::WAXED_OXIDIZED_COPPER_GRATE),
        BlockId::WEATHERED_COPPER_GRATE => Some(BlockId::WAXED_WEATHERED_COPPER_GRATE),
        BlockId::EXPOSED_COPPER_GRATE => Some(BlockId::WAXED_EXPOSED_COPPER_GRATE),
        BlockId::COPPER_GRATE => Some(BlockId::WAXED_COPPER_GRATE),
        BlockId::OXIDIZED_CUT_COPPER => Some(BlockId::WAXED_OXIDIZED_CUT_COPPER),
        BlockId::WEATHERED_CUT_COPPER => Some(BlockId::WAXED_WEATHERED_CUT_COPPER),
        BlockId::EXPOSED_CUT_COPPER => Some(BlockId::WAXED_EXPOSED_CUT_COPPER),
        BlockId::CUT_COPPER => Some(BlockId::WAXED_CUT_COPPER),
        BlockId::OXIDIZED_CUT_COPPER_STAIRS => Some(BlockId::WAXED_OXIDIZED_CUT_COPPER_STAIRS),
        BlockId::WEATHERED_CUT_COPPER_STAIRS => Some(BlockId::WAXED_WEATHERED_CUT_COPPER_STAIRS),
        BlockId::EXPOSED_CUT_COPPER_STAIRS => Some(BlockId::WAXED_EXPOSED_CUT_COPPER_STAIRS),
        BlockId::CUT_COPPER_STAIRS => Some(BlockId::WAXED_CUT_COPPER_STAIRS),
        BlockId::OXIDIZED_CUT_COPPER_SLAB => Some(BlockId::WAXED_OXIDIZED_CUT_COPPER_SLAB),
        BlockId::WEATHERED_CUT_COPPER_SLAB => Some(BlockId::WAXED_WEATHERED_CUT_COPPER_SLAB),
        BlockId::EXPOSED_CUT_COPPER_SLAB => Some(BlockId::WAXED_EXPOSED_CUT_COPPER_SLAB),
        BlockId::CUT_COPPER_SLAB => Some(BlockId::WAXED_CUT_COPPER_SLAB),
        BlockId::OXIDIZED_COPPER_BULB => Some(BlockId::WAXED_OXIDIZED_COPPER_BULB),
        BlockId::WEATHERED_COPPER_BULB => Some(BlockId::WAXED_WEATHERED_COPPER_BULB),
        BlockId::EXPOSED_COPPER_BULB => Some(BlockId::WAXED_EXPOSED_COPPER_BULB),
        BlockId::COPPER_BULB => Some(BlockId::WAXED_COPPER_BULB),
        BlockId::OXIDIZED_COPPER_DOOR => Some(BlockId::WAXED_OXIDIZED_COPPER_DOOR),
        BlockId::WEATHERED_COPPER_DOOR => Some(BlockId::WAXED_WEATHERED_COPPER_DOOR),
        BlockId::EXPOSED_COPPER_DOOR => Some(BlockId::WAXED_EXPOSED_COPPER_DOOR),
        BlockId::COPPER_DOOR => Some(BlockId::WAXED_COPPER_DOOR),
        BlockId::OXIDIZED_COPPER_TRAPDOOR => Some(BlockId::WAXED_OXIDIZED_COPPER_TRAPDOOR),
        BlockId::WEATHERED_COPPER_TRAPDOOR => Some(BlockId::WAXED_WEATHERED_COPPER_TRAPDOOR),
        BlockId::EXPOSED_COPPER_TRAPDOOR => Some(BlockId::WAXED_EXPOSED_COPPER_TRAPDOOR),
        BlockId::COPPER_TRAPDOOR => Some(BlockId::WAXED_COPPER_TRAPDOOR),
        BlockId::OXIDIZED_COPPER_BARS => Some(BlockId::WAXED_OXIDIZED_COPPER_BARS),
        BlockId::WEATHERED_COPPER_BARS => Some(BlockId::WAXED_WEATHERED_COPPER_BARS),
        BlockId::EXPOSED_COPPER_BARS => Some(BlockId::WAXED_EXPOSED_COPPER_BARS),
        BlockId::COPPER_BARS => Some(BlockId::WAXED_COPPER_BARS),
        BlockId::OXIDIZED_COPPER_CHEST => Some(BlockId::WAXED_OXIDIZED_COPPER_CHEST),
        BlockId::WEATHERED_COPPER_CHEST => Some(BlockId::WAXED_WEATHERED_COPPER_CHEST),
        BlockId::EXPOSED_COPPER_CHEST => Some(BlockId::WAXED_EXPOSED_COPPER_CHEST),
        BlockId::COPPER_CHEST => Some(BlockId::WAXED_COPPER_CHEST),
        BlockId::OXIDIZED_COPPER_GOLEM_STATUE => Some(BlockId::WAXED_OXIDIZED_COPPER_GOLEM_STATUE),
        BlockId::WEATHERED_COPPER_GOLEM_STATUE => {
            Some(BlockId::WAXED_WEATHERED_COPPER_GOLEM_STATUE)
        }
        BlockId::EXPOSED_COPPER_GOLEM_STATUE => Some(BlockId::WAXED_EXPOSED_COPPER_GOLEM_STATUE),
        BlockId::COPPER_GOLEM_STATUE => Some(BlockId::WAXED_COPPER_GOLEM_STATUE),
        BlockId::OXIDIZED_LIGHTNING_ROD => Some(BlockId::WAXED_OXIDIZED_LIGHTNING_ROD),
        BlockId::WEATHERED_LIGHTNING_ROD => Some(BlockId::WAXED_WEATHERED_LIGHTNING_ROD),
        BlockId::EXPOSED_LIGHTNING_ROD => Some(BlockId::WAXED_EXPOSED_LIGHTNING_ROD),
        BlockId::LIGHTNING_ROD => Some(BlockId::WAXED_LIGHTNING_ROD),
        BlockId::OXIDIZED_COPPER_LANTERN => Some(BlockId::WAXED_OXIDIZED_COPPER_LANTERN),
        BlockId::WEATHERED_COPPER_LANTERN => Some(BlockId::WAXED_WEATHERED_COPPER_LANTERN),
        BlockId::EXPOSED_COPPER_LANTERN => Some(BlockId::WAXED_EXPOSED_COPPER_LANTERN),
        BlockId::COPPER_LANTERN => Some(BlockId::WAXED_COPPER_LANTERN),
        BlockId::OXIDIZED_COPPER_CHAIN => Some(BlockId::WAXED_OXIDIZED_COPPER_CHAIN),
        BlockId::WEATHERED_COPPER_CHAIN => Some(BlockId::WAXED_WEATHERED_COPPER_CHAIN),
        BlockId::EXPOSED_COPPER_CHAIN => Some(BlockId::WAXED_EXPOSED_COPPER_CHAIN),
        BlockId::COPPER_CHAIN => Some(BlockId::WAXED_COPPER_CHAIN),
        _ => None,
    }
}
