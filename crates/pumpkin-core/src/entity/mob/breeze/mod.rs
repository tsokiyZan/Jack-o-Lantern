//! Vanilla `net.minecraft.world.entity.monster.breeze`.
//!
//! `Breeze.tick` only drives client effects (particles, animations, the whirl sound) and
//! its ambient sound is `playLocalSound`; the server sends neither, the client plays them
//! from the synced pose.

use std::sync::Arc;

use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::{EntityPose, EntityType};
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::vector3::Vector3;

use crate::entity::ai::brain::memory::{PackedMemories, types};
use crate::entity::ai::brain::{Brain, BrainTick, behavior::utils::is_alive};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::projectile_deflection::ProjectileDeflectionType;
use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity, mob_hurt_server},
};

pub mod breeze_ai;
pub mod breeze_util;
pub mod long_jump;
pub mod shoot;
pub mod shoot_when_stuck;
pub mod slide;

/// Vanilla `Breeze.getHeadRotSpeed`.
const HEAD_ROT_SPEED: f32 = 25.0;
/// Vanilla `Breeze.getMaxHeadYRot`.
const MAX_HEAD_Y_ROT: f32 = 30.0;
/// Vanilla `Breeze.JUMP_CIRCLE_DISTANCE_Y`.
const JUMP_CIRCLE_DISTANCE_Y: f64 = 10.0;

pub struct BreezeEntity {
    pub mob_entity: MobEntity,
}

impl BreezeEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `Breeze`'s constructor.
        mob_entity.set_pathfinding_malus(PathType::DangerTrapdoor, -1.0);
        mob_entity.set_pathfinding_malus(PathType::DamageFire, -1.0);
        let mob_arc = Arc::new(Self { mob_entity });
        mob_arc.mob_entity.init_brain(mob_arc.as_ref());
        mob_arc
    }
}

/// Vanilla `Breeze.getHurtBy`: the living entity behind the last damage taken.
#[must_use]
pub fn get_hurt_by(brain: &Brain) -> Option<Arc<dyn EntityBase>> {
    brain
        .get(types::HURT_BY)
        .and_then(|source| source.attacker.clone())
}

/// Vanilla `Breeze.withinInnerCircleRange`.
#[must_use]
pub fn within_inner_circle_range(breeze: &Entity, target: Vector3<f64>) -> bool {
    let our_position = breeze.block_pos.load().to_centered_f64();
    let dx = our_position.x - target.x;
    let dy = our_position.y - target.y;
    let dz = our_position.z - target.z;
    let radius = f64::from(breeze_ai::JUMP_CIRCLE_INNER_RADIUS);
    dx * dx + dz * dz < radius * radius && dy.abs() < JUMP_CIRCLE_DISTANCE_Y
}

/// Vanilla `Breeze.getFiringYPosition`.
#[must_use]
pub fn get_firing_y_position(breeze: &Entity) -> f64 {
    breeze.pos.load().y + f64::from(breeze.height() / 2.0) + f64::from(0.3f32)
}

/// The breeze's pose, which its behaviours read as their state.
#[must_use]
pub fn pose(tick: &BrainTick<'_>) -> EntityPose {
    tick.mob.get_entity().pose.load()
}

/// Vanilla `Breeze.PROJECTILE_DEFLECTION`: a whoosh, then `ProjectileDeflection.REVERSE`.
fn projectile_deflection(projectile: &dyn EntityBase, entity: Option<&dyn EntityBase>) {
    if let Some(entity) = entity {
        let breeze = entity.get_entity();
        breeze.world.load().play_sound_from_entity(
            breeze,
            Sound::EntityBreezeDeflect,
            entity.entity_sound_source(),
            1.0,
            1.0,
        );
    }
    ProjectileDeflectionType::Simple.deflect(projectile, entity);
}

impl Mob for BreezeEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn get_max_look_yaw_change(&self) -> f32 {
        HEAD_ROT_SPEED
    }

    fn get_max_head_rotation(&self) -> f32 {
        MAX_HEAD_Y_ROT
    }

    /// Vanilla `Breeze.deflection`: everything but wind charges bounces off.
    fn mob_deflection(&self, projectile: &dyn EntityBase) -> ProjectileDeflectionType {
        let projectile_type = projectile.get_entity().entity_type;
        if projectile_type == &EntityType::BREEZE_WIND_CHARGE
            || projectile_type == &EntityType::WIND_CHARGE
        {
            return ProjectileDeflectionType::None;
        }
        if self
            .get_entity()
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_DEFLECTS_PROJECTILES)
        {
            ProjectileDeflectionType::Custom(projectile_deflection)
        } else {
            ProjectileDeflectionType::None
        }
    }

    /// Vanilla `Breeze.makeBrain`.
    fn make_brain(&self, packed: &PackedMemories) -> Brain {
        let mut brain = breeze_ai::BREEZE_PROVIDER.make_brain(self, packed);
        brain.set_default_activity(pumpkin_data::environment_attribute::Activity::Fight);
        brain.use_default_activity();
        brain
    }

    /// Vanilla `Breeze.customServerAiStep`'s `BreezeAi.updateActivity`. `Breeze.getTarget`
    /// reads the brain; the mirror keeps the shared target code (path fall distance) seeing
    /// the same target.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        breeze_ai::update_activity(tick);
        self.mob_entity
            .set_target(tick.brain.get(types::ATTACK_TARGET).cloned());
    }

    /// Vanilla `Breeze.customServerAiStep`.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
    }

    /// Vanilla `Breeze.canAttack`.
    fn can_attack(&self, target: &dyn EntityBase) -> bool {
        let target_type = target.get_entity().entity_type;
        (target_type == &EntityType::PLAYER || target_type == &EntityType::IRON_GOLEM)
            && self.mob_entity.living_entity.can_attack(target)
    }

    /// Vanilla `Breeze.isInvulnerableTo`, checked where `LivingEntity.hurtServer` checks it.
    fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if cause.is_some_and(|cause| cause.get_entity().entity_type == &EntityType::BREEZE) {
            return false;
        }
        mob_hurt_server(self, caller, amount, damage_type, position, source, cause)
    }
}
