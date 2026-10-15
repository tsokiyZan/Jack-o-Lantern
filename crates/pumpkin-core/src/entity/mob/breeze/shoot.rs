//! Vanilla `Shoot`.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crossbeam::atomic::AtomicCell;
use pumpkin_data::entity::{EntityPose, EntityType};
use pumpkin_data::sound::Sound;
use pumpkin_util::math::vector3::Vector3;

use super::{get_firing_y_position, pose};
use crate::command::argument_types::entity_anchor::EntityAnchor;
use crate::entity::ai::brain::BrainTick;
use crate::entity::ai::brain::behavior::Behavior;
use crate::entity::ai::brain::memory::{MemoryModuleId, MemoryStatus, types};
use crate::entity::projectile::ThrownItemEntity;
use crate::entity::projectile::wind_charge::{WIND_CHARGE_GRAVITY, WindChargeEntity};
use crate::entity::{Entity, EntityBase};

const ATTACK_RANGE_MAX_SQRT: f64 = 256.0;
const UNCERTAINTY_BASE: i32 = 5;
const UNCERTAINTY_MULTIPLIER: i32 = 4;
const PROJECTILE_MOVEMENT_SCALE: f32 = 0.7;
const SHOOT_INITIAL_DELAY_TICKS: i32 = 15;
const SHOOT_RECOVER_DELAY_TICKS: i32 = 4;
const SHOOT_COOLDOWN_TICKS: i32 = 10;

const ENTRY_CONDITIONS: [(MemoryModuleId, MemoryStatus); 7] = [
    (types::ATTACK_TARGET.id(), MemoryStatus::ValuePresent),
    (types::BREEZE_SHOOT_COOLDOWN.id(), MemoryStatus::ValueAbsent),
    (types::BREEZE_SHOOT_CHARGING.id(), MemoryStatus::ValueAbsent),
    (
        types::BREEZE_SHOOT_RECOVERING.id(),
        MemoryStatus::ValueAbsent,
    ),
    (types::BREEZE_SHOOT.id(), MemoryStatus::ValuePresent),
    (types::WALK_TARGET.id(), MemoryStatus::ValueAbsent),
    (types::BREEZE_JUMP_TARGET.id(), MemoryStatus::ValueAbsent),
];

/// Vanilla `Shoot`: inhales, then fires one breeze wind charge at the attack target.
pub struct Shoot;

impl Shoot {
    /// Vanilla `Shoot.isTargetWithinRange`.
    fn is_target_within_range(body: &Entity, target: &dyn EntityBase) -> bool {
        body.pos
            .load()
            .squared_distance_to_vec(&target.get_entity().pos.load())
            < ATTACK_RANGE_MAX_SQRT
    }
}

impl Behavior for Shoot {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &ENTRY_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        SHOOT_INITIAL_DELAY_TICKS + 1 + SHOOT_RECOVER_DELAY_TICKS
    }

    fn max_duration(&self) -> i32 {
        self.min_duration()
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        if pose(tick) != EntityPose::Standing {
            return false;
        }
        let Some(within_range) = tick
            .brain
            .get(types::ATTACK_TARGET)
            .map(|target| Self::is_target_within_range(tick.mob.get_entity(), target.as_ref()))
        else {
            return false;
        };
        if !within_range {
            tick.brain.erase(types::BREEZE_SHOOT.id());
        }
        within_range
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        tick.brain.has_memory_value(types::ATTACK_TARGET.id())
            && tick.brain.has_memory_value(types::BREEZE_SHOOT.id())
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        if tick.brain.has_memory_value(types::ATTACK_TARGET.id()) {
            tick.mob.set_pose(EntityPose::Shooting);
        }
        tick.brain.set_with_expiry(
            types::BREEZE_SHOOT_CHARGING,
            (),
            i64::from(SHOOT_INITIAL_DELAY_TICKS),
        );
        tick.mob
            .play_sound_at_self(Sound::EntityBreezeInhale, 1.0, 1.0);
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        if pose(tick) == EntityPose::Shooting {
            tick.mob.set_pose(EntityPose::Standing);
        }
        tick.brain.set_with_expiry(
            types::BREEZE_SHOOT_COOLDOWN,
            (),
            i64::from(SHOOT_COOLDOWN_TICKS),
        );
        tick.brain.erase(types::BREEZE_SHOOT.id());
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let Some(target) = tick.brain.get(types::ATTACK_TARGET).map(Arc::clone) else {
            return;
        };
        let breeze = tick.mob.get_entity();
        let target_entity = target.get_entity();
        breeze.look_at(EntityAnchor::Eyes, target_entity.pos.load());
        if tick
            .brain
            .has_memory_value(types::BREEZE_SHOOT_CHARGING.id())
            || tick
                .brain
                .has_memory_value(types::BREEZE_SHOOT_RECOVERING.id())
        {
            return;
        }
        tick.brain.set_with_expiry(
            types::BREEZE_SHOOT_RECOVERING,
            (),
            i64::from(SHOOT_RECOVER_DELAY_TICKS),
        );
        let breeze_pos = breeze.pos.load();
        let target_pos = target_entity.pos.load();
        // Vanilla `target.getY(target.isPassenger() ? 0.8 : 0.3)`.
        let target_y_scale = if target_entity.has_vehicle() {
            0.8
        } else {
            0.3
        };
        let delta = Vector3::new(
            target_pos.x - breeze_pos.x,
            target_pos.y + f64::from(target_entity.height()) * target_y_scale
                - get_firing_y_position(breeze),
            target_pos.z - breeze_pos.z,
        );
        let difficulty = tick.world.level_info.load().difficulty as i32;
        let uncertainty = UNCERTAINTY_BASE - difficulty * UNCERTAINTY_MULTIPLIER;
        // Vanilla `new BreezeWindCharge(breeze, level)` through
        // `Projectile.spawnProjectileUsingShoot`.
        let wind_charge = ThrownItemEntity {
            entity: Entity::new(
                Arc::clone(tick.world),
                Vector3::new(breeze_pos.x, get_firing_y_position(breeze), breeze_pos.z),
                &EntityType::BREEZE_WIND_CHARGE,
            ),
            owner_id: AtomicCell::new(Some(breeze.entity_id)),
            collides_with_projectiles: false,
            has_hit: AtomicBool::new(false),
            gravity: WIND_CHARGE_GRAVITY,
        };
        wind_charge.set_velocity(
            delta.x,
            delta.y,
            delta.z,
            f64::from(PROJECTILE_MOVEMENT_SCALE),
            f64::from(uncertainty),
        );
        tick.world
            .spawn_entity(Arc::new(WindChargeEntity::new_breeze(wind_charge)));
        tick.mob
            .play_sound_at_self(Sound::EntityBreezeShoot, 1.5, 1.0);
    }

    fn debug_name(&self) -> &'static str {
        "Shoot"
    }
}
