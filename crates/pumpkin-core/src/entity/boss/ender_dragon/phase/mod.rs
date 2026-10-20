//! Vanilla `enderdragon.phases`: the dragon's phase instances and `EnderDragonPhaseManager`.

use std::sync::Arc;

use crossbeam::atomic::AtomicCell;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::tracked_data;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use super::EnderDragonEntity;
use super::path::DragonPath;
use crate::entity::EntityBase;

mod charge_player;
mod death;
mod holding_pattern;
mod hover;
mod landing;
mod landing_approach;
mod sitting_attacking;
mod sitting_flaming;
mod sitting_scanning;
mod strafe_player;
mod takeoff;

pub use charge_player::DragonChargePlayerPhase;
pub use death::DragonDeathPhase;
pub use holding_pattern::DragonHoldingPatternPhase;
pub use hover::DragonHoverPhase;
pub use landing::DragonLandingPhase;
pub use landing_approach::DragonLandingApproachPhase;
pub use sitting_attacking::DragonSittingAttackingPhase;
pub use sitting_flaming::DragonSittingFlamingPhase;
pub use sitting_scanning::DragonSittingScanningPhase;
pub use strafe_player::DragonStrafePlayerPhase;
pub use takeoff::DragonTakeoffPhase;

/// Vanilla `EnderDragonPhase`, numbered by its synced and saved id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnderDragonPhase {
    HoldingPattern = 0,
    StrafePlayer = 1,
    LandingApproach = 2,
    Landing = 3,
    Takeoff = 4,
    SittingFlaming = 5,
    SittingScanning = 6,
    SittingAttacking = 7,
    ChargingPlayer = 8,
    Dying = 9,
    Hover = 10,
}

impl EnderDragonPhase {
    #[must_use]
    pub const fn get_id(self) -> i32 {
        self as i32
    }

    /// Vanilla `EnderDragonPhase.getById`: an unknown id is the holding pattern.
    #[must_use]
    pub const fn get_by_id(id: i32) -> Self {
        match id {
            1 => Self::StrafePlayer,
            2 => Self::LandingApproach,
            3 => Self::Landing,
            4 => Self::Takeoff,
            5 => Self::SittingFlaming,
            6 => Self::SittingScanning,
            7 => Self::SittingAttacking,
            8 => Self::ChargingPlayer,
            9 => Self::Dying,
            10 => Self::Hover,
            _ => Self::HoldingPattern,
        }
    }
}

/// Vanilla `DragonPhaseInstance` with the `AbstractDragonPhaseInstance` defaults.
pub trait DragonPhaseInstance: Send + Sync {
    fn get_phase(&self) -> EnderDragonPhase;

    fn is_sitting(&self) -> bool {
        false
    }

    fn do_server_tick(&self, _dragon: &EnderDragonEntity) {}

    /// `player` is the one vanilla holds responsible for the crystal.
    fn on_crystal_destroyed(
        &self,
        _dragon: &EnderDragonEntity,
        _player: Option<Arc<dyn EntityBase>>,
    ) {
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {}

    fn end(&self, _dragon: &EnderDragonEntity) {}

    fn get_fly_speed(&self) -> f32 {
        0.6
    }

    fn get_turn_speed(&self, dragon: &EnderDragonEntity) -> f32 {
        let rot_speed = dragon.get_entity().velocity.load().horizontal_length() as f32 + 1.0;
        let dist = rot_speed.min(40.0);
        0.7 / dist / rot_speed
    }

    fn get_fly_target_location(&self) -> Option<Vector3<f64>> {
        None
    }

    /// `direct` is vanilla's `source.getDirectEntity()`.
    fn on_hurt(
        &self,
        _damage_type: DamageType,
        _direct: Option<&dyn EntityBase>,
        damage: f32,
    ) -> f32 {
        damage
    }
}

/// Vanilla `AbstractDragonSittingPhase.onHurt`: arrows and wind charges bounce off a sitting
/// dragon, set alight.
fn sitting_on_hurt(direct: Option<&dyn EntityBase>, damage: f32) -> f32 {
    let Some(direct) = direct else {
        return damage;
    };
    let entity_type = direct.get_entity().entity_type;
    if [
        &EntityType::ARROW,
        &EntityType::SPECTRAL_ARROW,
        &EntityType::TRIDENT,
        &EntityType::WIND_CHARGE,
    ]
    .contains(&entity_type)
    {
        direct.set_on_fire_for_ticks(20);
        0.0
    } else {
        damage
    }
}

/// The y the path phases aim for over a path node: up to 20 blocks above it.
fn random_height_above(dragon: &EnderDragonEntity, y: i32) -> f64 {
    loop {
        let y_target = f64::from(y as f32 + dragon.get_entity().random().next_f32() * 20.0);
        if y_target >= f64::from(y) {
            return y_target;
        }
    }
}

/// Vanilla `navigateToNextPathNode` of the holding pattern, strafe and landing approach
/// phases.
fn navigate_to_next_path_node(
    dragon: &EnderDragonEntity,
    current_path: Option<&mut DragonPath>,
    target_location: &AtomicCell<Option<Vector3<f64>>>,
) {
    if let Some(path) = current_path
        && !path.is_done()
    {
        let current = path.get_next_node_pos();
        path.advance();
        let y_target = random_height_above(dragon, current.y);
        target_location.store(Some(Vector3::new(
            f64::from(current.x),
            y_target,
            f64::from(current.z),
        )));
    }
}

/// Vanilla `EnderDragonPhaseManager`, holding one instance of each phase.
pub struct EnderDragonPhaseManager {
    current_phase: AtomicCell<EnderDragonPhase>,
    pub holding_pattern: DragonHoldingPatternPhase,
    pub strafe_player: DragonStrafePlayerPhase,
    pub landing_approach: DragonLandingApproachPhase,
    pub landing: DragonLandingPhase,
    pub takeoff: DragonTakeoffPhase,
    pub sitting_flaming: DragonSittingFlamingPhase,
    pub sitting_scanning: DragonSittingScanningPhase,
    pub sitting_attacking: DragonSittingAttackingPhase,
    pub charging_player: DragonChargePlayerPhase,
    pub dying: DragonDeathPhase,
    pub hover: DragonHoverPhase,
}

impl Default for EnderDragonPhaseManager {
    fn default() -> Self {
        Self::new()
    }
}

impl EnderDragonPhaseManager {
    /// Starts hovering, like vanilla's constructor; that phase's `begin` has nothing to reset
    /// on a fresh instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_phase: AtomicCell::new(EnderDragonPhase::Hover),
            holding_pattern: DragonHoldingPatternPhase::default(),
            strafe_player: DragonStrafePlayerPhase::default(),
            landing_approach: DragonLandingApproachPhase::default(),
            landing: DragonLandingPhase::default(),
            takeoff: DragonTakeoffPhase::default(),
            sitting_flaming: DragonSittingFlamingPhase::default(),
            sitting_scanning: DragonSittingScanningPhase::default(),
            sitting_attacking: DragonSittingAttackingPhase::default(),
            charging_player: DragonChargePlayerPhase::default(),
            dying: DragonDeathPhase::default(),
            hover: DragonHoverPhase::default(),
        }
    }

    pub fn set_phase(&self, dragon: &EnderDragonEntity, target: EnderDragonPhase) {
        let current = self.current_phase.load();
        if target == current {
            return;
        }
        self.get_phase(current).end(dragon);
        self.current_phase.store(target);
        dragon
            .get_entity()
            .set_synced_data(tracked_data::ender_dragon::DATA_PHASE, target.get_id());
        tracing::debug!("Dragon is now in phase {target:?} on the server");
        self.get_phase(target).begin(dragon);
    }

    #[must_use]
    pub fn get_current_phase(&self) -> &dyn DragonPhaseInstance {
        self.get_phase(self.current_phase.load())
    }

    #[must_use]
    pub fn get_phase(&self, phase: EnderDragonPhase) -> &dyn DragonPhaseInstance {
        match phase {
            EnderDragonPhase::HoldingPattern => &self.holding_pattern,
            EnderDragonPhase::StrafePlayer => &self.strafe_player,
            EnderDragonPhase::LandingApproach => &self.landing_approach,
            EnderDragonPhase::Landing => &self.landing,
            EnderDragonPhase::Takeoff => &self.takeoff,
            EnderDragonPhase::SittingFlaming => &self.sitting_flaming,
            EnderDragonPhase::SittingScanning => &self.sitting_scanning,
            EnderDragonPhase::SittingAttacking => &self.sitting_attacking,
            EnderDragonPhase::ChargingPlayer => &self.charging_player,
            EnderDragonPhase::Dying => &self.dying,
            EnderDragonPhase::Hover => &self.hover,
        }
    }
}
