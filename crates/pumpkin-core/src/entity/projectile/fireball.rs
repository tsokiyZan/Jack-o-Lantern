use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::damage::DamageType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer;
use pumpkin_util::math::atomic_f32::AtomicF32;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::{atan2, lerp};

use crate::{
    entity::{
        Entity, EntityBase,
        projectile::{ProjectileHit, ThrownItemEntity, is_deflected},
    },
    server::Server,
};

pub const MIN_CAMERA_DISTANCE_SQUARED: f32 = 12.25;
pub const INITIAL_ACCELERATION_POWER: f64 = 0.1;
pub const DEFLECTION_SCALE: f64 = 0.5;
pub const DEFAULT_EXPLOSION_POWER: f32 = 1.0;
/// Vanilla `AbstractHurtingProjectile.getInertia`, a float.
pub const AIR_INERTIA: f64 = 0.95f32 as f64;
/// Vanilla `AbstractHurtingProjectile.getLiquidInertia`, a float.
pub const WATER_INERTIA: f64 = 0.8f32 as f64;

/// Vanilla `Fireball` with its `AbstractHurtingProjectile` half: what the large and small
/// fireballs share.
pub struct Fireball {
    pub thrown: ThrownItemEntity,
    item_stack: RwLock<ItemStack>,
    acceleration_power: AtomicCell<f64>,
}

impl Fireball {
    /// Vanilla `Fireball(type, level)`.
    #[must_use]
    pub fn new(entity: Entity) -> Self {
        Self {
            thrown: ThrownItemEntity {
                entity,
                owner_id: AtomicCell::new(None),
                collides_with_projectiles: false,
                has_hit: AtomicBool::new(false),
                last_deflected_by: AtomicCell::new(None),
                gravity: 0.0,
                left_owner: AtomicBool::new(false),
            },
            item_stack: RwLock::new(Self::get_default_item()),
            acceleration_power: AtomicCell::new(INITIAL_ACCELERATION_POWER),
        }
    }

    /// Vanilla `AbstractHurtingProjectile(type, x, y, z, direction, level)`, at the entity's
    /// position.
    #[must_use]
    pub fn new_directional(entity: Entity, direction: Vector3<f64>) -> Self {
        let fireball = Self::new(entity);
        fireball.assign_directional_movement(direction, INITIAL_ACCELERATION_POWER);
        fireball
    }

    /// Vanilla `AbstractHurtingProjectile(type, mob, direction, level)`: at the mob's feet,
    /// owned by it and turned like it.
    #[must_use]
    pub fn new_from_mob(entity: Entity, mob: &Entity, direction: Vector3<f64>) -> Self {
        entity.set_pos(mob.pos.load());
        let fireball = Self::new_directional(entity, direction);
        fireball.thrown.owner_id.store(Some(mob.entity_id));
        // `setRot`.
        let entity = fireball.get_entity();
        entity.yaw.store(mob.yaw.load() % 360.0);
        entity.set_pitch(mob.pitch.load() % 360.0);
        fireball
    }

    pub const fn get_entity(&self) -> &Entity {
        &self.thrown.entity
    }

    /// Vanilla `AbstractHurtingProjectile.assignDirectionalMovement`.
    fn assign_directional_movement(&self, direction: Vector3<f64>, speed: f64) {
        let entity = self.get_entity();
        entity.velocity.store(direction.normalize() * speed);
        // `needsSync`.
        entity.velocity_dirty.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn get_default_item() -> ItemStack {
        ItemStack::new(1, &Item::FIRE_CHARGE)
    }

    pub fn get_item(&self) -> ItemStack {
        self.item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set_item(&self, source: ItemStack) {
        let new_item = if source.item_count == 0 {
            Self::get_default_item()
        } else {
            let mut item = source;
            item.item_count = 1;
            item
        };
        *self
            .item_stack
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new_item.clone();

        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::fireball::ITEM_STACK,
            ItemStackSerializer::from(new_item),
        );
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

    pub fn should_render_at_sqr_distance(&self, distance_sqr: f64) -> bool {
        if self.get_entity().tick_count.load(Ordering::Relaxed) < 2
            && distance_sqr < f64::from(MIN_CAMERA_DISTANCE_SQUARED)
        {
            false
        } else {
            let bb_size = self
                .get_entity()
                .bounding_box
                .load()
                .get_average_side_length()
                * 4.0;
            let size = if bb_size.is_nan() { 4.0 } else { bb_size } * 64.0;
            distance_sqr < size * size
        }
    }

    /// Vanilla `DamageSources.fireball`: unattributed without an owner.
    #[must_use]
    pub fn damage_type(owner: Option<&dyn EntityBase>) -> DamageType {
        if owner.is_some() {
            DamageType::FIREBALL
        } else {
            DamageType::UNATTRIBUTED_FIREBALL
        }
    }

    /// Vanilla `AbstractHurtingProjectile.tick`.
    pub fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        let entity = self.get_entity();
        // `applyInertia`.
        let movement = entity.velocity.load();
        let inertia = if entity.touching_water.load(Ordering::Relaxed) {
            WATER_INERTIA
        } else {
            AIR_INERTIA
        };
        entity
            .velocity
            .store((movement + movement.normalize() * self.get_acceleration_power()) * inertia);

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
        self.thrown.check_left_owner();
        EntityBase::tick(entity, caller, server);
        // `shouldBurn`.
        caller.set_on_fire_for(1.0);

        if let Some(hit) = hit
            && entity.is_alive()
        {
            // `hitTargetOrDeflectSelf`.
            let deflected_by = self.thrown.last_deflected_by.load();
            if is_deflected(caller, &self.thrown.last_deflected_by, &hit) {
                if self.thrown.last_deflected_by.load() != deflected_by {
                    self.on_deflection(false);
                }
                return;
            }
            self.thrown.on_hit_result(caller, hit);
        }
    }

    pub fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_double("acceleration_power", self.get_acceleration_power());
        let mut item_compound = NbtCompound::new();
        self.get_item().write_item_stack(&mut item_compound);
        nbt.put_compound("Item", item_compound);
    }

    pub fn read_nbt(&self, nbt: &NbtCompound) {
        self.set_acceleration_power(
            nbt.get_double("acceleration_power")
                .unwrap_or(INITIAL_ACCELERATION_POWER),
        );
        self.set_item(
            nbt.get_compound("Item")
                .and_then(ItemStack::read_item_stack)
                .unwrap_or_else(Self::get_default_item),
        );
    }

    pub fn init_data_tracker(&self) {
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::fireball::ITEM_STACK,
            ItemStackSerializer::from(self.get_item()),
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
    let degrees = 180.0 / f64::from(std::f32::consts::PI);
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

/// Vanilla `LargeFireball`: the ghast's exploding fireball.
pub struct FireballEntity {
    pub fireball: Fireball,
    pub explosion_power: AtomicF32,
}

impl FireballEntity {
    #[must_use]
    pub fn new(entity: Entity) -> Self {
        Self {
            fireball: Fireball::new(entity),
            explosion_power: AtomicF32::new(DEFAULT_EXPLOSION_POWER),
        }
    }

    /// Vanilla `LargeFireball(level, mob, direction, explosionPower)`.
    #[must_use]
    pub fn new_from_mob(
        entity: Entity,
        mob: &Entity,
        direction: Vector3<f64>,
        explosion_power: f32,
    ) -> Self {
        Self {
            fireball: Fireball::new_from_mob(entity, mob, direction),
            explosion_power: AtomicF32::new(explosion_power),
        }
    }

    pub fn get_explosion_power(&self) -> f32 {
        self.explosion_power.load(Ordering::Relaxed)
    }

    pub fn set_explosion_power(&self, power: f32) {
        self.explosion_power.store(power, Ordering::Relaxed);
    }
}

impl EntityBase for FireballEntity {
    fn get_owner_id(&self) -> Option<i32> {
        self.fireball.thrown.owner_id.load()
    }

    fn set_owner_id(&self, owner: Option<i32>) {
        self.fireball.thrown.owner_id.store(owner);
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        self.fireball.write_nbt(nbt);
        nbt.put_byte("ExplosionPower", self.get_explosion_power() as i8);
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        self.fireball.read_nbt(nbt);
        self.set_explosion_power(f32::from(nbt.get_byte("ExplosionPower").unwrap_or(1)));
    }

    fn init_data_tracker(&self) {
        self.fireball.init_data_tracker();
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.fireball.tick(caller, server);
    }

    fn get_entity(&self) -> &Entity {
        self.fireball.get_entity()
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        let world = self.get_entity().world.load();

        let this = world.get_entity_by_id(self.get_entity().entity_id);
        // Vanilla `LargeFireball.onHitEntity`: `fireball(this, owner)` for 6.
        if let ProjectileHit::Entity { ref entity, .. } = hit {
            let owner = self
                .get_owner_id()
                .and_then(|id| world.get_entity_by_id(id));
            // TODO: vanilla `EnchantmentHelper.doPostAttackEffects` (the target's thorns).
            let _ = entity.damage_with_context(
                entity.as_ref(),
                6.0,
                Fireball::damage_type(owner.as_deref()),
                None,
                this.as_deref(),
                owner.as_deref(),
            );
        }

        // Vanilla `LargeFireball.onHit`: at its own position, setting fire with mob griefing.
        let grief = world.level_info.load().game_rules.mob_griefing;
        world.explode_full(
            this,
            None,
            None,
            self.get_entity().pos.load(),
            self.get_explosion_power(),
            grief,
            crate::world::ExplosionInteraction::Mob,
            crate::world::ExplosionEffects::DEFAULT,
        );
    }
}
