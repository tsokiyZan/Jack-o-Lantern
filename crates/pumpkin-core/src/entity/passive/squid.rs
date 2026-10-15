use crate::entity::breath::BreathManager;
use pumpkin_data::entity::EntityPose;
use pumpkin_util::math::boundingbox::EntityDimensions;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crossbeam::atomic::AtomicCell;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crate::entity::{
    Entity, EntityBase,
    ai::util::goal_utils,
    living::LivingEntity,
    mob::{Mob, MobEntity},
};

/// Represents a Squid, a passive aquatic mob that swims by pulsing its tentacles.
///
/// Wiki: <https://minecraft.wiki/w/Squid>
pub struct SquidEntity {
    pub mob_entity: MobEntity,
    pub movement: SquidMovement,
}

impl SquidEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        Arc::new(Self {
            mob_entity: MobEntity::new(entity),
            movement: SquidMovement::new(),
        })
    }
}

/// Vanilla `Squid.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    EntityDimensions::scalable(0.5, 0.5).with_eye_height(0.37);

impl Mob for SquidEntity {
    /// Vanilla `Squid.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    /// Vanilla `AgeableWaterCreature.handleAirSupply`.
    fn handle_air_supply(&self, pre_tick_air: i32) {
        BreathManager::water_animal_handle_air_supply(self, pre_tick_air);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.movement.tick(self);
    }

    fn on_damage(
        &self,
        _damage_type: pumpkin_data::damage::DamageType,
        _source: Option<&dyn EntityBase>,
    ) {
        self.movement
            .spawn_ink_if_hurt_by_mob(self, Particle::SquidInk, Sound::EntitySquidSquirt);
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        SquidMovement::travel(self, caller)
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }
}

/// How vanilla squid get about, which is nothing like other mobs: no navigation,
/// just a heading picked now and then and a shove along it once per tentacle
/// stroke.
pub struct SquidMovement {
    movement_vector: AtomicCell<Vector3<f64>>,
    tentacle_movement: AtomicCell<f32>,
    tentacle_speed: AtomicCell<f32>,
    x_body_rot: AtomicCell<f32>,
    x_body_rot_o: AtomicCell<f32>,
    y_body_rot_o: AtomicCell<f32>,
}

impl Default for SquidMovement {
    fn default() -> Self {
        Self::new()
    }
}

impl SquidMovement {
    #[must_use]
    pub fn new() -> Self {
        Self {
            movement_vector: AtomicCell::new(Vector3::default()),
            tentacle_movement: AtomicCell::new(0.0),
            tentacle_speed: AtomicCell::new(random_tentacle_speed(&mut rand::rng())),
            x_body_rot: AtomicCell::new(0.0),
            x_body_rot_o: AtomicCell::new(0.0),
            y_body_rot_o: AtomicCell::new(0.0),
        }
    }

    /// One tick of vanilla's `Squid.aiStep`, with its two goals folded in:
    /// `SquidFleeGoal` steers away from whoever hurt the squid, and otherwise
    /// `SquidRandomMovementGoal` picks a new heading about every 50 ticks.
    pub fn tick(&self, mob: &dyn Mob) {
        let living = &mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        let in_water = entity.touching_water.load(Ordering::Relaxed);
        let mut rng = mob.get_random();

        self.x_body_rot_o.store(self.x_body_rot.load());
        self.y_body_rot_o.store(entity.body_yaw.load());

        let fleeing = in_water && self.flee(living);
        if !fleeing
            && (rng.random_range(0..50) == 0
                || !in_water
                || self.movement_vector.load() == Vector3::default())
        {
            let angle = rng.random::<f32>() * TAU;
            self.movement_vector.store(Vector3::new(
                f64::from(angle.cos() * 0.2),
                f64::from(-0.1 + rng.random::<f32>() * 0.2),
                f64::from(angle.sin() * 0.2),
            ));
        }

        let mut tentacle = self.tentacle_movement.load() + self.tentacle_speed.load();
        if tentacle > TAU {
            tentacle -= TAU;
            if rng.random_range(0..10) == 0 {
                self.tentacle_speed.store(random_tentacle_speed(&mut rng));
            }
            // Keeps the client's stroke animation in step with the shoves.
            entity
                .world
                .load()
                .send_entity_status(entity, EntityStatus::SquidAnimSynch, None);
        }
        self.tentacle_movement.store(tentacle);

        if in_water {
            // Plain stores throughout: the tracker syncs motion, and set_velocity
            // would send a packet every tick.
            if tentacle < PI {
                if tentacle / PI > 0.75 {
                    entity.velocity.store(self.movement_vector.load());
                }
            } else {
                entity.velocity.store(entity.velocity.load() * 0.9);
            }
            // The body swings round to face where the squid is going.
            let velocity = entity.velocity.load();
            let heading = -(velocity.x.atan2(velocity.z) as f32).to_degrees();
            let body_yaw = entity.body_yaw.load();
            let body_yaw = body_yaw + (heading - body_yaw) * 0.1;
            entity.body_yaw.store(body_yaw);
            entity.yaw.store(body_yaw);
            let pitch = -(velocity.horizontal_length().atan2(velocity.y) as f32).to_degrees();
            let x_body_rot = self.x_body_rot.load();
            self.x_body_rot
                .store(x_body_rot + (pitch - x_body_rot) * 0.1);
        } else {
            let fall = entity.velocity.load().y - living.get_attribute_value(&Attributes::GRAVITY);
            entity.velocity.store(Vector3::new(0.0, fall * 0.98, 0.0));
            let x_body_rot = self.x_body_rot.load();
            self.x_body_rot
                .store(x_body_rot + (-90.0 - x_body_rot) * 0.02);
        }
    }

    /// The tail of vanilla's `Squid.hurtServer`: a squid that was hurt while it
    /// still remembers a mob attacking it squirts ink.
    pub fn spawn_ink_if_hurt_by_mob(&self, mob: &dyn Mob, ink_particle: Particle, squirt: Sound) {
        let living = &mob.get_mob_entity().living_entity;
        if has_last_hurt_by_mob(living) {
            self.spawn_ink(mob, ink_particle, squirt);
        }
    }

    /// Vanilla's `Squid.rotateVector`: turns a body-relative vector by last
    /// tick's body pitch and yaw.
    fn rotate_vector(&self, vec: Vector3<f64>) -> Vector3<f64> {
        let (sin, cos) = self.x_body_rot_o.load().to_radians().sin_cos();
        let (sin, cos) = (f64::from(sin), f64::from(cos));
        let pitched = Vector3::new(vec.x, vec.y * cos + vec.z * sin, vec.z * cos - vec.y * sin);
        let (sin, cos) = (-self.y_body_rot_o.load()).to_radians().sin_cos();
        let (sin, cos) = (f64::from(sin), f64::from(cos));
        Vector3::new(
            pitched.x * cos + pitched.z * sin,
            pitched.y,
            pitched.z * cos - pitched.x * sin,
        )
    }

    /// Vanilla's `Squid.spawnInk`.
    fn spawn_ink(&self, mob: &dyn Mob, ink_particle: Particle, squirt: Sound) {
        let entity = &mob.get_mob_entity().living_entity.entity;
        let world = entity.world.load();
        let mut rng = mob.get_random();
        let entity_pos = entity.pos.load();

        let pitch = (rng.random::<f32>() - rng.random::<f32>()) * 0.2 + 1.0;
        world.play_sound_fine(
            squirt,
            SoundCategory::Neutral,
            &entity_pos,
            SQUID_SOUND_VOLUME,
            pitch,
        );

        let pos = self.rotate_vector(Vector3::new(0.0, -1.0, 0.0)) + entity_pos;
        for _ in 0..INK_PARTICLE_COUNT {
            let dir = self.rotate_vector(Vector3::new(
                f64::from(rng.random::<f32>()) * 0.6 - 0.3,
                -1.0,
                f64::from(rng.random::<f32>()) * 0.6 - 0.3,
            ));
            // Baby squid use 0.1 here; Pumpkin's squid has no baby state yet.
            let dir_offset = dir * f64::from(INK_POS_OFFSET_SCALE + rng.random::<f32>() * 2.0);
            // A count of zero makes the client read the offset as a velocity.
            world.spawn_particle(
                Vector3::new(pos.x, pos.y + 0.5, pos.z),
                Vector3::new(
                    dir_offset.x as f32,
                    dir_offset.y as f32,
                    dir_offset.z as f32,
                ),
                0.1,
                0,
                ink_particle,
            );
        }
    }

    /// Vanilla's `Squid.travel`: a squid only ever moves by the velocity `tick`
    /// gave it.
    pub fn travel(mob: &dyn Mob, caller: &dyn EntityBase) -> bool {
        mob.get_mob_entity().living_entity.make_move(caller);
        true
    }

    /// `SquidFleeGoal`: while whoever last hurt the squid is within ten blocks,
    /// head straight away from them. Returns whether the squid is fleeing.
    fn flee(&self, living: &LivingEntity) -> bool {
        let entity = &living.entity;
        if !has_last_hurt_by_mob(living) {
            return false;
        }
        let attacker_id = living.last_attacker_id.load(Ordering::Relaxed);
        let world = entity.world.load();
        let Some(attacker) = world.get_entity_by_id(attacker_id) else {
            return false;
        };
        let pos = entity.pos.load();
        let mut away = pos - attacker.get_entity().pos.load();
        if away.length_squared() >= 100.0 {
            return false;
        }

        let probe = BlockPos::floored(pos.x + away.x, pos.y + away.y, pos.z + away.z);
        let open_air = world.get_block_state(&probe).is_air();
        if goal_utils::is_water(&world, &probe) || open_air {
            let length = away.length();
            if length > 0.0 {
                // Vanilla normalizes a copy of the offset here and discards it,
                // so the scale applies to the raw offset.
                let mut scale = 3.0;
                if length > 5.0 {
                    scale -= (length - 5.0) / 5.0;
                }
                if scale > 0.0 {
                    away = away * scale;
                }
            }
            if open_air {
                away.y = 0.0;
            }
            self.movement_vector.store(away / 20.0);
        }
        true
    }
}

/// Vanilla `Squid.getSoundVolume`.
const SQUID_SOUND_VOLUME: f32 = 0.4;
const INK_PARTICLE_COUNT: usize = 30;
const INK_POS_OFFSET_SCALE: f32 = 0.3;

/// Whether vanilla's `getLastHurtByMob` would still return someone.
fn has_last_hurt_by_mob(living: &LivingEntity) -> bool {
    let entity = &living.entity;
    // Vanilla forgets an attacker a hundred ticks after the hit.
    let since_hit = entity.tick_count.load(Ordering::Relaxed)
        - living.last_attacked_time.load(Ordering::Relaxed);
    living.last_attacker_id.load(Ordering::Relaxed) != 0 && since_hit <= 100
}

fn random_tentacle_speed(rng: &mut impl RngExt) -> f32 {
    1.0 / (rng.random::<f32>() + 1.0) * 0.2
}
