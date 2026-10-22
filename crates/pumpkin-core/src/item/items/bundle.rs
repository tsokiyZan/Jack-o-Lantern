use crate::entity::player::Player;
use crate::item::interaction_result::InteractionResult;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component_impl::BundleContentsImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag;
use pumpkin_util::Hand;
use pumpkin_util::random::RandomImpl;

pub struct BundleItem;

impl ItemMetadata for BundleItem {
    fn ids() -> Box<[u16]> {
        tag::Item::MINECRAFT_BUNDLES.1.into()
    }
}

impl ItemBehaviour for BundleItem {
    /// Vanilla `BundleItem.use`: the bundle is used, and drops its contents as it is
    /// (`onUseTick`).
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let stack = player.inventory.get_stack_in_hand(hand);
        crate::item::start_using_item(player, hand, stack, Self::USE_DURATION);
        InteractionResult::SUCCESS
    }

    /// Vanilla `BundleItem.onUseTick`: one stack on the first tick, then one every other tick
    /// after the first ten.
    fn on_use_tick(&self, _stack: &ItemStack, player: &Player, remaining_use_ticks: i32) {
        let first_tick = remaining_use_ticks == Self::USE_DURATION;
        if first_tick
            || (remaining_use_ticks < Self::USE_DURATION - 10 && remaining_use_ticks % 2 == 0)
        {
            Self::drop_content(player);
        }
    }

    fn get_use_duration(&self, _stack: &ItemStack) -> i32 {
        Self::USE_DURATION
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl BundleItem {
    /// Vanilla `BundleItem.getUseDuration`.
    pub const USE_DURATION: i32 = 200;

    /// Vanilla `BundleItem.dropContent` for the bundle in use.
    fn drop_content(player: &Player) {
        let hand = player.living_entity.used_item_hand();
        let mut bundle = player.inventory.get_stack_in_hand(hand);
        // `removeOneItemFromBundle`.
        let Some(removed) = bundle
            .get_data_component_mut::<BundleContentsImpl>()
            .and_then(BundleContentsImpl::try_extract)
        else {
            return;
        };
        let world = player.world();
        // `playRemoveOneSound`: the player's own `playSound`, which it doesn't hear itself.
        let pitch = 0.8 + Self::world_random_f32(player) * 0.4;
        world.play_sound_raw_expect(
            player,
            Sound::ItemBundleRemoveOne as u16,
            SoundCategory::Players,
            &player.position(),
            0.8,
            pitch,
        );
        player.inventory.set_stack_in_hand(hand, bundle.clone());
        player.drop_from_hand(removed, true, false);
        // `playDropContentsSound`.
        let pitch = 0.8 + Self::world_random_f32(player) * 0.4;
        world.play_sound_fine(
            Sound::ItemBundleDropContents,
            SoundCategory::Players,
            &player
                .living_entity
                .entity
                .block_pos
                .load()
                .to_centered_f64(),
            0.8,
            pitch,
        );
        player.award_item_used(bundle.item);
    }

    /// `level.getRandom().nextFloat()`.
    fn world_random_f32(player: &Player) -> f32 {
        player
            .world()
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_f32()
    }
}
