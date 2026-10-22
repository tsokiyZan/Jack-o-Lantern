use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::{Controls, Goal};
use crate::block::blocks::doors::DoorBlock;
use crate::entity::ai::pathfinder::path::Path;
use crate::entity::ai::util::{default_random_pos, land_random_pos};
use crate::entity::mob::Mob;

/// How many visited POIs the goal remembers.
const MAX_VISITED: usize = 15;
/// Vanilla's radius for the POI search around a candidate spot.
const POI_SEARCH_RADIUS: i32 = 10;

/// Vanilla `MoveThroughVillageGoal`: near a village (by night, if `only_at_night`), walk
/// from one occupied village POI to the next, stopping short of wooden doors the mob
/// cannot deal with.
pub struct MoveThroughVillageGoal {
    speed_modifier: f64,
    path: Option<Path>,
    poi_pos: BlockPos,
    only_at_night: bool,
    visited: Vec<BlockPos>,
    distance_to_poi: i32,
    can_deal_with_doors: fn(&dyn Mob) -> bool,
}

impl MoveThroughVillageGoal {
    #[must_use]
    pub fn new(
        speed_modifier: f64,
        only_at_night: bool,
        distance_to_poi: i32,
        can_deal_with_doors: fn(&dyn Mob) -> bool,
    ) -> Box<Self> {
        Box::new(Self {
            speed_modifier,
            path: None,
            poi_pos: BlockPos::new(0, 0, 0),
            only_at_night,
            visited: Vec::new(),
            distance_to_poi,
            can_deal_with_doors,
        })
    }

    fn has_not_visited(&self, poi: &BlockPos) -> bool {
        !self.visited.contains(poi)
    }

    fn update_visited(&mut self) {
        if self.visited.len() > MAX_VISITED {
            self.visited.remove(0);
        }
    }

    /// Vanilla `Vec3i.closerToCenterThan`.
    fn closer_to_center_than(pos: &BlockPos, position: Vector3<f64>, distance: f64) -> bool {
        let center = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + 0.5,
            f64::from(pos.0.z) + 0.5,
        );
        center.squared_distance_to_vec(&position) < distance * distance
    }

    /// Vanilla `createPath` with `setCanOpenDoors(canDealWithDoors)` around it, which leaves
    /// the navigation able to open doors afterwards.
    fn create_path_with_doors(&self, mob: &dyn Mob, destination: Vector3<f64>) -> Option<Path> {
        let mob_entity = mob.get_mob_entity();
        let mut navigator = mob_entity.own_navigation();
        navigator.set_can_open_doors((self.can_deal_with_doors)(mob));
        let path = navigator.create_path(&mob_entity.living_entity, destination, 0);
        navigator.set_can_open_doors(true);
        path
    }
}

fn block_corner(pos: BlockPos) -> Vector3<f64> {
    Vector3::new(f64::from(pos.0.x), f64::from(pos.0.y), f64::from(pos.0.z))
}

impl Goal for MoveThroughVillageGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let mob_entity = mob.get_mob_entity();
        if !mob_entity.own_navigation().can_navigate_ground() {
            return false;
        }

        self.update_visited();
        let entity = mob.get_entity();
        let world = entity.world.load_full();
        if self.only_at_night && world.is_bright_outside() {
            return false;
        }

        let pos = entity.block_pos.load();
        if !world.is_close_to_village(&pos, 6) {
            return false;
        }

        let find_poi = |around: BlockPos| {
            world
                .villager_poi
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .find_occupied_village_poi(around, POI_SEARCH_RADIUS, |poi| {
                    self.has_not_visited(poi)
                })
        };
        let land_pos = land_random_pos::get_pos_scored(mob, 15, 7, |candidate| {
            if !world.is_village(candidate) {
                return f64::NEG_INFINITY;
            }
            find_poi(*candidate).map_or(f64::NEG_INFINITY, |poi| {
                let delta = poi.0 - pos.0;
                -(f64::from(delta.x).powi(2)
                    + f64::from(delta.y).powi(2)
                    + f64::from(delta.z).powi(2))
            })
        });
        let Some(land_pos) = land_pos else {
            return false;
        };

        let Some(target) = find_poi(BlockPos::floored_v(land_pos)) else {
            return false;
        };

        self.poi_pos = target;
        self.path = self.create_path_with_doors(mob, block_corner(self.poi_pos));
        if self.path.is_none() {
            // `Vec3.atBottomCenterOf`.
            let poi_bottom_center = Vector3::new(
                f64::from(self.poi_pos.0.x) + 0.5,
                f64::from(self.poi_pos.0.y),
                f64::from(self.poi_pos.0.z) + 0.5,
            );
            let Some(partial_step) = default_random_pos::get_pos_towards(
                mob,
                10,
                7,
                poi_bottom_center,
                f64::from(std::f32::consts::FRAC_PI_2),
            ) else {
                return false;
            };
            self.path = self.create_path_with_doors(mob, partial_step);
            if self.path.is_none() {
                return false;
            }
        }

        let door_node = self.path.as_ref().and_then(|path| {
            path.get_nodes()
                .iter()
                .find(|node| DoorBlock::is_wooden_door(&world, &node.pos.up()))
                .map(|node| node.pos)
        });
        if let Some(node_pos) = door_node {
            self.path = mob_entity.own_navigation().create_path(
                &mob_entity.living_entity,
                block_corner(node_pos),
                0,
            );
        }

        self.path.is_some()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let mob_entity = mob.get_mob_entity();
        if mob_entity.own_navigation().is_done() {
            return false;
        }
        let entity = &mob_entity.living_entity.entity;
        !Self::closer_to_center_than(
            &self.poi_pos,
            entity.pos.load(),
            f64::from(entity.width() + self.distance_to_poi as f32),
        )
    }

    fn start(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        mob_entity.own_navigation().move_to_path(
            self.path.take(),
            self.speed_modifier,
            &mob_entity.living_entity,
        );
    }

    fn stop(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        let is_done = mob_entity.own_navigation().is_done();
        if is_done
            || Self::closer_to_center_than(
                &self.poi_pos,
                mob_entity.living_entity.entity.pos.load(),
                f64::from(self.distance_to_poi),
            )
        {
            self.visited.push(self.poi_pos);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}
