use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::sync::Arc;

use crate::entity::player::Player;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::sound::Sound;
use pumpkin_util::math::vector3::Vector3;

use crate::entity::Entity;
use crate::entity::EntityBase;
use crate::entity::projectile::ThrownItemEntity;
use crate::entity::projectile::wind_charge::{WIND_CHARGE_GRAVITY, WindChargeEntity};
use crate::item::{ItemBehaviour, ItemMetadata};

pub struct WindChargeItem;

impl ItemMetadata for WindChargeItem {
    fn ids() -> Box<[u16]> {
        [Item::WIND_CHARGE.id].into()
    }
}

const POWER: f32 = 1.5;

impl ItemBehaviour for WindChargeItem {
    fn normal_use(&self, item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let world = player.world();
        let position = player.position();

        // `new WindCharge(player, level, x, eyeY, z)`: at the eyes, not the thrown-item height.
        let spawn_pos = Vector3::new(position.x, player.eye_position().y, position.z);
        let entity = Entity::new(world.clone(), spawn_pos, &EntityType::WIND_CHARGE);
        let wind_charge = ThrownItemEntity::unowned(entity, WIND_CHARGE_GRAVITY);
        wind_charge.projectile.set_owner(Some(player.get_entity()));
        let (yaw, pitch) = player.rotation();
        wind_charge.set_velocity_from(player, pitch, yaw, 0.0, POWER, 1.0);

        world.spawn_entity(Arc::new(WindChargeEntity::new_normal(wind_charge)));

        let sound_pitch = 0.4 / (world.random_f32() * 0.4 + 0.8);
        world.play_sound_fine(
            Sound::EntityWindChargeThrow,
            pumpkin_data::sound::SoundCategory::Neutral,
            &position,
            0.5,
            sound_pitch,
        );

        player.award_item_used(item);
        crate::item::consume_in_hand(player, hand);
        InteractionResult::SUCCESS
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
