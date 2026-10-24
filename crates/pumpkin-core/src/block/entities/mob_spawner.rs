use std::sync::{
    Arc,
    atomic::{AtomicI32, Ordering},
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::{entity::EntityType, world::WorldEvent};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use pumpkin_util::random::RandomImpl;

use crate::{
    block::entities::BlockEntity, entity::EntityBase, entity::mob::spawn::finalize_spawn,
    world::World,
};

pub struct MobSpawnerBlockEntity {
    pub position: BlockPos,
    pub delay: AtomicI32,
    pub max_delay: i32,
    pub min_delay: i32,
    pub spawn_count: i32,
    pub spawn_range: i32,
    pub max_nearby_entities: i32,
    pub required_player_range: i32,
    pub entity_type: AtomicCell<Option<&'static EntityType>>,
}

impl MobSpawnerBlockEntity {
    pub const ID: &'static str = "minecraft:mob_spawner";
    pub const DEFAULT_DELAY: i32 = 20;
    pub const DEFAULT_MAX_SPAWN_DELAY: i32 = 800;
    pub const DEFAULT_MIN_SPAWN_DELAY: i32 = 200;
    pub const DEFAULT_SPAWN_COUNT: i32 = 4;
    pub const DEFAULT_SPAWN_RANGE: i32 = 4;
    pub const DEFAULT_MAX_NEARBY_ENTITIES: i32 = 6;
    pub const DEFAULT_REQUIRED_PLAYER_RANGE: i32 = 16;

    #[must_use]
    pub const fn new(position: BlockPos, entity_type: Option<&'static EntityType>) -> Self {
        Self {
            position,
            delay: AtomicI32::new(Self::DEFAULT_DELAY),
            max_delay: Self::DEFAULT_MAX_SPAWN_DELAY,
            min_delay: Self::DEFAULT_MIN_SPAWN_DELAY,
            spawn_count: Self::DEFAULT_SPAWN_COUNT,
            spawn_range: Self::DEFAULT_SPAWN_RANGE,
            max_nearby_entities: Self::DEFAULT_MAX_NEARBY_ENTITIES,
            required_player_range: Self::DEFAULT_REQUIRED_PLAYER_RANGE,
            entity_type: AtomicCell::new(entity_type),
        }
    }

    pub fn write_spawner_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_short("Delay", self.delay.load(Ordering::Relaxed) as i16);
        nbt.put_short("MinSpawnDelay", self.min_delay as i16);
        nbt.put_short("MaxSpawnDelay", self.max_delay as i16);
        nbt.put_short("SpawnCount", self.spawn_count as i16);
        nbt.put_short("SpawnRange", self.spawn_range as i16);
        nbt.put_short("MaxNearbyEntities", self.max_nearby_entities as i16);
        nbt.put_short("RequiredPlayerRange", self.required_player_range as i16);

        if let Some(entity_type) = self.entity_type.load() {
            let mut spawn_entry = NbtCompound::new();

            let mut entity_nbt = NbtCompound::new();
            entity_nbt.put_string("id", format!("minecraft:{}", entity_type.resource_name));

            spawn_entry.put_compound("entity", entity_nbt);

            nbt.put_compound("SpawnData", spawn_entry);
        }
    }
}

impl MobSpawnerBlockEntity {
    /// Vanilla `BaseSpawner.delay`.
    fn delay(&self, world: &Arc<World>) {
        let min_delay = self.min_delay;
        let max_delay = self.max_delay;
        let mut random = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let new_delay = if max_delay <= min_delay {
            min_delay
        } else {
            min_delay + random.next_bounded_i32(max_delay - min_delay)
        };
        // Pumpkin has no spawn potentials list: skip the potential selection step.
        drop(random);
        self.delay.store(new_delay, Ordering::Relaxed);
        world.add_synced_block_event(self.position, 1, 0);
    }

    /// Vanilla `BaseSpawner.serverTick`.
    #[expect(clippy::too_many_lines)]
    fn server_tick(&self, world: &Arc<World>) {
        let Some(entity_type) = self.entity_type.load() else {
            return;
        };

        // Step 1: Check player proximity and spawner game rule.
        let center = self.position.to_centered_f64();
        if !world.has_nearby_alive_player(center, self.required_player_range as f64) {
            return;
        }
        if !world.level_info.load().game_rules.spawner_blocks_work {
            return;
        }

        // Step 2: Handle delay.
        let mut delay = self.delay.load(Ordering::Relaxed);
        if delay == -1 {
            self.delay(world);
            delay = self.delay.load(Ordering::Relaxed);
        }
        if delay > 0 {
            self.delay.store(delay - 1, Ordering::Relaxed);
            return;
        }

        // Step 3: Spawn loop.
        let spawn_range = self.spawn_range;
        let max_nearby = self.max_nearby_entities;
        let mut spawned_any = false;
        let mut cancelled_any = false;

        for _ in 0..self.spawn_count {
            // Draw spawn position from level random (one lock per draw).
            let mut rng = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let a = rng.next_f64();
            drop(rng);

            let mut rng = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let b = rng.next_f64();
            drop(rng);

            let mut rng = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let y_rand = rng.next_bounded_i32(3);
            drop(rng);

            let mut rng = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let c = rng.next_f64();
            drop(rng);

            let mut rng = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let d = rng.next_f64();
            drop(rng);

            let pos = self.position.0;
            let x = pos.x as f64 + (a - b) * spawn_range as f64 + 0.5;
            let y = (pos.y + y_rand - 1) as f64;
            let z = pos.z as f64 + (c - d) * spawn_range as f64 + 0.5;
            let spawn_pos = Vector3::new(x, y, z);

            // Check no collision.
            if !world.no_collision(entity_type.get_spawn_bounding_box(x, y, z)) {
                continue;
            }

            // Spawn rule check (takes &mut LegacyRand).
            let spawn_block_pos = BlockPos::containing(x, y, z);
            let mut rng = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let rules_ok = crate::entity::r#type::check_spawn_rules(
                entity_type,
                world,
                &spawn_block_pos,
                world.is_thundering(),
                crate::entity::mob::spawn::EntitySpawnReason::Spawner,
                &mut rng,
            );
            drop(rng);
            if !rules_ok {
                continue;
            }

            // Create entity.
            let entity = crate::entity::r#type::from_type(
                entity_type,
                spawn_pos,
                world,
                uuid::Uuid::new_v4(),
            );

            // Nearby count (after creating entity, before rotation draw).
            let nearby_count = {
                let target_bb = BoundingBox::new_array(
                    [pos.x as f64, pos.y as f64, pos.z as f64],
                    [(pos.x + 1) as f64, (pos.y + 1) as f64, (pos.z + 1) as f64],
                )
                .expand_all(spawn_range as f64);
                world
                    .entities
                    .load()
                    .iter()
                    .filter(|e| {
                        let ent = e.get_entity();
                        if ent.entity_type.id != entity_type.id {
                            return false;
                        }
                        if ent.is_spectator() {
                            return false;
                        }
                        ent.bounding_box.load().intersects(&target_bb)
                    })
                    .count()
            };

            if nearby_count as i32 >= max_nearby {
                self.delay(world);
                return;
            }

            // Rotation draw.
            let mut rng = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let yaw = rng.next_f32() * 360.0;
            drop(rng);
            entity.get_entity().set_rotation(yaw, 0.0);

            // Mob checks.
            if let Some(mob) = entity.get_mob() {
                if !crate::world::natural_spawner::mob_check_spawn_rules(mob)
                    || !mob.check_spawn_obstruction(world)
                {
                    continue;
                }
                // finalize_spawn for mobs only (keep existing behavior).
                finalize_spawn(
                    &entity,
                    world,
                    crate::entity::mob::spawn::EntitySpawnReason::Spawner,
                    None,
                );
            }

            // Plugin event.
            let mut event =
                crate::plugin::api::events::entity::spawner_spawn::SpawnerSpawnEvent::new(
                    entity.get_entity().entity_id,
                    self.position,
                );
            if let Some(server) = world.server.upgrade() {
                server.plugin_manager.fire_blocking(&server, &mut event);
            }
            if event.cancelled {
                cancelled_any = true;
                continue;
            }

            // Spawn entity.
            if !world.spawn_entity(entity.clone()) {
                self.delay(world);
                return;
            }

            // Particles (level event 2004).
            world.sync_world_event(WorldEvent::ParticlesMobblockSpawn, self.position, 0);

            // Game event.
            world.game_event_from(
                Some(entity.as_ref()),
                pumpkin_data::game_event::GameEvent::EntityPlace,
                spawn_block_pos.to_centered_f64(),
            );

            // SpawnAnim: entity event 20.
            if entity.get_mob().is_some() {
                world.send_entity_status(
                    entity.get_entity(),
                    pumpkin_data::entity::EntityStatus::SilverfishMergeAnim,
                    None,
                );
            }

            spawned_any = true;
        }

        // Step 4: After loop, delay if anything spawned or was cancelled.
        if spawned_any || cancelled_any {
            self.delay(world);
        }
    }

    pub fn set_entity_type(&self, entity_type: &'static EntityType) {
        self.entity_type.store(Some(entity_type));
    }
}

impl BlockEntity for MobSpawnerBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn tick(&self, world: &Arc<World>) {
        self.server_tick(world);
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let get_num = |name: &str| {
            nbt.get_short(name)
                .map(i32::from)
                .or_else(|| nbt.get_int(name))
                .or_else(|| nbt.get_byte(name).map(i32::from))
        };

        let delay = get_num("Delay").unwrap_or(Self::DEFAULT_DELAY);
        let min_delay = get_num("MinSpawnDelay").unwrap_or(Self::DEFAULT_MIN_SPAWN_DELAY);
        let max_delay = get_num("MaxSpawnDelay").unwrap_or(Self::DEFAULT_MAX_SPAWN_DELAY);
        let spawn_count = get_num("SpawnCount").unwrap_or(Self::DEFAULT_SPAWN_COUNT);
        let spawn_range = get_num("SpawnRange").unwrap_or(Self::DEFAULT_SPAWN_RANGE);
        let max_nearby_entities =
            get_num("MaxNearbyEntities").unwrap_or(Self::DEFAULT_MAX_NEARBY_ENTITIES);
        let required_player_range =
            get_num("RequiredPlayerRange").unwrap_or(Self::DEFAULT_REQUIRED_PLAYER_RANGE);

        let entity_type = nbt
            .get_compound("SpawnData")
            .and_then(|data| {
                data.get_compound("entity")
                    .and_then(|entity| entity.get_string("id"))
                    .or_else(|| data.get_string("id"))
            })
            .or_else(|| {
                nbt.get_list("SpawnPotentials")
                    .and_then(|list| list.first())
                    .and_then(|tag| tag.extract_compound())
                    .and_then(|entry| {
                        entry
                            .get_compound("data")
                            .and_then(|data| {
                                data.get_compound("entity")
                                    .and_then(|entity| entity.get_string("id"))
                                    .or_else(|| data.get_string("id"))
                            })
                            .or_else(|| {
                                entry
                                    .get_compound("entity")
                                    .and_then(|entity| entity.get_string("id"))
                            })
                    })
            })
            .or_else(|| nbt.get_string("EntityId"))
            .and_then(EntityType::from_name);

        Self {
            position,
            delay: AtomicI32::new(delay),
            max_delay,
            min_delay,
            spawn_count,
            spawn_range,
            max_nearby_entities,
            required_player_range,
            entity_type: AtomicCell::new(entity_type),
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_spawner_nbt(nbt);
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut final_nbt = NbtCompound::new();
        final_nbt.put_short("Delay", self.delay.load(Ordering::Relaxed) as i16);
        final_nbt.put_short("MinSpawnDelay", self.min_delay as i16);
        final_nbt.put_short("MaxSpawnDelay", self.max_delay as i16);
        final_nbt.put_short("SpawnCount", self.spawn_count as i16);
        final_nbt.put_short("SpawnRange", self.spawn_range as i16);
        final_nbt.put_short("MaxNearbyEntities", self.max_nearby_entities as i16);
        final_nbt.put_short("RequiredPlayerRange", self.required_player_range as i16);

        if let Some(entity_type) = self.entity_type.load() {
            let mut spawn_entry = NbtCompound::new();

            let mut entity_nbt = NbtCompound::new();
            entity_nbt.put_string("id", format!("minecraft:{}", entity_type.resource_name));

            spawn_entry.put_compound("entity", entity_nbt);

            final_nbt.put_compound("SpawnData", spawn_entry);
        }
        Some(final_nbt)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
