use std::sync::atomic::Ordering;

use crossbeam::atomic::AtomicCell;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::{RAD_TO_DEG, atan2, lerp};

use crate::{
    entity::{
        Entity, EntityBase,
        projectile::{ProjectileHit, ThrownItemEntity, is_deflected},
    },
    server::Server,
};

/// Vanilla `AbstractHurtingProjectile.INITAL_ACCELERATION_POWER`.
pub const INITIAL_ACCELERATION_POWER: f64 = 0.1;
/// Vanilla `AbstractHurtingProjectile.DEFLECTION_SCALE`.
pub const DEFLECTION_SCALE: f64 = 0.5;
/// Vanilla `AbstractHurtingProjectile.getInertia`, a float.
pub const AIR_INERTIA: f32 = 0.95;
/// Vanilla `AbstractHurtingProjectile.getLiquidInertia`, a float.
pub const WATER_INERTIA: f32 = 0.8;

/// What a subclass of vanilla `AbstractHurtingProjectile` overrides of its tick.
pub struct HurtingProjectileTick {
    /// `getInertia`.
    pub inertia: f32,
    /// `getLiquidInertia`.
    pub liquid_inertia: f32,
    /// `shouldBurn`.
    pub should_burn: bool,
}

impl Default for HurtingProjectileTick {
    fn default() -> Self {
        Self {
            inertia: AIR_INERTIA,
            liquid_inertia: WATER_INERTIA,
            should_burn: true,
        }
    }
}

/// Vanilla `AbstractHurtingProjectile`: a projectile without gravity that speeds up along its
/// heading. Its `on_hit` discards it, as each vanilla subclass's `onHit` does.
pub struct HurtingProjectile {
    pub thrown: ThrownItemEntity,
    acceleration_power: AtomicCell<f64>,
}

impl HurtingProjectile {
    /// Vanilla `AbstractHurtingProjectile(type, level)`.
    #[must_use]
    pub const fn new(entity: Entity) -> Self {
        Self {
            thrown: ThrownItemEntity::unowned(entity, 0.0),
            acceleration_power: AtomicCell::new(INITIAL_ACCELERATION_POWER),
        }
    }

    /// Vanilla `AbstractHurtingProjectile(type, x, y, z, direction, level)`, at the entity's
    /// position.
    #[must_use]
    pub fn new_directional(entity: Entity, direction: Vector3<f64>) -> Self {
        let projectile = Self::new(entity);
        projectile.assign_directional_movement(direction, INITIAL_ACCELERATION_POWER);
        projectile
    }

    /// Vanilla `AbstractHurtingProjectile(type, mob, direction, level)`: at the mob's feet,
    /// owned by it and turned like it.
    #[must_use]
    pub fn new_from_mob(entity: Entity, mob: &Entity, direction: Vector3<f64>) -> Self {
        entity.set_pos(mob.pos.load());
        let projectile = Self::new_directional(entity, direction);
        projectile.thrown.projectile.set_owner(Some(mob));
        // `setRot`.
        let entity = projectile.get_entity();
        entity.yaw.store(mob.yaw.load() % 360.0);
        entity.set_pitch(mob.pitch.load() % 360.0);
        projectile
    }

    pub const fn get_entity(&self) -> &Entity {
        &self.thrown.entity
    }

    /// Vanilla `Projectile.getOwner`.
    pub fn get_owner(&self) -> Option<std::sync::Arc<dyn EntityBase>> {
        self.thrown.projectile.get_owner(&self.thrown.entity)
    }

    /// Vanilla `AbstractHurtingProjectile.assignDirectionalMovement`.
    fn assign_directional_movement(&self, direction: Vector3<f64>, speed: f64) {
        let entity = self.get_entity();
        entity.velocity.store(direction.normalize() * speed);
        // `needsSync`.
        entity.velocity_dirty.store(true, Ordering::SeqCst);
    }

    pub fn get_acceleration_power(&self) -> f64 {
        self.acceleration_power.load()
    }

    pub fn set_acceleration_power(&self, power: f64) {
        self.acceleration_power.store(power);
    }

    /// Vanilla `AbstractHurtingProjectile.onDeflection`.
    pub fn on_deflection(&self, by_attack: bool) {
        if by_attack {
            self.set_acceleration_power(INITIAL_ACCELERATION_POWER);
        } else {
            self.set_acceleration_power(self.get_acceleration_power() * DEFLECTION_SCALE);
        }
    }

    /// Vanilla `AbstractHurtingProjectile.tick`.
    pub fn tick(
        &self,
        caller: &dyn EntityBase,
        server: &Server,
        overrides: &HurtingProjectileTick,
    ) {
        let entity = self.get_entity();
        // `applyInertia`.
        let movement = entity.velocity.load();
        let inertia = if entity.touching_water.load(Ordering::Relaxed) {
            overrides.liquid_inertia
        } else {
            overrides.inertia
        };
        entity.velocity.store(
            (movement + movement.normalize() * self.get_acceleration_power()) * f64::from(inertia),
        );

        // Vanilla discards it when its owner is removed or its chunk isn't loaded; `getOwner`
        // never returns a removed owner, and only entities in loaded chunks tick here.
        let start_pos = entity.pos.load();
        let delta = entity.velocity.load();
        let hit = self.thrown.hit_result_on_move_vector(start_pos, delta);
        let new_pos = hit
            .as_ref()
            .map_or_else(|| start_pos.add(&delta), ProjectileHit::hit_pos);

        rotate_towards_movement(entity, 0.2);
        entity.set_pos(new_pos);
        // `applyEffectsFromBlocks`.
        Entity::check_block_collision(caller, server);
        // `Projectile.tick`.
        self.thrown.projectile.tick(entity);
        EntityBase::tick(entity, caller, server);
        if overrides.should_burn {
            caller.set_on_fire_for(1.0);
        }

        if let Some(hit) = hit
            && entity.is_alive()
        {
            self.hit_target_or_deflect_self(caller, hit);
        }
    }

    /// Vanilla `Projectile.hitTargetOrDeflectSelf`.
    fn hit_target_or_deflect_self(&self, caller: &dyn EntityBase, hit: ProjectileHit) {
        let deflected_by = self.thrown.last_deflected_by.load();
        if is_deflected(caller, &self.thrown.last_deflected_by, &hit) {
            if self.thrown.last_deflected_by.load() != deflected_by {
                self.on_deflection(false);
            }
            return;
        }
        // `Projectile.onHitBlock`.
        if let ProjectileHit::Block { pos, hit_pos, .. } = &hit {
            let world = self.get_entity().world.load();
            let block = world.get_block(pos);
            let state = world.get_block_state(pos);
            if let Some(server) = world.server.upgrade() {
                world
                    .block_registry
                    .on_projectile_hit(block, &world, caller, pos, state, hit_pos, &server);
            }
        }
        caller.on_hit(hit);
    }

    pub fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_double("acceleration_power", self.get_acceleration_power());
    }

    pub fn read_nbt(&self, nbt: &NbtCompound) {
        self.set_acceleration_power(
            nbt.get_double("acceleration_power")
                .unwrap_or(INITIAL_ACCELERATION_POWER),
        );
    }
}

/// Vanilla `ProjectileUtil.rotateTowardsMovement`.
pub fn rotate_towards_movement(entity: &Entity, rotation_speed: f32) {
    let movement = entity.velocity.load();
    if movement.length_squared() == 0.0 {
        return;
    }
    // `xRotO` and `yRotO`: nothing else turns a projectile during its tick.
    let mut x_rot_o = entity.pitch.load();
    let mut y_rot_o = entity.yaw.load();
    let degrees = f64::from(RAD_TO_DEG);
    let sd = movement.horizontal_length();
    let y_rot = (atan2(movement.z, movement.x) * degrees) as f32 + 90.0;
    entity.set_pitch((atan2(sd, movement.y) * degrees) as f32 - 90.0);
    let x_rot = entity.pitch.load();

    while x_rot - x_rot_o < -180.0 {
        x_rot_o -= 360.0;
    }
    while x_rot - x_rot_o >= 180.0 {
        x_rot_o += 360.0;
    }
    while y_rot - y_rot_o < -180.0 {
        y_rot_o -= 360.0;
    }
    while y_rot - y_rot_o >= 180.0 {
        y_rot_o += 360.0;
    }

    entity.set_pitch(lerp(rotation_speed, x_rot_o, x_rot));
    entity.yaw.store(lerp(rotation_speed, y_rot_o, y_rot));
}
