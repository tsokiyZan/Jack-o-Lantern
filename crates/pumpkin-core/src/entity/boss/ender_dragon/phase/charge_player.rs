use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

use crossbeam::atomic::AtomicCell;
use pumpkin_util::math::vector3::Vector3;

use super::{DragonPhaseInstance, EnderDragonPhase};
use crate::entity::EntityBase;
use crate::entity::boss::ender_dragon::EnderDragonEntity;

const CHARGE_RECOVERY_TIME: i32 = 10;

/// Vanilla `DragonChargePlayerPhase`: diving at where a player stood.
#[derive(Default)]
pub struct DragonChargePlayerPhase {
    target_location: AtomicCell<Option<Vector3<f64>>>,
    time_since_charge: AtomicI32,
}

impl DragonChargePlayerPhase {
    pub fn set_target(&self, target: Vector3<f64>) {
        self.target_location.store(Some(target));
    }
}

impl DragonPhaseInstance for DragonChargePlayerPhase {
    fn get_phase(&self) -> EnderDragonPhase {
        EnderDragonPhase::ChargingPlayer
    }

    fn do_server_tick(&self, dragon: &EnderDragonEntity) {
        let Some(target_location) = self.target_location.load() else {
            tracing::warn!("Aborting charge player as no target was set.");
            dragon.set_phase(EnderDragonPhase::HoldingPattern);
            return;
        };
        // `timeSinceCharge > 0 && timeSinceCharge++ >= 10`: a recovering charge still falls
        // through to the distance check below.
        if self.time_since_charge.load(Relaxed) > 0
            && self.time_since_charge.fetch_add(1, Relaxed) >= CHARGE_RECOVERY_TIME
        {
            dragon.set_phase(EnderDragonPhase::HoldingPattern);
        } else {
            let dist_to_target =
                target_location.squared_distance_to_vec(&dragon.get_entity().pos.load());
            if !(100.0..=22500.0).contains(&dist_to_target) || dragon.has_collided() {
                self.time_since_charge.fetch_add(1, Relaxed);
            }
        }
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {
        self.target_location.store(None);
        self.time_since_charge.store(0, Relaxed);
    }

    fn get_fly_speed(&self) -> f32 {
        3.0
    }

    fn get_fly_target_location(&self) -> Option<Vector3<f64>> {
        self.target_location.load()
    }
}
