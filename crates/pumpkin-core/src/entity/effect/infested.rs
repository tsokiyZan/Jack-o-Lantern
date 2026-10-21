use uuid::Uuid;

use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;
use crate::entity::r#type::from_type;

pub struct InfestedMobEffect;

impl MobEffect for InfestedMobEffect {
    fn on_mob_hurt(
        &self,
        living: &LivingEntity,
        _amplifier: u8,
        _damage_type: &DamageType,
        _damage_amount: f32,
    ) {
        // Immune types never get the effect (`#immune_to_infested` in `canBeAffected`).
        let world = living.entity.world.load();

        // 10% chance to spawn
        // Vanilla `mob.getRandom().nextFloat() <= chanceToSpawn`, then
        // `Mth.randomBetweenInclusive(mob.getRandom(), 1, 2)`.
        let count = {
            let mut random = living.entity.random();
            if random.next_f32() <= 0.1 {
                random.next_bounded_i32(2) + 1
            } else {
                0
            }
        };
        if count > 0 {
            // `mob.getY() + mob.getBbHeight() / 2.0`.
            let pos = living.entity.pos.load();
            let center = Vector3::new(
                pos.x,
                pos.y + f64::from(living.entity.height()) / 2.0,
                pos.z,
            );

            let rot = living.entity.rotation();
            let vx = rot.x * 0.3;
            let vy = rot.y * 0.45;
            let vz = rot.z * 0.3;

            for _ in 0..count {
                // `Mth.randomBetween(random, (float) (-Math.PI / 2), (float) (Math.PI / 2))`.
                let half_pi = std::f64::consts::FRAC_PI_2 as f32;
                let random_angle =
                    living.entity.random().next_f32() * (half_pi - -half_pi) + -half_pi;
                let cos_a = random_angle.cos();
                let sin_a = random_angle.sin();
                let rx = vx * cos_a + vz * sin_a;
                let rz = -vx * sin_a + vz * cos_a;

                let silver = from_type(&EntityType::SILVERFISH, center, &world, Uuid::new_v4());
                let silver_entity = silver.get_entity();
                // `snapTo(x, y, z, level.getRandom().nextFloat() * 360.0F, 0.0F)`.
                let yaw = world
                    .random
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .next_f32()
                    * 360.0;
                silver_entity.set_pos(center);
                silver_entity.set_rotation(yaw, 0.0);
                silver_entity.velocity.store(Vector3::new(
                    f64::from(rx),
                    f64::from(vy),
                    f64::from(rz),
                ));

                world.spawn_entity(silver);
                // `silverfish.playSound(SILVERFISH_HURT)`, once per silverfish.
                world.play_sound(Sound::EntitySilverfishHurt, SoundCategory::Hostile, &center);
            }
        }
    }
}
