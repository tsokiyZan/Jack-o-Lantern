use crate::entity::player::Player;
use crate::item::interaction_result::InteractionResult;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component_impl::BundleContentsImpl;
use pumpkin_data::item::Item;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_util::Hand;

pub struct BundleItem;

impl ItemMetadata for BundleItem {
    fn ids() -> Box<[u16]> {
        tag::Item::MINECRAFT_BUNDLES.1.into()
    }
}

impl ItemBehaviour for BundleItem {
    /// Vanilla `BundleItem.use` returns `SUCCESS` whatever the bundle holds.
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        // TODO: vanilla starts using the bundle and drops its contents over the use
        // (`onUseTick`), not one stack at once.
        let mut bundle = player.inventory.get_stack_in_hand(hand);
        if let Some(bundle_contents) = bundle.get_data_component_mut::<BundleContentsImpl>()
            && let Some(extracted_stack) = bundle_contents.try_extract()
        {
            player.world().play_sound(
                Sound::ItemBundleRemoveOne,
                pumpkin_data::sound::SoundCategory::Players,
                &player.position(),
            );
            player.inventory.set_stack_in_hand(hand, bundle.clone());
            player.drop_item(extracted_stack);
            let slot_index = match hand {
                Hand::Right => player.inventory.get_selected_slot() as usize,
                Hand::Left => PlayerInventory::OFF_HAND_SLOT,
            };
            player.award_item_used(bundle.item);
            player.sync_hand_slot(slot_index, bundle);
        }
        InteractionResult::SUCCESS
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
