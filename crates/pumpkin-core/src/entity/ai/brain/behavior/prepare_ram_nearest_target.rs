use std::sync::Arc;

use pumpkin_data::entity::EntityStatus;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::super::BrainTick;
use super::super::memory::position_tracker::{EntityTracker, PositionTracker};
use super::super::memory::walk_target::WalkTarget;
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::move_to_target_sink::dist_manhattan;
use super::timed::Behavior;
use super::utils::is_alive;
use crate::entity::EntityBase;
use crate::entity::ai::pathfinder::pathfinding_context::PathfindingContext;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::ai::util::goal_utils;
use crate::entity::mob::Mob;

/// Vanilla `PrepareRamNearestTarget.TIME_OUT_DURATION`.
const TIME_OUT_DURATION: i32 = 160;
/// `Direction.Plane.HORIZONTAL`: north, east, south, west.
const HORIZONTAL: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// Vanilla `PrepareRamNearestTarget.RamCandidate`.
struct RamCandidate {
    start_position: BlockPos,
    target_position: BlockPos,
    target: Arc<dyn EntityBase>,
}

/// Vanilla `PrepareRamNearestTarget`: walks to a spot in line with the nearest rammable
/// entity, lowers its head there and, once the target holds still long enough, marks
/// where to charge.
pub struct PrepareRamNearestTarget {
    get_cooldown_on_fail: fn(&dyn Mob) -> i32,
    min_ram_distance: i32,
    max_ram_distance: i32,
    walk_speed: f32,
    ram_targeting: TargetPredicate,
    ram_prepare_time: i64,
    get_prepare_ram_sound: fn(&dyn Mob) -> Sound,
    reached_ram_position_timestamp: Option<i64>,
    ram_candidate: Option<RamCandidate>,
    conditions: [(MemoryModuleId, MemoryStatus); 4],
}

impl PrepareRamNearestTarget {
    #[must_use]
    pub const fn new(
        get_cooldown_on_fail: fn(&dyn Mob) -> i32,
        min_ram_distance: i32,
        max_ram_distance: i32,
        walk_speed: f32,
        ram_targeting: TargetPredicate,
        ram_prepare_time: i32,
        get_prepare_ram_sound: fn(&dyn Mob) -> Sound,
    ) -> Self {
        Self {
            get_cooldown_on_fail,
            min_ram_distance,
            max_ram_distance,
            walk_speed,
            ram_targeting,
            ram_prepare_time: ram_prepare_time as i64,
            get_prepare_ram_sound,
            reached_ram_position_timestamp: None,
            ram_candidate: None,
            conditions: [
                (types::LOOK_TARGET.id(), MemoryStatus::Registered),
                (types::RAM_COOLDOWN_TICKS.id(), MemoryStatus::ValueAbsent),
                (
                    types::NEAREST_VISIBLE_LIVING_ENTITIES.id(),
                    MemoryStatus::ValuePresent,
                ),
                (types::RAM_TARGET.id(), MemoryStatus::ValueAbsent),
            ],
        }
    }

    /// Vanilla `PrepareRamNearestTarget.getEdgeOfBlock`.
    fn get_edge_of_block(start_ram_pos: BlockPos, target_pos: BlockPos) -> Vector3<f64> {
        let offset_distance = 0.5;
        let x_offset = offset_distance * f64::from((target_pos.0.x - start_ram_pos.0.x).signum());
        let z_offset = offset_distance * f64::from((target_pos.0.z - start_ram_pos.0.z).signum());
        Vector3::new(
            f64::from(target_pos.0.x) + 0.5 + x_offset,
            f64::from(target_pos.0.y),
            f64::from(target_pos.0.z) + 0.5 + z_offset,
        )
    }

    /// Vanilla `PrepareRamNearestTarget.calculateRammingStartPosition`.
    fn calculate_ramming_start_position(
        &self,
        body: &dyn Mob,
        ramable_target: &dyn EntityBase,
    ) -> Option<BlockPos> {
        let target_pos = ramable_target.get_entity().block_pos.load();
        if !Self::is_walkable_block(body, target_pos) {
            return None;
        }

        let mut possible_ram_positions = Vec::new();
        for (dx, dz) in HORIZONTAL {
            let mut walkable_pos_furthest_away_from_target = target_pos;
            for _ in 0..self.max_ram_distance {
                walkable_pos_furthest_away_from_target =
                    walkable_pos_furthest_away_from_target.offset(Vector3::new(dx, 0, dz));
                if !Self::is_walkable_block(body, walkable_pos_furthest_away_from_target) {
                    walkable_pos_furthest_away_from_target =
                        walkable_pos_furthest_away_from_target.offset(Vector3::new(-dx, 0, -dz));
                    break;
                }
            }

            if dist_manhattan(&walkable_pos_furthest_away_from_target, &target_pos)
                >= self.min_ram_distance
            {
                possible_ram_positions.push(walkable_pos_furthest_away_from_target);
            }
        }

        let body_pos = body.get_entity().block_pos.load();
        possible_ram_positions.sort_by_key(|pos| body_pos.squared_distance(pos));
        let mob_entity = body.get_mob_entity();
        possible_ram_positions.into_iter().find(|pos| {
            mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .create_path(&mob_entity.living_entity, pos.to_f64(), 0)
                .is_some_and(|path| path.can_reach())
        })
    }

    /// Vanilla `PrepareRamNearestTarget.isWalkableBlock`.
    fn is_walkable_block(body: &dyn Mob, target_pos: BlockPos) -> bool {
        let entity = body.get_entity();
        let world = entity.world.load_full();
        if goal_utils::is_not_stable(body, &world, &target_pos) {
            return false;
        }
        let mut context = PathfindingContext::new(entity.block_pos.load().0, world);
        !goal_utils::has_malus(body, &mut context, &target_pos)
    }

    /// Vanilla `PrepareRamNearestTarget.chooseRamPosition`.
    fn choose_ram_position(&mut self, body: &dyn Mob, ramable_target: Arc<dyn EntityBase>) {
        self.reached_ram_position_timestamp = None;
        self.ram_candidate = self
            .calculate_ramming_start_position(body, ramable_target.as_ref())
            .map(|pos| RamCandidate {
                start_position: pos,
                target_position: ramable_target.get_entity().block_pos.load(),
                target: ramable_target,
            });
    }
}

impl Behavior for PrepareRamNearestTarget {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn min_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn max_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        let target = {
            let ctx = tick.visibility();
            ctx.brain
                .get(types::NEAREST_VISIBLE_LIVING_ENTITIES)
                .and_then(|living_entities| {
                    living_entities.find_closest(&ctx, |entity| {
                        self.ram_targeting
                            .test(tick.world, Some(tick.mob), entity.as_ref())
                    })
                })
        };
        if let Some(target) = target {
            self.choose_ram_position(tick.mob, target);
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        if !tick.brain.has_memory_value(types::RAM_TARGET.id()) {
            tick.world
                .send_entity_status(tick.mob.get_entity(), EntityStatus::EndRam, None);
            tick.brain.set(
                types::RAM_COOLDOWN_TICKS,
                (self.get_cooldown_on_fail)(tick.mob),
            );
        }
    }

    fn can_still_use(&mut self, _tick: &BrainTick<'_>) -> bool {
        self.ram_candidate
            .as_ref()
            .is_some_and(|candidate| is_alive(candidate.target.as_ref()))
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let Some(candidate) = &self.ram_candidate else {
            return;
        };
        tick.brain.set(
            types::WALK_TARGET,
            WalkTarget::from_block_pos(candidate.start_position, self.walk_speed, 0),
        );
        tick.brain.set(
            types::LOOK_TARGET,
            Arc::new(EntityTracker::new(Arc::clone(&candidate.target), true))
                as Arc<dyn PositionTracker>,
        );
        let did_target_move =
            candidate.target.get_entity().block_pos.load() != candidate.target_position;
        if did_target_move {
            tick.world
                .send_entity_status(tick.mob.get_entity(), EntityStatus::EndRam, None);
            tick.mob
                .get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .stop();
            let target = Arc::clone(&candidate.target);
            self.choose_ram_position(tick.mob, target);
            return;
        }

        let start_ram_pos = tick.mob.get_entity().block_pos.load();
        if start_ram_pos != candidate.start_position {
            return;
        }
        tick.world
            .send_entity_status(tick.mob.get_entity(), EntityStatus::StartRam, None);
        let reached = *self.reached_ram_position_timestamp.get_or_insert(tick.time);
        if tick.time - reached >= self.ram_prepare_time {
            tick.brain.set(
                types::RAM_TARGET,
                Self::get_edge_of_block(start_ram_pos, candidate.target_position),
            );
            let entity = tick.mob.get_entity();
            tick.world.play_sound_fine(
                (self.get_prepare_ram_sound)(tick.mob),
                SoundCategory::Neutral,
                &entity.pos.load(),
                1.0,
                tick.mob.voice_pitch(),
            );
            self.ram_candidate = None;
        }
    }

    fn debug_name(&self) -> &'static str {
        "PrepareRamNearestTarget"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ram_target_is_the_far_edge_of_the_target_block() {
        let start = BlockPos(Vector3::new(0, 64, 0));
        let target = BlockPos(Vector3::new(5, 64, 0));
        assert_eq!(
            PrepareRamNearestTarget::get_edge_of_block(start, target),
            Vector3::new(6.0, 64.0, 0.5)
        );
        let behind = BlockPos(Vector3::new(0, 64, -4));
        assert_eq!(
            PrepareRamNearestTarget::get_edge_of_block(start, behind),
            Vector3::new(0.5, 64.0, -4.0)
        );
    }
}
