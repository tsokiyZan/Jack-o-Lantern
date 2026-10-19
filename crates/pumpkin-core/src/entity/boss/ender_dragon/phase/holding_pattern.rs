use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use crossbeam::atomic::AtomicCell;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::chunk::ChunkHeightmapType;

use super::{DragonPhaseInstance, EnderDragonPhase, navigate_to_next_path_node};
use crate::entity::EntityBase;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::boss::ender_dragon::EnderDragonEntity;
use crate::entity::boss::ender_dragon::path::DragonPath;
use crate::entity::mob::Mob;

/// Vanilla `DragonHoldingPatternPhase`: circling the pillars between the other phases.
#[derive(Default)]
pub struct DragonHoldingPatternPhase {
    current_path: Mutex<Option<DragonPath>>,
    target_location: AtomicCell<Option<Vector3<f64>>>,
    clockwise: AtomicBool,
}

impl DragonHoldingPatternPhase {
    fn find_new_target(&self, dragon: &EnderDragonEntity) {
        let path_done = self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(DragonPath::is_done);
        if path_done {
            let egg = dragon.podium_heightmap_pos(ChunkHeightmapType::MotionBlockingNoLeaves);
            let crystals = dragon.alive_crystals().unwrap_or(0);
            if dragon.get_entity().random().next_bounded_i32(crystals + 3) == 0 {
                dragon.set_phase(EnderDragonPhase::LandingApproach);
                return;
            }

            let world = dragon.get_entity().world.load();
            // `NEW_TARGET_TARGETING`: `forCombat().ignoreLineOfSight()`.
            let player_nearest_to_egg = TargetPredicate::create_attackable()
                .ignore_visibility()
                .get_nearest_player(&world, Some(dragon), egg.to_f64());
            let dist_sqr = player_nearest_to_egg.as_ref().map_or(64.0, |player| {
                egg.to_centered_f64()
                    .squared_distance_to_vec(&player.get_entity().pos.load())
                    / 512.0
            });
            if let Some(player) = player_nearest_to_egg
                && {
                    // One guard for both draws: `||` keeps the first operand's temporaries.
                    let mut random = dragon.get_entity().random();
                    random.next_bounded_i32((dist_sqr + 2.0) as i32) == 0
                        || random.next_bounded_i32(crystals + 2) == 0
                }
            {
                Self::strafe_player(dragon, player);
                return;
            }
        }

        let mut current_path = self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current_path.as_ref().is_none_or(DragonPath::is_done) {
            let current_node_index = dragon.find_closest_node();
            let mut target_node_index = current_node_index as i32;
            if dragon.get_entity().random().next_bounded_i32(8) == 0 {
                self.clockwise.fetch_xor(true, Relaxed);
                target_node_index += 6;
            }
            if self.clockwise.load(Relaxed) {
                target_node_index += 1;
            } else {
                target_node_index -= 1;
            }

            // Unlike the other phases, any fight at all keeps it on the outer ring.
            if dragon
                .alive_crystals()
                .is_some_and(|crystals| crystals >= 0)
            {
                target_node_index %= 12;
                if target_node_index < 0 {
                    target_node_index += 12;
                }
            } else {
                target_node_index -= 12;
                target_node_index &= 7;
                target_node_index += 12;
            }

            *current_path = dragon.find_path(current_node_index, target_node_index as usize, None);
            if let Some(path) = current_path.as_mut() {
                path.advance();
            }
        }

        navigate_to_next_path_node(dragon, current_path.as_mut(), &self.target_location);
    }

    fn strafe_player(dragon: &EnderDragonEntity, player: Arc<dyn EntityBase>) {
        dragon.set_phase(EnderDragonPhase::StrafePlayer);
        dragon
            .phase_manager
            .strafe_player
            .set_target(dragon, player);
    }
}

impl DragonPhaseInstance for DragonHoldingPatternPhase {
    fn get_phase(&self) -> EnderDragonPhase {
        EnderDragonPhase::HoldingPattern
    }

    fn do_server_tick(&self, dragon: &EnderDragonEntity) {
        let dist_to_target = self.target_location.load().map_or(0.0, |target| {
            target.squared_distance_to_vec(&dragon.get_entity().pos.load())
        });
        if !(100.0..=22500.0).contains(&dist_to_target) || dragon.has_collided() {
            self.find_new_target(dragon);
        }
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {
        *self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.target_location.store(None);
    }

    fn get_fly_target_location(&self) -> Option<Vector3<f64>> {
        self.target_location.load()
    }

    fn on_crystal_destroyed(
        &self,
        dragon: &EnderDragonEntity,
        player: Option<Arc<dyn EntityBase>>,
    ) {
        if let Some(player) = player
            && dragon.can_attack(player.as_ref())
        {
            Self::strafe_player(dragon, player);
        }
    }
}
