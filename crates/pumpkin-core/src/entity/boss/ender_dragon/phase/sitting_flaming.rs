use std::sync::atomic::{AtomicI32, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::particle::Particle;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::{DragonPhaseInstance, EnderDragonPhase, sitting_on_hurt};
use crate::entity::area_effect_cloud::AreaEffectCloudEntity;
use crate::entity::boss::ender_dragon::{EnderDragonEntity, get_y};
use crate::entity::{Entity, EntityBase};

const FLAME_DURATION: i32 = 200;
const SITTING_FLAME_ATTACKS_COUNT: i32 = 4;
const WARMUP_TIME: i32 = 10;

/// Vanilla `DragonSittingFlamingPhase`: breathing a cloud of dragon's breath in front of the
/// podium. The breath particles from the head are the client's.
#[derive(Default)]
pub struct DragonSittingFlamingPhase {
    flame_ticks: AtomicI32,
    flame_count: AtomicI32,
    flame: Mutex<Option<Arc<dyn EntityBase>>>,
}

impl DragonSittingFlamingPhase {
    pub fn reset_flame_count(&self) {
        self.flame_count.store(0, Relaxed);
    }

    fn spawn_flame(&self, dragon: &EnderDragonEntity) {
        let entity = dragon.get_entity();
        let pos = entity.pos.load();
        let head = dragon.head().get_entity();
        let head_pos = head.pos.load();
        let look = Vector3::new(head_pos.x - pos.x, 0.0, head_pos.z - pos.z).normalize();
        let x = head_pos.x + look.x * 5.0 / 2.0;
        let z = head_pos.z + look.z * 5.0 / 2.0;
        let initial_y = get_y(head, 0.5);
        let mut y = initial_y;
        let world = entity.world.load_full();
        let mut block_pos = BlockPos::floored(x, y, z);
        while world.get_block_state(&block_pos).is_air() {
            y -= 1.0;
            if y < 0.0 {
                y = initial_y;
                break;
            }
            block_pos = BlockPos::floored(x, y, z);
        }
        y = f64::from(y.floor() as i32 + 1);

        let cloud = AreaEffectCloudEntity::with_vanilla_defaults(Entity::new(
            world.clone(),
            Vector3::new(x, y, z),
            &EntityType::AREA_EFFECT_CLOUD,
        ));
        // TODO: vanilla `setOwner(dragon)` and `setPotionDurationScale(0.25F)`; Pumpkin's
        // cloud applies its effects without an owner and with a fixed duration scale.
        cloud.set_radius(5.0);
        *cloud
            .duration
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = FLAME_DURATION;
        cloud.set_custom_particle(Particle::DragonBreath, Box::new(1.0f32.to_be_bytes()));
        cloud
            .effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((&StatusEffect::INSTANT_DAMAGE, 0, 0, false, true, true));
        let flame: Arc<dyn EntityBase> = Arc::new(cloud);
        *self
            .flame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(flame.clone());
        world.spawn_entity(flame);
    }
}

impl DragonPhaseInstance for DragonSittingFlamingPhase {
    fn get_phase(&self) -> EnderDragonPhase {
        EnderDragonPhase::SittingFlaming
    }

    fn is_sitting(&self) -> bool {
        true
    }

    fn do_server_tick(&self, dragon: &EnderDragonEntity) {
        let flame_ticks = self.flame_ticks.fetch_add(1, Relaxed) + 1;
        if flame_ticks >= FLAME_DURATION {
            if self.flame_count.load(Relaxed) >= SITTING_FLAME_ATTACKS_COUNT {
                dragon.set_phase(EnderDragonPhase::Takeoff);
            } else {
                dragon.set_phase(EnderDragonPhase::SittingScanning);
            }
        } else if flame_ticks == WARMUP_TIME {
            self.spawn_flame(dragon);
        }
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {
        self.flame_ticks.store(0, Relaxed);
        self.flame_count.fetch_add(1, Relaxed);
    }

    fn end(&self, _dragon: &EnderDragonEntity) {
        if let Some(flame) = self
            .flame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            flame.get_entity().remove();
        }
    }

    fn on_hurt(
        &self,
        _damage_type: DamageType,
        direct: Option<&dyn EntityBase>,
        damage: f32,
    ) -> f32 {
        sitting_on_hurt(direct, damage)
    }
}
