use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::entity::Entity;
use crate::entity::player::Player;
use crate::item::items::projectile_weapon::ProjectileWeaponItem;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::ChargedProjectilesImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::GameMode;
use pumpkin_util::math::DEG_TO_RAD;
use pumpkin_util::math::quaternion::Quaternionf;
use pumpkin_util::math::vector3::Vector3;

pub struct CrossbowItem;

impl ItemMetadata for CrossbowItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::CROSSBOW.id])
    }
}

impl ItemBehaviour for CrossbowItem {
    /// Vanilla `CrossbowItem.use`.
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let stack = player.inventory().get_stack_in_hand(hand);

        // Every crossbow carries a ChargedProjectiles component by default, so its mere
        // presence does not mean the crossbow is loaded. Vanilla checks the list is also
        // non-empty (CrossbowItem.java:68).
        if stack
            .get_data_component::<ChargedProjectilesImpl>()
            .is_some_and(|charged| !charged.projectiles.is_empty())
        {
            Self::fire_projectiles(player, hand);
            return InteractionResult::CONSUME;
        }

        let has_arrows = player.find_arrow().is_some();
        if !has_arrows && player.gamemode.load() != GameMode::Creative {
            return InteractionResult::FAIL;
        }

        crate::item::start_using_item(player, hand, stack, 72000);
        InteractionResult::CONSUME
    }

    /// Vanilla `CrossbowItem.releaseUsing` only reports whether it was charged; loading
    /// happens in `on_use_tick`.
    fn on_stopped_using(&self, _stack: &ItemStack, player: &Player) {
        player.living_entity.clear_active_hand();
    }

    /// Vanilla `CrossbowItem.onUseTick`: the charging sounds at 20% and 50% of the charge, and
    /// the load at 100%.
    fn on_use_tick(&self, stack: &ItemStack, player: &Player, remaining_use_ticks: i32) {
        let charge_duration =
            crate::enchantment::EnchantmentHelper::modify_crossbow_charge_time(stack, 25);
        let tick_percent = (72000 - remaining_use_ticks) as f32 / charge_duration as f32;
        let (start, mid, end) = Self::charging_sounds(stack);
        let world = player.world();
        let pos = player.position();
        if tick_percent < 0.2 {
            START_SOUND_PLAYED.store(false, Ordering::Relaxed);
            MID_LOAD_SOUND_PLAYED.store(false, Ordering::Relaxed);
        }
        if tick_percent >= 0.2 && !START_SOUND_PLAYED.swap(true, Ordering::Relaxed) {
            world.play_sound_fine(start, SoundCategory::Players, &pos, 0.5, 1.0);
        }
        // The flag is set even when quick charge has no middle sound.
        if tick_percent >= 0.5
            && !MID_LOAD_SOUND_PLAYED.swap(true, Ordering::Relaxed)
            && let Some(mid) = mid
        {
            world.play_sound_fine(mid, SoundCategory::Players, &pos, 0.5, 1.0);
        }
        if tick_percent >= 1.0 && !Self::is_charged(stack) && Self::try_load_projectiles(player) {
            let pitch = 1.0 / (world.random_f32() * 0.5 + 1.0) + 0.2;
            world.play_sound_fine(end, SoundCategory::Players, &pos, 1.0, pitch);
        }
    }

    fn get_use_duration(&self, _stack: &ItemStack) -> i32 {
        72000
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Vanilla `CrossbowItem.startSoundPlayed` and `midLoadSoundPlayed`: fields of the one
/// crossbow item, so every user shares them.
static START_SOUND_PLAYED: AtomicBool = AtomicBool::new(false);
static MID_LOAD_SOUND_PLAYED: AtomicBool = AtomicBool::new(false);

impl CrossbowItem {
    /// Vanilla `CrossbowItem.isCharged`.
    fn is_charged(stack: &ItemStack) -> bool {
        stack
            .get_data_component::<ChargedProjectilesImpl>()
            .is_some_and(|charged| !charged.projectiles.is_empty())
    }

    /// Vanilla `CrossbowItem.getChargingSounds`: the quick charge enchantment's
    /// `crossbow_charging_sounds` (a start sound per level, no middle one), or the default.
    fn charging_sounds(stack: &ItemStack) -> (Sound, Option<Sound>, Sound) {
        match stack.get_enchantment_level(&pumpkin_data::Enchantment::QUICK_CHARGE) {
            1 => (
                Sound::ItemCrossbowQuickCharge1,
                None,
                Sound::ItemCrossbowLoadingEnd,
            ),
            2 => (
                Sound::ItemCrossbowQuickCharge2,
                None,
                Sound::ItemCrossbowLoadingEnd,
            ),
            level if level >= 3 => (
                Sound::ItemCrossbowQuickCharge3,
                None,
                Sound::ItemCrossbowLoadingEnd,
            ),
            _ => (
                Sound::ItemCrossbowLoadingStart,
                Some(Sound::ItemCrossbowLoadingMiddle),
                Sound::ItemCrossbowLoadingEnd,
            ),
        }
    }

    /// Vanilla `CrossbowItem.tryLoadProjectiles`: `draw` the player's projectile into
    /// `CHARGED_PROJECTILES`.
    fn try_load_projectiles(player: &Player) -> bool {
        let hand = player.living_entity.used_item_hand();
        let mut stack = player.inventory().get_stack_in_hand(hand);
        let arrow_slot = player.find_arrow();
        let is_creative = player.gamemode.load() == GameMode::Creative;
        if arrow_slot.is_none() && !is_creative {
            return false;
        }
        let projectile = arrow_slot.map_or_else(
            || ItemStack::new(1, &Item::ARROW),
            |slot| player.inventory().get_slot(slot).copy_with_count(1),
        );
        let drawn = ProjectileWeaponItem::draw(&player.world(), &stack, &projectile, is_creative);
        if drawn.is_empty() {
            return false;
        }
        // The ammo `useAmmo` took (the first shot; multishot's extras are always free).
        let took_ammo = drawn[0]
            .get_data_component::<pumpkin_data::data_component_impl::IntangibleProjectileImpl>()
            .is_none();
        let charged_nbts = drawn
            .iter()
            .map(|item| {
                let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
                item.write_item_stack(&mut nbt);
                nbt
            })
            .collect();
        stack
            .patch
            .retain(|(id, _)| *id != DataComponent::ChargedProjectiles);
        stack.patch.push((
            DataComponent::ChargedProjectiles,
            Some(Box::new(ChargedProjectilesImpl {
                projectiles: charged_nbts,
            })),
        ));
        player.inventory().set_stack_in_hand(hand, stack);
        if let Some(slot) = arrow_slot
            && took_ammo
        {
            player.consume_arrow(slot);
        }
        true
    }
    /// Vanilla `CrossbowItem.shootProjectile`'s shot without a target: the view vector turned
    /// by `angle` degrees about the up vector, in float.
    #[must_use]
    pub fn projectile_shot_vector(x_rot: f32, y_rot: f32, angle: f32) -> Vector3<f64> {
        // `getUpVector`.
        let up = Entity::calculate_view_vector(x_rot - 90.0, y_rot);
        let up_quaternion =
            Quaternionf::from_angle_axis(f64::from(angle * DEG_TO_RAD), up.x, up.y, up.z);
        let view = Entity::calculate_view_vector(x_rot, y_rot);
        let shot =
            up_quaternion.transform_f32(Vector3::new(view.x as f32, view.y as f32, view.z as f32));
        Vector3::new(f64::from(shot.x), f64::from(shot.y), f64::from(shot.z))
    }

    pub const ARROW_POWER: f32 = 3.15;

    fn fire_projectiles(player: &Player, hand: Hand) {
        let mut held = player.inventory().get_stack_in_hand(hand);
        let charged_opt = held.get_data_component::<ChargedProjectilesImpl>().cloned();

        if let Some(charged) = charged_opt {
            let mut projectiles = Vec::new();
            for projectile_nbt in charged.projectiles {
                if let Some(projectile) = ItemStack::read_item_stack(&projectile_nbt) {
                    projectiles.push(projectile);
                }
            }

            if !projectiles.is_empty() {
                let world = player.world();
                world.play_sound(
                    Sound::ItemCrossbowShoot,
                    SoundCategory::Players,
                    &player.position(),
                );

                ProjectileWeaponItem::shoot_projectiles(
                    &world,
                    player,
                    &held,
                    &projectiles,
                    Self::ARROW_POWER,
                    1.0,
                    // `shooter instanceof Player`.
                    true,
                    false,
                );

                // `CrossbowItem.getDurabilityUse`: 3 for a firework, 1 otherwise, per shot.
                let durability: i32 = projectiles
                    .iter()
                    .map(|projectile| {
                        if projectile.item == &Item::FIREWORK_ROCKET {
                            3
                        } else {
                            1
                        }
                    })
                    .sum();
                held.patch
                    .retain(|(id, _)| *id != DataComponent::ChargedProjectiles);
                player.inventory().set_stack_in_hand(hand, held);
                player.damage_item_in_hand(hand, durability);
                player.award_item_used(&Item::CROSSBOW);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bits from vanilla's `Mth` view vectors and JOML 1.10.9 `Vector3f.rotate(new
    /// Quaternionf().setAngleAxis(...))`.
    #[test]
    fn multishot_side_arrows_turn_about_the_up_vector() {
        let cases: [(f32, f32, f32, [u32; 3]); 4] = [
            (0.0, 0.0, 10.0, [0x3e31_d0d3, 0x0000_0000, 0x3f7c_1c5d]),
            (
                -35.5,
                123.25,
                -10.0,
                [0xbf13_4826, 0x3f12_63e5, 0xbf15_b172],
            ),
            (80.0, -45.0, 10.0, [0x3e79_987e, 0xbf78_4745, 0xbaef_51c0]),
            (-89.9, 170.5, 10.0, [0xbe2f_a99a, 0x3f7c_1c41, 0x3cdd_2483]),
        ];
        for (x_rot, y_rot, angle, bits) in cases {
            let shot = CrossbowItem::projectile_shot_vector(x_rot, y_rot, angle);
            assert_eq!(
                [
                    (shot.x as f32).to_bits(),
                    (shot.y as f32).to_bits(),
                    (shot.z as f32).to_bits()
                ],
                bits,
                "{x_rot} {y_rot} {angle}"
            );
        }
    }
}
