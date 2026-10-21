use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;

/// Vanilla `SaturationMobEffect`, an instantaneous effect.
pub struct SaturationMobEffect;

impl MobEffect for SaturationMobEffect {
    fn is_instantaneous(&self) -> bool {
        true
    }

    fn should_apply_effect_tick(&self, tick_count: i32, _amplifier: u8) -> bool {
        tick_count >= 1
    }

    fn apply_effect_tick(&self, living: &LivingEntity, amplifier: u8) -> bool {
        let world = living.entity.world.load();
        if let Some(entity) = world.get_entity_by_id(living.entity.entity_id)
            && let Some(player) = entity.get_player()
        {
            // `FoodData.eat(amplification + 1, 1.0F)`: the food level caps at 20, so a larger
            // amount adds the same.
            let food = i32::from(amplifier) + 1;
            player.hunger_manager.eat(
                player,
                food.min(i32::from(u8::MAX)) as u8,
                food as f32 * 2.0,
            );
        }
        true
    }
}
