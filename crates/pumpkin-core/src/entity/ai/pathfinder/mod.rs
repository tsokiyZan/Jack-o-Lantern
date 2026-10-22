use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::wrap_degrees;

use crate::entity::Entity;
use crate::entity::living::LivingEntity;
use crate::entity::mob::Mob;
use crate::world::World;

use crate::entity::ai::pathfinder::amphibious_node_evaluator::AmphibiousNodeEvaluator;
use crate::entity::ai::pathfinder::binary_heap::BinaryHeap;
use crate::entity::ai::pathfinder::fly_node_evaluator::FlyNodeEvaluator;
use crate::entity::ai::pathfinder::node::{Node, PathType, Target};
use crate::entity::ai::pathfinder::node_evaluator::{BaseNodeEvaluator, MobData, NodeEvaluator};
use crate::entity::ai::pathfinder::path::Path;
use crate::entity::ai::pathfinder::pathfinding_context::PathfindingContext;
use crate::entity::ai::pathfinder::swim_node_evaluator::SwimNodeEvaluator;
use crate::entity::ai::pathfinder::walk_node_evaluator::WalkNodeEvaluator;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Taggable};
use rustc_hash::FxHashMap;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub mod amphibious_node_evaluator;
pub mod binary_heap;
pub mod fly_node_evaluator;
pub mod node;
pub mod node_evaluator;
pub mod path;
pub mod path_type_cache;
pub mod pathfinding_context;
pub mod swim_node_evaluator;
pub mod walk_node_evaluator;

/// Vanilla `PathFinder.FUDGING`: how much more the heuristic weighs than the cost so far.
const FUDGING: f32 = 1.5;

#[derive(Clone, Copy, Debug)]
pub struct NavigatorGoal {
    pub current_progress: Vector3<f64>,
    pub destination: Vector3<f64>,
    pub speed: f64,
    /// How close, in Manhattan distance, the path has to get to `destination`.
    pub reach_range: i32,
}

impl NavigatorGoal {
    /// Vanilla `PathNavigation.moveTo`'s reach when a caller passes none.
    pub const DEFAULT_REACH_RANGE: i32 = 1;

    #[must_use]
    pub const fn new(
        current_progress: Vector3<f64>,
        destination: Vector3<f64>,
        speed: f64,
    ) -> Self {
        Self {
            current_progress,
            destination,
            speed,
            reach_range: Self::DEFAULT_REACH_RANGE,
        }
    }

    /// Asks for a path that ends within `reach_range` of the destination, like the
    /// `reachRange` argument of vanilla's `moveTo` and `createPath`.
    #[must_use]
    pub const fn with_reach_range(mut self, reach_range: i32) -> Self {
        self.reach_range = reach_range;
        self
    }
}

pub enum EvaluatorKind {
    Walk(WalkNodeEvaluator),
    Fly(FlyNodeEvaluator),
    Swim(SwimNodeEvaluator),
    Amphibious(AmphibiousNodeEvaluator),
}

impl EvaluatorKind {
    pub fn prepare(&mut self, context: PathfindingContext, mob_data: MobData) {
        match self {
            Self::Walk(e) => e.prepare(context, mob_data),
            Self::Fly(e) => e.prepare(context, mob_data),
            Self::Swim(e) => e.prepare(context, mob_data),
            Self::Amphibious(e) => e.prepare(context, mob_data),
        }
    }

    pub fn done(&mut self) {
        match self {
            Self::Walk(e) => e.done(),
            Self::Fly(e) => e.done(),
            Self::Swim(e) => e.done(),
            Self::Amphibious(e) => e.done(),
        }
    }

    pub fn get_start(&mut self) -> Option<Node> {
        match self {
            Self::Walk(e) => e.get_start(),
            Self::Fly(e) => e.get_start(),
            Self::Swim(e) => e.get_start(),
            Self::Amphibious(e) => e.get_start(),
        }
    }

    pub fn get_target(&mut self, pos: BlockPos) -> Target {
        match self {
            Self::Walk(e) => e.get_target(pos),
            Self::Fly(e) => e.get_target(pos),
            Self::Swim(e) => e.get_target(pos),
            Self::Amphibious(e) => e.get_target(pos),
        }
    }

    /// The evaluator's shared node map and settings.
    pub const fn base_mut(&mut self) -> &mut BaseNodeEvaluator {
        match self {
            Self::Walk(e) => &mut e.base,
            Self::Fly(e) => &mut e.walk.base,
            Self::Swim(e) => &mut e.base,
            Self::Amphibious(e) => &mut e.walk.base,
        }
    }

    pub fn get_neighbors(&mut self, current: &Node, out: &mut Vec<Node>) {
        match self {
            Self::Walk(e) => e.get_neighbors(current, out),
            Self::Fly(e) => e.get_neighbors(current, out),
            Self::Swim(e) => e.get_neighbors(current, out),
            Self::Amphibious(e) => e.get_neighbors(current, out),
        }
    }

    pub fn set_can_float(&mut self, can_float: bool) {
        match self {
            Self::Walk(e) => e.set_can_float(can_float),
            Self::Fly(e) => e.set_can_float(can_float),
            Self::Swim(e) => e.set_can_float(can_float),
            Self::Amphibious(e) => e.set_can_float(can_float),
        }
    }

    pub fn set_can_open_doors(&mut self, can_open: bool) {
        match self {
            Self::Walk(e) => e.set_can_open_doors(can_open),
            Self::Fly(e) => e.set_can_open_doors(can_open),
            Self::Swim(e) => e.set_can_open_doors(can_open),
            Self::Amphibious(e) => e.set_can_open_doors(can_open),
        }
    }

    pub fn set_can_pass_doors(&mut self, can_pass: bool) {
        match self {
            Self::Walk(e) => e.set_can_pass_doors(can_pass),
            Self::Fly(e) => e.set_can_pass_doors(can_pass),
            Self::Swim(e) => e.set_can_pass_doors(can_pass),
            Self::Amphibious(e) => e.set_can_pass_doors(can_pass),
        }
    }

    pub fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        match self {
            Self::Walk(e) => e.set_can_walk_over_fences(can_walk),
            Self::Fly(e) => e.set_can_walk_over_fences(can_walk),
            Self::Swim(e) => e.set_can_walk_over_fences(can_walk),
            Self::Amphibious(e) => e.set_can_walk_over_fences(can_walk),
        }
    }
}

pub trait PathNavigationTrait: Send + Sync {
    fn set_progress(&mut self, goal: NavigatorGoal);
    fn set_speed(&mut self, speed: f64);
    fn stop(&mut self);
    fn is_idle(&self) -> bool;
    fn is_done(&self) -> bool;
    fn is_in_progress(&self) -> bool;
    fn is_stuck(&self) -> bool;
    fn get_path(&self) -> Option<&Path>;
    fn get_path_mut(&mut self) -> Option<&mut Path>;
    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32);
    fn get_pathfinding_malus(&self, path_type: PathType) -> f32;
    /// The mob's own `pathfindingMalus` map.
    fn path_type_overrides(&self) -> &FxHashMap<PathType, f32>;
    fn set_mob_dimensions(&mut self, width: f32, height: f32);
    fn can_reach_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool;
    fn tick(&mut self, entity: &LivingEntity);
    fn move_to_coords(&mut self, x: f64, y: f64, z: f64, speed: f64, entity: &LivingEntity)
    -> bool;
    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool;
    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, entity: &LivingEntity) -> bool;
    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool;
    fn create_path(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path>;
    /// Vanilla `PathNavigation.createPath(pos, reachRange, maxPathLength)`: `create_path`
    /// with a path length cap in place of the follow range.
    fn create_path_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
        max_path_length: f32,
    ) -> Option<Path>;
    fn recompute_path(&mut self, entity: &LivingEntity);
    fn set_avoid_sun(&mut self, avoid_sun: bool);
    fn set_can_walk_over_fences(&mut self, can_walk: bool);
    fn set_can_open_doors(&mut self, can_open: bool);
    fn set_can_pass_doors(&mut self, can_pass: bool);
    fn set_can_float(&mut self, can_float: bool);
    fn can_float(&self) -> bool;
    fn can_navigate_ground(&self) -> bool;
    fn set_required_path_length(&mut self, length: f32, entity: &LivingEntity);
    /// Vanilla `PathNavigation.updatePathfinderMaxVisitedNodes`, which `Mob` runs
    /// whenever its follow range changes.
    fn update_pathfinder_max_visited_nodes(&mut self, entity: &LivingEntity);
    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32);
    fn reset_max_visited_nodes_multiplier(&mut self);
    fn get_target_pos(&self) -> Option<BlockPos>;
    fn can_path_to_targets_below_surface(&self) -> bool;
    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool);

    /// Whether the mob can settle at `pos`, which random targets are checked
    /// against: vanilla's `PathNavigation.isStableDestination`.
    fn is_stable_destination(&self, world: &World, pos: &BlockPos) -> bool {
        world.get_block_state(&pos.down()).is_solid_render()
    }

    /// The waypoint and speed this tick's navigation wants the mob's move control
    /// to head for, for navigators that leave the steering to it as vanilla's
    /// `PathNavigation.tick` does with `moveControl.setWantedPosition`.
    fn take_wanted_position(&mut self) -> Option<(Vector3<f64>, f64)> {
        None
    }

    /// Vanilla `NodeEvaluator.getPathType(mob, pos)`, which `MoveControl.isWalkable` checks
    /// strafes against. Walking evaluators classify with `WalkNodeEvaluator.getPathTypeStatic`.
    fn get_path_type_at(&mut self, entity: &Entity, pos: BlockPos) -> Option<PathType> {
        let mut context =
            PathfindingContext::new(entity.block_pos.load().0, entity.world.load_full());
        Some(context.get_land_node_type(pos.0))
    }

    /// Whether this tick's navigation asked the mob to jump, which vanilla's
    /// `MoveControl` passes on to the `JumpControl`.
    fn take_jump(&mut self) -> bool {
        false
    }
}

pub struct PathNavigation {
    pub current_goal: Option<NavigatorGoal>,
    pub evaluator: EvaluatorKind,
    pub path: Option<Path>,
    pub speed_modifier: f64,
    pub tick_count: u32,
    pub last_stuck_check: u32,
    pub last_stuck_check_pos: Vector3<f64>,
    pub timeout_cached_node: Vector3<i32>,
    pub timeout_timer: u64,
    pub last_timeout_check: u64,
    pub timeout_limit: f64,
    pub max_distance_to_waypoint: f32,
    pub has_delayed_recomputation: bool,
    pub time_last_recompute: u64,
    pub target_pos: Option<BlockPos>,
    pub reach_range: i32,
    pub max_visited_nodes_multiplier: f32,
    /// Vanilla `PathFinder.maxVisitedNodes`, set from the follow range base value
    /// on the first search since the navigation has no mob when it is built.
    pub max_visited_nodes: Option<i32>,
    pub is_stuck: bool,
    pub required_path_length: f32,
    pub ticks_on_current_node: u32,
    pub last_node_index: usize,
    pub total_ticks: u32,
    pub path_start_pos: Option<Vector3<f64>>,
    pub path_type_overrides: FxHashMap<PathType, f32>,
    pub mob_width: f32,
    pub mob_height: f32,
    pub repath_cooldown: u32,
    pub can_float: bool,
    pub can_walk_over_fences: bool,
    pub can_open_doors: bool,
    pub can_pass_doors: bool,
    pub avoid_sun: bool,
    pub can_path_to_targets_below_surface: bool,
    pub open_set: BinaryHeap,
    pub neighbors_buf: Vec<Node>,
    pub is_idle: AtomicBool,
    /// A jump this tick's steering asked for, taken by [`PathNavigationTrait::take_jump`].
    pub jump: bool,
    /// A goal's `moveTo` this tick, resolved at the start of the navigation tick (ground).
    pub pending_move: Option<NavigatorGoal>,
    /// This tick's `moveControl.setWantedPosition`, taken by the mob's AI step (ground).
    pub wanted_position: Option<(Vector3<f64>, f64)>,
    /// Vanilla `FlyingPathNavigation`'s overrides of the base navigation.
    pub flying: bool,
    /// The `blockedByFluids` of a flying navigation's `canMoveDirectly`.
    pub direct_moves_blocked_by_fluids: bool,
}

impl Default for PathNavigation {
    fn default() -> Self {
        Self::new(EvaluatorKind::Walk(WalkNodeEvaluator::default()))
    }
}

impl PathNavigation {
    #[must_use]
    pub fn new(evaluator: EvaluatorKind) -> Self {
        Self {
            current_goal: None,
            evaluator,
            path: None,
            speed_modifier: 1.0,
            tick_count: 0,
            last_stuck_check: 0,
            last_stuck_check_pos: Vector3::new(0.0, 0.0, 0.0),
            timeout_cached_node: Vector3::new(0, 0, 0),
            timeout_timer: 0,
            last_timeout_check: 0,
            timeout_limit: 0.0,
            max_distance_to_waypoint: 0.5,
            has_delayed_recomputation: false,
            time_last_recompute: 0,
            target_pos: None,
            reach_range: 1,
            max_visited_nodes_multiplier: 1.0,
            max_visited_nodes: None,
            is_stuck: false,
            required_path_length: 16.0,
            ticks_on_current_node: 0,
            last_node_index: 0,
            total_ticks: 0,
            path_start_pos: None,
            path_type_overrides: FxHashMap::default(),
            mob_width: 0.6,
            mob_height: 1.95,
            repath_cooldown: 0,
            can_float: false,
            can_walk_over_fences: false,
            can_open_doors: false,
            can_pass_doors: true,
            avoid_sun: false,
            can_path_to_targets_below_surface: false,
            open_set: BinaryHeap::new(),
            neighbors_buf: Vec::new(),
            is_idle: AtomicBool::new(true),
            jump: false,
            pending_move: None,
            wanted_position: None,
            flying: false,
            direct_moves_blocked_by_fluids: true,
        }
    }

    pub fn set_progress(&mut self, goal: NavigatorGoal) {
        // Vanilla `PathNavigation.createPath` hands back the current path while it
        // is unfinished and still leads to the requested block, so a goal that
        // re-requests a still target keeps its first path instead of re-planning.
        let keep_path = self.path.as_ref().is_some_and(|path| {
            !path.is_done() && path.get_target() == BlockPos::floored_v(goal.destination)
        });
        self.is_idle.store(false, Ordering::Relaxed);
        self.speed_modifier = goal.speed;
        self.last_stuck_check = self.tick_count;
        self.last_stuck_check_pos = goal.current_progress;
        if !keep_path {
            self.reach_range = goal.reach_range;
        }
        self.current_goal = Some(goal);
        if !keep_path {
            self.path = None;
            self.reset_stuck_timeout();
        }
    }

    pub fn adopt_path(
        &mut self,
        path: Path,
        speed: f64,
        entity: &LivingEntity,
        stuck_check_pos: Vector3<f64>,
    ) -> bool {
        if !path.same_as(self.path.as_ref()) {
            self.path = Some(path);
        }
        if self.path.as_ref().is_none_or(Path::is_done) {
            return false;
        }
        self.trim_path(entity);
        let Some(target) = self
            .path
            .as_ref()
            .filter(|path| path.get_node_count() > 0)
            .map(Path::get_target)
        else {
            return false;
        };
        // The tick idles without a goal, so install the one matching this path
        self.current_goal = Some(NavigatorGoal::new(
            entity.entity.pos.load(),
            Vector3::new(
                f64::from(target.0.x) + 0.5,
                f64::from(target.0.y),
                f64::from(target.0.z) + 0.5,
            ),
            speed,
        ));
        self.speed_modifier = speed;
        self.last_stuck_check = self.tick_count;
        self.last_stuck_check_pos = stuck_check_pos;
        self.reset_stuck_timeout();
        self.ticks_on_current_node = 0;
        self.last_node_index = 0;
        self.path_start_pos = Some(entity.entity.pos.load());
        self.is_idle.store(false, Ordering::Relaxed);
        true
    }

    pub fn clear_path(&mut self) {
        self.path = None;
        self.current_goal = None;
        self.is_idle.store(true, Ordering::Relaxed);
    }

    pub const fn set_speed(&mut self, speed: f64) {
        self.speed_modifier = speed;
        if let Some(goal) = &mut self.current_goal {
            goal.speed = speed;
        }
    }

    pub fn stop(&mut self) {
        self.is_idle.store(true, Ordering::Relaxed);
        self.current_goal = None;
        self.path = None;
        self.ticks_on_current_node = 0;
        self.total_ticks = 0;
        self.path_start_pos = None;
    }

    pub fn finish_navigation(&mut self, entity: &LivingEntity) {
        self.stop();
        entity.movement_input.store(Vector3::new(0.0, 0.0, 0.0));
    }

    pub fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.path_type_overrides.insert(path_type, malus);
    }

    #[must_use]
    pub fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.path_type_overrides
            .get(&path_type)
            .copied()
            .unwrap_or_else(|| path_type.get_malus())
    }

    pub const fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.mob_width = width;
        self.mob_height = height;
    }

    pub fn can_reach_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.compute_path(entity, destination, 1)
            .is_some_and(|path| path.can_reach() || path.get_dist_to_target() <= distance)
    }

    /// Vanilla `PathNavigation.getMaxPathLength`.
    fn mob_max_follow_range(&self, entity: &LivingEntity) -> f32 {
        let follow_range = entity.get_attribute_value(&Attributes::FOLLOW_RANGE) as f32;
        follow_range.max(self.required_path_length)
    }

    /// Vanilla `PathNavigation.setRequiredPathLength`.
    pub fn set_required_path_length(&mut self, length: f32, entity: &LivingEntity) {
        self.required_path_length = length;
        self.update_pathfinder_max_visited_nodes(entity);
    }

    /// Vanilla `PathNavigation.updatePathfinderMaxVisitedNodes`.
    pub fn update_pathfinder_max_visited_nodes(&mut self, entity: &LivingEntity) {
        self.max_visited_nodes = Some((self.mob_max_follow_range(entity) * 16.0).floor() as i32);
    }

    /// What the node evaluators read from `entity`, vanilla's `Mob` getters.
    fn mob_data(&self, entity: &LivingEntity, world: &World) -> MobData {
        let mob = world.get_entity_by_id(entity.entity.entity_id);
        let mut mob_data = MobData::new(
            entity.entity.pos.load(),
            entity.entity.width(),
            entity.entity.height(),
        );

        let step_height = entity.get_attribute_value(&Attributes::STEP_HEIGHT) as f32;
        let player_controlled = mob
            .as_ref()
            .and_then(|mob| mob.controlling_passenger())
            .is_some_and(|passenger| passenger.get_player().is_some());
        mob_data.max_up_step = if player_controlled {
            step_height.max(1.0)
        } else {
            step_height
        };
        mob_data.max_fall_distance = mob
            .as_ref()
            .and_then(|mob| mob.get_mob().map(Mob::get_max_fall_distance))
            .unwrap_or_else(|| LivingEntity::get_comfortable_fall_distance(0.0));
        mob_data.can_stand_on_lava = entity.entity.entity_type == &EntityType::STRIDER;
        mob_data.on_ground = entity.entity.on_ground.load(Ordering::Relaxed);
        mob_data.is_in_water = entity.entity.is_in_water();
        mob_data.entity_id = Some(entity.entity.entity_id);

        // Vanilla's node evaluators call `mob.getPathfindingMalus`, which reads the controlled
        // vehicle's map when it passes its malus on.
        let inherited = mob
            .as_ref()
            .and_then(|mob| mob.get_mob())
            .and_then(|mob| mob.get_mob_entity().inherited_malus_overrides());
        for (&path_type, &malus) in inherited.as_ref().unwrap_or(&self.path_type_overrides) {
            mob_data.set_pathfinding_malus(path_type, malus);
        }
        if let Some(mob) = mob.as_ref().and_then(|mob| mob.get_mob()) {
            mob.on_pathfinding_start(&mut mob_data);
        }
        mob_data
    }

    /// Vanilla `PathFinder.findPath` for a single target, run with this
    /// navigation's evaluator.
    pub fn compute_path(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        let max_path_length = self.mob_max_follow_range(entity);
        self.compute_path_within(entity, destination, reach_range, max_path_length)
    }

    /// [`Self::compute_path`] with vanilla's explicit `maxPathLength`.
    pub fn compute_path_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
        max_path_length: f32,
    ) -> Option<Path> {
        let world = entity.entity.world.load_full();
        let mob_position = BlockPos::floored_v(entity.entity.pos.load()).0;
        let mob_data = self.mob_data(entity, &world);
        let context = PathfindingContext::new(mob_position, world);

        self.evaluator.set_can_float(self.can_float);
        self.evaluator.set_can_open_doors(self.can_open_doors);
        self.evaluator.set_can_pass_doors(self.can_pass_doors);
        self.evaluator
            .set_can_walk_over_fences(self.can_walk_over_fences);

        let max_visited_nodes = *self.max_visited_nodes.get_or_insert_with(|| {
            (entity.get_attribute_base(&Attributes::FOLLOW_RANGE) * 16.0).floor() as i32
        });

        self.open_set.clear();
        self.evaluator.prepare(context, mob_data);
        let path = self.find_path(
            BlockPos::floored_v(destination),
            max_path_length,
            reach_range,
            max_visited_nodes,
        );
        self.evaluator.done();
        path
    }

    fn find_path(
        &mut self,
        target_pos: BlockPos,
        max_path_length: f32,
        reach_range: i32,
        max_visited_nodes: i32,
    ) -> Option<Path> {
        let from = self.evaluator.get_start()?;
        let mut target = self.evaluator.get_target(target_pos);

        let from = {
            let node = self.evaluator.base_mut().node_mut(from.pos);
            node.g = 0.0;
            node.h = node.distance(&target);
            target.update_best(node.h, node);
            node.f = node.h;
            *node
        };
        self.open_set.clear();
        self.open_set.insert(from);

        let mut count = 0;
        let max_visited_nodes_adjusted =
            (max_visited_nodes as f32 * self.max_visited_nodes_multiplier) as i32;

        while !self.open_set.is_empty() {
            count += 1;
            if count >= max_visited_nodes_adjusted {
                break;
            }

            let Some(popped) = self.open_set.pop() else {
                break;
            };
            let current = {
                let node = self.evaluator.base_mut().node_mut(popped.pos);
                node.closed = true;
                *node
            };

            if current.distance_manhattan(&target) <= reach_range as f32 {
                target.set_reached();
                break;
            }

            if current.distance_to_node(&from) >= max_path_length {
                continue;
            }

            self.neighbors_buf.clear();
            self.evaluator
                .get_neighbors(&current, &mut self.neighbors_buf);

            for neighbor_pos in self.neighbors_buf.iter().map(|neighbor| neighbor.pos) {
                let in_open_set = self.open_set.contains(&neighbor_pos);
                let neighbor = self.evaluator.base_mut().node_mut(neighbor_pos);
                let distance = current.distance_to_node(neighbor);
                // Vanilla overwrites the walked distance even when the check below
                // turns the cheaper path down.
                neighbor.walked_dist = current.walked_dist + distance;
                let tentative_g = current.g + distance + neighbor.cost_malus;
                if neighbor.walked_dist < max_path_length
                    && (!in_open_set || tentative_g < neighbor.g)
                {
                    neighbor.came_from = Some(current.pos.0);
                    neighbor.g = tentative_g;
                    let h = neighbor.distance(&target);
                    target.update_best(h, neighbor);
                    neighbor.h = h * FUDGING;
                    neighbor.f = neighbor.g + neighbor.h;
                    let neighbor = *neighbor;
                    if in_open_set {
                        self.open_set.change_cost(&neighbor_pos, neighbor.f);
                    } else {
                        self.open_set.insert(neighbor);
                    }
                }
            }
        }

        let best = target.get_best_node()?;
        Some(Path::new(
            self.reconstruct_path(best.pos),
            target_pos,
            target.is_reached(),
        ))
    }

    /// Vanilla `PathFinder.reconstructPath`, following `came_from` through the
    /// evaluator's nodes as they stand when the search ends.
    fn reconstruct_path(&mut self, closest: BlockPos) -> Vec<Node> {
        let nodes = &self.evaluator.base_mut().nodes;
        let mut path = Vec::new();
        let mut node = nodes.get(&closest.0).copied();
        while let Some(current) = node {
            path.push(current);
            // Each node points at one closed before it, so the chain ends; the bound
            // only guards against a broken map.
            if path.len() > nodes.len() {
                break;
            }
            node = current.came_from.and_then(|pos| nodes.get(&pos).copied());
        }
        path.reverse();
        path
    }

    pub fn needs_new_path(&self, goal: &NavigatorGoal) -> bool {
        if self.path.is_none() {
            return true;
        }
        if self.repath_cooldown > 0 {
            return false;
        }
        self.path.as_ref().is_some_and(|p| {
            let path_target = p.get_target();
            let goal_target = BlockPos::floored_v(goal.destination).0;
            let dx = f64::from(path_target.0.x - goal_target.x);
            let dy = f64::from(path_target.0.y - goal_target.y);
            let dz = f64::from(path_target.0.z - goal_target.z);
            let distance_sq = dx * dx + dy * dy + dz * dz;
            let remaining = p.get_remaining_distance().clamp(4.0, 16.0);
            let threshold = remaining * 0.5;
            distance_sq > f64::from(threshold * threshold)
        })
    }

    /// Vanilla `PathNavigation.isClearForMovementBetween`: a collider ray from `start_pos` to
    /// half the mob's height above `stop_pos` hits nothing (and no fluid, if asked).
    pub fn can_move_directly(
        world: &World,
        start_pos: Vector3<f64>,
        stop_pos: Vector3<f64>,
        mob_height: f32,
        blocked_by_fluids: bool,
    ) -> bool {
        let to = Vector3::new(
            stop_pos.x,
            stop_pos.y + f64::from(mob_height) * 0.5,
            stop_pos.z,
        );
        let fluid = if blocked_by_fluids {
            pumpkin_world::clip::ClipFluid::Any
        } else {
            pumpkin_world::clip::ClipFluid::None
        };
        world
            .clip(&pumpkin_world::clip::ClipContext::new(
                start_pos,
                to,
                pumpkin_world::clip::ClipBlock::Collider,
                fluid,
            ))
            .is_miss
    }

    pub fn find_surface_position(world: &World, mut pos: BlockPos) -> BlockPos {
        let block_state = world.get_block_state(&pos);
        if block_state.is_air() {
            let mut column_pos = pos;
            while column_pos.0.y >= -64 && world.get_block_state(&column_pos).is_air() {
                column_pos.0.y -= 1;
            }
            if column_pos.0.y >= -64 {
                return BlockPos::new(column_pos.0.x, column_pos.0.y + 1, column_pos.0.z);
            }
            column_pos.0.y = pos.0.y + 1;
            while column_pos.0.y <= 320 && world.get_block_state(&column_pos).is_air() {
                column_pos.0.y += 1;
            }
            pos = column_pos;
        }
        if !world.get_block_state(&pos).is_solid() {
            return pos;
        }
        let mut column_pos = pos;
        while column_pos.0.y <= 320 && world.get_block_state(&column_pos).is_solid() {
            column_pos.0.y += 1;
        }
        column_pos
    }

    pub fn get_surface_y(&self, entity: &LivingEntity) -> f64 {
        let pos = entity.entity.pos.load();
        if entity.entity.touching_water.load(Ordering::Relaxed) && self.can_float {
            let mut surface = pos.y.floor() as i32;
            let world = entity.entity.world.load();
            let mut steps = 0;
            loop {
                let bp = BlockPos::new(pos.x.floor() as i32, surface, pos.z.floor() as i32);
                let state = world.get_block_state(&bp);
                if state.is_liquid() {
                    surface += 1;
                    steps += 1;
                    if steps > 16 {
                        return pos.y;
                    }
                } else {
                    break;
                }
            }
            f64::from(surface)
        } else {
            (pos.y + 0.5).floor()
        }
    }

    pub const fn reset_stuck_timeout(&mut self) {
        self.timeout_cached_node = Vector3::new(0, 0, 0);
        self.timeout_timer = 0;
        self.timeout_limit = 0.0;
        self.is_stuck = false;
    }

    pub fn do_stuck_detection(&mut self, mob_pos: Vector3<f64>, entity: &LivingEntity) {
        let world_age = entity.entity.world.load().get_world_age() as u64;
        if self.tick_count.saturating_sub(self.last_stuck_check) > 100 {
            // Vanilla `mob.getSpeed()`, the speed the move control last set.
            let speed = entity.speed.load();
            let effective_speed = if speed >= 1.0 { speed } else { speed * speed };
            let threshold_distance = effective_speed * 100.0 * 0.25;
            let dx = mob_pos.x - self.last_stuck_check_pos.x;
            let dy = mob_pos.y - self.last_stuck_check_pos.y;
            let dz = mob_pos.z - self.last_stuck_check_pos.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;

            if dist_sq < f64::from(threshold_distance * threshold_distance) {
                self.is_stuck = true;
                self.stop();
            } else {
                self.is_stuck = false;
            }
            self.last_stuck_check = self.tick_count;
            self.last_stuck_check_pos = mob_pos;
        }

        if let Some(path) = &self.path
            && !path.is_done()
            && let Some(pos) = path.get_next_node_pos()
        {
            if pos.0 == self.timeout_cached_node {
                self.timeout_timer = self
                    .timeout_timer
                    .saturating_add(world_age.saturating_sub(self.last_timeout_check));
            } else {
                self.timeout_cached_node = pos.0;
                let node_center = Vector3::new(
                    f64::from(pos.0.x) + 0.5,
                    f64::from(pos.0.y),
                    f64::from(pos.0.z) + 0.5,
                );
                let dist_to_node = (mob_pos - node_center).length();
                let speed = f64::from(entity.speed.load());
                self.timeout_limit = if speed > 0.0 {
                    dist_to_node / speed * 20.0
                } else {
                    0.0
                };
            }

            if self.timeout_limit > 0.0 && self.timeout_timer as f64 > self.timeout_limit * 3.0 {
                self.reset_stuck_timeout();
                self.stop();
            }
            self.last_timeout_check = world_age;
        }
    }

    /// Vanilla `shouldTargetNextNodeInDirection` with its `canMoveDirectly` short-cut, which
    /// runs after the node-count and distance checks.
    fn should_target_next_node_in_direction_or_move_directly(
        mob_pos: Vector3<f64>,
        path: &Path,
        moves_directly: impl FnOnce(&Path) -> bool,
    ) -> bool {
        if path.get_next_node_index() + 1 >= path.get_node_count() {
            return false;
        }
        let Some(current) = path.get_next_node_pos() else {
            return false;
        };
        let current_node = Vector3::new(
            f64::from(current.0.x) + 0.5,
            f64::from(current.0.y),
            f64::from(current.0.z) + 0.5,
        );
        if mob_pos.squared_distance_to_vec(&current_node) >= 4.0 {
            return false;
        }
        moves_directly(path) || Self::should_target_next_node_in_direction(mob_pos, path)
    }

    #[must_use]
    pub fn should_target_next_node_in_direction(mob_pos: Vector3<f64>, path: &Path) -> bool {
        if path.get_next_node_index() + 1 >= path.get_node_count() {
            return false;
        }
        let Some(curr_pos) = path.get_next_node_pos() else {
            return false;
        };
        let current_node = Vector3::new(
            f64::from(curr_pos.0.x) + 0.5,
            f64::from(curr_pos.0.y),
            f64::from(curr_pos.0.z) + 0.5,
        );
        let dx = mob_pos.x - current_node.x;
        let dy = mob_pos.y - current_node.y;
        let dz = mob_pos.z - current_node.z;
        // `!mobPosition.closerThan(currentNode, 2.0)`: closer means strictly within.
        if dx * dx + dy * dy + dz * dz >= 4.0 {
            return false;
        }

        let next_idx = path.get_next_node_index() + 1;
        let Some(next_pos) = path.get_node_pos(next_idx) else {
            return false;
        };
        let next_node = Vector3::new(
            f64::from(next_pos.0.x) + 0.5,
            f64::from(next_pos.0.y),
            f64::from(next_pos.0.z) + 0.5,
        );

        let mob_to_current = current_node - mob_pos;
        let mob_to_next = next_node - mob_pos;
        let mob_to_curr_sqr = mob_to_current.length_squared();
        let mob_to_next_sqr = mob_to_next.length_squared();

        let closer_to_next = mob_to_next_sqr < mob_to_curr_sqr;
        let within_curr = mob_to_curr_sqr < 0.5;
        if !closer_to_next && !within_curr {
            return false;
        }

        let curr_len = mob_to_current.length();
        let next_len = mob_to_next.length();
        if curr_len < 1e-5 || next_len < 1e-5 {
            return false;
        }
        let mob_dir = mob_to_current / curr_len;
        let path_dir = mob_to_next / next_len;
        path_dir.dot(&mob_dir) < 0.0
    }

    pub fn trim_path(&mut self, entity: &LivingEntity) {
        if self.avoid_sun {
            let pos = entity.entity.pos.load();
            let bp = BlockPos::new(
                pos.x.floor() as i32,
                (pos.y + 0.5).floor() as i32,
                pos.z.floor() as i32,
            );
            let world = entity.entity.world.load();
            if world.get_sky_light_level(&bp) < 15
                && let Some(path) = &mut self.path
            {
                let mut cut_index = None;
                for i in 0..path.get_node_count() {
                    if let Some(node) = path.get_node(i) {
                        let node_bp = BlockPos::new(node.pos.0.x, node.pos.0.y, node.pos.0.z);
                        if world.get_sky_light_level(&node_bp) >= 15 {
                            cut_index = Some(i);
                            break;
                        }
                    }
                }
                if let Some(idx) = cut_index {
                    path.truncate_nodes(idx);
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    /// Vanilla `GroundPathNavigation.canUpdatePath`, or `FlyingPathNavigation`'s.
    fn can_update_path(&self, entity: &LivingEntity) -> bool {
        let e = &entity.entity;
        let in_liquid =
            e.touching_water.load(Ordering::Relaxed) || e.touching_lava.load(Ordering::Relaxed);
        if self.flying {
            self.can_float && in_liquid || e.get_vehicle().is_none()
        } else {
            e.on_ground.load(Ordering::Relaxed) || in_liquid || e.get_vehicle().is_some()
        }
    }

    /// Vanilla `GroundPathNavigation.getSurfaceY`.
    fn ground_surface_y(&self, entity: &LivingEntity) -> i32 {
        let e = &entity.entity;
        let pos = e.pos.load();
        let block_y = e.block_pos.load().0.y;
        if e.touching_water.load(Ordering::Relaxed) && self.can_float {
            let world = e.world.load();
            let floatable = |y: i32| {
                let (fluid, _) =
                    world.get_fluid_and_fluid_state(&BlockPos::floored(pos.x, f64::from(y), pos.z));
                fluid.has_tag(&tag::Fluid::MINECRAFT_ENTITY_FLOATABLE)
            };
            let mut surface = block_y;
            let mut steps = 0;
            while floatable(surface) {
                surface += 1;
                steps += 1;
                if steps > 16 {
                    return block_y;
                }
            }
            surface
        } else {
            (pos.y + 0.5).floor() as i32
        }
    }

    /// Vanilla `GroundPathNavigation.getTempMobPos`; flying navigation uses the position.
    fn ground_temp_mob_pos(&self, entity: &LivingEntity) -> Vector3<f64> {
        let pos = entity.entity.pos.load();
        if self.flying {
            return pos;
        }
        Vector3::new(pos.x, f64::from(self.ground_surface_y(entity)), pos.z)
    }

    /// Vanilla `GroundPathNavigation.createPath(BlockPos, reachRange)` and the base
    /// `createPath(Set, 8, false, reachRange, maxPathLength)` it calls.
    pub fn ground_create_path(
        &mut self,
        entity: &LivingEntity,
        pos: BlockPos,
        reach_range: i32,
    ) -> Option<Path> {
        let max_path_length = self.mob_max_follow_range(entity);
        self.ground_create_path_within(entity, pos, reach_range, max_path_length)
    }

    /// [`Self::ground_create_path`] with vanilla's explicit `maxPathLength`.
    pub fn ground_create_path_within(
        &mut self,
        entity: &LivingEntity,
        mut pos: BlockPos,
        reach_range: i32,
        max_path_length: f32,
    ) -> Option<Path> {
        let world = entity.entity.world.load();
        // Only `GroundPathNavigation.createPath` needs a loaded chunk and moves to the surface.
        if !self.flying {
            let chunk = Vector2::new(pos.0.x >> 4, pos.0.z >> 4);
            if !world.level.is_chunk_loaded(&chunk) {
                return None;
            }
            if !self.can_path_to_targets_below_surface {
                pos = Self::find_surface_position(&world, pos);
            }
        }
        if entity.entity.pos.load().y < f64::from(world.get_bottom_y()) {
            return None;
        }
        if !self.can_update_path(entity) {
            return None;
        }
        if self.target_pos == Some(pos)
            && let Some(path) = self.path.as_ref().filter(|path| !path.is_done())
        {
            return Some(path.clone());
        }
        let destination = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y),
            f64::from(pos.0.z) + 0.5,
        );
        let path = self.compute_path_within(entity, destination, reach_range, max_path_length)?;
        self.target_pos = Some(path.get_target());
        self.reach_range = reach_range;
        self.reset_stuck_timeout();
        Some(path)
    }

    /// Vanilla `PathNavigation.moveTo(Path, speedModifier)`.
    pub fn ground_move_to_path(
        &mut self,
        entity: &LivingEntity,
        new_path: Option<Path>,
        speed_modifier: f64,
    ) -> bool {
        let Some(new_path) = new_path else {
            self.path = None;
            return false;
        };
        if !new_path.same_as(self.path.as_ref()) {
            self.path = Some(new_path);
        }
        if self.path.as_ref().is_none_or(Path::is_done) {
            return false;
        }
        self.ground_trim_path(entity);
        if self
            .path
            .as_ref()
            .is_none_or(|path| path.get_node_count() == 0)
        {
            return false;
        }
        self.speed_modifier = speed_modifier;
        self.last_stuck_check = self.tick_count;
        self.last_stuck_check_pos = self.ground_temp_mob_pos(entity);
        true
    }

    /// Vanilla `moveTo(x, y, z, reachRange, speedModifier)`: a path to the block holding the
    /// point, then `moveTo(Path)`.
    pub fn ground_move_to(&mut self, entity: &LivingEntity, goal: &NavigatorGoal) -> bool {
        let path = self.ground_create_path(
            entity,
            BlockPos::floored_v(goal.destination),
            goal.reach_range,
        );
        self.ground_move_to_path(entity, path, goal.speed)
    }

    /// Vanilla `PathNavigation.trimPath` and `GroundPathNavigation.trimPath`.
    fn ground_trim_path(&mut self, entity: &LivingEntity) {
        let world = entity.entity.world.load();
        let Some(path) = self.path.as_mut() else {
            return;
        };
        for i in 0..path.get_node_count() {
            let Some(node) = path.get_node(i).copied() else {
                continue;
            };
            if world
                .get_block(&node.pos)
                .has_tag(&tag::Block::MINECRAFT_CAULDRONS)
            {
                let next = path.get_node(i + 1).copied();
                path.replace_node(i, node.clone_and_move(node.pos.up()));
                if let Some(next) = next
                    && node.pos.0.y >= next.pos.0.y
                {
                    path.replace_node(
                        i + 1,
                        node.clone_and_move(BlockPos::new(
                            next.pos.0.x,
                            node.pos.0.y + 1,
                            next.pos.0.z,
                        )),
                    );
                }
            }
        }
        if self.avoid_sun && !self.flying {
            let pos = entity.entity.pos.load();
            if world.can_see_sky(&BlockPos::floored(pos.x, pos.y + 0.5, pos.z)) {
                return;
            }
            for i in 0..path.get_node_count() {
                if path
                    .get_node(i)
                    .is_some_and(|node| world.can_see_sky(&node.pos))
                {
                    path.truncate_nodes(i);
                    return;
                }
            }
        }
    }

    /// Vanilla `PathNavigation.recomputePath` with the ground `createPath`.
    pub fn ground_recompute_path(&mut self, entity: &LivingEntity) {
        let game_time = entity.entity.world.load().get_world_age() as u64;
        if game_time.saturating_sub(self.time_last_recompute) <= 20 || !self.can_update_path(entity)
        {
            self.has_delayed_recomputation = true;
        } else if let Some(target_pos) = self.target_pos {
            self.path = None;
            self.path = self.ground_create_path(entity, target_pos, self.reach_range);
            self.time_last_recompute = game_time;
            self.has_delayed_recomputation = false;
        }
    }

    /// Vanilla `PathNavigation.getGroundY`.
    fn ground_y(world: &World, target: Vector3<f64>) -> f64 {
        let block_pos = BlockPos::floored_v(target);
        if world.get_block_state(&block_pos.down()).is_air() {
            target.y
        } else {
            let below = block_pos.down();
            let max_y = world
                .get_block_state(&below)
                .get_block_collision_shapes_at(&below)
                .map(|shape| shape.max.y)
                .reduce(f64::max);
            f64::from(below.0.y) + max_y.unwrap_or(0.0)
        }
    }

    /// Runs a goal's queued `moveTo`, which vanilla ran during the goal tick, before navigation.
    pub fn resolve_pending_move(&mut self, entity: &LivingEntity) {
        if let Some(goal) = self.pending_move.take() {
            self.ground_move_to(entity, &goal);
        }
    }

    /// Vanilla `PathNavigation.tick` for ground navigation; the move control steers.
    pub fn tick_ground(&mut self, entity: &LivingEntity) {
        self.resolve_pending_move(entity);
        self.wanted_position = None;
        self.tick_count += 1;
        if self.has_delayed_recomputation {
            self.ground_recompute_path(entity);
        }
        if self.path.as_ref().is_none_or(Path::is_done) {
            return;
        }
        let width = entity.entity.entity_dimension.load().width;
        if self.can_update_path(entity) {
            self.ground_follow_the_path(entity);
        } else {
            let mob_pos = self.ground_temp_mob_pos(entity);
            let block_pos = entity.entity.block_pos.load().0;
            let flying = self.flying;
            if let Some(path) = self.path.as_mut().filter(|path| !path.is_done())
                && let Some(pos) = path.get_next_entity_pos(width)
                && if flying {
                    // `FlyingPathNavigation.tick`: advance once inside the waypoint's block.
                    block_pos.x == pos.x.floor() as i32
                        && block_pos.y == pos.y.floor() as i32
                        && block_pos.z == pos.z.floor() as i32
                } else {
                    mob_pos.y > pos.y
                        && !entity.entity.on_ground.load(Ordering::Relaxed)
                        && mob_pos.x.floor() == pos.x.floor()
                        && mob_pos.z.floor() == pos.z.floor()
                }
            {
                path.advance();
            }
        }
        if let Some(target) = self
            .path
            .as_ref()
            .filter(|path| !path.is_done())
            .and_then(|path| path.get_next_entity_pos(width))
        {
            let world = entity.entity.world.load();
            let y = if self.flying {
                target.y
            } else {
                Self::ground_y(&world, target)
            };
            self.wanted_position = Some((Vector3::new(target.x, y, target.z), self.speed_modifier));
        }
    }

    /// Vanilla `PathNavigation.followThePath`.
    fn ground_follow_the_path(&mut self, entity: &LivingEntity) {
        let mob_pos = self.ground_temp_mob_pos(entity);
        let width = entity.entity.entity_dimension.load().width;
        self.max_distance_to_waypoint = if width > 0.75 {
            width / 2.0
        } else {
            0.75 - width / 2.0
        };
        let Some(path) = self.path.as_mut() else {
            return;
        };
        let (Some(node_pos), Some(next_type)) = (
            path.get_next_node_pos(),
            path.get_next_node().map(|node| node.path_type),
        ) else {
            return;
        };
        let pos = entity.entity.pos.load();
        let x_distance = (pos.x - (f64::from(node_pos.0.x) + 0.5)).abs();
        let y_distance = (pos.y - f64::from(node_pos.0.y)).abs();
        let z_distance = (pos.z - (f64::from(node_pos.0.z) + 0.5)).abs();
        let max_distance = f64::from(self.max_distance_to_waypoint);
        let close_enough = x_distance < max_distance
            && z_distance < max_distance
            && y_distance < f64::from(Self::MAX_VERTICAL_DISTANCE_TO_WAYPOINT);
        let can_cut_corner = next_type != PathType::DangerFire
            && next_type != PathType::DangerOther
            && next_type != PathType::WalkableDoor;
        let flying = self.flying;
        // `canMoveDirectly`: ground navigation never does; flying clips.
        let blocked_by_fluids = self.direct_moves_blocked_by_fluids;
        let moves_directly = |path: &Path| {
            flying
                && path.get_next_entity_pos(width).is_some_and(|next| {
                    Self::can_move_directly(
                        &entity.entity.world.load(),
                        mob_pos,
                        next,
                        entity.entity.entity_dimension.load().height,
                        blocked_by_fluids,
                    )
                })
        };
        if close_enough
            || can_cut_corner
                && Self::should_target_next_node_in_direction_or_move_directly(
                    mob_pos,
                    path,
                    moves_directly,
                )
        {
            path.advance();
        }
        self.do_stuck_detection(mob_pos, entity);
    }

    /// `PathNavigation.getMaxVerticalDistanceToWaypoint`.
    const MAX_VERTICAL_DISTANCE_TO_WAYPOINT: f32 = 1.0;
}

pub struct GroundPathNavigation {
    pub inner: PathNavigation,
    /// Vanilla `StriderPathNavigation.isStableDestination`: lava counts as standing room.
    stable_on_lava: bool,
}

impl Default for GroundPathNavigation {
    fn default() -> Self {
        Self::new()
    }
}

impl GroundPathNavigation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: PathNavigation::new(EvaluatorKind::Walk(WalkNodeEvaluator::default())),
            stable_on_lava: false,
        }
    }

    /// Vanilla `Strider.StriderPathNavigation`.
    #[must_use]
    pub fn strider() -> Self {
        Self {
            stable_on_lava: true,
            ..Self::new()
        }
    }
}

impl PathNavigationTrait for GroundPathNavigation {
    fn is_stable_destination(&self, world: &World, pos: &BlockPos) -> bool {
        (self.stable_on_lava && world.get_block(pos) == &pumpkin_data::Block::LAVA)
            || world.get_block_state(&pos.down()).is_solid_render()
    }

    /// A goal's `moveTo(x, y, z, reachRange, speed)`, run at the start of this tick's
    /// navigation, before the path is followed, as in vanilla.
    fn set_progress(&mut self, goal: NavigatorGoal) {
        self.inner.pending_move = Some(goal);
    }

    fn set_speed(&mut self, speed: f64) {
        self.inner.set_speed(speed);
    }

    /// Vanilla `PathNavigation.stop`: drops the path, and any `moveTo` not yet run.
    fn stop(&mut self) {
        self.inner.path = None;
        self.inner.pending_move = None;
    }

    fn is_idle(&self) -> bool {
        self.is_done()
    }

    /// Vanilla `isDone`. A `moveTo` waiting for this tick's navigation counts as moving.
    fn is_done(&self) -> bool {
        self.inner.pending_move.is_none() && self.inner.path.as_ref().is_none_or(Path::is_done)
    }

    fn is_in_progress(&self) -> bool {
        !self.is_done()
    }

    fn is_stuck(&self) -> bool {
        self.inner.is_stuck
    }

    fn get_path(&self) -> Option<&Path> {
        self.inner.path.as_ref()
    }

    fn get_path_mut(&mut self) -> Option<&mut Path> {
        self.inner.path.as_mut()
    }

    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.inner.set_pathfinding_malus(path_type, malus);
    }

    fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.inner.get_pathfinding_malus(path_type)
    }

    fn path_type_overrides(&self) -> &FxHashMap<PathType, f32> {
        &self.inner.path_type_overrides
    }

    fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.inner.set_mob_dimensions(width, height);
    }

    fn can_reach_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.inner.can_reach_within(entity, destination, distance)
    }

    fn tick(&mut self, entity: &LivingEntity) {
        self.inner.tick_ground(entity);
    }

    fn take_wanted_position(&mut self) -> Option<(Vector3<f64>, f64)> {
        self.inner.wanted_position.take()
    }

    fn move_to_coords(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        speed: f64,
        entity: &LivingEntity,
    ) -> bool {
        self.inner.pending_move = None;
        let goal = NavigatorGoal::new(entity.entity.pos.load(), Vector3::new(x, y, z), speed);
        self.inner.ground_move_to(entity, &goal)
    }

    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool {
        self.inner.pending_move = None;
        let path = self
            .inner
            .ground_create_path(entity, pos, NavigatorGoal::DEFAULT_REACH_RANGE);
        self.inner.ground_move_to_path(entity, path, speed)
    }

    /// Vanilla `moveTo(Entity, speed)`; ground navigation paths to the entity's block.
    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, entity: &LivingEntity) -> bool {
        self.inner.pending_move = None;
        let path = self.inner.ground_create_path(
            entity,
            target.entity.block_pos.load(),
            NavigatorGoal::DEFAULT_REACH_RANGE,
        );
        path.is_some() && self.inner.ground_move_to_path(entity, path, speed)
    }

    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool {
        self.inner.pending_move = None;
        self.inner.ground_move_to_path(entity, path, speed)
    }

    fn create_path(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        self.inner
            .ground_create_path(entity, BlockPos::floored_v(destination), reach_range)
    }

    fn create_path_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
        max_path_length: f32,
    ) -> Option<Path> {
        self.inner.ground_create_path_within(
            entity,
            BlockPos::floored_v(destination),
            reach_range,
            max_path_length,
        )
    }

    fn recompute_path(&mut self, entity: &LivingEntity) {
        self.inner.ground_recompute_path(entity);
    }

    fn set_avoid_sun(&mut self, avoid_sun: bool) {
        self.inner.avoid_sun = avoid_sun;
    }

    fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        self.inner.can_walk_over_fences = can_walk;
    }

    fn set_can_open_doors(&mut self, can_open: bool) {
        self.inner.can_open_doors = can_open;
    }

    fn set_can_pass_doors(&mut self, can_pass: bool) {
        self.inner.can_pass_doors = can_pass;
    }

    fn set_can_float(&mut self, can_float: bool) {
        self.inner.can_float = can_float;
    }

    fn can_float(&self) -> bool {
        self.inner.can_float
    }

    fn can_navigate_ground(&self) -> bool {
        true
    }

    fn set_required_path_length(&mut self, length: f32, entity: &LivingEntity) {
        self.inner.set_required_path_length(length, entity);
    }

    fn update_pathfinder_max_visited_nodes(&mut self, entity: &LivingEntity) {
        self.inner.update_pathfinder_max_visited_nodes(entity);
    }

    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32) {
        self.inner.max_visited_nodes_multiplier = multiplier;
    }

    fn reset_max_visited_nodes_multiplier(&mut self) {
        self.inner.max_visited_nodes_multiplier = 1.0;
    }

    fn get_target_pos(&self) -> Option<BlockPos> {
        self.inner.target_pos
    }

    fn can_path_to_targets_below_surface(&self) -> bool {
        self.inner.can_path_to_targets_below_surface
    }

    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool) {
        self.inner.can_path_to_targets_below_surface = can_path;
    }
}

pub struct FlyingPathNavigation {
    pub inner: PathNavigation,
    /// Settles over any block that isn't air, as vanilla `Bee`'s navigation does.
    stable_over_any_block: bool,
}

impl Default for FlyingPathNavigation {
    fn default() -> Self {
        Self::new()
    }
}

impl FlyingPathNavigation {
    #[must_use]
    pub fn new() -> Self {
        let mut inner = PathNavigation::new(EvaluatorKind::Fly(FlyNodeEvaluator::default()));
        inner.flying = true;
        Self {
            inner,
            stable_over_any_block: false,
        }
    }

    /// Vanilla `HappyGhast.BabyFlyingPathNavigation.canMoveDirectly`: fluids don't block
    /// a direct move.
    #[must_use]
    pub const fn moves_directly_through_fluids(mut self) -> Self {
        self.inner.direct_moves_blocked_by_fluids = false;
        self
    }

    /// Vanilla `Bee.createNavigation`'s `isStableDestination`: the block below only has
    /// to be something other than air.
    #[must_use]
    pub const fn stable_over_any_block(mut self) -> Self {
        self.stable_over_any_block = true;
        self
    }
}

impl PathNavigationTrait for FlyingPathNavigation {
    /// A goal's `moveTo(x, y, z, reachRange, speed)`, run at the start of this tick's
    /// navigation, before the path is followed, as in vanilla.
    fn set_progress(&mut self, goal: NavigatorGoal) {
        self.inner.pending_move = Some(goal);
    }

    fn set_speed(&mut self, speed: f64) {
        self.inner.set_speed(speed);
    }

    /// Vanilla `PathNavigation.stop`: drops the path, and any `moveTo` not yet run.
    fn stop(&mut self) {
        self.inner.path = None;
        self.inner.pending_move = None;
    }

    fn is_idle(&self) -> bool {
        self.is_done()
    }

    /// Vanilla `isDone`. A `moveTo` waiting for this tick's navigation counts as moving.
    fn is_done(&self) -> bool {
        self.inner.pending_move.is_none() && self.inner.path.as_ref().is_none_or(Path::is_done)
    }

    fn is_in_progress(&self) -> bool {
        !self.is_done()
    }

    fn is_stuck(&self) -> bool {
        self.inner.is_stuck
    }

    fn get_path(&self) -> Option<&Path> {
        self.inner.path.as_ref()
    }

    fn get_path_mut(&mut self) -> Option<&mut Path> {
        self.inner.path.as_mut()
    }

    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.inner.set_pathfinding_malus(path_type, malus);
    }

    fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.inner.get_pathfinding_malus(path_type)
    }

    fn path_type_overrides(&self) -> &FxHashMap<PathType, f32> {
        &self.inner.path_type_overrides
    }

    fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.inner.set_mob_dimensions(width, height);
    }

    fn can_reach_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.inner.can_reach_within(entity, destination, distance)
    }

    fn tick(&mut self, entity: &LivingEntity) {
        self.inner.tick_ground(entity);
    }

    fn take_wanted_position(&mut self) -> Option<(Vector3<f64>, f64)> {
        self.inner.wanted_position.take()
    }

    fn move_to_coords(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        speed: f64,
        entity: &LivingEntity,
    ) -> bool {
        self.inner.pending_move = None;
        let goal = NavigatorGoal::new(entity.entity.pos.load(), Vector3::new(x, y, z), speed);
        self.inner.ground_move_to(entity, &goal)
    }

    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool {
        self.inner.pending_move = None;
        let path = self
            .inner
            .ground_create_path(entity, pos, NavigatorGoal::DEFAULT_REACH_RANGE);
        self.inner.ground_move_to_path(entity, path, speed)
    }

    /// Vanilla `moveTo(Entity, speed)`; ground navigation paths to the entity's block.
    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, entity: &LivingEntity) -> bool {
        self.inner.pending_move = None;
        let path = self.inner.ground_create_path(
            entity,
            target.entity.block_pos.load(),
            NavigatorGoal::DEFAULT_REACH_RANGE,
        );
        path.is_some() && self.inner.ground_move_to_path(entity, path, speed)
    }

    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool {
        self.inner.pending_move = None;
        self.inner.ground_move_to_path(entity, path, speed)
    }

    fn create_path(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        self.inner
            .ground_create_path(entity, BlockPos::floored_v(destination), reach_range)
    }

    fn create_path_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
        max_path_length: f32,
    ) -> Option<Path> {
        self.inner.ground_create_path_within(
            entity,
            BlockPos::floored_v(destination),
            reach_range,
            max_path_length,
        )
    }

    fn recompute_path(&mut self, entity: &LivingEntity) {
        self.inner.ground_recompute_path(entity);
    }

    fn set_avoid_sun(&mut self, avoid_sun: bool) {
        self.inner.avoid_sun = avoid_sun;
    }

    fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        self.inner.can_walk_over_fences = can_walk;
    }

    fn set_can_open_doors(&mut self, can_open: bool) {
        self.inner.can_open_doors = can_open;
    }

    fn set_can_pass_doors(&mut self, can_pass: bool) {
        self.inner.can_pass_doors = can_pass;
    }

    fn set_can_float(&mut self, can_float: bool) {
        self.inner.can_float = can_float;
    }

    fn can_float(&self) -> bool {
        self.inner.can_float
    }

    fn can_navigate_ground(&self) -> bool {
        false
    }

    fn set_required_path_length(&mut self, length: f32, entity: &LivingEntity) {
        self.inner.set_required_path_length(length, entity);
    }

    fn update_pathfinder_max_visited_nodes(&mut self, entity: &LivingEntity) {
        self.inner.update_pathfinder_max_visited_nodes(entity);
    }

    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32) {
        self.inner.max_visited_nodes_multiplier = multiplier;
    }

    fn reset_max_visited_nodes_multiplier(&mut self) {
        self.inner.max_visited_nodes_multiplier = 1.0;
    }

    fn get_target_pos(&self) -> Option<BlockPos> {
        self.inner.target_pos
    }

    fn can_path_to_targets_below_surface(&self) -> bool {
        self.inner.can_path_to_targets_below_surface
    }

    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool) {
        self.inner.can_path_to_targets_below_surface = can_path;
    }

    fn is_stable_destination(&self, world: &World, pos: &BlockPos) -> bool {
        if self.stable_over_any_block {
            !world.get_block_state(&pos.down()).is_air()
        } else {
            world.get_block_state(&pos.down()).is_solid_render()
        }
    }
}

pub struct WaterBoundPathNavigation {
    pub inner: PathNavigation,
    pub allow_breaching: bool,
    /// Follow the path like vanilla and leave the steering to the move control,
    /// instead of turning the mob towards each node here.
    steered_by_move_control: bool,
    wanted_position: Option<(Vector3<f64>, f64)>,
    /// The block the current path was asked for: vanilla's `PathNavigation.targetPos`.
    requested_target: Option<BlockPos>,
}

impl Default for WaterBoundPathNavigation {
    fn default() -> Self {
        Self::new(false)
    }
}

impl WaterBoundPathNavigation {
    #[must_use]
    pub fn new(allow_breaching: bool) -> Self {
        Self {
            inner: PathNavigation::new(EvaluatorKind::Swim(SwimNodeEvaluator::new(
                allow_breaching,
            ))),
            allow_breaching,
            steered_by_move_control: false,
            wanted_position: None,
            requested_target: None,
        }
    }

    /// Hands each waypoint to the mob's move control, like vanilla.
    #[must_use]
    pub const fn steered_by_move_control(mut self) -> Self {
        self.steered_by_move_control = true;
        self
    }

    /// Vanilla `PathNavigation.tick` past the path recomputation, with
    /// `WaterBoundPathNavigation`'s overrides.
    fn follow_for_move_control(&mut self, entity: &LivingEntity) {
        self.wanted_position = None;
        if self.is_done() {
            self.inner.finish_navigation(entity);
            return;
        }
        let pos = entity.entity.pos.load();
        // `WaterBoundPathNavigation.getTempMobPos`.
        let temp_mob_pos =
            Vector3::new(pos.x, pos.y + f64::from(self.inner.mob_height) * 0.5, pos.z);
        let world = entity.entity.world.load();
        let Some(path) = self.inner.path.as_mut() else {
            return;
        };
        let width = self.inner.mob_width;
        // `WaterBoundPathNavigation.canUpdatePath`.
        if self.allow_breaching
            || entity.entity.is_in_water()
            || entity.entity.touching_lava.load(Ordering::Relaxed)
        {
            // `PathNavigation.followThePath`.
            let max_distance = f64::from(if width > 0.75 {
                width / 2.0
            } else {
                0.75 - width / 2.0
            });
            if let Some(node) = path.get_next_node_pos() {
                let close_enough = (pos.x - (f64::from(node.0.x) + 0.5)).abs() < max_distance
                    && (pos.z - (f64::from(node.0.z) + 0.5)).abs() < max_distance
                    // `WaterBoundPathNavigation.getMaxVerticalDistanceToWaypoint`.
                    && (pos.y - f64::from(node.0.y)).abs() < 0.5;
                let can_cut_corner = path.get_next_node().is_some_and(|n| {
                    n.path_type != PathType::DangerFire
                        && n.path_type != PathType::DangerOther
                        && n.path_type != PathType::WalkableDoor
                });
                if close_enough
                    || can_cut_corner
                        && Self::should_target_next_node_in_direction(
                            &world,
                            temp_mob_pos,
                            path,
                            width,
                            self.inner.mob_height,
                        )
                {
                    path.advance();
                }
            }
        } else if let Some(next) = next_entity_pos(path, width)
            && temp_mob_pos.y > next.y
            && !entity.entity.on_ground.load(Ordering::Relaxed)
            && temp_mob_pos.x.floor() == next.x.floor()
            && temp_mob_pos.z.floor() == next.z.floor()
        {
            path.advance();
        }

        if path.is_done() {
            self.inner.finish_navigation(entity);
            return;
        }
        // `WaterBoundPathNavigation.getGroundY` keeps the node's own height.
        self.wanted_position =
            next_entity_pos(path, width).map(|target| (target, self.inner.speed_modifier));
    }

    /// Vanilla `PathNavigation.shouldTargetNextNodeInDirection`, whose
    /// `canMoveDirectly` is `WaterBoundPathNavigation`'s.
    fn should_target_next_node_in_direction(
        world: &World,
        mob_pos: Vector3<f64>,
        path: &Path,
        width: f32,
        height: f32,
    ) -> bool {
        if path.get_next_node_index() + 1 >= path.get_node_count() {
            return false;
        }
        let Some(current) = path.get_next_node_pos() else {
            return false;
        };
        let current_node = Vector3::new(
            f64::from(current.0.x) + 0.5,
            f64::from(current.0.y),
            f64::from(current.0.z) + 0.5,
        );
        if mob_pos.squared_distance_to_vec(&current_node) >= 4.0 {
            return false;
        }
        if let Some(next) = next_entity_pos(path, width)
            && PathNavigation::can_move_directly(world, mob_pos, next, height, false)
        {
            return true;
        }
        PathNavigation::should_target_next_node_in_direction(mob_pos, path)
    }
}

/// Vanilla `Path.getNextEntityPos`.
fn next_entity_pos(path: &Path, width: f32) -> Option<Vector3<f64>> {
    let node = path.get_next_node_pos()?;
    let offset = f64::from((width + 1.0) as i32) * 0.5;
    Some(Vector3::new(
        f64::from(node.0.x) + offset,
        f64::from(node.0.y),
        f64::from(node.0.z) + offset,
    ))
}

impl PathNavigationTrait for WaterBoundPathNavigation {
    fn set_progress(&mut self, goal: NavigatorGoal) {
        if self.steered_by_move_control {
            // Vanilla `PathNavigation.createPath` hands back the current path while
            // it is unfinished and still asked for the same block, so a goal that
            // re-targets every tick keeps following it.
            let target = BlockPos::floored_v(goal.destination);
            if self.requested_target == Some(target) && !self.is_done() {
                self.inner.speed_modifier = goal.speed;
                return;
            }
            self.requested_target = Some(target);
        }
        self.inner.set_progress(goal);
    }

    fn take_wanted_position(&mut self) -> Option<(Vector3<f64>, f64)> {
        self.wanted_position.take()
    }

    fn set_speed(&mut self, speed: f64) {
        self.inner.set_speed(speed);
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn is_idle(&self) -> bool {
        self.inner.is_idle.load(Ordering::Relaxed)
    }

    fn is_done(&self) -> bool {
        self.inner.path.as_ref().is_none_or(Path::is_done)
    }

    fn is_in_progress(&self) -> bool {
        !self.is_done()
    }

    fn is_stuck(&self) -> bool {
        self.inner.is_stuck
    }

    fn get_path(&self) -> Option<&Path> {
        self.inner.path.as_ref()
    }

    fn get_path_mut(&mut self) -> Option<&mut Path> {
        self.inner.path.as_mut()
    }

    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.inner.set_pathfinding_malus(path_type, malus);
    }

    fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.inner.get_pathfinding_malus(path_type)
    }

    fn path_type_overrides(&self) -> &FxHashMap<PathType, f32> {
        &self.inner.path_type_overrides
    }

    fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.inner.set_mob_dimensions(width, height);
    }

    fn can_reach_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.inner.can_reach_within(entity, destination, distance)
    }

    #[allow(clippy::too_many_lines)]
    fn tick(&mut self, entity: &LivingEntity) {
        self.inner.tick_count += 1;
        let world_age = entity.entity.world.load().get_world_age() as u64;

        if self.inner.has_delayed_recomputation {
            self.recompute_path(entity);
        }

        if let Some(goal) = self.inner.current_goal.take() {
            if self.inner.needs_new_path(&goal) {
                self.inner.path = self.inner.compute_path(entity, goal.destination, 1);
                self.inner.ticks_on_current_node = 0;
                self.inner.last_node_index = 0;
                self.inner.path_start_pos = Some(entity.entity.pos.load());
                self.inner.repath_cooldown = 15;
                self.inner.time_last_recompute = world_age;
            }
            self.inner.current_goal = Some(goal);
        }

        let mob_pos = Vector3::new(
            entity.entity.pos.load().x,
            entity.entity.pos.load().y + f64::from(self.inner.mob_height) * 0.5,
            entity.entity.pos.load().z,
        );
        self.inner.do_stuck_detection(mob_pos, entity);

        if self.steered_by_move_control {
            self.follow_for_move_control(entity);
            return;
        }

        if self.is_done() {
            self.inner.finish_navigation(entity);
        } else {
            if let Some(path) = &mut self.inner.path
                && let Some(pos) = path.get_next_node_pos()
            {
                let target_pos = Vector3::new(
                    f64::from(pos.0.x) + 0.5,
                    f64::from(pos.0.y) + 0.5,
                    f64::from(pos.0.z) + 0.5,
                );
                let current_pos = entity.entity.pos.load();
                let dx = target_pos.x - current_pos.x;
                let dy = target_pos.y - current_pos.y;
                let dz = target_pos.z - current_pos.z;
                let dist_sq = dx * dx + dy * dy + dz * dz;

                if dist_sq < 0.5 * 0.5 {
                    path.advance();
                }
            }

            if !self.is_done()
                && let Some(path) = &self.inner.path
                && let Some(next_block) = path.get_next_node_pos()
            {
                let target_pos = Vector3::new(
                    f64::from(next_block.0.x) + 0.5,
                    f64::from(next_block.0.y) + 0.5,
                    f64::from(next_block.0.z) + 0.5,
                );
                let current_pos = entity.entity.pos.load();
                let dx = target_pos.x - current_pos.x;
                let dy = target_pos.y - current_pos.y;
                let dz = target_pos.z - current_pos.z;
                let sd = dx.hypot(dz);

                let desired_yaw = wrap_degrees((dz.atan2(dx) as f32).to_degrees() - 90.0);
                let desired_pitch = wrap_degrees(-(dy.atan2(sd) as f32).to_degrees());

                entity.entity.yaw.store(desired_yaw);
                entity.entity.head_yaw.store(desired_yaw);
                entity.entity.body_yaw.store(desired_yaw);
                entity.entity.pitch.store(desired_pitch);

                let speed = self.inner.speed_modifier
                    * entity.get_attribute_value(&Attributes::MOVEMENT_SPEED);
                let y_input = if dy.abs() > 0.1 {
                    if dy > 0.0 { speed } else { -speed }
                } else {
                    0.0
                };
                entity.speed.store(speed as f32);
                entity
                    .movement_input
                    .store(Vector3::new(0.0, y_input, speed));
            }
        }
    }

    fn move_to_coords(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        speed: f64,
        entity: &LivingEntity,
    ) -> bool {
        let pos = entity.entity.pos.load();
        self.set_progress(NavigatorGoal::new(pos, Vector3::new(x, y, z), speed));
        true
    }

    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool {
        let p = entity.entity.pos.load();
        let target = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + 0.5,
            f64::from(pos.0.z) + 0.5,
        );
        self.set_progress(NavigatorGoal::new(p, target, speed));
        true
    }

    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, entity: &LivingEntity) -> bool {
        let p = entity.entity.pos.load();
        let target_pos = target.entity.pos.load();
        self.set_progress(NavigatorGoal::new(p, target_pos, speed));
        true
    }

    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool {
        let Some(new_path) = path else {
            self.inner.clear_path();
            return false;
        };
        let pos = entity.entity.pos.load();
        let stuck_check_pos =
            Vector3::new(pos.x, pos.y + f64::from(self.inner.mob_height) * 0.5, pos.z);
        self.inner
            .adopt_path(new_path, speed, entity, stuck_check_pos)
    }

    fn create_path(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        self.inner.compute_path(entity, destination, reach_range)
    }

    fn create_path_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
        max_path_length: f32,
    ) -> Option<Path> {
        self.inner
            .compute_path_within(entity, destination, reach_range, max_path_length)
    }

    fn recompute_path(&mut self, entity: &LivingEntity) {
        let world_age = entity.entity.world.load().get_world_age() as u64;
        if world_age.saturating_sub(self.inner.time_last_recompute) <= 20 {
            self.inner.has_delayed_recomputation = true;
        } else if let Some(target_pos) = self.inner.target_pos {
            let target_v = Vector3::new(
                f64::from(target_pos.0.x) + 0.5,
                f64::from(target_pos.0.y) + 0.5,
                f64::from(target_pos.0.z) + 0.5,
            );
            self.inner.path = self
                .inner
                .compute_path(entity, target_v, self.inner.reach_range);
            self.inner.time_last_recompute = world_age;
            self.inner.has_delayed_recomputation = false;
        }
    }

    fn set_avoid_sun(&mut self, avoid_sun: bool) {
        self.inner.avoid_sun = avoid_sun;
    }

    fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        self.inner.can_walk_over_fences = can_walk;
    }

    fn set_can_open_doors(&mut self, can_open: bool) {
        self.inner.can_open_doors = can_open;
    }

    fn set_can_pass_doors(&mut self, can_pass: bool) {
        self.inner.can_pass_doors = can_pass;
    }

    fn set_can_float(&mut self, _can_float: bool) {}

    fn can_float(&self) -> bool {
        true
    }

    fn can_navigate_ground(&self) -> bool {
        false
    }

    fn set_required_path_length(&mut self, length: f32, entity: &LivingEntity) {
        self.inner.set_required_path_length(length, entity);
    }

    fn update_pathfinder_max_visited_nodes(&mut self, entity: &LivingEntity) {
        self.inner.update_pathfinder_max_visited_nodes(entity);
    }

    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32) {
        self.inner.max_visited_nodes_multiplier = multiplier;
    }

    fn reset_max_visited_nodes_multiplier(&mut self) {
        self.inner.max_visited_nodes_multiplier = 1.0;
    }

    fn get_target_pos(&self) -> Option<BlockPos> {
        self.inner.target_pos
    }

    fn can_path_to_targets_below_surface(&self) -> bool {
        self.inner.can_path_to_targets_below_surface
    }

    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool) {
        self.inner.can_path_to_targets_below_surface = can_path;
    }

    /// A swimmer can hold still anywhere it is not inside a block.
    fn is_stable_destination(&self, world: &World, pos: &BlockPos) -> bool {
        !world.get_block_state(pos).is_solid_render()
    }
}

pub struct WallClimberNavigation {
    pub inner: GroundPathNavigation,
    pub path_to_position: Option<BlockPos>,
}

impl Default for WallClimberNavigation {
    fn default() -> Self {
        Self::new()
    }
}

impl WallClimberNavigation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: GroundPathNavigation::new(),
            path_to_position: None,
        }
    }
}

impl PathNavigationTrait for WallClimberNavigation {
    fn take_wanted_position(&mut self) -> Option<(Vector3<f64>, f64)> {
        self.inner.take_wanted_position()
    }

    fn set_progress(&mut self, goal: NavigatorGoal) {
        self.path_to_position = Some(BlockPos::floored_v(goal.destination));
        self.inner.set_progress(goal);
    }

    fn set_speed(&mut self, speed: f64) {
        self.inner.set_speed(speed);
    }

    fn stop(&mut self) {
        self.path_to_position = None;
        self.inner.stop();
    }

    fn is_idle(&self) -> bool {
        self.inner.is_idle()
    }

    fn is_done(&self) -> bool {
        self.inner.is_done()
    }

    fn is_in_progress(&self) -> bool {
        !self.is_done()
    }

    fn is_stuck(&self) -> bool {
        self.inner.is_stuck()
    }

    fn get_path(&self) -> Option<&Path> {
        self.inner.get_path()
    }

    fn get_path_mut(&mut self) -> Option<&mut Path> {
        self.inner.get_path_mut()
    }

    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.inner.set_pathfinding_malus(path_type, malus);
    }

    fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.inner.get_pathfinding_malus(path_type)
    }

    fn path_type_overrides(&self) -> &FxHashMap<PathType, f32> {
        self.inner.path_type_overrides()
    }

    fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.inner.set_mob_dimensions(width, height);
    }

    fn can_reach_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.inner.can_reach_within(entity, destination, distance)
    }

    /// Vanilla `WallClimberNavigation.tick`: with no path left, head straight for the block
    /// the last path was asked for, which lets spiders climb to it.
    fn tick(&mut self, entity: &LivingEntity) {
        self.inner.inner.resolve_pending_move(entity);
        if !self.inner.is_done() {
            self.inner.tick(entity);
        } else if let Some(target) = self.path_to_position {
            let pos = entity.entity.pos.load();
            let width = f64::from(entity.entity.entity_dimension.load().width);
            let closer_to_center = |block: BlockPos| {
                let center = block.to_centered_f64();
                (center - pos).length_squared() < width * width
            };
            if !closer_to_center(target)
                && (pos.y <= f64::from(target.0.y)
                    || !closer_to_center(BlockPos::floored(
                        f64::from(target.0.x),
                        pos.y,
                        f64::from(target.0.z),
                    )))
            {
                self.inner.inner.wanted_position = Some((
                    Vector3::new(
                        f64::from(target.0.x),
                        f64::from(target.0.y),
                        f64::from(target.0.z),
                    ),
                    self.inner.inner.speed_modifier,
                ));
            } else {
                self.path_to_position = None;
            }
        }
    }

    fn move_to_coords(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        speed: f64,
        entity: &LivingEntity,
    ) -> bool {
        self.path_to_position = Some(BlockPos::new(
            x.floor() as i32,
            y.floor() as i32,
            z.floor() as i32,
        ));
        self.inner.move_to_coords(x, y, z, speed, entity)
    }

    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool {
        self.path_to_position = Some(pos);
        self.inner.move_to_pos(pos, speed, entity)
    }

    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, entity: &LivingEntity) -> bool {
        self.inner.inner.pending_move = None;
        let target_pos = target.entity.block_pos.load();
        self.path_to_position = Some(target_pos);
        let path = self.inner.inner.ground_create_path(entity, target_pos, 0);
        if path.is_some() {
            self.inner.inner.ground_move_to_path(entity, path, speed)
        } else {
            self.path_to_position = Some(target_pos);
            self.inner.inner.speed_modifier = speed;
            true
        }
    }

    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool {
        self.inner.move_to_path(path, speed, entity)
    }

    fn create_path(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        self.path_to_position = Some(BlockPos::floored_v(destination));
        self.inner.create_path(entity, destination, reach_range)
    }

    fn create_path_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
        max_path_length: f32,
    ) -> Option<Path> {
        self.path_to_position = Some(BlockPos::floored_v(destination));
        self.inner
            .create_path_within(entity, destination, reach_range, max_path_length)
    }

    fn recompute_path(&mut self, entity: &LivingEntity) {
        self.inner.recompute_path(entity);
    }

    fn set_avoid_sun(&mut self, avoid_sun: bool) {
        self.inner.set_avoid_sun(avoid_sun);
    }

    fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        self.inner.set_can_walk_over_fences(can_walk);
    }

    fn set_can_open_doors(&mut self, can_open: bool) {
        self.inner.set_can_open_doors(can_open);
    }

    fn set_can_pass_doors(&mut self, can_pass: bool) {
        self.inner.set_can_pass_doors(can_pass);
    }

    fn set_can_float(&mut self, can_float: bool) {
        self.inner.set_can_float(can_float);
    }

    fn can_float(&self) -> bool {
        self.inner.can_float()
    }

    fn can_navigate_ground(&self) -> bool {
        true
    }

    fn set_required_path_length(&mut self, length: f32, entity: &LivingEntity) {
        self.inner.set_required_path_length(length, entity);
    }

    fn update_pathfinder_max_visited_nodes(&mut self, entity: &LivingEntity) {
        self.inner.update_pathfinder_max_visited_nodes(entity);
    }

    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32) {
        self.inner.set_max_visited_nodes_multiplier(multiplier);
    }

    fn reset_max_visited_nodes_multiplier(&mut self) {
        self.inner.reset_max_visited_nodes_multiplier();
    }

    fn get_target_pos(&self) -> Option<BlockPos> {
        self.inner.get_target_pos()
    }

    fn can_path_to_targets_below_surface(&self) -> bool {
        self.inner.can_path_to_targets_below_surface()
    }

    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool) {
        self.inner.set_can_path_to_targets_below_surface(can_path);
    }
}

pub struct AmphibiousPathNavigation {
    pub inner: PathNavigation,
    /// Hand waypoints to the mob's move control, as vanilla's `PathNavigation.tick`
    /// does, instead of steering the mob here.
    steered_by_move_control: bool,
    wanted_position: Option<(Vector3<f64>, f64)>,
    /// Vanilla `TurtlePathNavigation`: while this is set (the turtle has a travel
    /// target), only water is a stable destination.
    water_destinations: Option<Arc<AtomicBool>>,
    /// Vanilla `Frog.FrogPathNavigation.canCutCorner`: never across a water border.
    keeps_water_border_corners: bool,
}

impl Default for AmphibiousPathNavigation {
    fn default() -> Self {
        Self::new(false)
    }
}

impl AmphibiousPathNavigation {
    #[must_use]
    pub fn new(prefers_shallow_swimming: bool) -> Self {
        Self {
            inner: PathNavigation::new(EvaluatorKind::Amphibious(AmphibiousNodeEvaluator::new(
                prefers_shallow_swimming,
            ))),
            steered_by_move_control: false,
            wanted_position: None,
            water_destinations: None,
            keeps_water_border_corners: false,
        }
    }

    /// Vanilla `Frog.FrogPathNavigation`.
    #[must_use]
    pub fn frog() -> Self {
        Self {
            inner: PathNavigation::new(EvaluatorKind::Amphibious(AmphibiousNodeEvaluator::frog())),
            keeps_water_border_corners: true,
            ..Self::new(true)
        }
    }

    /// Vanilla `Turtle.TurtlePathNavigation`, with `travelling` standing for
    /// `travelPos != null`.
    #[must_use]
    pub fn turtle(travelling: Arc<AtomicBool>) -> Self {
        Self {
            water_destinations: Some(travelling),
            ..Self::new(false)
        }
    }

    /// Leave the steering to the mob's move control, which reads the next waypoint
    /// through `take_wanted_position`.
    #[must_use]
    pub const fn steered_by_move_control(mut self) -> Self {
        self.steered_by_move_control = true;
        self
    }

    /// Vanilla `PathNavigation.tick` for `AmphibiousPathNavigation`, whose
    /// `canUpdatePath` is always true and whose `getGroundY` keeps the node's height.
    fn tick_for_move_control(&mut self, entity: &LivingEntity) {
        self.wanted_position = None;
        self.inner.tick_count += 1;
        let world_age = entity.entity.world.load().get_world_age() as u64;
        if self.inner.has_delayed_recomputation {
            self.recompute_path(entity);
        }
        if let Some(goal) = self.inner.current_goal.take() {
            if self.inner.needs_new_path(&goal) {
                self.inner.path =
                    self.inner
                        .compute_path(entity, goal.destination, self.inner.reach_range);
                self.inner.ticks_on_current_node = 0;
                self.inner.last_node_index = 0;
                self.inner.path_start_pos = Some(entity.entity.pos.load());
                self.inner.repath_cooldown = 15;
                self.inner.time_last_recompute = world_age;
            }
            self.inner.current_goal = Some(goal);
        }
        if self.is_done() {
            self.inner.finish_navigation(entity);
            return;
        }

        let pos = entity.entity.pos.load();
        // `AmphibiousPathNavigation.getTempMobPos`.
        let temp_mob_pos =
            Vector3::new(pos.x, pos.y + f64::from(self.inner.mob_height) * 0.5, pos.z);
        let in_liquid =
            entity.entity.is_in_water() || entity.entity.touching_lava.load(Ordering::Relaxed);
        let world = entity.entity.world.load();
        let width = self.inner.mob_width;
        let height = self.inner.mob_height;
        if let Some(path) = self.inner.path.as_mut() {
            // `PathNavigation.followThePath`.
            let max_distance = f64::from(if width > 0.75 {
                width / 2.0
            } else {
                0.75 - width / 2.0
            });
            if let Some(node) = path.get_next_node_pos() {
                let close_enough = (pos.x - (f64::from(node.0.x) + 0.5)).abs() < max_distance
                    && (pos.z - (f64::from(node.0.z) + 0.5)).abs() < max_distance
                    && (pos.y - f64::from(node.0.y)).abs() < 1.0;
                let keeps_water_border_corners = self.keeps_water_border_corners;
                let can_cut_corner = path.get_next_node().is_some_and(|n| {
                    !(keeps_water_border_corners && n.path_type == PathType::WaterBorder)
                        && n.path_type != PathType::DangerFire
                        && n.path_type != PathType::DangerOther
                        && n.path_type != PathType::WalkableDoor
                });
                if close_enough
                    || can_cut_corner
                        && Self::should_target_next_node_in_direction(
                            &world,
                            temp_mob_pos,
                            path,
                            width,
                            height,
                            in_liquid,
                        )
                {
                    path.advance();
                }
            }
        }
        self.inner.do_stuck_detection(temp_mob_pos, entity);
        if self.is_done() {
            self.inner.finish_navigation(entity);
            return;
        }
        self.wanted_position = self
            .inner
            .path
            .as_ref()
            .and_then(|path| next_entity_pos(path, width))
            .map(|target| (target, self.inner.speed_modifier));
    }

    /// Vanilla `PathNavigation.shouldTargetNextNodeInDirection`, with
    /// `AmphibiousPathNavigation.canMoveDirectly`: only straight swims in liquid.
    fn should_target_next_node_in_direction(
        world: &World,
        mob_pos: Vector3<f64>,
        path: &Path,
        width: f32,
        height: f32,
        in_liquid: bool,
    ) -> bool {
        if path.get_next_node_index() + 1 >= path.get_node_count() {
            return false;
        }
        let Some(current) = path.get_next_node_pos() else {
            return false;
        };
        let current_node = Vector3::new(
            f64::from(current.0.x) + 0.5,
            f64::from(current.0.y),
            f64::from(current.0.z) + 0.5,
        );
        if mob_pos.squared_distance_to_vec(&current_node) >= 4.0 {
            return false;
        }
        if in_liquid
            && let Some(next) = next_entity_pos(path, width)
            && PathNavigation::can_move_directly(world, mob_pos, next, height, false)
        {
            return true;
        }
        PathNavigation::should_target_next_node_in_direction(mob_pos, path)
    }
}

impl PathNavigationTrait for AmphibiousPathNavigation {
    fn set_progress(&mut self, goal: NavigatorGoal) {
        self.inner.set_progress(goal);
    }

    fn set_speed(&mut self, speed: f64) {
        self.inner.set_speed(speed);
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn is_idle(&self) -> bool {
        self.inner.is_idle.load(Ordering::Relaxed)
    }

    fn is_done(&self) -> bool {
        self.inner.path.as_ref().is_none_or(Path::is_done)
    }

    fn is_in_progress(&self) -> bool {
        !self.is_done()
    }

    fn is_stuck(&self) -> bool {
        self.inner.is_stuck
    }

    fn get_path(&self) -> Option<&Path> {
        self.inner.path.as_ref()
    }

    fn get_path_mut(&mut self) -> Option<&mut Path> {
        self.inner.path.as_mut()
    }

    fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.inner.set_pathfinding_malus(path_type, malus);
    }

    fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.inner.get_pathfinding_malus(path_type)
    }

    fn path_type_overrides(&self) -> &FxHashMap<PathType, f32> {
        &self.inner.path_type_overrides
    }

    fn set_mob_dimensions(&mut self, width: f32, height: f32) {
        self.inner.set_mob_dimensions(width, height);
    }

    fn can_reach_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        distance: f32,
    ) -> bool {
        self.inner.can_reach_within(entity, destination, distance)
    }

    #[allow(clippy::too_many_lines)]
    /// Vanilla `AmphibiousPathNavigation.isStableDestination`: anything but air below.
    fn is_stable_destination(&self, world: &World, pos: &BlockPos) -> bool {
        if self
            .water_destinations
            .as_ref()
            .is_some_and(|travelling| travelling.load(Ordering::Relaxed))
        {
            return world.get_block(pos) == &pumpkin_data::Block::WATER;
        }
        !world.get_block_state(&pos.down()).is_air()
    }

    fn take_wanted_position(&mut self) -> Option<(Vector3<f64>, f64)> {
        self.wanted_position.take()
    }

    fn take_jump(&mut self) -> bool {
        std::mem::take(&mut self.inner.jump)
    }

    fn tick(&mut self, entity: &LivingEntity) {
        if self.steered_by_move_control {
            self.tick_for_move_control(entity);
            return;
        }
        if entity.entity.touching_water.load(Ordering::Relaxed) {
            self.inner.tick_count += 1;
            let world_age = entity.entity.world.load().get_world_age() as u64;

            if self.inner.has_delayed_recomputation {
                self.recompute_path(entity);
            }

            if let Some(goal) = self.inner.current_goal.take() {
                if self.inner.needs_new_path(&goal) {
                    self.inner.path = self.inner.compute_path(entity, goal.destination, 1);
                    self.inner.ticks_on_current_node = 0;
                    self.inner.last_node_index = 0;
                    self.inner.path_start_pos = Some(entity.entity.pos.load());
                    self.inner.repath_cooldown = 15;
                    self.inner.time_last_recompute = world_age;
                }
                self.inner.current_goal = Some(goal);
            }

            let mob_pos = Vector3::new(
                entity.entity.pos.load().x,
                entity.entity.pos.load().y + f64::from(self.inner.mob_height) * 0.5,
                entity.entity.pos.load().z,
            );
            self.inner.do_stuck_detection(mob_pos, entity);

            if self.is_done() {
                self.inner.finish_navigation(entity);
            } else {
                if let Some(path) = &mut self.inner.path
                    && let Some(pos) = path.get_next_node_pos()
                {
                    let target_pos = Vector3::new(
                        f64::from(pos.0.x) + 0.5,
                        f64::from(pos.0.y) + 0.5,
                        f64::from(pos.0.z) + 0.5,
                    );
                    let current_pos = entity.entity.pos.load();
                    let dx = target_pos.x - current_pos.x;
                    let dy = target_pos.y - current_pos.y;
                    let dz = target_pos.z - current_pos.z;
                    let dist_sq = dx * dx + dy * dy + dz * dz;

                    if dist_sq < 0.5 * 0.5 {
                        path.advance();
                    }
                }

                if !self.is_done()
                    && let Some(path) = &self.inner.path
                    && let Some(next_block) = path.get_next_node_pos()
                {
                    let target_pos = Vector3::new(
                        f64::from(next_block.0.x) + 0.5,
                        f64::from(next_block.0.y) + 0.5,
                        f64::from(next_block.0.z) + 0.5,
                    );
                    let current_pos = entity.entity.pos.load();
                    let dx = target_pos.x - current_pos.x;
                    let dy = target_pos.y - current_pos.y;
                    let dz = target_pos.z - current_pos.z;
                    let sd = dx.hypot(dz);

                    let desired_yaw = wrap_degrees((dz.atan2(dx) as f32).to_degrees() - 90.0);
                    let desired_pitch = wrap_degrees(-(dy.atan2(sd) as f32).to_degrees());

                    entity.entity.yaw.store(desired_yaw);
                    entity.entity.head_yaw.store(desired_yaw);
                    entity.entity.body_yaw.store(desired_yaw);
                    entity.entity.pitch.store(desired_pitch);

                    let speed = self.inner.speed_modifier
                        * entity.get_attribute_value(&Attributes::MOVEMENT_SPEED);
                    let y_input = if dy.abs() > 0.1 {
                        if dy > 0.0 { speed } else { -speed }
                    } else {
                        0.0
                    };
                    entity.speed.store(speed as f32);
                    entity
                        .movement_input
                        .store(Vector3::new(0.0, y_input, speed));
                }
            }
        } else {
            self.inner.tick_ground(entity);
        }
    }

    fn move_to_coords(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        speed: f64,
        entity: &LivingEntity,
    ) -> bool {
        let pos = entity.entity.pos.load();
        self.set_progress(NavigatorGoal::new(pos, Vector3::new(x, y, z), speed));
        true
    }

    fn move_to_pos(&mut self, pos: BlockPos, speed: f64, entity: &LivingEntity) -> bool {
        let p = entity.entity.pos.load();
        let target = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + 0.5,
            f64::from(pos.0.z) + 0.5,
        );
        self.set_progress(NavigatorGoal::new(p, target, speed));
        true
    }

    fn move_to_entity(&mut self, target: &LivingEntity, speed: f64, entity: &LivingEntity) -> bool {
        let p = entity.entity.pos.load();
        let target_pos = target.entity.pos.load();
        self.set_progress(NavigatorGoal::new(p, target_pos, speed));
        true
    }

    fn move_to_path(&mut self, path: Option<Path>, speed: f64, entity: &LivingEntity) -> bool {
        let Some(new_path) = path else {
            self.inner.clear_path();
            return false;
        };
        let pos = entity.entity.pos.load();
        let stuck_check_pos =
            Vector3::new(pos.x, pos.y + f64::from(self.inner.mob_height) * 0.5, pos.z);
        self.inner
            .adopt_path(new_path, speed, entity, stuck_check_pos)
    }

    fn create_path(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
    ) -> Option<Path> {
        self.inner.compute_path(entity, destination, reach_range)
    }

    fn create_path_within(
        &mut self,
        entity: &LivingEntity,
        destination: Vector3<f64>,
        reach_range: i32,
        max_path_length: f32,
    ) -> Option<Path> {
        self.inner
            .compute_path_within(entity, destination, reach_range, max_path_length)
    }

    fn recompute_path(&mut self, entity: &LivingEntity) {
        let world_age = entity.entity.world.load().get_world_age() as u64;
        if world_age.saturating_sub(self.inner.time_last_recompute) <= 20 {
            self.inner.has_delayed_recomputation = true;
        } else if let Some(target_pos) = self.inner.target_pos {
            let target_v = Vector3::new(
                f64::from(target_pos.0.x) + 0.5,
                f64::from(target_pos.0.y) + 0.5,
                f64::from(target_pos.0.z) + 0.5,
            );
            self.inner.path = self
                .inner
                .compute_path(entity, target_v, self.inner.reach_range);
            self.inner.time_last_recompute = world_age;
            self.inner.has_delayed_recomputation = false;
        }
    }

    fn set_avoid_sun(&mut self, avoid_sun: bool) {
        self.inner.avoid_sun = avoid_sun;
    }

    fn set_can_walk_over_fences(&mut self, can_walk: bool) {
        self.inner.can_walk_over_fences = can_walk;
    }

    fn set_can_open_doors(&mut self, can_open: bool) {
        self.inner.can_open_doors = can_open;
    }

    fn set_can_pass_doors(&mut self, can_pass: bool) {
        self.inner.can_pass_doors = can_pass;
    }

    fn set_can_float(&mut self, _can_float: bool) {}

    fn can_float(&self) -> bool {
        true
    }

    fn can_navigate_ground(&self) -> bool {
        true
    }

    fn set_required_path_length(&mut self, length: f32, entity: &LivingEntity) {
        self.inner.set_required_path_length(length, entity);
    }

    fn update_pathfinder_max_visited_nodes(&mut self, entity: &LivingEntity) {
        self.inner.update_pathfinder_max_visited_nodes(entity);
    }

    fn set_max_visited_nodes_multiplier(&mut self, multiplier: f32) {
        self.inner.max_visited_nodes_multiplier = multiplier;
    }

    fn reset_max_visited_nodes_multiplier(&mut self) {
        self.inner.max_visited_nodes_multiplier = 1.0;
    }

    fn get_target_pos(&self) -> Option<BlockPos> {
        self.inner.target_pos
    }

    fn can_path_to_targets_below_surface(&self) -> bool {
        self.inner.can_path_to_targets_below_surface
    }

    fn set_can_path_to_targets_below_surface(&mut self, can_path: bool) {
        self.inner.can_path_to_targets_below_surface = can_path;
    }
}

pub struct Navigator {
    inner: Box<dyn PathNavigationTrait>,
}

impl Default for Navigator {
    fn default() -> Self {
        Self::ground()
    }
}

impl Navigator {
    #[must_use]
    pub fn new<N: PathNavigationTrait + 'static>(nav: N) -> Self {
        Self {
            inner: Box::new(nav),
        }
    }

    #[must_use]
    pub fn ground() -> Self {
        Self::new(GroundPathNavigation::new())
    }

    #[must_use]
    pub fn flying() -> Self {
        Self::new(FlyingPathNavigation::new())
    }

    #[must_use]
    pub fn water_bound(allow_breaching: bool) -> Self {
        Self::new(WaterBoundPathNavigation::new(allow_breaching))
    }

    #[must_use]
    pub fn wall_climber() -> Self {
        Self::new(WallClimberNavigation::new())
    }

    #[must_use]
    pub fn amphibious(prefers_shallow_swimming: bool) -> Self {
        Self::new(AmphibiousPathNavigation::new(prefers_shallow_swimming))
    }
}

impl Deref for Navigator {
    type Target = dyn PathNavigationTrait;

    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}

impl DerefMut for Navigator {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.inner
    }
}
