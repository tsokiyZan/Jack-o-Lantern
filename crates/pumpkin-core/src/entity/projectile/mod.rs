use super::{
    Entity, EntityBase, living::LivingEntity, projectile_deflection::ProjectileDeflectionType,
};
use crate::entity::entity_reference::EntityReference;
use crate::world::game_event::GameEventContext;
use pumpkin_data::BlockDirection;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::java::client::play::CEntityVelocity;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::{DEG_TO_RAD, atan2, cos, position::BlockPos, sin, vector3::Vector3};
use pumpkin_util::random::RandomImpl;
use std::{
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
};
/// Vanilla `Projectile.shoot`: the movement from `getMovementToShoot`, with its three
/// `random.triangle`s from the projectile's random, and the rotation facing along it.
pub fn shoot(entity: &Entity, x: f64, y: f64, z: f64, power: f64, uncertainty: f64) {
    let (dx, dy, dz) = {
        let mut random = entity.random();
        let dx = random.next_triangular(0.0, 0.017_227_5 * uncertainty);
        let dy = random.next_triangular(0.0, 0.017_227_5 * uncertainty);
        let dz = random.next_triangular(0.0, 0.017_227_5 * uncertainty);
        (dx, dy, dz)
    };
    let velocity = Vector3::new(x, y, z)
        .normalize()
        .add_raw(dx, dy, dz)
        .multiply(power, power, power);

    entity.velocity.store(velocity);
    // `(float)(Mth.atan2(..) * 180.0F / (float)Math.PI)`.
    let to_degrees = |radians: f64| (radians * 180.0 / f64::from(std::f32::consts::PI)) as f32;
    entity.set_rotation(
        to_degrees(atan2(velocity.x, velocity.z)),
        to_degrees(atan2(velocity.y, velocity.horizontal_length())),
    );
}

/// The direction `Projectile.shootFromRotation` shoots along, in `Mth` table trig.
#[must_use]
pub fn rotation_shot_vector(x_rot: f32, y_rot: f32, y_offset: f32) -> Vector3<f64> {
    let xd = -sin(y_rot * DEG_TO_RAD) * cos(x_rot * DEG_TO_RAD);
    let yd = -sin((x_rot + y_offset) * DEG_TO_RAD);
    let zd = cos(y_rot * DEG_TO_RAD) * cos(x_rot * DEG_TO_RAD);
    Vector3::new(f64::from(xd), f64::from(yd), f64::from(zd))
}

/// Vanilla `Projectile.shootFromRotation`: `shoot` along the rotation with `Mth` table trig,
/// then the source's known movement on top (its vertical part only off the ground).
pub fn shoot_from_rotation(
    entity: &Entity,
    source: &dyn EntityBase,
    x_rot: f32,
    y_rot: f32,
    y_offset: f32,
    power: f32,
    uncertainty: f32,
) {
    let direction = rotation_shot_vector(x_rot, y_rot, y_offset);
    shoot(
        entity,
        direction.x,
        direction.y,
        direction.z,
        f64::from(power),
        f64::from(uncertainty),
    );
    let source_movement = source.get_known_movement();
    let source_y = if source
        .get_entity()
        .on_ground
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        0.0
    } else {
        source_movement.y
    };
    entity.velocity.store(
        entity.velocity.load() + Vector3::new(source_movement.x, source_y, source_movement.z),
    );
}

pub mod arrow;
pub mod dragon_fireball;
pub mod egg;
pub mod ender_pearl;
pub mod evoker_fangs;
pub mod eye_of_ender;
pub mod fireball;
pub mod firework_rocket;
pub mod fishing_bobber;
pub mod hurting_projectile;
pub mod lingering_potion;
pub mod llama_spit;
pub mod shulker_bullet;
pub mod small_fireball;
pub mod snowball;
pub mod splash_potion;
pub mod trident;
pub mod wind_charge;
pub mod wither_skull;

use pumpkin_data::item_stack::ItemStack;

/// The entity half of vanilla `Projectile.hitTargetOrDeflectSelf`: whether the entity `hit`
/// turned `projectile` away instead of being hit. The same entity never deflects it twice in
/// a row.
pub fn is_deflected(
    projectile: &dyn EntityBase,
    last_deflected_by: &crossbeam::atomic::AtomicCell<Option<i32>>,
    hit: &ProjectileHit,
) -> bool {
    let ProjectileHit::Entity { entity: target, .. } = hit else {
        return false;
    };
    let deflection = target.deflection(projectile);
    if matches!(deflection, ProjectileDeflectionType::None) {
        return false;
    }
    let target_id = target.get_entity().entity_id;
    if last_deflected_by.load() != Some(target_id) {
        // Vanilla `Projectile.deflect` keeps the owner; the caller runs `onDeflection`.
        deflection.deflect(projectile, Some(target.as_ref()));
        last_deflected_by.store(Some(target_id));
    }
    true
}

#[must_use]
pub fn is_projectile(entity_type: &EntityType) -> bool {
    *entity_type == EntityType::ARROW
        || *entity_type == EntityType::SPECTRAL_ARROW
        || *entity_type == EntityType::TRIDENT
        || *entity_type == EntityType::EXPERIENCE_BOTTLE
        || *entity_type == EntityType::EGG
        || *entity_type == EntityType::SNOWBALL
        || *entity_type == EntityType::FIREWORK_ROCKET
        || *entity_type == EntityType::WIND_CHARGE
        || *entity_type == EntityType::BREEZE_WIND_CHARGE
        || *entity_type == EntityType::SPLASH_POTION
        || *entity_type == EntityType::LINGERING_POTION
        || *entity_type == EntityType::ENDER_PEARL
        || *entity_type == EntityType::SHULKER_BULLET
        || *entity_type == EntityType::FIREBALL
        || *entity_type == EntityType::SMALL_FIREBALL
        || *entity_type == EntityType::FISHING_BOBBER
        || *entity_type == EntityType::WITHER_SKULL
        || *entity_type == EntityType::DRAGON_FIREBALL
        || *entity_type == EntityType::LLAMA_SPIT
}

/// Helper to apply projectile spawned enchantment effects matching vanilla `Projectile::applyOnProjectileSpawned`.
pub fn apply_on_projectile_spawned(
    projectile_entity: &Entity,
    pickup_item_stack: &ItemStack,
    weapon: Option<&ItemStack>,
    arrow: Option<&arrow::ArrowEntity>,
) {
    crate::enchantment::EnchantmentHelper::on_projectile_spawned(
        pickup_item_stack,
        projectile_entity,
        arrow,
    );
    if let Some(weapon) = weapon
        && weapon.item_count > 0
        && weapon.item.id != pickup_item_stack.item.id
    {
        crate::enchantment::EnchantmentHelper::on_projectile_spawned(
            weapon,
            projectile_entity,
            arrow,
        );
    }
}

/// Vanilla `Projectile`'s own state: who shot it, whether it has left them, and whether it has
/// been shot yet.
pub struct Projectile {
    /// Vanilla `Projectile.owner`; an explosion can hand a redirectable projectile over.
    owner: EntityReference,
    /// Vanilla `Projectile.leftOwner`: until then, the owner's stack of riders can't be hit.
    left_owner: AtomicBool,
    /// Vanilla `Projectile.hasBeenShot`.
    has_been_shot: AtomicBool,
}

impl Projectile {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            owner: EntityReference::none(),
            left_owner: AtomicBool::new(false),
            has_been_shot: AtomicBool::new(false),
        }
    }

    #[must_use]
    pub fn owned_by(owner: &Entity) -> Self {
        let projectile = Self::new();
        projectile.set_owner(Some(owner));
        projectile
    }

    /// Vanilla `Projectile.getOwner`, for the projectile `entity`.
    #[must_use]
    pub fn get_owner(&self, entity: &Entity) -> Option<Arc<dyn EntityBase>> {
        self.owner.get(&entity.world.load())
    }

    #[must_use]
    pub fn get_owner_id(&self, entity: &Entity) -> Option<i32> {
        self.get_owner(entity)
            .map(|owner| owner.get_entity().entity_id)
    }

    /// Vanilla `Projectile.setOwner`.
    pub fn set_owner(&self, owner: Option<&Entity>) {
        self.owner.set(owner);
    }

    /// Vanilla `Projectile.ownedBy`.
    #[must_use]
    pub fn owned_by_entity(&self, entity: &Entity) -> bool {
        self.owner.matches(entity)
    }

    #[must_use]
    pub fn has_left_owner(&self) -> bool {
        self.left_owner.load(Ordering::Relaxed)
    }

    /// The start of vanilla `Projectile.tick`, before `Entity.tick`: the shoot game event on
    /// the first tick, then `checkLeftOwner`.
    pub fn tick(&self, entity: &Entity) {
        if !self.has_been_shot.swap(true, Ordering::Relaxed) {
            // `gameEvent(PROJECTILE_SHOOT, getOwner())`, at the projectile.
            let world = entity.world.load();
            let owner = self.get_owner(entity);
            world.game_event_from(
                owner.as_deref(),
                GameEvent::ProjectileShoot,
                entity.pos.load(),
            );
        }
        self.check_left_owner(entity);
    }

    /// Vanilla `Projectile.checkLeftOwner`.
    pub fn check_left_owner(&self, entity: &Entity) {
        if !self.left_owner.load(Ordering::Relaxed) {
            self.left_owner.store(
                self.is_outside_owner_collision_range(entity),
                Ordering::Relaxed,
            );
        }
    }

    /// Vanilla `Projectile.isOutsideOwnerCollisionRange`: no pickable entity of the owner's
    /// stack touches this tick's movement, grown by a block.
    fn is_outside_owner_collision_range(&self, entity: &Entity) -> bool {
        let Some(owner) = self.get_owner(entity) else {
            return true;
        };
        let world = entity.world.load();
        let aabb = entity
            .bounding_box
            .load()
            .stretch(entity.velocity.load())
            .expand(1.0, 1.0, 1.0);
        let Some(root) = world.get_entity_by_id(owner.get_entity().root_vehicle_id()) else {
            return true;
        };
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if !entity.is_spectator()
                && entity.can_hit()
                && aabb.intersects(&entity.get_entity().bounding_box.load())
            {
                return false;
            }
            stack.extend(
                entity
                    .get_entity()
                    .passengers
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .cloned(),
            );
        }
        true
    }

    /// The owner half of vanilla `Projectile.canHitEntity`: not the owner's stack of riders
    /// until the projectile has left it.
    #[must_use]
    pub fn can_hit_owner_stack(&self, entity: &Entity, other: &Entity) -> bool {
        self.left_owner.load(Ordering::Relaxed)
            || self
                .get_owner(entity)
                .is_none_or(|owner| owner.get_entity().root_vehicle_id() != other.root_vehicle_id())
    }

    /// Vanilla `Projectile.addAdditionalSaveData`.
    pub fn write_nbt(&self, nbt: &mut NbtCompound) {
        if let Some(owner) = self.owner.uuid() {
            nbt.put_uuid("Owner", owner);
        }
        if self.left_owner.load(Ordering::Relaxed) {
            nbt.put_bool("LeftOwner", true);
        }
        nbt.put_bool("HasBeenShot", self.has_been_shot.load(Ordering::Relaxed));
    }

    /// Vanilla `Projectile.readAdditionalSaveData`.
    pub fn read_nbt(&self, nbt: &NbtCompound) {
        self.owner.set_uuid(nbt.get_uuid("Owner"));
        self.left_owner.store(
            nbt.get_bool("LeftOwner").unwrap_or(false),
            Ordering::Relaxed,
        );
        self.has_been_shot.store(
            nbt.get_bool("HasBeenShot").unwrap_or(false),
            Ordering::Relaxed,
        );
    }
}

impl Default for Projectile {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ThrownItemEntity {
    pub entity: Entity,
    pub projectile: Projectile,
    pub collides_with_projectiles: bool,
    pub has_hit: AtomicBool,
    /// Vanilla `Projectile.lastDeflectedBy`.
    pub last_deflected_by: crossbeam::atomic::AtomicCell<Option<i32>>,
    pub gravity: f64,
}

impl ThrownItemEntity {
    pub fn new(entity: Entity, owner: &Entity, gravity: f64) -> Self {
        let mut owner_pos = owner.pos.load();
        owner_pos.y += owner.get_eye_height() - 0.1;
        entity.pos.store(owner_pos);
        let thrown = Self::unowned(entity, gravity);
        thrown.projectile.set_owner(Some(owner));
        thrown
    }

    /// A projectile without an owner, where `entity` stands.
    #[must_use]
    pub const fn unowned(entity: Entity, gravity: f64) -> Self {
        Self {
            entity,
            projectile: Projectile::new(),
            collides_with_projectiles: false,
            has_hit: AtomicBool::new(false),
            last_deflected_by: crossbeam::atomic::AtomicCell::new(None),
            gravity,
        }
    }

    /// Vanilla `Projectile.shootFromRotation`.
    pub fn set_velocity_from(
        &self,
        source: &dyn EntityBase,
        pitch: f32,
        yaw: f32,
        roll: f32,
        speed: f32,
        divergence: f32,
    ) {
        shoot_from_rotation(&self.entity, source, pitch, yaw, roll, speed, divergence);
    }

    /// Vanilla `Projectile.shoot`.
    pub fn set_velocity(&self, x: f64, y: f64, z: f64, power: f64, uncertainty: f64) {
        shoot(&self.entity, x, y, z, power, uncertainty);
    }
}

impl ThrownItemEntity {
    /// Process a tick for projectile movement and collisions
    pub fn process_tick(&self, caller: &dyn EntityBase) {
        let entity = self.get_entity();
        let world = entity.world.load();

        entity.update_last_pos();

        // Apply gravity and inertia
        let mut velocity = entity.velocity.load();
        // Vanilla `getGravity`: none while the entity has `NoGravity`.
        if !entity.has_no_gravity() {
            velocity.y -= self.get_gravity();
        }

        // Vanilla `ThrowableProjectile.applyInertia`: float inertia, widened.
        let inertia = f64::from(if entity.touching_water.load(Ordering::Relaxed) {
            0.8f32
        } else {
            0.99f32
        });
        velocity = velocity.multiply(inertia, inertia, inertia);

        // Store velocity
        entity.velocity.store(velocity);

        let start_pos = entity.pos.load();
        let delta = velocity;

        // Update position
        let new_pos = start_pos.add(&delta);
        entity.set_pos(new_pos);

        // Send updated velocity to clients
        let packet = CEntityVelocity::new(entity.entity_id.into(), velocity);
        let chunk_pos = entity.chunk_pos.load();
        world.broadcast_to_chunk(chunk_pos, &packet);

        let hit = self.hit_result_on_move_vector(start_pos, delta);
        // `Projectile.tick`, between the move and `hitTargetOrDeflectSelf`.
        self.projectile.tick(entity);
        if let Some(h) = hit {
            if is_deflected(caller, &self.last_deflected_by, &h) {
                return;
            }
            // Vanilla moves the projectile to the hit location, not the full step, before
            // `hitTargetOrDeflectSelf`, so effects such as a wind charge's burst start there.
            entity.set_pos(h.hit_pos());
            self.on_hit_result(caller, h);
        }
    }

    /// Vanilla `ProjectileUtil.getHitResultOnMoveVector` with a `COLLIDER` clip, from
    /// `start_pos` along `delta`.
    pub(crate) fn hit_result_on_move_vector(
        &self,
        start_pos: Vector3<f64>,
        delta: Vector3<f64>,
    ) -> Option<ProjectileHit> {
        let entity = &self.entity;
        let world = entity.world.load();
        let new_pos = start_pos.add(&delta);

        // Calculate search box for collisions
        let search_box = BoundingBox::new(
            Vector3::new(
                start_pos.x.min(new_pos.x),
                start_pos.y.min(new_pos.y),
                start_pos.z.min(new_pos.z),
            ),
            Vector3::new(
                start_pos.x.max(new_pos.x),
                start_pos.y.max(new_pos.y),
                start_pos.z.max(new_pos.z),
            ),
        )
        .expand(0.3, 0.3, 0.3);

        let mut closest_t = 1.0f64;
        let mut hit = None;

        // Vanilla `ProjectileUtil.getHitResult`: a `COLLIDER` clip along the movement.
        let clip = world.clip(&pumpkin_world::clip::ClipContext::new(
            start_pos,
            new_pos,
            pumpkin_world::clip::ClipBlock::Collider,
            pumpkin_world::clip::ClipFluid::None,
        ));
        if !clip.is_miss {
            let length_squared = delta.length_squared();
            if length_squared > 0.0 {
                closest_t = clip.location.sub(&start_pos).dot(&delta) / length_squared;
            }
            hit = Some(ProjectileHit::Block {
                pos: clip.block_pos,
                face: clip.direction,
                hit_pos: clip.location,
                normal: delta.normalize().multiply(-1.0, -1.0, -1.0),
            });
        }

        // Vanilla `ProjectileUtil.getEntityHitResult` searches every entity, players included,
        // with `computeMargin`: target boxes grow from 0 to 0.3 over the first ticks.
        let candidates = world.get_all_at_box(&search_box);
        let margin = f64::from(
            ((entity.tick_count.load(Ordering::Relaxed) - 2) as f32 / 20.0).clamp(0.0, 0.3),
        );
        for cand in candidates {
            if self.should_skip_collision(entity, &cand) {
                continue;
            }

            let ebb = cand
                .get_entity()
                .bounding_box
                .load()
                .expand(margin, margin, margin);
            if let Some(t) = calculate_ray_intersection(&start_pos, &delta, &ebb)
                && t < closest_t
            {
                closest_t = t;
                let hit_pos = start_pos.add(&delta.multiply(t, t, t));
                hit = Some(ProjectileHit::Entity {
                    entity: cand.clone(),
                    hit_pos,
                    normal: delta.normalize().multiply(-1.0, -1.0, -1.0),
                });
            }
        }
        hit
    }

    /// The `onHit` half of vanilla `Projectile.hitTargetOrDeflectSelf`, then the `discard`
    /// every projectile here does after a hit.
    pub(crate) fn on_hit_result(&self, caller: &dyn EntityBase, h: ProjectileHit) {
        // Ensure hit is only processed once per projectile
        if self.has_hit.swap(true, Ordering::SeqCst) {
            return;
        }
        let world = self.entity.world.load();
        if let ProjectileHit::Block { pos, hit_pos, .. } = &h {
            let block = world.get_block(pos);
            let state = world.get_block_state(pos);
            if let Some(server) = world.server.upgrade() {
                world
                    .block_registry
                    .on_projectile_hit(block, &world, caller, pos, state, hit_pos, &server);
            }
        }

        // Just trigger hit effects and remove
        on_hit(caller, h);
        self.entity.remove();
    }

    /// Returns if collision should be skipped (e.g. owner or projectile vs projectile)
    fn should_skip_collision(&self, self_ent: &Entity, other: &Arc<dyn EntityBase>) -> bool {
        let other_ent = other.get_entity();
        if other_ent.entity_id == self_ent.entity_id {
            return true;
        }

        // Vanilla `Projectile.canHitEntity`: not the owner's stack until it has left it.
        if !self.projectile.can_hit_owner_stack(self_ent, other_ent) {
            return true;
        }

        // Vanilla `canHitEntity`'s `canBeHitByProjectile`: alive and pickable, which leaves out
        // spectators, clouds, the dragon's body, dying mobs and most projectiles.
        if !other.can_be_hit_by_projectile() {
            return true;
        }

        false
    }

    const fn get_entity(&self) -> &Entity {
        &self.entity
    }

    #[allow(dead_code, clippy::unused_self)]
    const fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    const fn get_gravity(&self) -> f64 {
        self.gravity
    }
}

/// Vanilla `AABB.clip` as a fraction of `dir`: where the segment enters `bb`, or `None` when
/// it misses or starts inside.
fn calculate_ray_intersection(
    start: &Vector3<f64>,
    dir: &Vector3<f64>,
    bb: &BoundingBox,
) -> Option<f64> {
    let mut t_min = f64::NEG_INFINITY;
    let mut t_max = 1.0f64;

    let b_min = [bb.min.x, bb.min.y, bb.min.z];
    let b_max = [bb.max.x, bb.max.y, bb.max.z];
    let s = [start.x, start.y, start.z];
    let d = [dir.x, dir.y, dir.z];

    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            if s[i] < b_min[i] || s[i] > b_max[i] {
                return None;
            }
        } else {
            let t1 = (b_min[i] - s[i]) / d[i];
            let t2 = (b_max[i] - s[i]) / d[i];
            t_min = t_min.max(t1.min(t2));
            t_max = t_max.min(t1.max(t2));
        }
    }

    (t_min <= t_max && (0.0..=1.0).contains(&t_min)).then_some(t_min)
}

/// Vanilla `Projectile.onHit`: the projectile's own hit handling, then `projectile_land` at
/// the entity's hit location or the hit block.
pub fn on_hit(projectile: &dyn EntityBase, hit: ProjectileHit) {
    let hit_block = match &hit {
        ProjectileHit::Block { pos, .. } => Some(*pos),
        ProjectileHit::Entity { .. } => None,
    };
    let hit_pos = hit.hit_pos();
    projectile.on_hit(hit);
    let world = projectile.get_entity().world.load();
    let (position, affected_state) = hit_block.map_or((hit_pos, None), |pos| {
        (pos.to_centered_f64(), Some(world.get_block_state(&pos)))
    });
    world.game_event(
        GameEvent::ProjectileLand,
        position,
        GameEventContext::of(Some(projectile), affected_state),
    );
}

/// Get the face of the block that was hit
pub enum ProjectileHit {
    Block {
        pos: BlockPos,
        face: BlockDirection,
        hit_pos: Vector3<f64>,
        normal: Vector3<f64>,
    },
    Entity {
        entity: Arc<dyn EntityBase>,
        hit_pos: Vector3<f64>,
        normal: Vector3<f64>,
    },
}

impl ProjectileHit {
    /// Returns the exact impact coordinates regardless of what was hit.
    #[must_use]
    pub const fn hit_pos(&self) -> Vector3<f64> {
        match self {
            Self::Block { hit_pos, .. } | Self::Entity { hit_pos, .. } => *hit_pos,
        }
    }

    /// Returns the surface normal of the impact.
    #[must_use]
    pub const fn normal(&self) -> Vector3<f64> {
        match self {
            Self::Block { normal, .. } | Self::Entity { normal, .. } => *normal,
        }
    }

    /// Safely returns the face hit if it was a block, otherwise None.
    #[must_use]
    pub const fn face(&self) -> Option<BlockDirection> {
        match self {
            Self::Block { face, .. } => Some(*face),
            Self::Entity { .. } => None,
        }
    }
}
