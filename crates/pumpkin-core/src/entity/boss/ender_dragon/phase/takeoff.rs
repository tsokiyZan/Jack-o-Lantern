use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use crossbeam::atomic::AtomicCell;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::chunk::ChunkHeightmapType;

use super::{DragonPhaseInstance, EnderDragonPhase, random_height_above};
use crate::entity::EntityBase;
use crate::entity::boss::ender_dragon::EnderDragonEntity;
use crate::entity::boss::ender_dragon::path::DragonPath;

/// Vanilla `DragonTakeoffPhase`: climbing off the podium back into the node graph.
#[derive(Default)]
pub struct DragonTakeoffPhase {
    first_tick: AtomicBool,
    current_path: Mutex<Option<DragonPath>>,
    target_location: AtomicCell<Option<Vector3<f64>>>,
}

impl DragonTakeoffPhase {
    fn find_new_target(&self, dragon: &EnderDragonEntity) {
        let current_node_index = dragon.find_closest_node();
        let look_vector = dragon.get_head_look_vector();
        let mut target_node_index =
            dragon.find_closest_node_at(-look_vector.x * 40.0, 105.0, -look_vector.z * 40.0) as i32;
        if dragon.alive_crystals().is_some_and(|crystals| crystals > 0) {
            target_node_index %= 12;
            if target_node_index < 0 {
                target_node_index += 12;
            }
        } else {
            target_node_index -= 12;
            target_node_index &= 7;
            target_node_index += 12;
        }

        let mut current_path = self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *current_path = dragon.find_path(current_node_index, target_node_index as usize, None);
        self.navigate_to_next_path_node(dragon, current_path.as_mut());
    }

    /// Unlike the other path phases, this one skips the node it starts from here.
    fn navigate_to_next_path_node(
        &self,
        dragon: &EnderDragonEntity,
        current_path: Option<&mut DragonPath>,
    ) {
        if let Some(path) = current_path {
            path.advance();
            if !path.is_done() {
                let current = path.get_next_node_pos();
                path.advance();
                let y_target = random_height_above(dragon, current.y);
                self.target_location.store(Some(Vector3::new(
                    f64::from(current.x),
                    y_target,
                    f64::from(current.z),
                )));
            }
        }
    }
}

impl DragonPhaseInstance for DragonTakeoffPhase {
    fn get_phase(&self) -> EnderDragonPhase {
        EnderDragonPhase::Takeoff
    }

    fn do_server_tick(&self, dragon: &EnderDragonEntity) {
        let has_path = self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some();
        if !self.first_tick.load(Relaxed) && has_path {
            let egg = dragon.podium_heightmap_pos(ChunkHeightmapType::MotionBlockingNoLeaves);
            // `BlockPos.closerToCenterThan(position, 10.0)`.
            if egg
                .to_centered_f64()
                .squared_distance_to_vec(&dragon.get_entity().pos.load())
                >= 100.0
            {
                dragon.set_phase(EnderDragonPhase::HoldingPattern);
            }
        } else {
            self.first_tick.store(false, Relaxed);
            self.find_new_target(dragon);
        }
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {
        self.first_tick.store(true, Relaxed);
        *self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.target_location.store(None);
    }

    fn get_fly_target_location(&self) -> Option<Vector3<f64>> {
        self.target_location.load()
    }
}
