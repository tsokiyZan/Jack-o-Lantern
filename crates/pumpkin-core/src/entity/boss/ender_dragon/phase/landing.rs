use crossbeam::atomic::AtomicCell;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::chunk::ChunkHeightmapType;

use super::{DragonPhaseInstance, EnderDragonPhase};
use crate::entity::EntityBase;
use crate::entity::boss::ender_dragon::EnderDragonEntity;

/// Vanilla `DragonLandingPhase`: the last stretch down onto the podium.
#[derive(Default)]
pub struct DragonLandingPhase {
    target_location: AtomicCell<Option<Vector3<f64>>>,
}

impl DragonPhaseInstance for DragonLandingPhase {
    fn get_phase(&self) -> EnderDragonPhase {
        EnderDragonPhase::Landing
    }

    fn do_server_tick(&self, dragon: &EnderDragonEntity) {
        let target_location = self.target_location.load().unwrap_or_else(|| {
            // `Vec3.atBottomCenterOf`.
            let egg = dragon.podium_heightmap_pos(ChunkHeightmapType::MotionBlockingNoLeaves);
            let target = Vector3::new(
                f64::from(egg.0.x) + 0.5,
                f64::from(egg.0.y),
                f64::from(egg.0.z) + 0.5,
            );
            self.target_location.store(Some(target));
            target
        });

        if target_location.squared_distance_to_vec(&dragon.get_entity().pos.load()) < 1.0 {
            dragon.phase_manager.sitting_flaming.reset_flame_count();
            dragon.set_phase(EnderDragonPhase::SittingScanning);
        }
    }

    fn get_fly_speed(&self) -> f32 {
        1.5
    }

    fn get_turn_speed(&self, dragon: &EnderDragonEntity) -> f32 {
        let rot_speed = dragon.get_entity().velocity.load().horizontal_length() as f32 + 1.0;
        let dist = rot_speed.min(40.0);
        dist / rot_speed
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {
        self.target_location.store(None);
    }

    fn get_fly_target_location(&self) -> Option<Vector3<f64>> {
        self.target_location.load()
    }
}
