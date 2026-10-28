use std::sync::Arc;

use crate::block::registry::BlockActionResult;
use crate::entity::Entity;
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::entity::projectile::ThrownItemEntity;
use crate::entity::projectile::{
    lingering_potion::LingeringPotionEntity, splash_potion::SplashPotionEntity,
};
use crate::item::create_filled_result;
use crate::item::interaction_result::InteractionResult;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::data_component_impl::PotionContentsImpl;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::particle::Particle;
use pumpkin_data::potion::Potion;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockDirection};
use pumpkin_util::Hand;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::world::BlockFlags;
use rand::{RngExt, rng};

pub struct PotionItem;
pub struct SplashPotionItem;
pub struct LingeringPotionItem;

impl ItemMetadata for PotionItem {
    fn ids() -> Box<[u16]> {
        [Item::POTION.id].into()
    }
}

impl ItemMetadata for SplashPotionItem {
    fn ids() -> Box<[u16]> {
        [Item::SPLASH_POTION.id].into()
    }
}

impl ItemMetadata for LingeringPotionItem {
    fn ids() -> Box<[u16]> {
        [Item::LINGERING_POTION.id].into()
    }
}

const ROLL: f32 = -20.0;
const POWER: f32 = 0.5;

impl ItemBehaviour for PotionItem {
    /// Vanilla `PotionItem.useOn`: a water potion turns `#convertible_to_mud` into mud. The
    /// thrown potions have no such `useOn`.
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        _hand: Hand,
        location: BlockPos,
        face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        // `potionContents.is(Potions.WATER)`: custom effects or colours don't matter.
        let is_water = item
            .get_data_component::<PotionContentsImpl>()
            .is_some_and(|contents| contents.potion_id == Some(i32::from(Potion::WATER.id)));
        if face == BlockDirection::Down
            || !block.has_tag(&tag::Block::MINECRAFT_CONVERTIBLE_TO_MUD)
            || !is_water
        {
            return BlockActionResult::Pass;
        }

        let world = player.world();
        let center = location.to_centered_f64();
        world.play_sound(Sound::EntityGenericSplash, SoundCategory::Blocks, &center);
        create_filled_result(item, player, ItemStack::new(1, &Item::GLASS_BOTTLE), true);
        for _ in 0..5 {
            let (dx, dz) = {
                let mut random = world
                    .random
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (random.next_f64(), random.next_f64())
            };
            world.spawn_particle(
                Vector3::new(
                    f64::from(location.0.x) + dx,
                    f64::from(location.0.y + 1),
                    f64::from(location.0.z) + dz,
                ),
                Vector3::new(0.0, 0.0, 0.0),
                1.0,
                1,
                Particle::Splash,
            );
        }
        world.play_sound(Sound::ItemBottleEmpty, SoundCategory::Blocks, &center);
        world.game_event_from(None, GameEvent::FluidPlace, center);
        world.set_block_state(
            &location,
            Block::MUD.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        BlockActionResult::Success
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Vanilla `ThrowablePotionItem.use`, after the subclass's throw sound.
fn throw_potion<E>(
    item: &Item,
    player: &Player,
    hand: Hand,
    entity_type: &'static EntityType,
    new_shot: impl FnOnce(Entity, &Entity) -> E,
    thrown: impl FnOnce(&E) -> &ThrownItemEntity,
    set_stack: impl FnOnce(&E, ItemStack),
) -> InteractionResult
where
    E: EntityBase + 'static,
{
    let world = player.world();
    let entity = Entity::new(world.clone(), player.position(), entity_type);
    let potion = new_shot(entity, player.get_entity());
    set_stack(&potion, player.inventory.get_stack_in_hand(hand));
    let (yaw, pitch) = player.rotation();
    thrown(&potion).set_velocity_from(player, pitch, yaw, ROLL, POWER, 1.0);
    world.spawn_entity(Arc::new(potion));

    player.award_item_used(item);
    crate::item::consume_in_hand(player, hand);
    InteractionResult::SUCCESS
}

fn play_throw_sound(player: &Player, sound: Sound, category: SoundCategory) {
    player.world().play_sound_fine(
        sound,
        category,
        &player.position(),
        0.5,
        0.4 / (rng().random::<f32>() * 0.4 + 0.8),
    );
}

impl ItemBehaviour for SplashPotionItem {
    fn normal_use(&self, item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        play_throw_sound(
            player,
            Sound::EntitySplashPotionThrow,
            SoundCategory::Players,
        );
        throw_potion(
            item,
            player,
            hand,
            &EntityType::SPLASH_POTION,
            SplashPotionEntity::new_shot,
            |potion| &potion.thrown,
            SplashPotionEntity::set_item_stack,
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ItemBehaviour for LingeringPotionItem {
    fn normal_use(&self, item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        play_throw_sound(
            player,
            Sound::EntityLingeringPotionThrow,
            SoundCategory::Neutral,
        );
        throw_potion(
            item,
            player,
            hand,
            &EntityType::LINGERING_POTION,
            LingeringPotionEntity::new_shot,
            |potion| &potion.thrown,
            LingeringPotionEntity::set_item_stack,
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
