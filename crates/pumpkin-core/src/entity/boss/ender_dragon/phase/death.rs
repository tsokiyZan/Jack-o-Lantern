use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

use crossbeam::atomic::AtomicCell;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::chunk::ChunkHeightmapType;

use super::{DragonPhaseInstance, EnderDragonPhase};
use crate::entity::EntityBase;
use crate::entity::boss::ender_dragon::EnderDragonEntity;

/// Vanilla `DragonDeathPhase`: gliding to the podium on 1 health, then dropping to 0 there to
/// start `tickDeath`. The explosion particles are the client's.
#[derive(Default)]
pub struct DragonDeathPhase {
    target_location: AtomicCell<Option<Vector3<f64>>>,
    time: AtomicI32,
}

impl DragonPhaseInstance for DragonDeathPhase {
    fn get_phase(&self) -> EnderDragonPhase {
        EnderDragonPhase::Dying
    }

    fn do_server_tick(&self, dragon: &EnderDragonEntity) {
        self.time.fetch_add(1, Relaxed);
        let target_location = self.target_location.load().unwrap_or_else(|| {
            // `Vec3.atBottomCenterOf`.
            let egg = dragon.podium_heightmap_pos(ChunkHeightmapType::MotionBlocking);
            let target = Vector3::new(
                f64::from(egg.0.x) + 0.5,
                f64::from(egg.0.y),
                f64::from(egg.0.z) + 0.5,
            );
            self.target_location.store(Some(target));
            target
        });

        let dist_to_target =
            target_location.squared_distance_to_vec(&dragon.get_entity().pos.load());
        let living = &dragon.mob_entity.living_entity;
        if (100.0..=22500.0).contains(&dist_to_target) && !dragon.has_collided() {
            living.set_health(1.0);
        } else {
            living.set_health(0.0);
        }
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {
        self.target_location.store(None);
        self.time.store(0, Relaxed);
    }

    fn get_fly_speed(&self) -> f32 {
        3.0
    }

    fn get_fly_target_location(&self) -> Option<Vector3<f64>> {
        self.target_location.load()
    }
}
