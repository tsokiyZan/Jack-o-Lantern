use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

use pumpkin_data::damage::DamageType;
use pumpkin_util::math::vector3::Vector3;

use super::{DragonPhaseInstance, EnderDragonPhase, sitting_on_hurt};
use crate::entity::EntityBase;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::boss::ender_dragon::EnderDragonEntity;

const SITTING_SCANNING_IDLE_TICKS: i32 = 100;
const SITTING_ATTACK_Y_VIEW_RANGE: f64 = 10.0;
const SITTING_ATTACK_VIEW_RANGE: f64 = 20.0;
const SITTING_CHARGE_VIEW_RANGE: f64 = 150.0;

/// Vanilla `DragonSittingScanningPhase`: turning on the podium towards a nearby player.
#[derive(Default)]
pub struct DragonSittingScanningPhase {
    scanning_time: AtomicI32,
}

impl DragonPhaseInstance for DragonSittingScanningPhase {
    fn get_phase(&self) -> EnderDragonPhase {
        EnderDragonPhase::SittingScanning
    }

    fn is_sitting(&self) -> bool {
        true
    }

    fn do_server_tick(&self, dragon: &EnderDragonEntity) {
        let scanning_time = self.scanning_time.fetch_add(1, Relaxed) + 1;
        let entity = dragon.get_entity();
        let world = entity.world.load();
        let pos = entity.pos.load();
        // `scanTargeting`: `forCombat().range(20)` within 10 blocks of the dragon's height.
        let mut scan_targeting =
            TargetPredicate::create_attackable().set_base_max_distance(SITTING_ATTACK_VIEW_RANGE);
        let dragon_y = pos.y;
        scan_targeting.set_predicate(move |target, _world| {
            (target.entity.pos.load().y - dragon_y).abs() <= SITTING_ATTACK_Y_VIEW_RANGE
        });
        if let Some(attack_target) = scan_targeting.get_nearest_player(&world, Some(dragon), pos) {
            if scanning_time > 25 {
                dragon.set_phase(EnderDragonPhase::SittingAttacking);
            } else {
                let target_pos = attack_target.get_entity().pos.load();
                let aim = Vector3::new(target_pos.x - pos.x, 0.0, target_pos.z - pos.z).normalize();
                let yaw = entity.yaw.load();
                let yaw_rad = yaw * (std::f64::consts::PI / 180.0) as f32;
                let dir = Vector3::new(
                    f64::from(pumpkin_util::math::sin(yaw_rad)),
                    0.0,
                    f64::from(-pumpkin_util::math::cos(yaw_rad)),
                )
                .normalize();
                let dot = dir.dot(&aim) as f32;
                let angle =
                    (f64::from(dot).acos() * 180.0 / f64::from(std::f32::consts::PI)) as f32 + 0.5;
                if angle < 0.0 || angle > 10.0 {
                    let head_pos = dragon.head().get_entity().pos.load();
                    let x_attack_dist = target_pos.x - head_pos.x;
                    let z_attack_dist = target_pos.z - head_pos.z;
                    let y_rot_delta = pumpkin_util::math::wrap_degrees_f64(
                        180.0
                            - pumpkin_util::math::atan2(x_attack_dist, z_attack_dist) * 180.0
                                / f64::from(std::f32::consts::PI)
                            - f64::from(yaw),
                    )
                    .clamp(-100.0, 100.0);
                    let mut y_rot_a = dragon.y_rot_a.load() * 0.8;
                    #[expect(
                        clippy::imprecise_flops,
                        reason = "vanilla rounds Math.sqrt(x * x + z * z), not a hypot"
                    )]
                    let mut dist = (x_attack_dist * x_attack_dist + z_attack_dist * z_attack_dist)
                        .sqrt() as f32
                        + 1.0;
                    let rot_speed = dist;
                    if dist > 40.0 {
                        dist = 40.0;
                    }
                    y_rot_a += y_rot_delta as f32 * (0.7 / dist / rot_speed);
                    dragon.y_rot_a.store(y_rot_a);
                    entity.yaw.store(yaw + y_rot_a);
                }
            }
        } else if scanning_time >= SITTING_SCANNING_IDLE_TICKS {
            // `CHARGE_TARGETING`: `forCombat().range(150)`.
            let attack_target = TargetPredicate::create_attackable()
                .set_base_max_distance(SITTING_CHARGE_VIEW_RANGE)
                .get_nearest_player(&world, Some(dragon), pos);
            dragon.set_phase(EnderDragonPhase::Takeoff);
            if let Some(attack_target) = attack_target {
                dragon.set_phase(EnderDragonPhase::ChargingPlayer);
                dragon
                    .phase_manager
                    .charging_player
                    .set_target(attack_target.get_entity().pos.load());
            }
        }
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {
        self.scanning_time.store(0, Relaxed);
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
