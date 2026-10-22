use std::any::Any;
use std::sync::atomic::Ordering;

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
    fn normal_use(&self, _item: &Item, player: &Player) {
        let inventory = player.inventory();
        let stack = inventory.held_item();

        // Every crossbow carries a ChargedProjectiles component by default, so its mere
        // presence does not mean the crossbow is loaded. Vanilla checks the list is also
        // non-empty (CrossbowItem.java:68).
        if stack
            .get_data_component::<ChargedProjectilesImpl>()
            .is_some_and(|charged| !charged.projectiles.is_empty())
        {
            Self::fire_projectiles(player);
            return;
        }

        let has_arrows = player.find_arrow().is_some();
        if !has_arrows && player.gamemode.load() != GameMode::Creative {
            return;
        }

        player
            .living_entity
            .set_active_hand(pumpkin_util::Hand::Right, stack, 72000);
    }

    fn on_stopped_using(&self, _stack: &ItemStack, player: &Player) {
        let use_ticks = player.living_entity.item_use_time.load(Ordering::Relaxed);
        let use_ticks = 72000 - use_ticks;

        let mut stack = player.inventory().held_item();
        let charge_time =
            crate::enchantment::EnchantmentHelper::modify_crossbow_charge_time(&stack, 25);

        if use_ticks >= charge_time {
            let arrow_slot = player.find_arrow();
            let gamemode = player.gamemode.load();
            let is_creative = gamemode == GameMode::Creative;

            if arrow_slot.is_some() || is_creative {
                let projectile = arrow_slot.map_or_else(
                    || ItemStack::new(1, &Item::ARROW),
                    |slot| {
                        let inventory = player.inventory();
                        inventory.get_slot(slot).copy_with_count(1)
                    },
                );

                let drawn =
                    ProjectileWeaponItem::draw(&player.world(), &stack, &projectile, is_creative);
                if !drawn.is_empty() {
                    let mut charged_nbts = Vec::new();
                    for item in drawn {
                        let mut arrow_nbt = pumpkin_nbt::compound::NbtCompound::new();
                        item.write_item_stack(&mut arrow_nbt);
                        charged_nbts.push(arrow_nbt);
                    }

                    stack.patch.push((
                        DataComponent::ChargedProjectiles,
                        Some(Box::new(ChargedProjectilesImpl {
                            projectiles: charged_nbts,
                        })),
                    ));
                    player.inventory().set_held_item(stack);

                    if let Some(slot) = arrow_slot
                        && !is_creative
                    {
                        player.consume_arrow(slot);
                    }

                    player.world().play_sound(
                        Sound::ItemCrossbowLoadingEnd,
                        SoundCategory::Players,
                        &player.position(),
                    );
                }
            }
        }
        player.living_entity.clear_active_hand();
    }

    fn get_use_duration(&self) -> i32 {
        72000
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CrossbowItem {
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

    fn fire_projectiles(player: &Player) {
        let mut held = player.inventory().held_item();
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

                let is_creative = player.gamemode.load() == GameMode::Creative;
                ProjectileWeaponItem::shoot_projectiles(
                    &world,
                    player,
                    &held,
                    &projectiles,
                    Self::ARROW_POWER,
                    1.0,
                    false,
                    is_creative,
                    false,
                );

                held.patch
                    .retain(|(id, _)| *id != DataComponent::ChargedProjectiles);
                player.inventory().set_held_item(held);
                player.damage_held_item(1);
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
