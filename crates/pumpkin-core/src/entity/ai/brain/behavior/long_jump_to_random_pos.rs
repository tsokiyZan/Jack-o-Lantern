use pumpkin_util::random::RandomImpl;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use pumpkin_data::attributes::Attributes;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{Tag, Taggable};
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::seq::SliceRandom;

use super::super::BrainTick;
use super::super::memory::position_tracker::{BlockPosTracker, PositionTracker};
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::timed::Behavior;
use crate::entity::ai::pathfinder::pathfinding_context::PathfindingContext;
use crate::entity::ai::pathfinder::walk_node_evaluator::WalkNodeEvaluator;
use crate::entity::mob::Mob;
use crate::world::World;

/// Vanilla `LongJumpToRandomPos.FIND_JUMP_TRIES`.
const FIND_JUMP_TRIES: i32 = 20;
/// Vanilla `LongJumpToRandomPos.PREPARE_JUMP_DURATION`.
const PREPARE_JUMP_DURATION: i64 = 40;
/// Vanilla `LongJumpToRandomPos.MIN_PATHFIND_DISTANCE_TO_VALID_JUMP`.
const MIN_PATHFIND_DISTANCE_TO_VALID_JUMP: f32 = 8.0;
/// Vanilla `LongJumpToRandomPos.TIME_OUT_DURATION`.
const TIME_OUT_DURATION: i32 = 200;
/// Vanilla `LongJumpToRandomPos.ALLOWED_ANGLES`.
const ALLOWED_ANGLES: [i32; 4] = [65, 70, 75, 80];

const ENTRY_CONDITIONS: [(MemoryModuleId, MemoryStatus); 3] = [
    (types::LOOK_TARGET.id(), MemoryStatus::Registered),
    (
        types::LONG_JUMP_COOLDOWN_TICKS.id(),
        MemoryStatus::ValueAbsent,
    ),
    (types::LONG_JUMP_MID_JUMP.id(), MemoryStatus::ValueAbsent),
];

/// Vanilla `LongJumpToRandomPos.PossibleJump`.
#[derive(Clone, Copy)]
struct PossibleJump {
    target_pos: BlockPos,
    weight: i32,
}

/// Vanilla `LongJumpToPreferredBlock`'s extra state.
struct PreferredBlock {
    tag: &'static Tag,
    chance: f32,
    not_preferred_jump_candidates: Vec<PossibleJump>,
    currently_wanting_preferred_ones: bool,
}

pub type AcceptableLandingSpot = fn(&BrainTick<'_>, &BlockPos) -> bool;

/// Vanilla `LongJumpToRandomPos`: leaps to a spot it can reach by air but not on foot.
///
/// Built with [`LongJumpToRandomPos::to_preferred_block`] it is vanilla's
/// `LongJumpToPreferredBlock`.
pub struct LongJumpToRandomPos {
    /// Vanilla `timeBetweenLongJumps`, a `UniformInt` with both ends inclusive.
    time_between_long_jumps: (i32, i32),
    max_long_jump_height: i32,
    max_long_jump_width: i32,
    max_jump_velocity_multiplier: f32,
    get_jump_sound: fn(&dyn Mob) -> Sound,
    acceptable_landing_spot: AcceptableLandingSpot,
    preferred: Option<PreferredBlock>,
    jump_candidates: Vec<PossibleJump>,
    initial_position: Option<Vector3<f64>>,
    chosen_jump: Option<Vector3<f64>>,
    find_jump_tries: i32,
    prepare_jump_start: i64,
    /// What the last `can_still_use` decided; its side effects run in `stop`, the
    /// only place this port may write the brain.
    stopped_invalid: bool,
}

impl LongJumpToRandomPos {
    #[must_use]
    pub const fn new(
        time_between_long_jumps: (i32, i32),
        max_long_jump_height: i32,
        max_long_jump_width: i32,
        max_jump_velocity_multiplier: f32,
        get_jump_sound: fn(&dyn Mob) -> Sound,
        acceptable_landing_spot: AcceptableLandingSpot,
    ) -> Self {
        Self {
            time_between_long_jumps,
            max_long_jump_height,
            max_long_jump_width,
            max_jump_velocity_multiplier,
            get_jump_sound,
            acceptable_landing_spot,
            preferred: None,
            jump_candidates: Vec::new(),
            initial_position: None,
            chosen_jump: None,
            find_jump_tries: 0,
            prepare_jump_start: 0,
            stopped_invalid: false,
        }
    }

    /// Vanilla `LongJumpToPreferredBlock`.
    #[expect(
        clippy::too_many_arguments,
        reason = "vanilla's LongJumpToPreferredBlock constructor"
    )]
    #[must_use]
    pub fn to_preferred_block(
        time_between_long_jumps: (i32, i32),
        max_long_jump_height: i32,
        max_long_jump_width: i32,
        max_jump_velocity_multiplier: f32,
        get_jump_sound: fn(&dyn Mob) -> Sound,
        preferred_block_tag: &'static Tag,
        preferred_blocks_chance: f32,
        acceptable_landing_spot: AcceptableLandingSpot,
    ) -> Self {
        let mut behavior = Self::new(
            time_between_long_jumps,
            max_long_jump_height,
            max_long_jump_width,
            max_jump_velocity_multiplier,
            get_jump_sound,
            acceptable_landing_spot,
        );
        behavior.preferred = Some(PreferredBlock {
            tag: preferred_block_tag,
            chance: preferred_blocks_chance,
            not_preferred_jump_candidates: Vec::new(),
            currently_wanting_preferred_ones: false,
        });
        behavior
    }

    /// `timeBetweenLongJumps.sample(level.getRandom())` (LongJumpToRandomPos.java:96/109),
    /// `UniformInt`.
    fn sample_time_between_long_jumps(&self, world: &World) -> i32 {
        let (min, max) = self.time_between_long_jumps;
        world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_int_between_inclusive(min, max)
    }

    /// Vanilla `LongJumpToRandomPos.getJumpCandidate`: `WeightedRandom.getRandomItem`,
    /// removing the pick. Draws from `level.getRandom()` (LongJumpToRandomPos.java:179).
    fn get_random_jump_candidate(&mut self, world: &World) -> Option<PossibleJump> {
        let total_weight: i32 = self.jump_candidates.iter().map(|jump| jump.weight).sum();
        if total_weight <= 0 {
            return None;
        }
        let mut selection = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_bounded_i32(total_weight);
        let index = self.jump_candidates.iter().position(|jump| {
            selection -= jump.weight;
            selection < 0
        })?;
        Some(self.jump_candidates.remove(index))
    }

    /// Vanilla `LongJumpToPreferredBlock.getJumpCandidate`, falling back to
    /// `LongJumpToRandomPos.getJumpCandidate`.
    fn get_jump_candidate(&mut self, tick: &BrainTick<'_>) -> Option<PossibleJump> {
        let wanting_preferred = self
            .preferred
            .as_ref()
            .is_some_and(|preferred| preferred.currently_wanting_preferred_ones);
        if !wanting_preferred {
            return self.get_random_jump_candidate(tick.world);
        }
        while !self.jump_candidates.is_empty() {
            if let Some(jump) = self.get_random_jump_candidate(tick.world)
                && let Some(preferred) = self.preferred.as_mut()
            {
                if tick
                    .world
                    .get_block(&jump.target_pos.down())
                    .has_tag(preferred.tag)
                {
                    return Some(jump);
                }
                preferred.not_preferred_jump_candidates.push(jump);
            }
        }
        let preferred = self.preferred.as_mut()?;
        (!preferred.not_preferred_jump_candidates.is_empty())
            .then(|| preferred.not_preferred_jump_candidates.remove(0))
    }

    /// Vanilla `LongJumpToRandomPos.pickCandidate`.
    fn pick_candidate(&mut self, tick: &mut BrainTick<'_>) {
        while !self.jump_candidates.is_empty() {
            let Some(jump) = self.get_jump_candidate(tick) else {
                continue;
            };
            let target_pos = jump.target_pos;
            if !self.is_acceptable_landing_position(tick, &target_pos) {
                continue;
            }
            let target_position = target_pos.to_centered_f64();
            let Some(jump_vector) = self.calculate_optimal_jump_vector(tick.mob, target_position)
            else {
                continue;
            };
            tick.brain.set(
                types::LOOK_TARGET,
                Arc::new(BlockPosTracker::new(target_pos)) as Arc<dyn PositionTracker>,
            );
            let mob_entity = tick.mob.get_mob_entity();
            let path = mob_entity.with_navigation(|navigation, owner| {
                navigation.create_path_within(
                    &owner.living_entity,
                    target_pos.to_centered_f64(),
                    0,
                    MIN_PATHFIND_DISTANCE_TO_VALID_JUMP,
                )
            });
            if path.is_none_or(|path| !path.can_reach()) {
                self.chosen_jump = Some(jump_vector);
                self.prepare_jump_start = tick.time;
                return;
            }
        }
    }

    /// Vanilla `LongJumpToRandomPos.isAcceptableLandingPosition`.
    fn is_acceptable_landing_position(&self, tick: &BrainTick<'_>, target_pos: &BlockPos) -> bool {
        let body_pos = tick.mob.get_entity().block_pos.load();
        if body_pos.0.x == target_pos.0.x && body_pos.0.z == target_pos.0.z {
            return false;
        }
        (self.acceptable_landing_spot)(tick, target_pos)
    }

    /// Vanilla `LongJumpToRandomPos.calculateOptimalJumpVector`.
    fn calculate_optimal_jump_vector(
        &self,
        mob: &dyn Mob,
        target_pos: Vector3<f64>,
    ) -> Option<Vector3<f64>> {
        let mut allowed_angles = ALLOWED_ANGLES;
        allowed_angles.shuffle(&mut rand::rng());
        let max_jump_velocity = (mob
            .get_mob_entity()
            .living_entity
            .get_attribute_value(&Attributes::JUMP_STRENGTH)
            * f64::from(self.max_jump_velocity_multiplier)) as f32;
        allowed_angles.into_iter().find_map(|angle| {
            calculate_jump_vector_for_angle(mob, target_pos, max_jump_velocity, angle, true)
        })
    }
}

impl Behavior for LongJumpToRandomPos {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &ENTRY_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn max_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        let entity = tick.mob.get_entity();
        let can_start = entity.on_ground.load(Ordering::Relaxed)
            && !entity.is_in_water()
            && !entity.touching_lava.load(Ordering::Relaxed)
            && tick.world.get_block(&entity.block_pos.load()) != &pumpkin_data::Block::HONEY_BLOCK;
        if !can_start {
            let cooldown = self.sample_time_between_long_jumps(tick.world) / 2;
            tick.brain.set(types::LONG_JUMP_COOLDOWN_TICKS, cooldown);
        }
        can_start
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        let entity = tick.mob.get_entity();
        let is_valid = self.initial_position == Some(entity.pos.load())
            && self.find_jump_tries > 0
            && !entity.is_in_water()
            && (self.chosen_jump.is_some() || !self.jump_candidates.is_empty());
        self.stopped_invalid = !is_valid;
        is_valid
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        self.stopped_invalid = false;
        self.chosen_jump = None;
        self.find_jump_tries = FIND_JUMP_TRIES;
        let entity = tick.mob.get_entity();
        self.initial_position = Some(entity.pos.load());
        let mob_pos = entity.block_pos.load();
        let (width, height) = (self.max_long_jump_width, self.max_long_jump_height);
        self.jump_candidates = BlockPos::between_closed_coords(
            mob_pos.0.x - width,
            mob_pos.0.y - height,
            mob_pos.0.z - width,
            mob_pos.0.x + width,
            mob_pos.0.y + height,
            mob_pos.0.z + width,
        )
        .filter(|pos| *pos != mob_pos)
        .map(|pos| PossibleJump {
            target_pos: pos,
            weight: f64::from(mob_pos.squared_distance(&pos)).ceil() as i32,
        })
        .collect();
        if let Some(preferred) = self.preferred.as_mut() {
            preferred.not_preferred_jump_candidates.clear();
            preferred.currently_wanting_preferred_ones =
                tick.mob.get_entity().random().next_f32() < preferred.chance;
        }
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(chosen_jump) = self.chosen_jump {
            if tick.time - self.prepare_jump_start >= PREPARE_JUMP_DURATION {
                let living = &tick.mob.get_mob_entity().living_entity;
                let entity = &living.entity;
                entity.yaw.store(entity.body_yaw.load());
                living.discard_friction.store(true, Ordering::Relaxed);
                let org_length = chosen_jump.length();
                let length_with_jump_boost = org_length + f64::from(living.get_jump_boost_power());
                entity
                    .velocity
                    .store(chosen_jump * (length_with_jump_boost / org_length));
                tick.brain.set(types::LONG_JUMP_MID_JUMP, true);
                tick.world.play_sound_fine(
                    (self.get_jump_sound)(tick.mob),
                    SoundCategory::Neutral,
                    &entity.pos.load(),
                    1.0,
                    1.0,
                );
            }
        } else {
            self.find_jump_tries -= 1;
            self.pick_candidate(tick);
        }
    }

    /// The side effects of vanilla `canStillUse` turning false.
    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        if self.stopped_invalid && !tick.brain.has_memory_value(types::LONG_JUMP_MID_JUMP.id()) {
            let cooldown = self.sample_time_between_long_jumps(tick.world) / 2;
            tick.brain.set(types::LONG_JUMP_COOLDOWN_TICKS, cooldown);
            tick.brain.erase(types::LOOK_TARGET.id());
        }
        self.stopped_invalid = false;
    }

    fn debug_name(&self) -> &'static str {
        if self.preferred.is_some() {
            "LongJumpToPreferredBlock"
        } else {
            "LongJumpToRandomPos"
        }
    }
}

/// Vanilla `LongJumpToRandomPos.defaultAcceptableLandingSpot`: solid ground below and
/// a spot the mob paths over at no extra cost.
#[must_use]
pub fn default_acceptable_landing_spot(tick: &BrainTick<'_>, target_pos: &BlockPos) -> bool {
    if !tick
        .world
        .get_block_state(&target_pos.down())
        .is_solid_render()
    {
        return false;
    }
    let mut context = PathfindingContext::new(
        tick.mob.get_entity().block_pos.load().0,
        Arc::clone(tick.world),
    );
    let path_type = WalkNodeEvaluator::get_path_type_static(&mut context, target_pos.0);
    tick.mob.get_mob_entity().get_pathfinding_malus(path_type) == 0.0
}

/// Vanilla `LongJumpUtil.calculateJumpVectorForAngle`.
#[must_use]
pub fn calculate_jump_vector_for_angle(
    mob: &dyn Mob,
    target_pos: Vector3<f64>,
    max_jump_velocity: f32,
    angle: i32,
    check_collision: bool,
) -> Option<Vector3<f64>> {
    let mob_pos = mob.get_entity().pos.load();
    let direction_vector_plane =
        Vector3::new(target_pos.x - mob_pos.x, 0.0, target_pos.z - mob_pos.z).normalize() * 0.5;
    let target_position = target_pos - direction_vector_plane;
    let direction_vector = target_position - mob_pos;
    let angrad = angle as f32 * std::f32::consts::PI / 180.0;
    let xz_ang = direction_vector.z.atan2(direction_vector.x);
    let r2 = Vector3::new(direction_vector.x, 0.0, direction_vector.z).length_squared();
    let r = r2.sqrt();
    let y = direction_vector.y;
    let g = mob.current_gravity();
    let sin2ang = f64::from(2.0 * angrad).sin();
    let cosangsqr = f64::from(angrad).cos().powi(2);
    let sinangrad = f64::from(angrad).sin();
    let cosangrad = f64::from(angrad).cos();
    let sinxz_ang = xz_ang.sin();
    let cosxz_ang = xz_ang.cos();
    let v0sqr = r2 * g / (r * sin2ang - 2.0 * y * cosangsqr);
    if v0sqr < 0.0 {
        return None;
    }
    let v0 = v0sqr.sqrt();
    if v0 > f64::from(max_jump_velocity) {
        return None;
    }
    let v0r = v0 * cosangrad;
    let v0y = v0 * sinangrad;
    if check_collision {
        let samples = (r / v0r).ceil() as i32 * 2;
        let mut ri = 0.0;
        let mut previous_pos: Option<Vector3<f64>> = None;
        let mob_dimensions = mob.dimensions_for_pose(EntityPose::LongJumping);
        for _ in 0..samples - 1 {
            ri += r / f64::from(samples);
            let yi =
                sinangrad / cosangrad * ri - ri.powi(2) * g / (2.0 * v0sqr * cosangrad.powi(2));
            let sample_pos = Vector3::new(
                mob_pos.x + ri * cosxz_ang,
                mob_pos.y + yi,
                mob_pos.z + ri * sinxz_ang,
            );
            if let Some(previous_pos) = previous_pos
                && !is_clear_transition(mob, &mob_dimensions, previous_pos, sample_pos)
            {
                return None;
            }
            previous_pos = Some(sample_pos);
        }
    }
    Some(Vector3::new(v0r * cosxz_ang, v0y, v0r * sinxz_ang) * f64::from(0.95f32))
}

/// Vanilla `LongJumpUtil.isClearTransition`.
fn is_clear_transition(
    mob: &dyn Mob,
    dimensions: &EntityDimensions,
    position1: Vector3<f64>,
    position2: Vector3<f64>,
) -> bool {
    let direction = position2 - position1;
    let min_dimension = f64::from(dimensions.width.min(dimensions.height));
    let checks = (direction.length() / min_dimension).ceil() as i32;
    let normalized_direction = direction.normalize();
    let mut next_point_to_check = position1;
    let world = mob.get_entity().world.load();
    for i in 0..checks {
        next_point_to_check = if i == checks - 1 {
            position2
        } else {
            next_point_to_check + normalized_direction * (min_dimension * f64::from(0.9f32))
        };
        // Vanilla `noCollision` also counts hard-colliding entities such as boats,
        // which `is_space_empty` does not.
        if !world.is_space_empty(BoundingBox::new_from_pos(
            next_point_to_check.x,
            next_point_to_check.y,
            next_point_to_check.z,
            dimensions,
        )) {
            return false;
        }
    }
    true
}
