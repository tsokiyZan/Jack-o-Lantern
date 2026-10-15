use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

use pumpkin_data::damage::DamageType;

use super::{DragonPhaseInstance, EnderDragonPhase, sitting_on_hurt};
use crate::entity::EntityBase;
use crate::entity::boss::ender_dragon::EnderDragonEntity;

const ROAR_DURATION: i32 = 40;

/// Vanilla `DragonSittingAttackingPhase`: roaring at a player before breathing on them. The
/// roar sound is the client's.
#[derive(Default)]
pub struct DragonSittingAttackingPhase {
    attacking_ticks: AtomicI32,
}

impl DragonPhaseInstance for DragonSittingAttackingPhase {
    fn get_phase(&self) -> EnderDragonPhase {
        EnderDragonPhase::SittingAttacking
    }

    fn is_sitting(&self) -> bool {
        true
    }

    fn do_server_tick(&self, dragon: &EnderDragonEntity) {
        if self.attacking_ticks.fetch_add(1, Relaxed) >= ROAR_DURATION {
            dragon.set_phase(EnderDragonPhase::SittingFlaming);
        }
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {
        self.attacking_ticks.store(0, Relaxed);
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
