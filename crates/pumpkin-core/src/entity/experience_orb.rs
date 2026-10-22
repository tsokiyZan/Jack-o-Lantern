use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicI32, Ordering},
};

use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::{server::Server, world::World};

use super::{Entity, EntityBase, living::LivingEntity, player::Player};

/// Vanilla `ExperienceOrb.LIFETIME`.
const LIFETIME: i32 = 6000;
/// Vanilla `ExperienceOrb.ENTITY_SCAN_PERIOD`.
const ENTITY_SCAN_PERIOD: i32 = 20;
/// Vanilla `ExperienceOrb.MAX_FOLLOW_DIST`.
const MAX_FOLLOW_DIST: f64 = 8.0;
/// Vanilla `ExperienceOrb.ORB_GROUPS_PER_AREA`.
const ORB_GROUPS_PER_AREA: i32 = 40;
/// Vanilla `ExperienceOrb.DEFAULT_HEALTH`.
const DEFAULT_HEALTH: i32 = 5;

/// Vanilla `ExperienceOrb`.
pub struct ExperienceOrbEntity {
    entity: Entity,
    value: AtomicI32,
    age: AtomicI32,
    health: AtomicI32,
    count: AtomicI32,
    following_player: Mutex<Option<Weak<Player>>>,
}

impl ExperienceOrbEntity {
    /// Vanilla `ExperienceOrb(type, level)`: a value 0 orb, as `/summon` makes.
    pub const fn new(entity: Entity) -> Self {
        Self {
            entity,
            value: AtomicI32::new(0),
            age: AtomicI32::new(0),
            health: AtomicI32::new(DEFAULT_HEALTH),
            count: AtomicI32::new(1),
            following_player: Mutex::new(None),
        }
    }

    /// Vanilla `ExperienceOrb(level, pos, roughly, value)`: turned and thrown at random, the
    /// throw flipped to lean along `roughly` and the orb moved half its size that way.
    fn new_awarded(entity: Entity, roughly: Vector3<f64>, value: i32) -> Self {
        let (yaw, random_movement) = {
            let mut random = entity.random();
            let yaw = random.next_f32() * 360.0;
            let x = (random.next_f64() * 0.2 - 0.1) * 2.0;
            let y = random.next_f64() * 0.2 * 2.0;
            let z = (random.next_f64() * 0.2 - 0.1) * 2.0;
            (yaw, Vector3::new(x, y, z))
        };
        let mut random_movement = random_movement;
        if roughly.length_squared() > 0.0 && roughly.dot(&random_movement) < 0.0 {
            random_movement = random_movement * -1.0;
        }
        // `Vec3.normalize`: too short a vector normalizes to zero.
        let length = roughly.length();
        if length >= 1.0E-5 {
            let size = entity.bounding_box.load().get_average_side_length();
            let pos = entity.pos.load();
            let normal = Vector3::new(roughly.x / length, roughly.y / length, roughly.z / length);
            entity.set_pos(pos.add(&(normal * (size * 0.5))));
        }
        entity.yaw.store(yaw);
        entity.velocity.store(random_movement);
        // TODO: vanilla `unstuckIfPossible` when spawned inside a block; it needs
        // `CollisionGetter.findFreePosition`.
        let orb = Self::new(entity);
        orb.set_value(value);
        orb
    }

    /// Vanilla `ExperienceOrb.award`: orbs worth `amount` in all, each one merged into a
    /// matching orb nearby when one exists.
    pub fn spawn(world: &Arc<World>, position: Vector3<f64>, amount: u32) {
        Self::spawn_with_direction(world, position, Vector3::new(0.0, 0.0, 0.0), amount);
    }

    /// Vanilla `ExperienceOrb.awardWithDirection`.
    pub fn spawn_with_direction(
        world: &Arc<World>,
        position: Vector3<f64>,
        rough_direction: Vector3<f64>,
        amount: u32,
    ) {
        let mut amount = amount as i32;
        while amount > 0 {
            let value = Self::get_experience_value(amount);
            amount -= value;
            if !Self::try_merge_to_existing(world, position, value) {
                let entity = Entity::new(world.clone(), position, &EntityType::EXPERIENCE_ORB);
                world.spawn_entity(Arc::new(Self::new_awarded(entity, rough_direction, value)));
            }
        }
    }

    /// Vanilla `ExperienceOrb.tryMergeToExisting`.
    fn try_merge_to_existing(world: &World, position: Vector3<f64>, value: i32) -> bool {
        // `level.getRandom().nextInt(40)`.
        let id = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_bounded_i32(ORB_GROUPS_PER_AREA);
        let area = pumpkin_util::math::boundingbox::BoundingBox::new(
            position.sub(&Vector3::new(0.5, 0.5, 0.5)),
            position.add(&Vector3::new(0.5, 0.5, 0.5)),
        );
        for candidate in world.get_all_at_box(&area) {
            if let Some(orb) = candidate.cast_any().downcast_ref::<Self>()
                && orb.can_merge_with(id, value)
            {
                orb.count.fetch_add(1, Ordering::Relaxed);
                orb.age.store(0, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    /// Vanilla static `ExperienceOrb.canMerge(orb, id, value)`.
    fn can_merge_with(&self, id: i32, value: i32) -> bool {
        !self.entity.is_removed()
            && (self.entity.entity_id - id) % ORB_GROUPS_PER_AREA == 0
            && self.get_value() == value
    }

    /// Vanilla `ExperienceOrb.scanForMerges`.
    fn scan_for_merges(&self) {
        let world = self.entity.world.load();
        let area = self.entity.bounding_box.load().expand(0.5, 0.5, 0.5);
        for candidate in world.get_all_at_box(&area) {
            if let Some(orb) = candidate.cast_any().downcast_ref::<Self>()
                && orb.entity.entity_id != self.entity.entity_id
                && orb.can_merge_with(self.entity.entity_id, self.get_value())
            {
                // `merge`.
                self.count
                    .fetch_add(orb.count.load(Ordering::Relaxed), Ordering::Relaxed);
                self.age
                    .fetch_min(orb.age.load(Ordering::Relaxed), Ordering::Relaxed);
                orb.entity.remove();
            }
        }
    }

    /// Vanilla `ExperienceOrb.followNearbyPlayer`.
    fn follow_nearby_player(&self) {
        let entity = &self.entity;
        let pos = entity.pos.load();
        let mut following = self
            .following_player
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = following.as_ref().and_then(Weak::upgrade);
        if current.as_ref().is_none_or(|player| {
            player.is_spectator()
                || player.get_entity().pos.load().squared_distance_to_vec(&pos) > 64.0
        }) {
            // `getNearestPlayer(this, 8.0)`, which already leaves spectators out.
            let nearest = entity
                .world
                .load()
                .get_nearest_player(pos, MAX_FOLLOW_DIST, |player| !player.is_spectator())
                .filter(|player| player.living_entity.health.load() > 0.0);
            *following = nearest.as_ref().map(Arc::downgrade);
        }

        if let Some(player) = following.as_ref().and_then(Weak::upgrade) {
            let player_entity = player.get_entity();
            let player_pos = player_entity.pos.load();
            let delta = Vector3::new(
                player_pos.x - pos.x,
                player_pos.y + player_entity.get_eye_height() / 2.0 - pos.y,
                player_pos.z - pos.z,
            );
            let power = 1.0 - delta.length_squared().sqrt() / 8.0;
            entity
                .velocity
                .store(entity.velocity.load() + delta.normalize() * (power * power * 0.1));
        }
    }

    /// Vanilla `ExperienceOrb.getExperienceValue`.
    #[must_use]
    pub const fn get_experience_value(max_value: i32) -> i32 {
        if max_value >= 2477 {
            2477
        } else if max_value >= 1237 {
            1237
        } else if max_value >= 617 {
            617
        } else if max_value >= 307 {
            307
        } else if max_value >= 149 {
            149
        } else if max_value >= 73 {
            73
        } else if max_value >= 37 {
            37
        } else if max_value >= 17 {
            17
        } else if max_value >= 7 {
            7
        } else if max_value >= 3 {
            3
        } else {
            1
        }
    }

    pub fn get_value(&self) -> i32 {
        self.value.load(Ordering::Relaxed)
    }

    fn set_value(&self, value: i32) {
        self.value.store(value, Ordering::Relaxed);
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::experience_orb::DATA_VALUE,
            VarInt(value),
        );
    }
}

impl EntityBase for ExperienceOrbEntity {
    /// Vanilla `ExperienceOrb.isAttackable`.
    fn is_attackable(&self) -> bool {
        false
    }

    /// Vanilla `ExperienceOrb.tick`.
    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        let entity = &self.entity;
        entity.tick(caller, server);
        if entity.is_removed() {
            return;
        }
        let world = entity.world.load();
        let colliding = !world.is_space_empty(entity.bounding_box.load());
        let mut velocity = entity.velocity.load();
        if entity.eye_in_water.load(Ordering::Relaxed) {
            // `setUnderwaterMovement`.
            velocity = Vector3::new(
                velocity.x * f64::from(0.99f32),
                (velocity.y + f64::from(5.0E-4f32)).min(f64::from(0.06f32)),
                velocity.z * f64::from(0.99f32),
            );
        } else if !colliding {
            velocity.y -= self.current_gravity();
        }
        entity.velocity.store(velocity);

        if world
            .get_fluid(&entity.block_pos.load())
            .has_tag(&tag::Fluid::MINECRAFT_LAVA)
        {
            let velocity = {
                let mut random = entity.random();
                let x = (random.next_f32() - random.next_f32()) * 0.2;
                let z = (random.next_f32() - random.next_f32()) * 0.2;
                Vector3::new(f64::from(x), f64::from(0.2f32), f64::from(z))
            };
            entity.velocity.store(velocity);
        }

        if entity.tick_count.load(Ordering::Relaxed) % ENTITY_SCAN_PERIOD == 1 {
            self.scan_for_merges();
        }

        self.follow_nearby_player();
        let following = self
            .following_player
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some();
        if !following && colliding {
            let bounding_box = entity.bounding_box.load();
            let velocity = entity.velocity.load();
            let next = bounding_box.shift(velocity);
            if !world.is_space_empty(next) {
                // `moveTowardsClosestSpace`.
                entity.push_out_of_blocks(Vector3::new(
                    entity.pos.load().x,
                    f64::midpoint(bounding_box.min.y, bounding_box.max.y),
                    entity.pos.load().z,
                ));
                entity.velocity_dirty.store(true, Ordering::SeqCst);
            }
        }

        let fall_speed = entity.velocity.load().y;
        entity.move_entity(caller, entity.velocity.load());
        // `applyEffectsFromBlocks`.
        entity.tick_block_collisions(caller);
        // `getAirDrag`, times the friction of the block below on the ground, all floats.
        let mut friction = 0.98f32;
        if entity.on_ground.load(Ordering::SeqCst) {
            friction *= entity.get_block_with_y_offset(0.999_999).1.slipperiness;
        }
        let mut velocity = entity.velocity.load() * f64::from(friction);
        if entity.vertical_collision_below.load(Ordering::Relaxed)
            && fall_speed < -self.current_gravity()
        {
            velocity.y = -fall_speed * 0.4;
        }
        entity.velocity.store(velocity);

        if self.age.fetch_add(1, Ordering::Relaxed) + 1 >= LIFETIME {
            entity.remove();
        }
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    /// Vanilla `ExperienceOrb.playerTouch`.
    fn on_player_collision(&self, player: &Arc<Player>) {
        if player.living_entity.health.load() > 0.0 {
            let can_pickup = if let Ok(mut delay) = player.experience_pick_up_delay.try_lock()
                && *delay == 0
            {
                *delay = 2;
                true
            } else {
                false
            };
            if can_pickup {
                player.living_entity.pickup(&self.entity, 1);
                let remaining = player.apply_mending_from_xp(self.get_value());
                if remaining > 0 {
                    player.add_experience_points(remaining);
                }
                if self.count.fetch_sub(1, Ordering::Relaxed) - 1 == 0 {
                    self.entity.remove();
                }
            }
        }
    }

    /// Vanilla `ExperienceOrb.hurtServer`: an int health that only base invulnerability
    /// protects.
    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        _source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if self.entity.is_invulnerable_to(&damage_type, cause) {
            return false;
        }
        // `markHurt`.
        self.entity.velocity_dirty.store(true, Ordering::SeqCst);
        let health = (self.health.load(Ordering::Relaxed) as f32 - amount) as i32;
        self.health.store(health, Ordering::Relaxed);
        if health <= 0 {
            self.entity.remove();
        }
        true
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_short("Health", self.health.load(Ordering::Relaxed) as i16);
        nbt.put_short("Age", self.age.load(Ordering::Relaxed) as i16);
        nbt.put_short("Value", self.get_value() as i16);
        nbt.put_int("Count", self.count.load(Ordering::Relaxed));
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        self.health.store(
            i32::from(nbt.get_short("Health").unwrap_or(DEFAULT_HEALTH as i16)),
            Ordering::Relaxed,
        );
        self.age.store(
            i32::from(nbt.get_short("Age").unwrap_or(0)),
            Ordering::Relaxed,
        );
        self.set_value(i32::from(nbt.get_short("Value").unwrap_or(0)));
        // `ExtraCodecs.POSITIVE_INT`: anything else falls back to 1.
        self.count.store(
            nbt.get_int("Count").filter(|count| *count > 0).unwrap_or(1),
            Ordering::Relaxed,
        );
    }

    fn init_data_tracker(&self) {
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::experience_orb::DATA_VALUE,
            VarInt(self.get_value()),
        );
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    /// Vanilla `ExperienceOrb.getDefaultGravity`.
    fn get_gravity(&self) -> f64 {
        0.03
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn movement_emission(&self) -> crate::entity::movement_emission::MovementEmission {
        crate::entity::movement_emission::MovementEmission::None
    }
}
