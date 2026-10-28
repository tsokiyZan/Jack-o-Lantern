use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;
use std::sync::atomic::Ordering;

use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::entity::projectile::arrow::ArrowEntity;
use crate::item::items::projectile_weapon::ProjectileWeaponItem;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_protocol::IdOr;
use pumpkin_protocol::java::client::play::CSoundEffect;
use pumpkin_util::GameMode;

pub struct BowItem;

impl ItemMetadata for BowItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::BOW.id])
    }
}

impl ItemBehaviour for BowItem {
    /// Vanilla `BowItem.use`.
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        if !Self::has_arrows(player) && player.gamemode.load() != GameMode::Creative {
            return InteractionResult::FAIL;
        }
        let stack = player.inventory().get_stack_in_hand(hand);
        crate::item::start_using_item(player, hand, stack, Self::USE_DURATION);
        InteractionResult::CONSUME
    }

    fn on_stopped_using(&self, _stack: &ItemStack, player: &Player) {
        Self::release_bow(player, player.living_entity.used_item_hand());
    }

    fn get_use_duration(&self, _stack: &ItemStack) -> i32 {
        Self::USE_DURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl BowItem {
    /// The maximum number of ticks a bow can be drawn for
    pub const USE_DURATION: i32 = 72000;
    const MAX_DRAW_DURATION: f32 = 20.0;
    pub const ARROW_SPEED_MULTIPLIER: f32 = 3.0;

    /// Vanilla `BowItem.releaseUsing` for the bow in `hand`.
    pub fn release_bow(player: &Player, hand: Hand) {
        let weapon = &player.inventory().get_stack_in_hand(hand);
        // Get the used ticks
        let use_ticks = player.living_entity.item_use_time.load(Ordering::Relaxed);
        let use_ticks = Self::USE_DURATION - use_ticks;

        // Check minimum draw time
        if use_ticks < 3 {
            return;
        }

        // Check arrows again
        let arrow_slot = player.find_arrow();
        let gamemode = player.gamemode.load();

        if arrow_slot.is_none() && gamemode != GameMode::Creative {
            return;
        }

        let projectile = arrow_slot.map_or_else(
            || ItemStack::new(1, &Item::ARROW),
            |slot| {
                let stack = player.inventory.get_slot(slot);
                stack.copy_with_count(1)
            },
        );
        // Vanilla `draw`: an arrow `useAmmo` doesn't take (Infinity, creative) is marked
        // `INTANGIBLE_PROJECTILE`, so it can't be picked up.
        let projectiles = ProjectileWeaponItem::draw(
            &player.world(),
            weapon,
            &projectile,
            gamemode == GameMode::Creative,
        );

        // Calculate power and fire
        let power = Self::get_power_for_time(use_ticks);

        let is_crit = (power - 1.0).abs() < f32::EPSILON;
        Self::shoot(player, weapon, &projectiles, power, 1.0, is_crit);

        // The ammo `useAmmo` took.
        if let Some(slot) = arrow_slot
            && projectiles.first().is_some_and(|drawn| {
                drawn
                .get_data_component::<pumpkin_data::data_component_impl::IntangibleProjectileImpl>()
                .is_none()
            })
        {
            player.consume_arrow(slot);
        }

        // Damage bow
        player.damage_item_in_hand(hand, 1);
        player.award_item_used(&Item::BOW);
    }

    /// Check if player has arrows in their inventory
    fn has_arrows(player: &Player) -> bool {
        player.find_arrow().is_some()
    }

    /// Calculate the power/charge of the bow based on time held
    #[must_use]
    pub fn get_power_for_time(time_held: i32) -> f32 {
        let mut power = time_held as f32 / Self::MAX_DRAW_DURATION;
        power = (power * power + power * 2.0) / 3.0;
        if power > 1.0 {
            power = 1.0;
        }
        power
    }

    /// Creates projectile matching vanilla `ProjectileWeaponItem::createProjectile`.
    pub fn create_projectile(
        player: &Player,
        weapon: &ItemStack,
        projectile: &ItemStack,
        is_crit: bool,
    ) -> ArrowEntity {
        let world = player.world();
        ProjectileWeaponItem::create_projectile(
            world,
            player.get_entity(),
            weapon,
            projectile,
            is_crit,
        )
    }

    /// Shoot projectile matching vanilla `ProjectileWeaponItem::shoot`.
    pub fn shoot(
        player: &Player,
        weapon: &ItemStack,
        projectiles: &[ItemStack],
        power: f32,
        uncertainty: f32,
        is_crit: bool,
    ) {
        if power < 0.1 || projectiles.is_empty() {
            return;
        }

        let world = player.world();
        let speed = power * ProjectileWeaponItem::ARROW_SPEED_MULTIPLIER;

        ProjectileWeaponItem::shoot_projectiles(
            &world,
            player,
            weapon,
            projectiles,
            speed,
            uncertainty,
            is_crit,
            true,
        );

        let sound_pitch = 1.0 / (rand::random::<f32>() * 0.4 + 1.2) + power * 0.5;
        let sound_packet = CSoundEffect::new(
            IdOr::Id(Sound::EntityArrowShoot as u16),
            SoundCategory::Neutral,
            &player.position(),
            1.0,
            sound_pitch,
            0,
        );
        let chunk_pos = player.get_entity().chunk_pos.load();
        world.broadcast_to_chunk(chunk_pos, &sound_packet);
    }
}
