use rustc_hash::FxHashSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering},
};

use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::Taggable;
use pumpkin_data::world::WorldEvent;
use pumpkin_util::Difficulty;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;

use crate::block::blocks::fire::FireBlockBase;
use crate::block::blocks::weathering_copper;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};
use crate::server::Server;
use crate::world::World;

pub struct LightningBoltEntity {
    entity: Entity,
    life: AtomicI32,
    seed: AtomicI64,
    flashes: AtomicI32,
    visual_only: AtomicBool,
    cause: Mutex<Option<Arc<Player>>>,
    hit_entities: Mutex<FxHashSet<i32>>,
    blocks_set_on_fire: AtomicI32,
}

impl LightningBoltEntity {
    pub fn new(entity: Entity) -> Self {
        entity.no_physics.store(true, Ordering::Relaxed);
        let (seed, flashes) = {
            let mut random = entity.random();
            let seed = random.next_i64();
            (seed, random.next_bounded_i32(3) + 1)
        };
        Self {
            entity,
            life: AtomicI32::new(2),
            seed: AtomicI64::new(seed),
            flashes: AtomicI32::new(flashes),
            visual_only: AtomicBool::new(false),
            cause: Mutex::new(None),
            hit_entities: Mutex::new(FxHashSet::default()),
            blocks_set_on_fire: AtomicI32::new(0),
        }
    }

    pub fn set_visual_only(&self, visual_only: bool) {
        self.visual_only.store(visual_only, Ordering::Relaxed);
    }

    pub fn is_visual_only(&self) -> bool {
        self.visual_only.load(Ordering::Relaxed)
    }

    pub fn set_cause(&self, cause: Option<Arc<Player>>) {
        if let Ok(mut c) = self.cause.lock() {
            *c = cause;
        }
    }

    pub fn get_cause(&self) -> Option<Arc<Player>> {
        self.cause.lock().ok().and_then(|c| c.clone())
    }

    pub fn get_blocks_set_on_fire(&self) -> i32 {
        self.blocks_set_on_fire.load(Ordering::Relaxed)
    }

    pub fn get_seed(&self) -> i64 {
        self.seed.load(Ordering::Relaxed)
    }

    pub fn set_seed(&self, seed: i64) {
        self.seed.store(seed, Ordering::Relaxed);
    }

    fn get_strike_position(&self) -> BlockPos {
        let pos = self.entity.pos.load();
        Vector3::new(pos.x, pos.y - 1.0e-6, pos.z).to_block_pos()
    }

    fn power_lightning_rod(&self, world: &Arc<World>) {
        let strike_pos = self.get_strike_position();
        let block = world.get_block(&strike_pos);
        // Vanilla `instanceof LightningRodBlock`: every lightning rod, waxed or weathered.
        if block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_LIGHTNING_RODS) {
            crate::block::blocks::redstone::lightning_rod::LightningRodBlock::trigger(
                world,
                &strike_pos,
            );
        }
    }

    fn spawn_fire(&self, world: &Arc<World>, additional_sources: i32) {
        if self.visual_only.load(Ordering::Relaxed) {
            return;
        }

        let pos = self.entity.block_pos.load();

        let try_place = |p: BlockPos| {
            if world.get_block_state(&p).is_air() && FireBlockBase::can_place_at(world, &p) {
                let fire_block = FireBlockBase::get_fire_type(world, &p);
                world.set_block_state(&p, fire_block.default_state.id, BlockFlags::NOTIFY_ALL);
                self.blocks_set_on_fire.fetch_add(1, Ordering::Relaxed);
            }
        };

        try_place(pos);

        for _ in 0..additional_sources {
            let offset = {
                let mut random = self.entity.random();
                let dx = random.next_bounded_i32(3) - 1;
                let dy = random.next_bounded_i32(3) - 1;
                let dz = random.next_bounded_i32(3) - 1;
                Vector3::new(dx, dy, dz)
            };
            let nearby_pos = pos.offset(offset);
            try_place(nearby_pos);
        }
    }

    /// Vanilla `clearCopperOnLightningStrike` (LightningBolt.java:190-207).
    fn clear_copper_on_lightning_strike(&self, world: &Arc<World>) {
        let strike_pos = self.get_strike_position();
        let struck_state = world.get_block_state(&strike_pos);
        let struck_block = struck_state.id.to_block();

        let is_waxed = weathering_copper::is_waxed(struck_block);
        let is_weathering = weathering_copper::get_weather_state(struck_block).is_some();

        if is_weathering || is_waxed {
            if is_weathering {
                // `setBlockAndUpdate(struckPos, WeatheringCopper.getFirst(state))`; setting the
                // state it already has changes nothing.
                let first_state = weathering_copper::get_first_state(struck_block, struck_state.id);
                if first_state != struck_state.id {
                    world.set_block_state(&strike_pos, first_state, BlockFlags::NOTIFY_ALL);
                }
            }

            // `level.getRandom()`: `nextInt(3) + 3` strikes, `nextInt(8) + 1` steps each
            // (LightningBolt.java:201-205).
            let strikes_count = Self::level_random(world).next_bounded_i32(3) + 3;

            for _ in 0..strikes_count {
                let step_count = Self::level_random(world).next_bounded_i32(8) + 1;
                Self::random_walk_cleaning_copper(world, &strike_pos, step_count);
            }
        }
    }

    /// `level.getRandom()`.
    fn level_random(
        world: &World,
    ) -> std::sync::MutexGuard<'_, pumpkin_util::random::legacy_rand::LegacyRand> {
        world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn random_walk_cleaning_copper(
        world: &Arc<World>,
        original_strike_pos: &BlockPos,
        step_count: i32,
    ) {
        let mut work_pos = *original_strike_pos;

        for _ in 0..step_count {
            if let Some(next_pos) = Self::random_step_cleaning_copper(world, &work_pos) {
                work_pos = next_pos;
            } else {
                break;
            }
        }
    }

    /// Vanilla `randomStepCleaningCopper` (LightningBolt.java:224-235).
    /// `BlockPos.randomInCube(level.getRandom(), 10, pos, 1)` is a lazy iterator, so each
    /// candidate is drawn (x, y, z) only when the previous one was rejected. Any weathering
    /// copper block stops the step, unaffected copper too: it has no previous stage to set,
    /// but still gets the spark (level event 3002).
    fn random_step_cleaning_copper(world: &Arc<World>, pos: &BlockPos) -> Option<BlockPos> {
        for _ in 0..10 {
            let candidate = {
                let mut random = Self::level_random(world);
                let dx = random.next_bounded_i32(3) - 1;
                let dy = random.next_bounded_i32(3) - 1;
                let dz = random.next_bounded_i32(3) - 1;
                pos.offset(Vector3::new(dx, dy, dz))
            };
            let state = world.get_block_state(&candidate);
            let block = state.id.to_block();

            if weathering_copper::get_weather_state(block).is_some() {
                if let Some(previous) = weathering_copper::get_previous_state(block, state.id) {
                    world.set_block_state(&candidate, previous, BlockFlags::NOTIFY_ALL);
                }
                world.sync_world_event(WorldEvent::ParticlesElectricSpark, candidate, -1);
                return Some(candidate);
            }
        }
        None
    }
}

impl EntityBase for LightningBoltEntity {
    fn tick(&self, _caller: &dyn EntityBase, _server: &Server) {
        let entity = &self.entity;
        let life = self.life.load(Ordering::Relaxed);

        if life == 2 {
            let world = entity.world.load();
            let pos = entity.pos.load();

            // Vanilla plays both sounds on the client only (LightningBolt.java:79), so these
            // pitches must not draw from the bolt's server random.
            let pitch_thunder = 0.8 + rand::rng().random::<f32>() * 0.2;
            world.play_sound_fine(
                Sound::EntityLightningBoltThunder,
                SoundCategory::Weather,
                &pos,
                10000.0,
                pitch_thunder,
            );
            let pitch_impact = 0.5 + rand::rng().random::<f32>() * 0.2;
            world.play_sound_fine(
                Sound::EntityLightningBoltImpact,
                SoundCategory::Weather,
                &pos,
                2.0,
                pitch_impact,
            );

            let difficulty = world.level_info.load().difficulty;
            if difficulty == Difficulty::Normal || difficulty == Difficulty::Hard {
                self.spawn_fire(&world, 4);
            }

            self.power_lightning_rod(&world);
            self.clear_copper_on_lightning_strike(&world);
        }

        let new_life = life - 1;
        self.life.store(new_life, Ordering::Relaxed);

        if new_life < 0 {
            let flashes = self.flashes.load(Ordering::Relaxed);
            if flashes == 0 {
                entity.remove();
                return;
            } else if new_life < -entity.random().next_bounded_i32(10) {
                self.flashes.store(flashes - 1, Ordering::Relaxed);
                self.life.store(1, Ordering::Relaxed);
                let seed = entity.random().next_i64();
                self.seed.store(seed, Ordering::Relaxed);
                let world = entity.world.load();
                self.spawn_fire(&world, 0);
            }
        }

        let current_life = self.life.load(Ordering::Relaxed);
        if current_life >= 0 && !self.visual_only.load(Ordering::Relaxed) {
            let world = entity.world.load();
            let pos = entity.pos.load();

            let damage_box = BoundingBox::new(
                Vector3::new(pos.x - 3.0, pos.y - 3.0, pos.z - 3.0),
                Vector3::new(pos.x + 3.0, pos.y + 9.0, pos.z + 3.0),
            );

            let entities = world.get_all_at_box(&damage_box);
            if let Ok(mut hit_guard) = self.hit_entities.lock() {
                for hit_entity in entities {
                    if hit_entity.get_entity().entity_id == entity.entity_id {
                        continue;
                    }
                    let hit_id = hit_entity.get_entity().entity_id;
                    if hit_guard.insert(hit_id) {
                        hit_entity.on_lightning_strike(hit_entity.as_ref(), self);
                    }
                }
            }
        }
    }

    fn init_data_tracker(&self) {}

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}
