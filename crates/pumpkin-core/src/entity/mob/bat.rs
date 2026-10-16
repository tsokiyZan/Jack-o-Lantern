use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use pumpkin_data::damage::DamageType;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_data::world::WorldEvent;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::chunk::ChunkHeightmapType;
use rand::RngExt;

use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::mob::{Mob, MobEntity};
use crate::entity::{Entity, EntityBase};
use crate::world::World;

const ROOSTING_FLAG: u8 = 1;
pub struct BatEntity {
    pub mob_entity: MobEntity,
    hanging_position: Mutex<Option<BlockPos>>,
    roosting: AtomicBool,
}

impl BatEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let bat = Self {
            mob_entity,
            hanging_position: Mutex::new(None),
            roosting: AtomicBool::new(true),
        };
        Arc::new(bat)
    }

    #[must_use]
    pub fn check_bat_spawn_rules(world: &World, pos: &BlockPos) -> bool {
        if pos.0.y >= world.get_heightmap_height(ChunkHeightmapType::WorldSurface, pos.0.x, pos.0.z)
        {
            return false;
        }
        if rand::random_bool(0.5) {
            return false;
        }
        if world.get_max_local_raw_brightness(pos) > rand::random_range(0..4) {
            return false;
        }
        let below_pos = BlockPos::new(pos.0.x, pos.0.y - 1, pos.0.z);
        if !world
            .get_block(&below_pos)
            .has_tag(&tag::Block::MINECRAFT_BATS_SPAWNABLE_ON)
        {
            return false;
        }
        true
    }

    #[must_use]
    pub fn is_roosting(&self) -> bool {
        self.roosting.load(Relaxed)
    }

    pub fn set_roosting(&self, roosting: bool) {
        self.roosting.store(roosting, Relaxed);
        let flags: u8 = if roosting { ROOSTING_FLAG } else { 0 };
        self.mob_entity
            .living_entity
            .entity
            .set_synced_data(tracked_data::bat::DATA_ID_FLAGS, flags);
    }

    /// The `liftoff` level event a resting bat makes as it takes off.
    fn take_off(&self, world: &World, pos: BlockPos) {
        self.set_roosting(false);
        if !self.get_entity().silent.load(Relaxed) {
            world.sync_world_event(WorldEvent::SoundBatLiftoff, pos, 0);
        }
    }
}

/// Java's `Math.signum`, which is 0 for 0.
fn signum(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value.signum() }
}

impl Mob for BatEntity {
    /// Vanilla `Bat.checkFallDamage` does nothing.
    fn mob_check_fall_damage(
        &self,
        _caller: &dyn crate::entity::EntityBase,
        _ya: f64,
        _on_ground: bool,
        _state: &'static pumpkin_data::BlockState,
        _pos: pumpkin_util::math::position::BlockPos,
    ) {
    }

    /// Vanilla `Bat.getAmbientSound`: a resting bat squeaks one time in four.
    fn ambient_sound(&self) -> Option<Sound> {
        if self.is_roosting() && self.get_random().random_range(0..4) != 0 {
            None
        } else {
            Some(Sound::EntityBatAmbient)
        }
    }

    /// Vanilla `Bat.getVoicePitch`.
    fn voice_pitch(&self) -> f32 {
        let base = if self.get_entity().is_baby() {
            1.5
        } else {
            1.0
        };
        let mut random = self.get_random();
        ((random.random::<f32>() - random.random::<f32>()) * 0.2 + base) * 0.95
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let flags: u8 = if self.is_roosting() { ROOSTING_FLAG } else { 0 };
        entity.set_synced_data(tracked_data::bat::DATA_ID_FLAGS, flags);
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        let flags: u8 = if self.is_roosting() { ROOSTING_FLAG } else { 0 };
        nbt.put_byte("BatFlags", i8::try_from(flags).unwrap_or(0));
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        let flags = u8::try_from(nbt.get_byte("BatFlags").unwrap_or(0)).unwrap_or(0);
        let roosting = (flags & ROOSTING_FLAG) != 0;
        self.set_roosting(roosting);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `Bat.customServerAiStep`.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load_full();
        let pos = entity.block_pos.load();
        let above = pos.up();
        if self.is_roosting() {
            if world.get_block_state(&above).is_solid_block() {
                {
                    let mut rng = self.get_random();
                    if rng.random_range(0..200) == 0 {
                        entity.head_yaw.store(rng.random_range(0..360) as f32);
                    }
                }
                // `BAT_RESTING_TARGETING`: non-combat, within 4 blocks.
                if TargetPredicate::create_non_attackable()
                    .set_base_max_distance(4.0)
                    .get_nearest_player(&world, Some(self), entity.pos.load())
                    .is_some()
                {
                    self.take_off(&world, pos);
                }
            } else {
                self.take_off(&world, pos);
            }
            return;
        }
        let position = entity.pos.load();
        let mut target_position = self
            .hanging_position
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(target) = *target_position
            && (!world.get_block_state(&target).is_air() || target.0.y <= world.dimension.min_y)
        {
            *target_position = None;
        }
        let mut rng = self.get_random();
        let pick_new = target_position.is_none_or(|target| {
            rng.random_range(0..30) == 0 || {
                // `closerToCenterThan(position, 2.0)`.
                let center = target.0.to_f64() + Vector3::new(0.5, 0.5, 0.5);
                center.squared_distance_to_vec(&position) < 4.0
            }
        });
        if pick_new {
            let (x1, x2) = (rng.random_range(0..7), rng.random_range(0..7));
            let y = rng.random_range(0..6);
            let (z1, z2) = (rng.random_range(0..7), rng.random_range(0..7));
            *target_position = Some(BlockPos::floored(
                position.x + f64::from(x1) - f64::from(x2),
                position.y + f64::from(y) - 2.0,
                position.z + f64::from(z1) - f64::from(z2),
            ));
        }
        let Some(target) = *target_position else {
            return;
        };
        drop(target_position);
        let dx = f64::from(target.0.x) + 0.5 - position.x;
        let dy = f64::from(target.0.y) + 0.1 - position.y;
        let dz = f64::from(target.0.z) + 0.5 - position.z;
        let movement = entity.velocity.load();
        let new_movement = movement
            + Vector3::new(
                (signum(dx) * 0.5 - movement.x) * f64::from(0.1f32),
                (signum(dy) * f64::from(0.7f32) - movement.y) * f64::from(0.1f32),
                (signum(dz) * 0.5 - movement.z) * f64::from(0.1f32),
            );
        entity.velocity.store(new_movement);
        let y_rot_d = (pumpkin_util::math::atan2(new_movement.z, new_movement.x)
            * f64::from(180.0f32)
            / f64::from(std::f32::consts::PI)) as f32
            - 90.0;
        let rot_diff = pumpkin_util::math::wrap_degrees(y_rot_d - entity.yaw.load());
        let living = &self.mob_entity.living_entity;
        let input = living.movement_input.load();
        living
            .movement_input
            .store(Vector3::new(input.x, input.y, 0.5));
        entity.yaw.store(entity.yaw.load() + rot_diff);
        if rng.random_range(0..100) == 0 && world.get_block_state(&above).is_solid_block() {
            drop(rng);
            self.set_roosting(true);
        }
    }

    /// The part of vanilla `Bat.tick` after `super.tick()`.
    fn post_tick(&self) {
        let entity = &self.mob_entity.living_entity.entity;
        if self.is_roosting() {
            entity.velocity.store(Vector3::new(0.0, 0.0, 0.0));
            let pos = entity.pos.load();
            entity.set_pos(Vector3::new(
                pos.x,
                pos.y.floor() + 1.0 - f64::from(entity.height()),
                pos.z,
            ));
        } else {
            let velocity = entity.velocity.load();
            entity
                .velocity
                .store(Vector3::new(velocity.x, velocity.y * 0.6, velocity.z));
        }
    }

    fn mob_is_pushable(&self) -> bool {
        false
    }

    fn pushes_entities(&self) -> bool {
        false
    }

    fn get_mob_gravity(&self) -> f64 {
        0.0
    }

    /// Vanilla `Bat.hurtServer`: a resting bat that is hurt stops resting.
    fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if self.get_entity().is_invulnerable_to(&damage_type, cause) {
            return false;
        }
        if self.is_roosting() {
            self.set_roosting(false);
        }
        crate::entity::mob::mob_hurt_server(
            self,
            caller,
            amount,
            damage_type,
            position,
            source,
            cause,
        )
    }
}
