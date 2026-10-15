use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::world::WorldEvent;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use super::{DragonPhaseInstance, EnderDragonPhase, navigate_to_next_path_node};
use crate::entity::boss::ender_dragon::path::{DragonPath, Node};
use crate::entity::boss::ender_dragon::{EnderDragonEntity, get_y};
use crate::entity::mob::Mob;
use crate::entity::projectile::dragon_fireball::DragonFireballEntity;
use crate::entity::{Entity, EntityBase};
use pumpkin_data::entity::EntityType;

const FIREBALL_CHARGE_AMOUNT: i32 = 5;

/// Vanilla `DragonStrafePlayerPhase`: flying at a player to spit a fireball.
#[derive(Default)]
pub struct DragonStrafePlayerPhase {
    fireball_charge: AtomicI32,
    current_path: Mutex<Option<DragonPath>>,
    target_location: AtomicCell<Option<Vector3<f64>>>,
    attack_target: Mutex<Option<Arc<dyn EntityBase>>>,
    holding_pattern_clockwise: AtomicBool,
}

/// How far above a strafed target the dragon aims, more the further off it is.
fn height_offset(dist: f64) -> f64 {
    (f64::from(0.4f32) + dist / 80.0 - 1.0).min(10.0)
}

impl DragonStrafePlayerPhase {
    fn find_new_target(&self, dragon: &EnderDragonEntity) {
        let mut current_path = self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current_path.as_ref().is_none_or(DragonPath::is_done) {
            let current_node_index = dragon.find_closest_node();
            let mut target_node_index = current_node_index as i32;
            if dragon.get_random().random_range(0..8) == 0 {
                self.holding_pattern_clockwise.fetch_xor(true, Relaxed);
                target_node_index += 6;
            }
            if self.holding_pattern_clockwise.load(Relaxed) {
                target_node_index += 1;
            } else {
                target_node_index -= 1;
            }

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

            *current_path = dragon.find_path(current_node_index, target_node_index as usize, None);
            if let Some(path) = current_path.as_mut() {
                path.advance();
            }
        }

        navigate_to_next_path_node(dragon, current_path.as_mut(), &self.target_location);
    }

    /// Vanilla `setTarget`, called right after switching to this phase.
    pub fn set_target(&self, dragon: &EnderDragonEntity, target: Arc<dyn EntityBase>) {
        let target_pos = target.get_entity().pos.load();
        *self
            .attack_target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(target);
        let current_node_index = dragon.find_closest_node();
        let target_node_index =
            dragon.find_closest_node_at(target_pos.x, target_pos.y, target_pos.z);
        let final_x_target = target_pos.x.floor() as i32;
        let final_z_target = target_pos.z.floor() as i32;
        let pos = dragon.get_entity().pos.load();
        let xd = f64::from(final_x_target) - pos.x;
        let zd = f64::from(final_z_target) - pos.z;
        #[expect(
            clippy::imprecise_flops,
            reason = "vanilla rounds Math.sqrt(x * x + z * z), not a hypot"
        )]
        let sd = (xd * xd + zd * zd).sqrt();
        let final_y_target = (target_pos.y + height_offset(sd)).floor() as i32;
        let final_node = Node::new(final_x_target, final_y_target, final_z_target);
        let mut current_path = self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *current_path = dragon.find_path(current_node_index, target_node_index, Some(final_node));
        if let Some(path) = current_path.as_mut() {
            path.advance();
            navigate_to_next_path_node(dragon, Some(path), &self.target_location);
        }
    }

    /// The fireball part of `doServerTick`, once the dragon faces its target.
    fn shoot_fireball(&self, dragon: &EnderDragonEntity, target: &dyn EntityBase) {
        let entity = dragon.get_entity();
        let view_vector = Entity::calculate_view_vector(entity.pitch.load(), entity.yaw.load());
        let head = dragon.head().get_entity();
        let head_pos = head.pos.load();
        let starting_x = head_pos.x - view_vector.x;
        let starting_y = get_y(head, 0.5) + 0.5;
        let starting_z = head_pos.z - view_vector.z;
        let target_entity = target.get_entity();
        let target_pos = target_entity.pos.load();
        let direction = Vector3::new(
            target_pos.x - starting_x,
            get_y(target_entity, 0.5) - starting_y,
            target_pos.z - starting_z,
        );
        let world = entity.world.load_full();
        if !entity.is_silent() {
            world.sync_world_event(WorldEvent::SoundDragonFireball, entity.block_pos.load(), 0);
        }

        let fireball_entity = Entity::new(
            world.clone(),
            Vector3::new(starting_x, starting_y, starting_z),
            &EntityType::DRAGON_FIREBALL,
        );
        let fireball =
            DragonFireballEntity::new_shot(fireball_entity, entity, direction.normalize());
        world.spawn_entity(Arc::new(fireball));
        self.fireball_charge.store(0, Relaxed);
        if let Some(path) = self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            while !path.is_done() {
                path.advance();
            }
        }
        dragon.set_phase(EnderDragonPhase::HoldingPattern);
    }
}

impl DragonPhaseInstance for DragonStrafePlayerPhase {
    fn get_phase(&self) -> EnderDragonPhase {
        EnderDragonPhase::StrafePlayer
    }

    fn do_server_tick(&self, dragon: &EnderDragonEntity) {
        let attack_target = self
            .attack_target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(attack_target) = attack_target else {
            tracing::warn!("Skipping player strafe phase because no player was found");
            dragon.set_phase(EnderDragonPhase::HoldingPattern);
            return;
        };

        let pos = dragon.get_entity().pos.load();
        let target_pos = attack_target.get_entity().pos.load();
        let path_done = self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(DragonPath::is_done);
        if path_done {
            let x_target_dist = target_pos.x - pos.x;
            let z_target_dist = target_pos.z - pos.z;
            #[expect(
                clippy::imprecise_flops,
                reason = "vanilla rounds Math.sqrt(x * x + z * z), not a hypot"
            )]
            let dist = (x_target_dist * x_target_dist + z_target_dist * z_target_dist).sqrt();
            self.target_location.store(Some(Vector3::new(
                target_pos.x,
                target_pos.y + height_offset(dist),
                target_pos.z,
            )));
        }

        let dist_to_target = self
            .target_location
            .load()
            .map_or(0.0, |target| target.squared_distance_to_vec(&pos));
        if !(100.0..=22500.0).contains(&dist_to_target) {
            self.find_new_target(dragon);
        }

        if target_pos.squared_distance_to_vec(&pos) < 4096.0
            && dragon
                .get_entity()
                .has_line_of_sight(attack_target.get_entity())
        {
            let charge = self.fireball_charge.fetch_add(1, Relaxed) + 1;
            let aim = Vector3::new(target_pos.x - pos.x, 0.0, target_pos.z - pos.z).normalize();
            let yaw = dragon.get_entity().yaw.load() * (std::f64::consts::PI / 180.0) as f32;
            let dir = Vector3::new(
                f64::from(pumpkin_util::math::sin(yaw)),
                0.0,
                f64::from(-pumpkin_util::math::cos(yaw)),
            )
            .normalize();
            let dot = dir.dot(&aim) as f32;
            let mut angle_degs =
                (f64::from(dot).acos() * 180.0 / f64::from(std::f32::consts::PI)) as f32;
            angle_degs += 0.5;
            if charge >= FIREBALL_CHARGE_AMOUNT && (0.0..10.0).contains(&angle_degs) {
                self.shoot_fireball(dragon, attack_target.as_ref());
            }
        } else if self.fireball_charge.load(Relaxed) > 0 {
            self.fireball_charge.fetch_sub(1, Relaxed);
        }
    }

    fn begin(&self, _dragon: &EnderDragonEntity) {
        self.fireball_charge.store(0, Relaxed);
        self.target_location.store(None);
        *self
            .current_path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        *self
            .attack_target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    fn get_fly_target_location(&self) -> Option<Vector3<f64>> {
        self.target_location.load()
    }
}
