//! Vanilla `Squid` and its subclass `GlowSquid`.

use std::f32::consts::{PI, TAU};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::{EntityPose, EntityStatus};
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::entity::ageable::{AgeableData, AgeableMob};
use crate::entity::ai::goal::{Goal, to_goal_ticks};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::util::goal_utils;
use crate::entity::breath::BreathManager;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::{Mob, MobEntity, mob_hurt_server};
use crate::entity::{Entity, EntityBase};
use crate::world::World;

/// Vanilla `Squid.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::SQUID_BABY_DIMENSIONS;

pub struct SquidEntity {
    pub mob_entity: MobEntity,
    ageable_data: AgeableData,
    /// A `GlowSquid`.
    glow: bool,
    /// `GlowSquid.DATA_DARK_TICKS_REMAINING`.
    dark_ticks_remaining: AtomicI32,
    movement: Arc<SquidMovement>,
}

/// The squid fields its goals share.
struct SquidMovement {
    movement_vector: AtomicCell<Vector3<f64>>,
    tentacle_movement: AtomicCell<f32>,
    tentacle_speed: AtomicCell<f32>,
    x_body_rot: AtomicCell<f32>,
    x_body_rot_o: AtomicCell<f32>,
    y_body_rot_o: AtomicCell<f32>,
}

impl SquidMovement {
    /// `hasMovementVector`.
    fn has_movement_vector(&self) -> bool {
        self.movement_vector.load().length_squared() > f64::from(1.0E-5f32)
    }
}

fn random_tentacle_speed(random: &mut impl RandomImpl) -> f32 {
    1.0 / (random.next_f32() + 1.0) * 0.2
}

impl SquidEntity {
    pub fn new(entity: Entity, glow: bool) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `AgeableWaterCreature`'s constructor.
        mob_entity.set_pathfinding_malus(PathType::Water, 0.0);
        // `Squid`'s constructor: `this.tentacleSpeed = 1.0F / (this.random.nextFloat() + 1.0F) * 0.2F`.
        let tentacle_speed = random_tentacle_speed(&mut *mob_entity.living_entity.entity.random());
        let movement = Arc::new(SquidMovement {
            movement_vector: AtomicCell::new(Vector3::default()),
            tentacle_movement: AtomicCell::new(0.0),
            tentacle_speed: AtomicCell::new(tentacle_speed),
            x_body_rot: AtomicCell::new(0.0),
            x_body_rot_o: AtomicCell::new(0.0),
            y_body_rot_o: AtomicCell::new(0.0),
        });
        let squid = Arc::new(Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            glow,
            dark_ticks_remaining: AtomicI32::new(0),
            movement,
        });
        let mut goals = squid
            .mob_entity
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        goals.add_goal(
            0,
            Box::new(SquidRandomMovementGoal {
                movement: squid.movement.clone(),
            }),
        );
        goals.add_goal(
            1,
            Box::new(SquidFleeGoal {
                movement: squid.movement.clone(),
                flee_ticks: 0,
            }),
        );
        drop(goals);
        squid
    }

    fn set_dark_ticks(&self, ticks: i32) {
        self.dark_ticks_remaining.store(ticks, Relaxed);
        self.get_entity()
            .set_synced_data(tracked_data::glow_squid::DATA_DARK_TICKS_REMAINING, ticks);
    }

    /// `rotateVector`: turns a body-relative vector by last tick's body pitch and yaw.
    fn rotate_vector(&self, vec: Vector3<f64>) -> Vector3<f64> {
        vec.x_rot(self.movement.x_body_rot_o.load() * (PI / 180.0))
            .y_rot(-self.movement.y_body_rot_o.load() * (PI / 180.0))
    }

    /// `spawnInk`.
    fn spawn_ink(&self) {
        let entity = self.get_entity();
        let world = entity.world.load();
        let entity_pos = entity.pos.load();
        let is_baby = self.is_baby();
        // `makeSound` with `getVoicePitch`: the pitch is drawn before `playSound` checks
        // `isSilent`.
        let pitch = {
            let mut random = entity.random();
            (random.next_f32() - random.next_f32()) * 0.2 + if is_baby { 1.5 } else { 1.0 }
        };
        if !entity.silent.load(Relaxed) {
            let squirt = if self.glow {
                Sound::EntityGlowSquidSquirt
            } else {
                Sound::EntitySquidSquirt
            };
            world.play_sound_fine(
                squirt,
                SoundCategory::Neutral,
                &entity_pos,
                self.sound_volume(),
                pitch,
            );
        }
        let ink = if self.glow {
            Particle::GlowSquidInk
        } else {
            Particle::SquidInk
        };
        let pos = self.rotate_vector(Vector3::new(0.0, -1.0, 0.0)) + entity_pos;
        // Three `nextFloat`s per particle, in order: x, z, then the offset scale.
        let draws: [[f32; 3]; 30] = {
            let mut random = entity.random();
            std::array::from_fn(|_| {
                let x = random.next_f32();
                let z = random.next_f32();
                [x, z, random.next_f32()]
            })
        };
        for [x, z, scale] in draws {
            let dir = self.rotate_vector(Vector3::new(
                f64::from(x) * 0.6 - 0.3,
                -1.0,
                f64::from(z) * 0.6 - 0.3,
            ));
            let ink_pos_offset_scale = if is_baby { 0.1f32 } else { 0.3 };
            let dir_offset = dir * f64::from(ink_pos_offset_scale + scale * 2.0);
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
                ink,
            );
        }
    }
}

impl AgeableMob for SquidEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Mob for SquidEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    /// Vanilla `Squid.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    /// Vanilla `Squid.getDefaultGravity`.
    fn get_mob_gravity(&self) -> f64 {
        0.08
    }

    /// Vanilla `AgeableWaterCreature.handleAirSupply`.
    fn handle_air_supply(&self, pre_tick_air: i32) {
        BreathManager::water_animal_handle_air_supply(self, pre_tick_air);
    }

    /// Vanilla `AgeableWaterCreature.getBaseExperienceReward`.
    fn get_base_experience_reward(&self) -> u32 {
        1 + self.get_entity().random().next_bounded_i32(3) as u32
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }

    /// Vanilla `Squid.finalizeSpawn`: an `AgeableMobGroupData(0.05F)` group.
    fn finalize_spawn(
        &self,
        _world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        let group_data =
            self.finalize_ageable_spawn(group_data.or(Some(SpawnGroupData::ageable(0.05))));
        self.mob_entity.finalize_spawn_base();
        Some(group_data)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
        if self.glow {
            nbt.put_int(
                "DarkTicksRemaining",
                self.dark_ticks_remaining.load(Relaxed),
            );
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        if self.glow {
            self.set_dark_ticks(nbt.get_int("DarkTicksRemaining").unwrap_or(0));
        }
    }

    /// `LivingEntity.baseTick` keeps last tick's body yaw.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.movement
            .y_body_rot_o
            .store(self.get_entity().body_yaw.load());
    }

    /// Vanilla `Squid.hurtServer`: squirts ink when a mob hurt it; a glow squid then goes
    /// dark for 100 ticks.
    fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        let hurt = mob_hurt_server(self, caller, amount, damage_type, position, source, cause)
            && self
                .mob_entity
                .living_entity
                .get_last_hurt_by_mob()
                .is_some();
        if hurt {
            self.spawn_ink();
        }
        if self.glow && hurt {
            self.set_dark_ticks(100);
        }
        hurt
    }

    /// Vanilla `Squid.travel`: a squid only moves by the velocity `aiStep` gave it.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        self.mob_entity.living_entity.make_move(caller);
        true
    }

    /// Vanilla `Squid.aiStep` after `super.aiStep()`, which ends with `AgeableMob.aiStep`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        if entity.is_alive() {
            self.ageable_ai_step();
        }
        let movement = &self.movement;
        movement.x_body_rot_o.store(movement.x_body_rot.load());
        let mut tentacle_movement =
            movement.tentacle_movement.load() + movement.tentacle_speed.load();
        if tentacle_movement > TAU {
            tentacle_movement -= TAU;
            {
                let mut random = entity.random();
                if random.next_bounded_i32(10) == 0 {
                    movement
                        .tentacle_speed
                        .store(random_tentacle_speed(&mut *random));
                }
            }
            entity
                .world
                .load()
                .send_entity_status(entity, EntityStatus::SquidAnimSynch, None);
        }
        movement.tentacle_movement.store(tentacle_movement);

        if entity.touching_water.load(Relaxed) {
            if tentacle_movement < PI {
                if tentacle_movement / PI > 0.75 {
                    entity.velocity.store(movement.movement_vector.load());
                }
            } else {
                entity.velocity.store(entity.velocity.load() * 0.9);
            }
            let velocity = entity.velocity.load();
            let horizontal_movement = velocity.horizontal_length();
            let y_body_rot = entity.body_yaw.load();
            let y_body_rot = y_body_rot
                + (-(velocity.x.atan2(velocity.z) as f32) * (180.0 / PI) - y_body_rot) * 0.1;
            entity.body_yaw.store(y_body_rot);
            entity.yaw.store(y_body_rot);
            let x_body_rot = movement.x_body_rot.load();
            movement.x_body_rot.store(
                x_body_rot
                    + (-(horizontal_movement.atan2(velocity.y) as f32) * (180.0 / PI) - x_body_rot)
                        * 0.1,
            );
        } else {
            let mut yd = entity.velocity.load().y;
            if let Some(levitation) = living.get_effect(&StatusEffect::LEVITATION) {
                yd = 0.05 * f64::from(i32::from(levitation.amplifier) + 1);
            } else if !entity.has_no_gravity.load(Relaxed) {
                // `getGravity`.
                yd -= self.get_mob_gravity();
            }
            let air_drag = living.get_air_drag(self);
            entity
                .velocity
                .store(Vector3::new(0.0, yd * f64::from(air_drag), 0.0));
            let x_body_rot = movement.x_body_rot.load();
            movement
                .x_body_rot
                .store(x_body_rot + (-90.0 - x_body_rot) * 0.02);
        }

        if self.glow {
            // `GlowSquid.aiStep`.
            let dark_ticks = self.dark_ticks_remaining.load(Relaxed);
            if dark_ticks > 0 {
                self.set_dark_ticks(dark_ticks - 1);
            }
            // The glow particle is the client's, but the server draws its random numbers:
            // `getRandomX(0.6)`, `getRandomY()` and `getRandomZ(0.6)`, one `nextDouble` each.
            let mut random = entity.random();
            for _ in 0..3 {
                random.next_f64();
            }
        }
    }
}

/// Vanilla `SquidRandomMovementGoal`.
struct SquidRandomMovementGoal {
    movement: Arc<SquidMovement>,
}

impl Goal for SquidRandomMovementGoal {
    fn can_start(&mut self, _mob: &dyn Mob) -> bool {
        true
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let living = &mob.get_mob_entity().living_entity;
        if living.no_action_time.load(Relaxed) > 100 {
            self.movement.movement_vector.store(Vector3::default());
            return;
        }
        if living.entity.random().next_bounded_i32(to_goal_ticks(50)) == 0
            || !living.entity.touching_water.load(Relaxed)
            || !self.movement.has_movement_vector()
        {
            let (angle, y) = {
                let mut random = living.entity.random();
                let angle = random.next_f32() * TAU;
                (angle, random.next_f32())
            };
            self.movement.movement_vector.store(Vector3::new(
                f64::from(pumpkin_util::math::cos(angle) * 0.2),
                f64::from(-0.1 + y * 0.2),
                f64::from(pumpkin_util::math::sin(angle) * 0.2),
            ));
        }
    }
}

/// Vanilla `SquidFleeGoal`.
struct SquidFleeGoal {
    movement: Arc<SquidMovement>,
    flee_ticks: i32,
}

impl Goal for SquidFleeGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let living = &mob.get_mob_entity().living_entity;
        living.entity.touching_water.load(Relaxed)
            && living.get_last_hurt_by_mob().is_some_and(|attacker| {
                living
                    .entity
                    .pos
                    .load()
                    .squared_distance_to_vec(&attacker.get_entity().pos.load())
                    < 100.0
            })
    }

    fn start(&mut self, _mob: &dyn Mob) {
        self.flee_ticks = 0;
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.flee_ticks += 1;
        let living = &mob.get_mob_entity().living_entity;
        let Some(attacker) = living.get_last_hurt_by_mob() else {
            return;
        };
        let pos = living.entity.pos.load();
        let mut flee_to = pos - attacker.get_entity().pos.load();
        let world = living.entity.world.load();
        let probe = BlockPos::floored(pos.x + flee_to.x, pos.y + flee_to.y, pos.z + flee_to.z);
        let is_air = world.get_block_state(&probe).is_air();
        if goal_utils::is_water(&world, &probe) || is_air {
            let length = flee_to.length();
            if length > 0.0 {
                // Vanilla normalizes a copy here and drops it, so the raw offset is scaled.
                let mut avoid_speed = 3.0;
                if length > 5.0 {
                    avoid_speed -= (length - 5.0) / 5.0;
                }
                if avoid_speed > 0.0 {
                    flee_to = flee_to * avoid_speed;
                }
            }
            if is_air {
                flee_to.y = 0.0;
            }
            self.movement.movement_vector.store(flee_to / 20.0);
        }
        // The bubble every 10 ticks is a client particle.
    }
}
