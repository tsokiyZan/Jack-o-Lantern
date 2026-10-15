use std::collections::VecDeque;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::block_properties::{
    CreakingHeartLikeProperties, CreakingHeartState, GlowLichenLikeProperties,
};
use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::fluid::Fluid;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockDirection, BlockStateId};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::java::client::play::CParticle;
use pumpkin_protocol::{VarInt, ser::NetworkWriteExt};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;
use rand::seq::SliceRandom;
use rustc_hash::FxHashSet;
use uuid::Uuid;

use crate::block::blocks::creaking_heart::CreakingHeartBlock;
use crate::block::blocks::sculk::sculk_vein::{has_face, set_face};
use crate::entity::EntityBase;
use crate::entity::mob::Mob;
use crate::entity::mob::creaking::{CREAKING_GRAY, CREAKING_ORANGE, CreakingEntity, DamageSource};
use crate::entity::spawn_util::{SpawnStrategy, try_spawn_mob};
use crate::world::World;

use super::BlockEntity;

const PLAYER_DETECTION_RANGE: f64 = 32.0;
pub const CREAKING_ROAMING_RADIUS: f64 = 32.0;
const DISTANCE_CREAKING_TOO_FAR: f64 = 34.0;
const SPAWN_RANGE_XZ: i32 = 16;
const SPAWN_RANGE_Y: i32 = 8;
const ATTEMPTS_PER_SPAWN: i32 = 5;
const UPDATE_TICKS: i32 = 20;
const UPDATE_TICKS_VARIANCE: i32 = 5;
const HURT_CALL_TOTAL_TICKS: i32 = 100;
const HURT_CALL_INTERVAL: i32 = 10;
const HURT_CALL_PARTICLE_TICKS: i32 = 50;
const MAX_DEPTH: i32 = 2;
const MAX_COUNT: i32 = 64;
const TICKS_GRACE_PERIOD: i64 = 30;
/// Vanilla `ServerLevel.sendParticles`' range for a particle that overrides the limiter.
const OVERRIDE_LIMITER_PARTICLE_RANGE: f64 = 512.0;

/// Vanilla `creakingInfo`: the spawned creaking, or its UUID until it is found again.
#[derive(Clone)]
enum CreakingInfo {
    Creaking(Arc<dyn EntityBase>),
    Uuid(Uuid),
}

impl CreakingInfo {
    fn uuid(&self) -> Uuid {
        match self {
            Self::Creaking(creaking) => creaking.get_entity().entity_uuid,
            Self::Uuid(uuid) => *uuid,
        }
    }
}

/// Vanilla `BlockPos.TraversalNodeStatus`.
#[derive(PartialEq, Eq)]
enum TraversalNodeStatus {
    Accept,
    Skip,
    Stop,
}

pub struct CreakingHeartBlockEntity {
    pub position: BlockPos,
    creaking_info: Mutex<Option<CreakingInfo>>,
    ticks_existed: AtomicI64,
    ticker: AtomicI32,
    emitter: AtomicI32,
    emitter_target: AtomicCell<Option<Vector3<f64>>>,
    output_signal: AtomicI32,
}

impl BlockEntity for CreakingHeartBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    /// Vanilla `loadAdditional`.
    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let entity = Self::new(position);
        if let Some(uuid) = nbt.get_uuid("creaking") {
            entity.set_creaking_info_uuid(uuid);
        }
        entity
    }

    /// Vanilla `saveAdditional`.
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        if let Some(info) = self.creaking_info() {
            nbt.put_uuid("creaking", info.uuid());
        }
    }

    /// Vanilla `getUpdateTag`: `saveCustomOnly`.
    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        self.write_nbt(&mut nbt);
        Some(nbt)
    }

    /// Vanilla `CreakingHeartBlock.getTicker`: only a rooted heart ticks.
    fn tick(&self, world: &Arc<World>) {
        let state = world.get_block_state_id(&self.position);
        if CreakingHeartLikeProperties::from_state_id(state).creaking_heart_state
            != CreakingHeartState::Uprooted
        {
            self.server_tick(world, state);
        }
    }

    /// Vanilla `preRemoveSideEffects`.
    fn on_block_replaced(self: Arc<Self>, world: &Arc<World>, _position: &BlockPos) {
        self.remove_protector(world, None);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl CreakingHeartBlockEntity {
    pub const ID: &'static str = "minecraft:creaking_heart";

    #[must_use]
    pub const fn new(position: BlockPos) -> Self {
        Self {
            position,
            creaking_info: Mutex::new(None),
            ticks_existed: AtomicI64::new(0),
            ticker: AtomicI32::new(0),
            emitter: AtomicI32::new(0),
            emitter_target: AtomicCell::new(None),
            output_signal: AtomicI32::new(0),
        }
    }

    /// The UUID of the creaking this heart spawned, if it has one.
    #[must_use]
    pub fn creaking_uuid(&self) -> Option<Uuid> {
        self.creaking_info().map(|info| info.uuid())
    }

    fn creaking_info(&self) -> Option<CreakingInfo> {
        self.creaking_info
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Vanilla `CreakingHeartBlockEntity.serverTick`.
    fn server_tick(&self, world: &Arc<World>, state: BlockStateId) {
        let pos = self.position;
        self.ticks_existed.fetch_add(1, Ordering::Relaxed);
        let computed_output_signal = self.compute_analog_output_signal(world);
        if self
            .output_signal
            .swap(computed_output_signal, Ordering::Relaxed)
            != computed_output_signal
        {
            world.update_neighbour_for_output_signal(&pos, &Block::CREAKING_HEART);
        }

        let emitter = self.emitter.load(Ordering::Relaxed);
        if emitter > 0 {
            if emitter > HURT_CALL_PARTICLE_TICKS {
                self.emit_particles(world, 1, true);
                self.emit_particles(world, 1, false);
            }

            if emitter % HURT_CALL_INTERVAL == 0
                && let Some(mut emitter_target) = self.emitter_target.load()
            {
                if let Some(creaking) = self.get_creaking_protector(world) {
                    emitter_target = creaking.get_entity().bounding_box.load().get_center();
                    self.emitter_target.store(Some(emitter_target));
                }
                let heart_position = pos.to_centered_f64();
                let progress = 0.2f32
                    + 0.8 * (HURT_CALL_TOTAL_TICKS - emitter) as f32 / HURT_CALL_TOTAL_TICKS as f32;
                let sound_location =
                    (heart_position - emitter_target) * f64::from(progress) + emitter_target;
                let sound_pos = BlockPos::floored_v(sound_location);
                let volume = emitter as f32 / 2.0 / HURT_CALL_TOTAL_TICKS as f32 + 0.5;
                world.play_sound_fine(
                    Sound::BlockCreakingHeartHurt,
                    SoundCategory::Blocks,
                    &sound_pos.to_centered_f64(),
                    volume,
                    1.0,
                );
            }

            self.emitter.fetch_sub(1, Ordering::Relaxed);
        }

        if self.ticker.fetch_sub(1, Ordering::Relaxed) < 0 {
            self.ticker.store(
                rand::rng().random_range(0..UPDATE_TICKS_VARIANCE) + UPDATE_TICKS,
                Ordering::Relaxed,
            );
            let updated_state = self.update_creaking_state(world, state);
            if updated_state != state {
                world.set_block_state(&pos, updated_state, BlockFlags::NOTIFY_ALL);
                if CreakingHeartLikeProperties::from_state_id(updated_state).creaking_heart_state
                    == CreakingHeartState::Uprooted
                {
                    return;
                }
            }

            if self.creaking_info().is_none() {
                if CreakingHeartLikeProperties::from_state_id(updated_state).creaking_heart_state
                    == CreakingHeartState::Awake
                    && world.should_spawn_monsters()
                    && world
                        .get_nearest_player(pos.to_f64(), PLAYER_DETECTION_RANGE, |player| {
                            !player.is_spectator()
                        })
                        .is_some()
                    && let Some(creaking) = self.spawn_protector(world)
                {
                    self.set_creaking_info(creaking.clone());
                    creaking.make_sound(Sound::EntityCreakingSpawn);
                    world.play_block_sound(
                        Sound::BlockCreakingHeartSpawn,
                        SoundCategory::Blocks,
                        pos,
                    );
                }
            } else if let Some(creaking) = self.get_creaking_protector(world)
                && let Some(creaking) = creaking.cast_any().downcast_ref::<CreakingEntity>()
                && ((!world.creaking_active(&pos)
                    && !creaking
                        .mob_entity
                        .persistence_required
                        .load(Ordering::Relaxed))
                    || self.distance_to_creaking(world) > DISTANCE_CREAKING_TOO_FAR
                    || creaking.player_is_stuck_in_you())
            {
                self.remove_protector(world, None);
            }
        }
    }

    /// Vanilla `CreakingHeartBlockEntity.updateCreakingState`.
    fn update_creaking_state(&self, world: &World, state: BlockStateId) -> BlockStateId {
        let mut props = CreakingHeartLikeProperties::from_state_id(state);
        props.creaking_heart_state =
            if !CreakingHeartBlock::has_required_logs(world, &self.position, props.axis)
                && self.creaking_info().is_none()
            {
                CreakingHeartState::Uprooted
            } else if world.creaking_active(&self.position) {
                CreakingHeartState::Awake
            } else {
                CreakingHeartState::Dormant
            };
        props.to_state_id(&Block::CREAKING_HEART)
    }

    /// Vanilla `CreakingHeartBlockEntity.distanceToCreaking`.
    fn distance_to_creaking(&self, world: &World) -> f64 {
        let bottom_center = Vector3::new(
            f64::from(self.position.0.x) + 0.5,
            f64::from(self.position.0.y),
            f64::from(self.position.0.z) + 0.5,
        );
        self.get_creaking_protector(world).map_or(0.0, |creaking| {
            creaking
                .get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&bottom_center)
                .sqrt()
        })
    }

    fn clear_creaking_info(&self) {
        *self
            .creaking_info
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Vanilla `CreakingHeartBlockEntity.setCreakingInfo(Creaking)`.
    pub fn set_creaking_info(&self, creaking: Arc<dyn EntityBase>) {
        *self
            .creaking_info
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(CreakingInfo::Creaking(creaking));
    }

    /// Vanilla `CreakingHeartBlockEntity.setCreakingInfo(UUID)`.
    pub fn set_creaking_info_uuid(&self, uuid: Uuid) {
        *self
            .creaking_info
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(CreakingInfo::Uuid(uuid));
        self.ticks_existed.store(0, Ordering::Relaxed);
    }

    /// Vanilla `CreakingHeartBlockEntity.getCreakingProtector`: the creaking, looked up by
    /// UUID while it is unloaded and forgotten once the grace period passes without it.
    fn get_creaking_protector(&self, world: &World) -> Option<Arc<dyn EntityBase>> {
        let uuid = match self.creaking_info()? {
            CreakingInfo::Creaking(creaking) => {
                let creaking_entity = creaking.get_entity();
                if !creaking_entity.is_removed() {
                    return Some(creaking);
                }
                self.set_creaking_info_uuid(creaking_entity.entity_uuid);
                creaking_entity.entity_uuid
            }
            CreakingInfo::Uuid(uuid) => uuid,
        };
        if let Some(resolved) = world
            .get_entity_by_uuid(uuid)
            .filter(|entity| entity.cast_any().is::<CreakingEntity>())
        {
            self.set_creaking_info(resolved.clone());
            return Some(resolved);
        }
        if self.ticks_existed.load(Ordering::Relaxed) >= TICKS_GRACE_PERIOD {
            self.clear_creaking_info();
        }
        None
    }

    /// Vanilla `CreakingHeartBlockEntity.spawnProtector`.
    fn spawn_protector(&self, world: &Arc<World>) -> Option<Arc<CreakingEntity>> {
        let pos = self.position;
        let spawned_creaking = try_spawn_mob(
            &EntityType::CREAKING,
            CreakingEntity::new,
            world,
            &pos,
            ATTEMPTS_PER_SPAWN,
            SPAWN_RANGE_XZ,
            SPAWN_RANGE_Y,
            SpawnStrategy::OnTopOfColliderNoLeaves,
            true,
        )?;
        // `SpawnUtil.trySpawnMob`'s `playAmbientSound`.
        spawned_creaking.play_ambient_sound();
        let creaking_entity = spawned_creaking.get_entity();
        world.emit_game_event(GameEvent::EntityPlace.name(), creaking_entity.pos.load());
        world.send_entity_status(creaking_entity, EntityStatus::Poof, None);
        spawned_creaking.set_transient(pos);
        Some(spawned_creaking)
    }

    /// Vanilla `CreakingHeartBlockEntity.creakingHurt`.
    pub fn creaking_hurt(&self, world: &Arc<World>) {
        let Some(creaking) = self.get_creaking_protector(world) else {
            return;
        };
        if self.emitter.load(Ordering::Relaxed) > 0 {
            return;
        }
        self.emit_particles(world, 20, false);
        if CreakingHeartLikeProperties::from_state_id(world.get_block_state_id(&self.position))
            .creaking_heart_state
            == CreakingHeartState::Awake
        {
            let number_of_clumps = rand::rng().random_range(2..=3);
            for _ in 0..number_of_clumps {
                if let Some(block_pos) = self.spread_resin(world) {
                    world.play_block_sound(
                        Sound::BlockResinPlace,
                        SoundCategory::Blocks,
                        block_pos,
                    );
                    world
                        .emit_game_event(GameEvent::BlockPlace.name(), block_pos.to_centered_f64());
                }
            }
        }
        self.emitter.store(HURT_CALL_TOTAL_TICKS, Ordering::Relaxed);
        self.emitter_target
            .store(Some(creaking.get_entity().bounding_box.load().get_center()));
    }

    /// Vanilla `CreakingHeartBlockEntity.spreadResin`.
    fn spread_resin(&self, world: &Arc<World>) -> Option<BlockPos> {
        let is_pale_oak_log = |pos: &BlockPos| {
            world
                .get_block(pos)
                .has_tag(&tag::Block::MINECRAFT_PALE_OAK_LOGS)
        };
        let mut placed_resin = None;
        breadth_first_traversal(
            self.position,
            MAX_DEPTH,
            MAX_COUNT,
            |pos, accept| {
                for dir in shuffled_directions() {
                    let neighbour_pos = pos.offset(dir.to_offset());
                    if is_pale_oak_log(&neighbour_pos) {
                        accept(neighbour_pos);
                    }
                }
            },
            |pos| {
                if !is_pale_oak_log(&pos) {
                    return TraversalNodeStatus::Accept;
                }
                for dir in shuffled_directions() {
                    let neighbour_pos = pos.offset(dir.to_offset());
                    let mut neighbour_state = world.get_block_state_id(&neighbour_pos);
                    let opposite = dir.opposite();
                    if neighbour_state.to_state().is_air() {
                        neighbour_state = Block::RESIN_CLUMP.default_state.id;
                    } else if neighbour_state.to_block() == &Block::WATER
                        && Fluid::from_state_id(neighbour_state)
                            .is_some_and(|fluid| fluid.is_source(neighbour_state))
                    {
                        let mut props = GlowLichenLikeProperties::from_state_id(
                            Block::RESIN_CLUMP.default_state.id,
                        );
                        props.waterlogged = true;
                        neighbour_state = props.to_state_id(&Block::RESIN_CLUMP);
                    }

                    if neighbour_state.to_block() == &Block::RESIN_CLUMP {
                        let mut props = GlowLichenLikeProperties::from_state_id(neighbour_state);
                        if !has_face(props, opposite) {
                            set_face(&mut props, opposite);
                            world.set_block_state(
                                &neighbour_pos,
                                props.to_state_id(&Block::RESIN_CLUMP),
                                BlockFlags::NOTIFY_ALL,
                            );
                            placed_resin = Some(neighbour_pos);
                            return TraversalNodeStatus::Stop;
                        }
                    }
                }
                TraversalNodeStatus::Accept
            },
        );
        placed_resin
    }

    /// Vanilla `CreakingHeartBlockEntity.emitParticles`.
    fn emit_particles(&self, world: &World, count: i32, towards_creaking: bool) {
        let Some(creaking) = self.get_creaking_protector(world) else {
            return;
        };
        let color = if towards_creaking {
            CREAKING_ORANGE
        } else {
            CREAKING_GRAY
        };
        let mut random = rand::rng();
        for _ in 0..count {
            let bounding_box = creaking.get_entity().bounding_box.load();
            let size = bounding_box.max - bounding_box.min;
            let mut source = bounding_box.min
                + Vector3::new(
                    random.random::<f64>() * size.x,
                    random.random::<f64>() * size.y,
                    random.random::<f64>() * size.z,
                );
            let mut destination = self.position.to_f64()
                + Vector3::new(
                    random.random::<f64>(),
                    random.random::<f64>(),
                    random.random::<f64>(),
                );
            if towards_creaking {
                std::mem::swap(&mut source, &mut destination);
            }
            let duration = random.random_range(0..40) + 10;
            send_trail_particle(world, source, destination, color, duration);
        }
    }

    /// Vanilla `CreakingHeartBlockEntity.removeProtector`.
    pub fn remove_protector(&self, world: &World, damage_source: Option<DamageSource<'_>>) {
        let Some(protector) = self.get_creaking_protector(world) else {
            return;
        };
        if let Some(creaking) = protector.cast_any().downcast_ref::<CreakingEntity>() {
            match damage_source {
                None => creaking.tear_down(),
                Some(damage_source) => {
                    creaking.creaking_death_effects(&damage_source);
                    creaking.set_tearing_down();
                    creaking.mob_entity.living_entity.set_health(0.0);
                }
            }
        }
        self.clear_creaking_info();
    }

    /// Vanilla `CreakingHeartBlockEntity.isProtector`.
    #[must_use]
    pub fn is_protector(&self, world: &World, creaking: &CreakingEntity) -> bool {
        self.get_creaking_protector(world).is_some_and(|protector| {
            protector.get_entity().entity_id == creaking.get_entity().entity_id
        })
    }

    /// Vanilla `CreakingHeartBlockEntity.getAnalogOutputSignal`.
    #[must_use]
    pub fn get_analog_output_signal(&self) -> u8 {
        self.output_signal.load(Ordering::Relaxed) as u8
    }

    /// Vanilla `CreakingHeartBlockEntity.computeAnalogOutputSignal`.
    fn compute_analog_output_signal(&self, world: &World) -> i32 {
        if self.creaking_info().is_none() || self.get_creaking_protector(world).is_none() {
            return 0;
        }
        let distance = self.distance_to_creaking(world);
        let scaled_distance =
            distance.clamp(0.0, CREAKING_ROAMING_RADIUS) / CREAKING_ROAMING_RADIUS;
        15 - (scaled_distance * 15.0).floor() as i32
    }
}

/// Vanilla `Util.shuffledCopy(Direction.values(), random)`.
fn shuffled_directions() -> [BlockDirection; 6] {
    let mut directions = BlockDirection::all();
    directions.shuffle(&mut rand::rng());
    directions
}

/// Vanilla `BlockPos.breadthFirstTraversal`.
fn breadth_first_traversal(
    start_pos: BlockPos,
    max_depth: i32,
    max_count: i32,
    mut neighbour_provider: impl FnMut(BlockPos, &mut dyn FnMut(BlockPos)),
    mut node_processor: impl FnMut(BlockPos) -> TraversalNodeStatus,
) -> i32 {
    let mut nodes = VecDeque::from([(start_pos, 0)]);
    let mut visited = FxHashSet::default();
    let mut count = 0;
    while let Some((current_pos, depth)) = nodes.pop_front() {
        if !visited.insert(current_pos.as_long()) {
            continue;
        }
        let next = node_processor(current_pos);
        if next == TraversalNodeStatus::Skip {
            continue;
        }
        if next == TraversalNodeStatus::Stop {
            break;
        }
        count += 1;
        if count >= max_count {
            return count;
        }
        if depth < max_depth {
            neighbour_provider(current_pos, &mut |pos| nodes.push_back((pos, depth + 1)));
        }
    }
    count
}

/// `ServerLevel.sendParticles(new TrailParticleOption(target, color, duration), true, true,
/// x, y, z, 1, 0, 0, 0, 0)`.
fn send_trail_particle(
    world: &World,
    position: Vector3<f64>,
    target: Vector3<f64>,
    color: i32,
    duration: i32,
) {
    // `TrailParticleOption.STREAM_CODEC`: target, color, duration.
    let mut data = Vec::new();
    let written = data
        .write_f64_be(target.x)
        .and_then(|()| data.write_f64_be(target.y))
        .and_then(|()| data.write_f64_be(target.z))
        .and_then(|()| data.write_i32_be(color))
        .and_then(|()| data.write_var_int(&VarInt(duration)));
    if written.is_err() {
        return;
    }
    let packet = CParticle::new(
        true,
        true,
        position,
        Vector3::new(0.0, 0.0, 0.0),
        0.0,
        1,
        VarInt(Particle::Trail as i32),
        &data,
    );
    for player in world.players.load().iter() {
        if player
            .get_entity()
            .block_pos
            .load()
            .dist_to_center_sqr(position.x, position.y, position.z)
            < OVERRIDE_LIMITER_PARTICLE_RANGE * OVERRIDE_LIMITER_PARTICLE_RANGE
        {
            player.try_send_client_packet(&packet);
        }
    }
}
