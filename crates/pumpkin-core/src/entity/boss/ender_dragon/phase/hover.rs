use crossbeam::atomic::AtomicCell;
use pumpkin_util::math::vector3::Vector3;

use super::{DragonPhaseInstance, EnderDragonPhase};
use crate::entity::EntityBase;
use crate::entity::boss::ender_dragon::EnderDragonEntity;

/// Vanilla `DragonHoverPhase`: holding still where it started, the phase a summoned dragon
/// begins in.
#[derive(Default)]
pub struct DragonHoverPhase {
    target_location: AtomicCell<Option<Vector3<f64>>>,
}

impl DragonPhaseInstance for DragonHoverPhase {
    fn get_phase(&self) -> EnderDragonPhase {
        EnderDragonPhase::Hover
    }

    fn do_server_tick(&self, dragon: &EnderDragonEntity) {
        if self.target_location.load().is_none() {
            self.target_location
                .store(Some(dragon.get_entity().pos.load()));
        }
    }

    fn is_sitting(&self) -> bool {
        true
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {
        self.target_location.store(None);
    }

    fn get_fly_speed(&self) -> f32 {
        1.0
    }

    fn get_fly_target_location(&self) -> Option<Vector3<f64>> {
        self.target_location.load()
    }
}
