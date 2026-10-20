//! Vanilla `LongJump`, the breeze's own leap behind its target.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use pumpkin_data::Block;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::clip::{ClipBlock, ClipContext, ClipFluid};

use super::breeze_ai::JUMP_CIRCLE_INNER_RADIUS;
use super::breeze_util::{has_line_of_sight, random_point_behind_target};
use super::pose;
use crate::command::argument_types::entity_anchor::EntityAnchor;
use crate::entity::ai::brain::BrainTick;
use crate::entity::ai::brain::behavior::Behavior;
use crate::entity::ai::brain::behavior::long_jump_to_random_pos::calculate_jump_vector_for_angle;
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, types};
use crate::entity::ai::goal::swim::SwimGoal;
use crate::entity::mob::Mob;
use crate::entity::{Entity, EntityBase};
use crate::world::natural_spawner::is_block_dangerous;

const REQUIRED_AIR_BLOCKS_ABOVE: i32 = 4;
const JUMP_COOLDOWN_TICKS: i64 = 10;
const JUMP_COOLDOWN_WHEN_HURT_TICKS: i64 = 2;
const INHALING_DURATION_TICKS: i64 = 10;
const MAX_JUMP_VELOCITY_MULTIPLIER: f32 = 0.058_333_334;
const ALLOWED_ANGLES: [i32; 5] = [40, 55, 60, 75, 80];
/// Vanilla `LongJump`'s `Behavior` duration.
const TIME_OUT_DURATION: i32 = 200;
/// How far `snapToSurface` looks down, then up, for ground.
const SNAP_TO_SURFACE_DISTANCE: f64 = 10.0;
/// How long a landed breeze has a shot ready for.
const SHOOT_AFTER_LANDING_TICKS: i64 = 100;

const ENTRY_CONDITIONS: [(MemoryModuleId, MemoryStatus); 7] = [
    (types::ATTACK_TARGET.id(), MemoryStatus::ValuePresent),
    (types::BREEZE_JUMP_COOLDOWN.id(), MemoryStatus::ValueAbsent),
    (types::BREEZE_JUMP_INHALING.id(), MemoryStatus::Registered),
    (types::BREEZE_JUMP_TARGET.id(), MemoryStatus::Registered),
    (types::BREEZE_SHOOT.id(), MemoryStatus::ValueAbsent),
    (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
    (types::BREEZE_LEAVING_WATER.id(), MemoryStatus::Registered),
];

/// Vanilla `LongJump`: inhales, then leaps to a spot behind its target and lands.
pub struct LongJump;

impl LongJump {
    /// Vanilla `LongJump.canRun`: finds and remembers a spot behind the target to jump to.
    pub fn can_run(tick: &mut BrainTick<'_>) -> bool {
        let entity = tick.mob.get_entity();
        if !entity.on_ground.load(Ordering::Relaxed) && !entity.is_in_water() {
            return false;
        }
        // `isInFluidDeeperThan(getFluidJumpThreshold(), ENTITY_FLOATABLE) || isInLava()`.
        if SwimGoal::is_in_fluid(tick.mob) {
            return false;
        }
        if tick
            .brain
            .check(types::BREEZE_JUMP_TARGET.id(), MemoryStatus::ValuePresent)
        {
            return true;
        }
        let Some(attack_target) = tick.brain.get(types::ATTACK_TARGET).map(Arc::clone) else {
            return false;
        };
        if Self::out_of_aggro_range(tick.mob, attack_target.as_ref()) {
            tick.brain.erase(types::ATTACK_TARGET.id());
            return false;
        }
        if Self::too_close_for_jump(entity, attack_target.get_entity()) {
            return false;
        }
        if !Self::can_jump_from_current_position(tick) {
            return false;
        }
        let behind = random_point_behind_target(
            attack_target.as_ref(),
            &mut *tick.mob.get_entity().random(),
        );
        let Some(target_pos) = Self::snap_to_surface(entity, behind) else {
            return false;
        };
        let below = tick.world.get_block_state(&target_pos.down());
        if is_block_dangerous(entity.entity_type, below) {
            return false;
        }
        if !has_line_of_sight(tick.mob, target_pos.to_centered_f64())
            && !has_line_of_sight(
                tick.mob,
                target_pos
                    .up_height(REQUIRED_AIR_BLOCKS_ABOVE)
                    .to_centered_f64(),
            )
        {
            return false;
        }
        tick.brain.set(types::BREEZE_JUMP_TARGET, target_pos);
        true
    }

    /// Vanilla `LongJump.isFinishedInhaling`.
    fn is_finished_inhaling(tick: &BrainTick<'_>) -> bool {
        !tick
            .brain
            .has_memory_value(types::BREEZE_JUMP_INHALING.id())
            && pose(tick) == EntityPose::Inhaling
    }

    /// Vanilla `LongJump.isFinishedJumping`.
    fn is_finished_jumping(tick: &BrainTick<'_>) -> bool {
        let entity = tick.mob.get_entity();
        let is_jumping = pose(tick) == EntityPose::LongJumping;
        let landed_on_ground = entity.on_ground.load(Ordering::Relaxed);
        let landed_in_water = entity.is_in_water()
            && !tick
                .brain
                .has_memory_value(types::BREEZE_LEAVING_WATER.id());
        is_jumping && (landed_on_ground || landed_in_water)
    }

    /// Vanilla `LongJump.snapToSurface`: the block above the ground found within ten blocks
    /// below `target`, else above it.
    fn snap_to_surface(entity: &Entity, target: Vector3<f64>) -> Option<BlockPos> {
        let world = entity.world.load();
        [-SNAP_TO_SURFACE_DISTANCE, SNAP_TO_SURFACE_DISTANCE]
            .into_iter()
            .find_map(|offset| {
                let hit = world.clip(&ClipContext::new(
                    target,
                    Vector3::new(target.x, target.y + offset, target.z),
                    ClipBlock::Collider,
                    ClipFluid::None,
                ));
                (!hit.is_miss).then(|| BlockPos::floored_v(hit.location).up())
            })
    }

    /// Vanilla `LongJump.outOfAggroRange`.
    fn out_of_aggro_range(breeze: &dyn Mob, attack_target: &dyn EntityBase) -> bool {
        let follow_range = breeze
            .get_mob_entity()
            .living_entity
            .get_attribute_value(&Attributes::FOLLOW_RANGE);
        attack_target
            .get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&breeze.get_entity().pos.load())
            >= follow_range * follow_range
    }

    /// Vanilla `LongJump.tooCloseForJump`, with `Entity.distanceTo`'s float maths.
    fn too_close_for_jump(breeze: &Entity, attack_target: &Entity) -> bool {
        let delta = attack_target.pos.load() - breeze.pos.load();
        let (dx, dy, dz) = (delta.x as f32, delta.y as f32, delta.z as f32);
        (dx * dx + dy * dy + dz * dz).sqrt() - JUMP_CIRCLE_INNER_RADIUS <= 0.0
    }

    /// Vanilla `LongJump.canJumpFromCurrentPosition`: not on honey, with air or water
    /// above.
    fn can_jump_from_current_position(tick: &BrainTick<'_>) -> bool {
        let current_pos = tick.mob.get_entity().block_pos.load();
        if tick.world.get_block(&current_pos) == &Block::HONEY_BLOCK {
            return false;
        }
        (1..=REQUIRED_AIR_BLOCKS_ABOVE).all(|i| {
            let offset_pos = current_pos.up_height(i);
            tick.world.get_block_state(&offset_pos).is_air()
                || tick
                    .world
                    .get_fluid_type(&offset_pos)
                    .has_tag(&tag::Fluid::MINECRAFT_WATER)
        })
    }

    /// Vanilla `LongJump.calculateOptimalJumpVector`.
    fn calculate_optimal_jump_vector(
        body: &dyn Mob,
        target_pos: Vector3<f64>,
    ) -> Option<Vector3<f64>> {
        let mut angles = ALLOWED_ANGLES;
        pumpkin_util::random::shuffle(&mut angles, &mut *body.get_entity().random());
        let living = &body.get_mob_entity().living_entity;
        angles.into_iter().find_map(|angle| {
            let max_jump_velocity = MAX_JUMP_VELOCITY_MULTIPLIER
                * living.get_attribute_value(&Attributes::FOLLOW_RANGE) as f32;
            let velocity =
                calculate_jump_vector_for_angle(body, target_pos, max_jump_velocity, angle, false)?;
            if living.has_effect(&StatusEffect::JUMP_BOOST) {
                let jump_effect_amplifier =
                    velocity.normalize().y * f64::from(living.get_jump_boost_power());
                return Some(velocity + Vector3::new(0.0, jump_effect_amplifier, 0.0));
            }
            Some(velocity)
        })
    }
}

impl Behavior for LongJump {
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
        Self::can_run(tick)
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        pose(tick) != EntityPose::Standing
            && !tick
                .brain
                .has_memory_value(types::BREEZE_JUMP_COOLDOWN.id())
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        if !tick
            .brain
            .has_memory_value(types::BREEZE_JUMP_INHALING.id())
        {
            tick.brain
                .set_with_expiry(types::BREEZE_JUMP_INHALING, (), INHALING_DURATION_TICKS);
        }
        tick.mob.set_pose(EntityPose::Inhaling);
        let entity = tick.mob.get_entity();
        tick.world.play_sound_from_entity(
            entity,
            Sound::EntityBreezeCharge,
            pumpkin_data::sound::SoundCategory::Hostile,
            1.0,
            1.0,
        );
        if let Some(target_pos) = tick.brain.get(types::BREEZE_JUMP_TARGET) {
            entity.look_at(EntityAnchor::Eyes, target_pos.to_centered_f64());
        }
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let living = &tick.mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        let in_water = entity.is_in_water();
        if !in_water
            && tick
                .brain
                .has_memory_value(types::BREEZE_LEAVING_WATER.id())
        {
            tick.brain.erase(types::BREEZE_LEAVING_WATER.id());
        }

        if Self::is_finished_inhaling(tick) {
            let velocity =
                tick.brain
                    .get(types::BREEZE_JUMP_TARGET)
                    .copied()
                    .and_then(|target_pos| {
                        // `Vec3.atBottomCenterOf`.
                        let bottom_center = Vector3::new(
                            f64::from(target_pos.0.x) + 0.5,
                            f64::from(target_pos.0.y),
                            f64::from(target_pos.0.z) + 0.5,
                        );
                        Self::calculate_optimal_jump_vector(tick.mob, bottom_center)
                    });
            let Some(velocity) = velocity else {
                tick.mob.set_pose(EntityPose::Standing);
                return;
            };
            if in_water {
                tick.brain.set(types::BREEZE_LEAVING_WATER, ());
            }
            tick.mob
                .play_sound_at_self(Sound::EntityBreezeJump, 1.0, 1.0);
            tick.mob.set_pose(EntityPose::LongJumping);
            entity.yaw.store(entity.body_yaw.load());
            living.discard_friction.store(true, Ordering::Relaxed);
            entity.velocity.store(velocity);
        } else if Self::is_finished_jumping(tick) {
            tick.mob
                .play_sound_at_self(Sound::EntityBreezeLand, 1.0, 1.0);
            tick.mob.set_pose(EntityPose::Standing);
            living.discard_friction.store(false, Ordering::Relaxed);
            let was_hurt = tick.brain.has_memory_value(types::HURT_BY.id());
            tick.brain.set_with_expiry(
                types::BREEZE_JUMP_COOLDOWN,
                (),
                if was_hurt {
                    JUMP_COOLDOWN_WHEN_HURT_TICKS
                } else {
                    JUMP_COOLDOWN_TICKS
                },
            );
            tick.brain
                .set_with_expiry(types::BREEZE_SHOOT, (), SHOOT_AFTER_LANDING_TICKS);
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        let pose = pose(tick);
        if pose == EntityPose::LongJumping || pose == EntityPose::Inhaling {
            tick.mob.set_pose(EntityPose::Standing);
        }
        tick.brain.erase(types::BREEZE_JUMP_TARGET.id());
        tick.brain.erase(types::BREEZE_JUMP_INHALING.id());
        tick.brain.erase(types::BREEZE_LEAVING_WATER.id());
    }

    fn debug_name(&self) -> &'static str {
        "LongJump"
    }
}
