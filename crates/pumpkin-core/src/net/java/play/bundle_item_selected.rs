#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_data::data_component_impl::BundleContentsImpl;

impl JavaClient {
    pub fn handle_bundle_item_selected(&self, player: &Arc<Player>, packet: &SBundleItemSelected) {
        if !player.has_client_loaded() {
            return;
        }
        player.update_last_action_time();

        let selected_item_index = packet.selected_item_index.0;
        if selected_item_index < 0 && selected_item_index != -1 {
            self.try_kick(&TextComponent::text("Invalid selected item index"));
            return;
        }

        // `containerMenu.setSelectedBundleItemIndex`: `BundleItem.toggleSelectedItem` on the
        // slot's stack, a change no client is told of.
        let screen_handler = player
            .current_screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let screen_handler = screen_handler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let slot_index = packet.slot_id.0;
        let Some(slot) = usize::try_from(slot_index)
            .ok()
            .and_then(|index| screen_handler.get_behaviour().slots.get(index))
        else {
            return;
        };
        let mut stack = slot.get_cloned_stack();
        if let Some(contents) = stack.get_data_component_mut::<BundleContentsImpl>() {
            contents.toggle_selected_item(selected_item_index);
            slot.get_inventory().set_stack(slot.get_index(), stack);
        }
    }
}
