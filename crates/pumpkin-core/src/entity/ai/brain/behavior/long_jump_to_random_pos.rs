use std::sync::Arc;
use std::sync::atomic::Ordering::{Relaxed, SeqCst};

use pumpkin_data::Block;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;
use rand::seq::SliceRandom;

use super::super::BrainTick;
use super::super::memory::position_tracker::{BlockPosTracker, PositionTracker};
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::long_jump_util;
use super::timed::Behavior;
use crate::entity::ai::pathfinder::pathfinding_context::PathfindingContext;
use crate::entity::ai::util::goal_utils;
use crate::entity::mob::Mob;
use crate::entity::mob::piglin_ai::sample;

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

/// Vanilla `LongJumpToRandomPos.PossibleJump`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct PossibleJump {
    target_pos: BlockPos,
    weight: i32,
}

/// Vanilla `LongJumpToRandomPos`: picks a random reachable-only-by-jumping spot nearby,
/// looks at it for two seconds, then leaps.
pub struct LongJumpToRandomPos {
    time_between_long_jumps: (i32, i32),
    max_long_jump_height: i32,
    max_long_jump_width: i32,
    max_jump_velocity_multiplier: f32,
    jump_candidates: Vec<PossibleJump>,
    initial_position: Option<Vector3<f64>>,
    chosen_jump: Option<Vector3<f64>>,
    find_jump_tries: i32,
    prepare_jump_start: i64,
    get_jump_sound: fn(&dyn Mob) -> Sound,
    acceptable_landing_spot: fn(&dyn Mob, &BlockPos) -> bool,
    /// Vanilla's `canStillUse` writes the brain; the read-only `can_still_use` here leaves
    /// that to the `stop` that follows it.
    reset_cooldown_on_stop: bool,
    conditions: [(MemoryModuleId, MemoryStatus); 3],
}

impl LongJumpToRandomPos {
    #[must_use]
    pub fn new(
        time_between_long_jumps: (i32, i32),
        max_long_jump_height: i32,
        max_long_jump_width: i32,
        max_jump_velocity_multiplier: f32,
        get_jump_sound: fn(&dyn Mob) -> Sound,
    ) -> Self {
        Self {
            time_between_long_jumps,
            max_long_jump_height,
            max_long_jump_width,
            max_jump_velocity_multiplier,
            jump_candidates: Vec::new(),
            initial_position: None,
            chosen_jump: None,
            find_jump_tries: 0,
            prepare_jump_start: 0,
            get_jump_sound,
            acceptable_landing_spot: default_acceptable_landing_spot,
            reset_cooldown_on_stop: false,
            conditions: [
                (types::LOOK_TARGET.id(), MemoryStatus::Registered),
                (
                    types::LONG_JUMP_COOLDOWN_TICKS.id(),
                    MemoryStatus::ValueAbsent,
                ),
                (types::LONG_JUMP_MID_JUMP.id(), MemoryStatus::ValueAbsent),
            ],
        }
    }

    fn half_cooldown(&self, tick: &BrainTick<'_>) -> i32 {
        sample(self.time_between_long_jumps, &mut tick.mob.get_random()) / 2
    }

    /// Vanilla `LongJumpToRandomPos.pickCandidate`.
    fn pick_candidate(&mut self, tick: &mut BrainTick<'_>) {
        while !self.jump_candidates.is_empty() {
            let Some(position) = self.get_jump_candidate(tick) else {
                continue;
            };
            let target_pos = position.target_pos;
            if !self.is_acceptable_landing_position(tick.mob, &target_pos) {
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
            let path = mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .create_path_within(
                    &mob_entity.living_entity,
                    target_position,
                    0,
                    MIN_PATHFIND_DISTANCE_TO_VALID_JUMP,
                );
            if path.is_none_or(|path| !path.can_reach()) {
                self.chosen_jump = Some(jump_vector);
                self.prepare_jump_start = tick.time;
                return;
            }
        }
    }

    /// Vanilla `LongJumpToRandomPos.getJumpCandidate`: `WeightedRandom.getRandomItem`, then
    /// removes the pick.
    fn get_jump_candidate(&mut self, tick: &BrainTick<'_>) -> Option<PossibleJump> {
        let total_weight: i32 = self
            .jump_candidates
            .iter()
            .map(|candidate| candidate.weight)
            .sum();
        if total_weight == 0 {
            return None;
        }
        let mut selection = tick.mob.get_random().random_range(0..total_weight);
        let index = self.jump_candidates.iter().position(|candidate| {
            selection -= candidate.weight;
            selection < 0
        })?;
        Some(self.jump_candidates.remove(index))
    }

    /// Vanilla `LongJumpToRandomPos.isAcceptableLandingPosition`.
    fn is_acceptable_landing_position(&self, body: &dyn Mob, target_pos: &BlockPos) -> bool {
        let body_pos = body.get_entity().block_pos.load();
        if body_pos.0.x == target_pos.0.x && body_pos.0.z == target_pos.0.z {
            return false;
        }
        (self.acceptable_landing_spot)(body, target_pos)
    }

    /// Vanilla `LongJumpToRandomPos.calculateOptimalJumpVector`.
    fn calculate_optimal_jump_vector(
        &self,
        body: &dyn Mob,
        target_pos: Vector3<f64>,
    ) -> Option<Vector3<f64>> {
        let mut allowed_angles = ALLOWED_ANGLES;
        allowed_angles.shuffle(&mut body.get_random());
        let max_jump_velocity = (body
            .get_mob_entity()
            .living_entity
            .get_attribute_value(&Attributes::JUMP_STRENGTH)
            * f64::from(self.max_jump_velocity_multiplier)) as f32;
        allowed_angles.into_iter().find_map(|angle| {
            long_jump_util::calculate_jump_vector_for_angle(
                body,
                target_pos,
                max_jump_velocity,
                angle,
                true,
            )
        })
    }
}

/// Vanilla `LongJumpToRandomPos.defaultAcceptableLandingSpot`: solid ground below and a
/// spot the mob paths through freely.
#[must_use]
pub fn default_acceptable_landing_spot(body: &dyn Mob, target_pos: &BlockPos) -> bool {
    let entity = body.get_entity();
    let world = entity.world.load_full();
    if !world.get_block_state(&target_pos.down()).is_solid_render() {
        return false;
    }
    let mut context = PathfindingContext::new(entity.block_pos.load().0, world);
    !goal_utils::has_malus(body, &mut context, target_pos)
}

impl Behavior for LongJumpToRandomPos {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn min_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn max_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        let entity = tick.mob.get_entity();
        let can_start = entity.on_ground.load(Relaxed)
            && !entity.is_in_water()
            && !entity.touching_lava.load(Relaxed)
            && tick.world.get_block(&entity.block_pos.load()) != &Block::HONEY_BLOCK;
        if !can_start {
            let cooldown = self.half_cooldown(tick);
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
        if !is_valid && !tick.brain.has_memory_value(types::LONG_JUMP_MID_JUMP.id()) {
            self.reset_cooldown_on_stop = true;
        }
        is_valid
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        self.chosen_jump = None;
        self.find_jump_tries = FIND_JUMP_TRIES;
        let entity = tick.mob.get_entity();
        self.initial_position = Some(entity.pos.load());
        let mob_pos = entity.block_pos.load();
        let (width, height) = (self.max_long_jump_width, self.max_long_jump_height);
        // `BlockPos.betweenClosedStream` walks x fastest, then y, then z.
        self.jump_candidates.clear();
        for z in mob_pos.0.z - width..=mob_pos.0.z + width {
            for y in mob_pos.0.y - height..=mob_pos.0.y + height {
                for x in mob_pos.0.x - width..=mob_pos.0.x + width {
                    let pos = BlockPos(Vector3::new(x, y, z));
                    if pos == mob_pos {
                        continue;
                    }
                    self.jump_candidates.push(PossibleJump {
                        target_pos: pos,
                        // `Mth.ceil(distSqr)` of whole blocks.
                        weight: mob_pos.squared_distance(&pos),
                    });
                }
            }
        }
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let Some(chosen_jump) = self.chosen_jump else {
            self.find_jump_tries -= 1;
            self.pick_candidate(tick);
            return;
        };
        if tick.time - self.prepare_jump_start < PREPARE_JUMP_DURATION {
            return;
        }
        let living = &tick.mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        entity.yaw.store(entity.body_yaw.load());
        living.discard_friction.store(true, Relaxed);
        let org_length = chosen_jump.length();
        let length_with_jump_boost = org_length + f64::from(living.get_jump_boost_power());
        entity
            .velocity
            .store(chosen_jump * (length_with_jump_boost / org_length));
        entity.velocity_dirty.store(true, SeqCst);
        tick.brain.set(types::LONG_JUMP_MID_JUMP, true);
        tick.world.play_sound_fine(
            (self.get_jump_sound)(tick.mob),
            SoundCategory::Neutral,
            &entity.pos.load(),
            1.0,
            1.0,
        );
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        if std::mem::take(&mut self.reset_cooldown_on_stop) {
            let cooldown = self.half_cooldown(tick);
            tick.brain.set(types::LONG_JUMP_COOLDOWN_TICKS, cooldown);
            tick.brain.erase(types::LOOK_TARGET.id());
        }
    }

    fn debug_name(&self) -> &'static str {
        "LongJumpToRandomPos"
    }
}
