use uuid::Uuid;

use pumpkin_data::entity::EntityType;
use pumpkin_data::game_rules::{GameRule, GameRuleValue};
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;
use crate::entity::mob::cube_mob::CubeMobEntity;
use crate::entity::r#type::from_type;

/// Vanilla `MobEffects.OOZING`'s `spawnedCount`.
const SLIMES_REQUESTED: i32 = 2;
/// Vanilla `OozingMobEffect.SLIME_SIZE`.
const SLIME_SIZE: i32 = 2;
/// Vanilla `OozingMobEffect.RADIUS_TO_CHECK_SLIMES`.
const RADIUS_TO_CHECK_SLIMES: f64 = 2.0;

pub struct OozingMobEffect;

impl MobEffect for OozingMobEffect {
    /// Vanilla `OozingMobEffect.onMobRemoved`.
    fn on_mob_death(&self, living: &LivingEntity, _amplifier: u8) {
        let entity = &living.entity;
        let world = entity.world.load_full();
        let max_entity_cramming = match world.get_game_rule(&GameRule::MaxEntityCramming) {
            GameRuleValue::Int(value) => value as i32,
            GameRuleValue::Bool(_) => 0,
        };
        // `numberOfSlimesToSpawn` with `NearbySlimes.closeTo`.
        let count = if max_entity_cramming < 1 {
            SLIMES_REQUESTED
        } else {
            let area = entity
                .bounding_box
                .load()
                .expand_all(RADIUS_TO_CHECK_SLIMES);
            let nearby = world
                .get_entities_at_box(&area)
                .iter()
                .filter(|other| {
                    other.get_entity().entity_type == &EntityType::SLIME
                        && other.get_entity().entity_id != entity.entity_id
                })
                .take(max_entity_cramming as usize)
                .count() as i32;
            (max_entity_cramming - nearby).max(0).min(SLIMES_REQUESTED)
        };
        let pos = entity.pos.load();
        for _ in 0..count {
            // `spawnSlimeOffspring`.
            let slime = from_type(&EntityType::SLIME, pos, &world, Uuid::new_v4());
            if let Some(cube) = slime.cast_any().downcast_ref::<CubeMobEntity>() {
                cube.set_size(SLIME_SIZE, true);
            }
            let slime_entity = slime.get_entity();
            slime_entity.set_pos(Vector3::new(pos.x, pos.y + 0.5, pos.z));
            slime_entity.set_rotation(rand::rng().random::<f32>() * 360.0, 0.0);
            world.spawn_entity(slime);
        }
    }
}
