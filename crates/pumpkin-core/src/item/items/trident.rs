use crate::item::interaction_result::InteractionResult;
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::enchantment::{EnchantmentHelper, HurtAndBreak};
use crate::entity::player::Player;
use crate::entity::projectile::arrow::ArrowPickup;
use crate::entity::projectile::trident::TridentEntity;
use crate::entity::{Entity, EntityBase};
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::{DEG_TO_RAD, cos, sin};
use pumpkin_util::{GameMode, Hand};

pub struct TridentItem;

impl TridentItem {
    const USE_DURATION: i32 = 72000;
    const THROW_THRESHOLD_TIME: i32 = 10;
    const PROJECTILE_SHOOT_POWER: f32 = 2.5;
    const SPIN_ATTACK_TICKS: i32 = 20;
    const SPIN_ATTACK_DAMAGE: f32 = 8.0;
    const ON_GROUND_LIFT: f32 = 1.199_999_9;

    /// Vanilla `EnchantmentHelper.getTridentSpinAttackStrength`.
    fn spin_attack_strength(stack: &ItemStack) -> f32 {
        EnchantmentHelper::modify_trident_spin_attack_strength(stack, 0.0)
    }

    /// Writes the used stack back to its slot and sends it to the player.
    fn set_slot_stack(player: &Player, slot: usize, stack: ItemStack) {
        player.inventory().set_slot(slot, stack.clone());
        player.sync_hand_slot(slot, stack);
    }
}

impl ItemMetadata for TridentItem {
    fn ids() -> Box<[u16]> {
        [Item::TRIDENT.id].into()
    }
}

impl ItemBehaviour for TridentItem {
    /// Vanilla `TridentItem.use`.
    /// Vanilla `TridentItem.use`.
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let stack = player.inventory().get_stack_in_hand(hand);
        if stack.next_damage_will_break() {
            return InteractionResult::FAIL;
        }
        if Self::spin_attack_strength(&stack) > 0.0
            && !player.living_entity.entity.is_in_water_or_rain()
        {
            return InteractionResult::FAIL;
        }
        crate::item::start_using_item(player, hand, stack, Self::USE_DURATION);
        InteractionResult::CONSUME
    }

    /// Vanilla `TridentItem.releaseUsing`.
    #[expect(
        clippy::too_many_lines,
        reason = "mirrors vanilla TridentItem.releaseUsing"
    )]
    fn on_stopped_using(&self, _stack: &ItemStack, player: &Player) {
        let living = &player.living_entity;
        let entity = &living.entity;
        let time_held = Self::USE_DURATION - living.item_use_time.load(Ordering::Relaxed);
        if time_held < Self::THROW_THRESHOLD_TIME {
            return;
        }
        let hand = living
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .unwrap_or(Hand::Right);
        // Vanilla's stack is the one in the hand, which the wear below changes.
        let slot = player.inventory().slot_for_hand(hand);
        let mut stack = player.inventory().get_slot(slot);

        let riptide_strength = Self::spin_attack_strength(&stack);
        if riptide_strength > 0.0 && (!entity.is_in_water_or_rain() || entity.has_vehicle()) {
            return;
        }
        if stack.next_damage_will_break() {
            return;
        }

        let world = player.world();
        if riptide_strength > 0.0
            && let Some(player_arc) = world.get_player_by_uuid(player.gameprofile.id)
            && let Some(server) = world.server.upgrade()
        {
            let mut event =
                crate::plugin::api::events::player::player_riptide::PlayerRiptideEvent {
                    player: player_arc,
                    item_name: "trident".to_string(),
                    cancelled: false,
                };
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
        }

        let sound = EnchantmentHelper::pick_highest_level_trident_sound(&stack)
            .unwrap_or(Sound::ItemTridentThrow);
        player.award_item_used(&Item::TRIDENT);
        let (old_damage, old_count) = (stack.get_damage(), stack.item_count);
        stack.hurt_without_breaking(&world, 1, player);

        if riptide_strength == 0.0 {
            // `consumeAndReturn(1)`.
            let thrown = stack.copy_with_count(1);
            if !player.has_infinite_materials() {
                stack.decrement(1);
            }
            if stack.get_damage() != old_damage || stack.item_count != old_count {
                Self::set_slot_stack(player, slot, stack);
            }

            // `spawnProjectileFromRotation(..., 0.0, 2.5, 1.0)`.
            let (yaw, pitch) = player.rotation();
            let trident_entity =
                Entity::new(world.clone(), player.position(), &EntityType::TRIDENT);
            let pickup = if player.has_infinite_materials() {
                ArrowPickup::CreativeOnly
            } else {
                ArrowPickup::Allowed
            };
            let trident = TridentEntity::new_shot(
                trident_entity,
                player.get_entity(),
                thrown.clone(),
                pickup,
            );
            trident.set_velocity_from_rotation(
                player,
                pitch,
                yaw,
                0.0,
                Self::PROJECTILE_SHOOT_POWER,
                1.0,
            );
            let trident = Arc::new(trident);
            world.spawn_entity(trident.clone());
            trident.apply_on_projectile_spawned(&thrown);

            world.play_sound_from_entity(
                trident.get_entity(),
                sound,
                SoundCategory::Players,
                1.0,
                1.0,
            );
            return;
        }
        if stack.get_damage() != old_damage {
            Self::set_slot_stack(player, slot, stack.clone());
        }

        let (yaw, pitch) = player.rotation();
        let mut xd = -sin(yaw * DEG_TO_RAD) * cos(pitch * DEG_TO_RAD);
        let mut yd = -sin(pitch * DEG_TO_RAD);
        let mut zd = cos(yaw * DEG_TO_RAD) * cos(pitch * DEG_TO_RAD);
        // `Mth.sqrt`.
        let dist = (xd * xd + yd * yd + zd * zd).sqrt();
        xd *= riptide_strength / dist;
        yd *= riptide_strength / dist;
        zd *= riptide_strength / dist;
        // `push`: the player's client spins itself, so only the other viewers are sent this.
        entity.push_impulse(Vector3::new(f64::from(xd), f64::from(yd), f64::from(zd)));
        player.start_auto_spin_attack(
            Self::SPIN_ATTACK_TICKS,
            Self::SPIN_ATTACK_DAMAGE,
            stack,
            slot,
        );
        if entity.on_ground.load(Ordering::Relaxed) {
            entity.move_entity(
                player,
                Vector3::new(0.0, f64::from(Self::ON_GROUND_LIFT), 0.0),
            );
        }

        world.play_sound_from_entity(entity, sound, SoundCategory::Players, 1.0, 1.0);
    }

    fn can_mine(&self, player: &Player) -> bool {
        player.gamemode.load() != GameMode::Creative
    }

    fn get_use_duration(&self) -> i32 {
        Self::USE_DURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
