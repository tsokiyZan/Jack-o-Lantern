use std::any::Any;
use std::sync::Arc;

use crate::entity::player::Player;
use crate::entity::projectile::experience_bottle::ExperienceBottleEntity;
use crate::entity::{Entity, EntityBase};
use crate::item::interaction_result::InteractionResult;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::Hand;
use pumpkin_util::random::RandomImpl;

pub struct ExperienceBottleItem;

impl ItemMetadata for ExperienceBottleItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::EXPERIENCE_BOTTLE.id])
    }
}

impl ItemBehaviour for ExperienceBottleItem {
    /// Vanilla `ExperienceBottleItem.use`: thrown like a potion, the experience comes out
    /// where it lands.
    fn normal_use(&self, item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let world = player.world();
        let position = player.position();
        world.play_sound_fine(
            Sound::EntityExperienceBottleThrow,
            SoundCategory::Neutral,
            &position,
            0.5,
            // `level.getRandom()`.
            0.4 / (world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .next_f32()
                * 0.4
                + 0.8),
        );

        let entity = Entity::new(world.clone(), position, &EntityType::EXPERIENCE_BOTTLE);
        let bottle = ExperienceBottleEntity::new_shot(entity, player.get_entity());
        let (yaw, pitch) = player.rotation();
        bottle
            .thrown
            .set_velocity_from(player, pitch, yaw, -20.0, 0.7, 1.0);
        world.spawn_entity(Arc::new(bottle));

        player.award_item_used(item);
        crate::item::consume_in_hand(player, hand);
        InteractionResult::SUCCESS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
