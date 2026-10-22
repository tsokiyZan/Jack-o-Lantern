use crate::entity::player::Player;
use crate::item::ItemBehaviour;
use crate::item::ItemMetadata;
use crate::item::interaction_result::InteractionResult;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::DataComponentImpl;
use pumpkin_data::data_component_impl::MapIdImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::Hand;
use std::any::Any;

pub struct MapItem;

impl ItemMetadata for MapItem {
    fn ids() -> Box<[u16]> {
        [Item::MAP.id].into()
    }
}

impl ItemBehaviour for MapItem {
    /// Vanilla `EmptyMapItem.use`.
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let Some(server) = player.world().server.upgrade() else {
            return InteractionResult::SUCCESS;
        };
        let inventory = player.inventory();
        crate::item::consume_in_hand(player, hand);
        player.award_item_used(&Item::MAP);
        // TODO: the `ui.cartography_table.take_result` sound.

        let map_id = server.next_map_id();
        let _ = server.map_manager.create_map(
            map_id,
            player.world().dimension.clone(),
            player.position().x as i32,
            player.position().z as i32,
            0, // Default scale
        );
        let mut filled_map = ItemStack::new(1, &Item::FILLED_MAP);
        filled_map.patch.push((
            DataComponent::MapId,
            Some(MapIdImpl { id: map_id }.to_dyn()),
        ));

        if inventory.get_stack_in_hand(hand).is_empty() {
            inventory.set_stack_in_hand(hand, filled_map);
        } else if !inventory.insert_stack_anywhere(&mut filled_map) && !filled_map.is_empty() {
            player
                .world()
                .drop_stack(&player.position().to_block_pos(), filled_map);
        }
        InteractionResult::SUCCESS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
