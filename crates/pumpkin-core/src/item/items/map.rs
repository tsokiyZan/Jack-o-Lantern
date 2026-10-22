use crate::entity::player::Player;
use crate::item::ItemBehaviour;
use crate::item::ItemMetadata;
use crate::item::interaction_result::InteractionResult;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::DataComponentImpl;
use pumpkin_data::data_component_impl::MapIdImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
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
        player.world().play_sound_from_entity(
            &player.living_entity.entity,
            Sound::UiCartographyTableTakeResult,
            SoundCategory::Players,
            1.0,
            1.0,
        );

        let map_id = server.next_map_id();
        let _ = server.map_manager.create_map(
            map_id,
            player.world().dimension.clone(),
            // `getBlockX` / `getBlockZ`.
            player.position().x.floor() as i32,
            player.position().z.floor() as i32,
            0, // Default scale
        );
        let mut filled_map = ItemStack::new(1, &Item::FILLED_MAP);
        filled_map.patch.push((
            DataComponent::MapId,
            Some(MapIdImpl { id: map_id }.to_dyn()),
        ));

        if inventory.get_stack_in_hand(hand).is_empty() {
            inventory.set_stack_in_hand(hand, filled_map);
        } else if !inventory.insert_stack_anywhere(&mut filled_map) {
            player.drop_from_hand(filled_map, false, true);
        }
        InteractionResult::SUCCESS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
