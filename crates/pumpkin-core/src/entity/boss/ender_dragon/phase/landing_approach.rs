use std::sync::Mutex;

use crossbeam::atomic::AtomicCell;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::chunk::ChunkHeightmapType;

use super::{DragonPhaseInstance, EnderDragonPhase, navigate_to_next_path_node};
use crate::entity::EntityBase;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::boss::ender_dragon::EnderDragonEntity;
use crate::entity::boss::ender_dragon::path::{DragonPath, Node};

/// Vanilla `DragonLandingApproachPhase`: flying the node graph down to the podium.
#[derive(Default)]
pub struct DragonLandingApproachPhase {
    current_path: Mutex<Option<DragonPath>>,
    target_location: AtomicCell<Option<Vector3<f64>>>,
}

impl DragonLandingApproachPhase {
    fn find_new_target(&self, dragon: &EnderDragonEntity) {
        let mut current_path = self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current_path.as_ref().is_none_or(DragonPath::is_done) {
            let current_node_index = dragon.find_closest_node();
            let egg = dragon.podium_heightmap_pos(ChunkHeightmapType::MotionBlockingNoLeaves);
            let world = dragon.get_entity().world.load();
            // `NEAR_EGG_TARGETING`: `forCombat().ignoreLineOfSight()`.
            let player_nearest_to_egg = TargetPredicate::create_attackable()
                .ignore_visibility()
                .get_nearest_player(&world, Some(dragon), egg.to_f64());
            let target_node_index = player_nearest_to_egg.map_or_else(
                || dragon.find_closest_node_at(40.0, f64::from(egg.0.y), 0.0),
                |player| {
                    let player_pos = player.get_entity().pos.load();
                    let aim = Vector3::new(player_pos.x, 0.0, player_pos.z).normalize();
                    dragon.find_closest_node_at(-aim.x * 40.0, 105.0, -aim.z * 40.0)
                },
            );

            let final_node = Node::new(egg.0.x, egg.0.y, egg.0.z);
            *current_path =
                dragon.find_path(current_node_index, target_node_index, Some(final_node));
            if let Some(path) = current_path.as_mut() {
                path.advance();
            }
        }

        navigate_to_next_path_node(dragon, current_path.as_mut(), &self.target_location);
        let path_done = current_path.as_ref().is_some_and(DragonPath::is_done);
        drop(current_path);
        if path_done {
            dragon.set_phase(EnderDragonPhase::Landing);
        }
    }
}

impl DragonPhaseInstance for DragonLandingApproachPhase {
    fn get_phase(&self) -> EnderDragonPhase {
        EnderDragonPhase::LandingApproach
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {
        *self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.target_location.store(None);
    }

    fn do_server_tick(&self, dragon: &EnderDragonEntity) {
        let dist_to_target = self.target_location.load().map_or(0.0, |target| {
            target.squared_distance_to_vec(&dragon.get_entity().pos.load())
        });
        if !(100.0..=22500.0).contains(&dist_to_target) || dragon.has_collided() {
            self.find_new_target(dragon);
        }
    }

    fn get_fly_target_location(&self) -> Option<Vector3<f64>> {
        self.target_location.load()
    }
}
