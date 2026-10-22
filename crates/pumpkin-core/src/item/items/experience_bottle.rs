use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;

use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::item::Item;
use pumpkin_data::sound::{Sound, SoundCategory};

pub struct ExperienceBottleItem;

impl ItemMetadata for ExperienceBottleItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::EXPERIENCE_BOTTLE.id])
    }
}

impl ItemBehaviour for ExperienceBottleItem {
    fn normal_use(&self, item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let world = player.world();
        let pos = player.eye_position();
        world.play_sound(
            Sound::EntityExperienceBottleThrow,
            SoundCategory::Players,
            &pos,
        );

        let amount = (rand::random::<u32>() % 9 + 3) as u32; // 3..=11 exp
        ExperienceOrbEntity::spawn(&world, pos, amount);

        player.award_item_used(item);
        crate::item::consume_in_hand(player, hand);
        InteractionResult::SUCCESS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
