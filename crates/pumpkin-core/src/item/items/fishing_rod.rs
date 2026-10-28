use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::entity::Entity;
use crate::entity::player::Player;
use crate::entity::projectile::fishing_bobber::FishingBobberEntity;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::sound::{Sound, SoundCategory};

pub struct FishingRodItem;

impl ItemMetadata for FishingRodItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::FISHING_ROD.id])
    }
}

impl ItemBehaviour for FishingRodItem {
    /// Vanilla `FishingRodItem.use`: `SUCCESS` whether it casts or reels in.
    fn normal_use(&self, item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let world = player.world();
        let bobber_id = player.fishing_bobber.load(Ordering::Relaxed);

        let stack = player.inventory.get_stack_in_hand(hand);

        if bobber_id == -1 {
            // Cast
            let pitch = 0.4 / (world.random_f32() * 0.4 + 0.8);
            world.play_sound_fine(
                Sound::EntityFishingBobberThrow,
                SoundCategory::Neutral,
                &player.position(),
                0.5,
                pitch,
            );

            // TODO: lure speed and luck from the rod's enchantments.
            let bobber_entity = Entity::new(
                world.clone(),
                player.position(),
                &EntityType::FISHING_BOBBER,
            );
            let bobber = FishingBobberEntity::new_cast(bobber_entity, player);

            player
                .fishing_bobber
                .store(bobber.entity.entity_id, Ordering::Relaxed);

            let bobber_arc: Arc<FishingBobberEntity> = Arc::new(bobber);
            world.spawn_entity(bobber_arc);
            player.award_item_used(item);
            crate::item::cause_use_vibration(&stack, player, GameEvent::ItemInteractStart);
        } else {
            // Reel in
            if let Some(bobber_base) = world.get_entity_by_id(bobber_id) {
                if let Some(bobber) = bobber_base.cast_any().downcast_ref::<FishingBobberEntity>() {
                    // `itemStack.hurtAndBreak(retrieve(itemStack), player, hand)`.
                    let damage = bobber.reel_in(player);
                    if damage > 0 {
                        player.damage_item_in_hand(hand, damage);
                    }
                    // TODO: give items
                }
                bobber_base.get_entity().remove();
            }
            player.fishing_bobber.store(-1, Ordering::Relaxed);

            let pitch = 0.4 / (world.random_f32() * 0.4 + 0.8);
            world.play_sound_fine(
                Sound::EntityFishingBobberRetrieve,
                SoundCategory::Neutral,
                &player.position(),
                1.0,
                pitch,
            );
            // Vanilla checks the stack after the durability loss, so a rod that broke is empty.
            let stack = player.inventory.get_stack_in_hand(hand);
            crate::item::cause_use_vibration(&stack, player, GameEvent::ItemInteractFinish);
        }
        InteractionResult::SUCCESS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
