use std::sync::{Arc, Mutex};

use pumpkin_data::Block;
use pumpkin_data::block_properties::BeeNestLikeProperties;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::chunk::io::Dirtiable;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;
use uuid::Uuid;

use super::BlockEntity;
use crate::entity::EntityBase;
use crate::entity::ageable::AgeableMob;
use crate::entity::passive::bee::BeeEntity;
use crate::entity::r#type::from_type;
use crate::world::World;

/// Vanilla `BeehiveBlockEntity.IGNORED_BEE_TAGS`: dropped when a bee is stored and again
/// when it is released.
const IGNORED_BEE_TAGS: [&str; 25] = [
    "Air",
    "drop_chances",
    "equipment",
    "Brain",
    "CanPickUpLoot",
    "DeathTime",
    "fall_distance",
    "FallFlying",
    "Fire",
    "HurtTime",
    "LeftHanded",
    "Motion",
    "NoGravity",
    "OnGround",
    "PortalCooldown",
    "Pos",
    "Rotation",
    "sleeping_pos",
    "CannotEnterHiveTicks",
    "TicksSincePollination",
    "CropsGrownSincePollination",
    "hive_pos",
    "Passengers",
    "leash",
    "UUID",
];
pub const MAX_OCCUPANTS: usize = 3;
pub const MIN_TICKS_BEFORE_REENTERING_HIVE: i32 = 400;
const MIN_OCCUPATION_TICKS_NECTAR: i32 = 2400;
pub const MIN_OCCUPATION_TICKS_NECTARLESS: i32 = 600;
/// Vanilla `BeehiveBlock.MAX_HONEY_LEVELS`.
const MAX_HONEY_LEVELS: u8 = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BeeReleaseStatus {
    HoneyDelivered,
    BeeReleased,
    Emergency,
}

/// Vanilla `BeehiveBlockEntity.Occupant`: a stored bee's saved data and how long it stays.
#[derive(Clone)]
pub struct Occupant {
    /// The entity's tags, with its `id`.
    pub entity_data: NbtCompound,
    pub ticks_in_hive: i32,
    pub min_ticks_in_hive: i32,
}

impl Occupant {
    /// Vanilla `Occupant.of`.
    #[must_use]
    pub fn of(entity: &dyn EntityBase) -> Self {
        let mut entity_data = NbtCompound::new();
        entity.write_nbt(&mut entity_data);
        for ignored in IGNORED_BEE_TAGS {
            entity_data.child_tags.remove(ignored);
        }
        let has_nectar = entity_data.get_bool("HasNectar").unwrap_or(false);
        Self {
            entity_data,
            ticks_in_hive: 0,
            min_ticks_in_hive: if has_nectar {
                MIN_OCCUPATION_TICKS_NECTAR
            } else {
                MIN_OCCUPATION_TICKS_NECTARLESS
            },
        }
    }

    fn from_nbt(nbt: &NbtCompound) -> Option<Self> {
        Some(Self {
            entity_data: nbt.get_compound("entity_data")?.clone(),
            ticks_in_hive: nbt.get_int("ticks_in_hive")?,
            min_ticks_in_hive: nbt.get_int("min_ticks_in_hive")?,
        })
    }

    fn to_nbt(&self) -> NbtCompound {
        let mut nbt = NbtCompound::new();
        nbt.put_compound("entity_data", self.entity_data.clone());
        nbt.put_int("ticks_in_hive", self.ticks_in_hive);
        nbt.put_int("min_ticks_in_hive", self.min_ticks_in_hive);
        nbt
    }

    /// Vanilla `Occupant.createEntity`.
    fn create_entity(&self, world: &Arc<World>, hive_pos: BlockPos) -> Option<Arc<dyn EntityBase>> {
        let mut entity_tag = self.entity_data.clone();
        let id = entity_tag.get_string("id")?.to_string();
        entity_tag.child_tags.remove("id");
        for ignored in IGNORED_BEE_TAGS {
            entity_tag.child_tags.remove(ignored);
        }
        let entity_type = EntityType::from_name(id.strip_prefix("minecraft:").unwrap_or(&id))?;
        if !entity_type.has_tag(&tag::EntityType::MINECRAFT_BEEHIVE_INHABITORS) {
            return None;
        }
        let entity = from_type(entity_type, Vector3::default(), world, Uuid::new_v4());
        entity.read_nbt_non_mut(&entity_tag);
        entity.init_data_tracker();
        entity.get_entity().set_has_no_gravity(true);
        if let Some(bee) = entity.cast_any().downcast_ref::<BeeEntity>() {
            bee.set_hive_pos(Some(hive_pos));
            Self::set_bee_release_data(self.ticks_in_hive, bee);
        }
        Some(entity)
    }

    /// Vanilla `Occupant.setBeeReleaseData`.
    fn set_bee_release_data(ticks_in_hive: i32, bee: &BeeEntity) {
        Self::update_bee_age(ticks_in_hive, bee);
        let love_ticks = &bee.mob_entity.love_ticks;
        love_ticks.store(
            (love_ticks.load(std::sync::atomic::Ordering::Relaxed) - ticks_in_hive).max(0),
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// Vanilla `Occupant.updateBeeAge`.
    fn update_bee_age(ticks_in_hive: i32, bee: &BeeEntity) {
        if !bee.is_age_locked() {
            let age = bee.get_age();
            if age < 0 {
                bee.set_age((age + ticks_in_hive).min(0));
            } else if age > 0 {
                bee.set_age((age - ticks_in_hive).max(0));
            }
        }
    }
}

/// Vanilla `BeehiveBlockEntity.BeeData`.
struct BeeData {
    occupant: Occupant,
    ticks_in_hive: i32,
}

impl BeeData {
    const fn new(occupant: Occupant) -> Self {
        let ticks_in_hive = occupant.ticks_in_hive;
        Self {
            occupant,
            ticks_in_hive,
        }
    }

    /// Vanilla `BeeData.tick`: the comparison sees the count from before the increment.
    const fn tick(&mut self) -> bool {
        let ticks = self.ticks_in_hive;
        self.ticks_in_hive += 1;
        ticks > self.occupant.min_ticks_in_hive
    }

    fn to_occupant(&self) -> Occupant {
        Occupant {
            entity_data: self.occupant.entity_data.clone(),
            ticks_in_hive: self.ticks_in_hive,
            min_ticks_in_hive: self.occupant.min_ticks_in_hive,
        }
    }

    fn has_nectar(&self) -> bool {
        self.occupant
            .entity_data
            .get_bool("HasNectar")
            .unwrap_or(false)
    }
}

pub struct BeehiveBlockEntity {
    pub position: BlockPos,
    stored: Mutex<Vec<BeeData>>,
    saved_flower_pos: Mutex<Option<BlockPos>>,
}

impl BlockEntity for BeehiveBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let stored = nbt
            .get_list("bees")
            .map(|bees| {
                bees.iter()
                    .filter_map(NbtTag::extract_compound)
                    .filter_map(Occupant::from_nbt)
                    .map(BeeData::new)
                    .collect()
            })
            .unwrap_or_default();
        let saved_flower_pos = nbt
            .get_int_array("flower_pos")
            .and_then(|arr| (arr.len() == 3).then(|| BlockPos::new(arr[0], arr[1], arr[2])));
        Self {
            position,
            stored: Mutex::new(stored),
            saved_flower_pos: Mutex::new(saved_flower_pos),
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        let bees = self
            .stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|data| NbtTag::Compound(data.to_occupant().to_nbt()))
            .collect();
        nbt.put_list("bees", bees);
        if let Some(pos) = *self
            .saved_flower_pos
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            nbt.put(
                "flower_pos",
                NbtTag::IntArray(vec![pos.0.x, pos.0.y, pos.0.z]),
            );
        }
    }

    /// Vanilla `BeehiveBlockEntity.serverTick`.
    fn tick(&self, world: &Arc<World>) {
        let state_id = world.get_block_state_id(&self.position);
        let saved_flower_pos = *self
            .saved_flower_pos
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut stored = self
            .stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut changed = false;
        let mut i = 0;
        while i < stored.len() {
            if stored[i].tick() {
                let release_status = if stored[i].has_nectar() {
                    BeeReleaseStatus::HoneyDelivered
                } else {
                    BeeReleaseStatus::BeeReleased
                };
                if Self::release_occupant(
                    world,
                    self.position,
                    state_id,
                    &stored[i].to_occupant(),
                    release_status,
                    saved_flower_pos,
                ) {
                    changed = true;
                    stored.remove(i);
                    continue;
                }
            }
            i += 1;
        }
        let has_occupants = !stored.is_empty();
        drop(stored);
        if changed {
            self.mark_chunk_dirty(world);
        }
        if has_occupants && rand::rng().random::<f64>() < 0.005 {
            world.play_sound(
                Sound::BlockBeehiveWork,
                SoundCategory::Blocks,
                &Vector3::new(
                    f64::from(self.position.0.x) + 0.5,
                    f64::from(self.position.0.y),
                    f64::from(self.position.0.z) + 0.5,
                ),
            );
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl BeehiveBlockEntity {
    pub const ID: &'static str = "minecraft:beehive";

    #[must_use]
    pub const fn new(position: BlockPos) -> Self {
        Self {
            position,
            stored: Mutex::new(Vec::new()),
            saved_flower_pos: Mutex::new(None),
        }
    }

    /// Vanilla `isFireNearby`: a fire block in the 3x3x3 cube around the hive.
    #[must_use]
    pub fn is_fire_nearby(&self, world: &World) -> bool {
        BlockPos::iterate(self.position.add(-1, -1, -1), self.position.add(1, 1, 1))
            .any(|pos| world.get_block(&pos) == &Block::FIRE)
    }

    /// Vanilla `getOccupantCount`, or `None` while the hive is busy with its occupants.
    #[must_use]
    pub fn try_get_occupant_count(&self) -> Option<usize> {
        self.stored.try_lock().ok().map(|stored| stored.len())
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
            == MAX_OCCUPANTS
    }

    /// Vanilla `addOccupant`: the bee leaves the world and waits inside.
    pub fn add_occupant(&self, world: &Arc<World>, bee: &BeeEntity) {
        let mut stored = self
            .stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if stored.len() >= MAX_OCCUPANTS {
            return;
        }
        let entity = &bee.mob_entity.living_entity.entity;
        if let Some(vehicle) = entity.get_vehicle() {
            vehicle.get_entity().remove_passenger(entity.entity_id);
        }
        while let Some(passenger) = entity.get_first_passenger() {
            entity.remove_passenger(passenger.get_entity().entity_id);
        }
        entity.unleash();
        stored.push(BeeData::new(Occupant::of(bee)));
        drop(stored);

        if let Some(flower_pos) = bee.get_saved_flower_pos() {
            let mut saved = self
                .saved_flower_pos
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if saved.is_none() || rand::rng().random::<bool>() {
                *saved = Some(flower_pos);
            }
        }
        world.play_sound(
            Sound::BlockBeehiveEnter,
            SoundCategory::Blocks,
            &self.position.to_f64(),
        );
        world.game_event(
            GameEvent::BlockChange,
            self.position.to_centered_f64(),
            crate::world::game_event::GameEventContext::of(
                Some(bee),
                Some(world.get_block_state(&self.position)),
            ),
        );
        entity.remove();
        self.mark_chunk_dirty(world);
    }

    /// Vanilla `releaseOccupant`. Returns whether the bee is back in the world.
    fn release_occupant(
        world: &Arc<World>,
        pos: BlockPos,
        state_id: pumpkin_data::BlockStateId,
        occupant: &Occupant,
        release_status: BeeReleaseStatus,
        saved_flower_pos: Option<BlockPos>,
    ) -> bool {
        if world.bees_stay_in_hive(&pos) && release_status != BeeReleaseStatus::Emergency {
            return false;
        }

        let block = world.get_block(&pos);
        if !block.has_tag(&tag::Block::MINECRAFT_BEEHIVES) {
            return false;
        }
        let props = BeeNestLikeProperties::from_state_id(state_id);
        let facing = props.facing.to_offset();
        let facing_pos = pos.offset(facing);
        let front_blocked = world
            .get_block_state(&facing_pos)
            .get_block_collision_shapes()
            .next()
            .is_some();
        if front_blocked && release_status != BeeReleaseStatus::Emergency {
            return false;
        }

        let Some(entity) = occupant.create_entity(world, pos) else {
            return false;
        };
        if let Some(bee) = entity.cast_any().downcast_ref::<BeeEntity>() {
            let mut random = rand::rng();
            if let Some(flower_pos) = saved_flower_pos
                && !bee.has_saved_flower_pos()
                && random.random::<f32>() < 0.9
            {
                bee.set_saved_flower_pos(flower_pos);
            }

            if release_status == BeeReleaseStatus::HoneyDelivered {
                bee.drop_off_nectar();
                let honey_level = props.honey_level;
                if honey_level < MAX_HONEY_LEVELS {
                    let mut level_increase = if random.random_range(0..100) == 0 {
                        2
                    } else {
                        1
                    };
                    if honey_level + level_increase > MAX_HONEY_LEVELS {
                        level_increase -= 1;
                    }
                    let mut new_props = props;
                    new_props.honey_level = honey_level + level_increase;
                    world.set_block_state(
                        &pos,
                        new_props.to_state_id(block),
                        BlockFlags::NOTIFY_ALL,
                    );
                }
            }

            let base = entity.get_entity();
            let dimension = base.entity_dimension.load();
            let delta = if front_blocked {
                0.0
            } else {
                0.55 + f64::from(dimension.width / 2.0)
            };
            let spawn = Vector3::new(
                f64::from(pos.0.x) + 0.5 + delta * f64::from(facing.x),
                f64::from(pos.0.y) + 0.5 - f64::from(dimension.height / 2.0),
                f64::from(pos.0.z) + 0.5 + delta * f64::from(facing.z),
            );
            base.set_pos(spawn);
            base.last_sent_pos.store(spawn);
        }

        world.play_block_sound(Sound::BlockBeehiveExit, SoundCategory::Blocks, pos);
        world.game_event(
            GameEvent::BlockChange,
            pos.to_centered_f64(),
            crate::world::game_event::GameEventContext::of(
                Some(entity.as_ref()),
                Some(world.get_block_state(&pos)),
            ),
        );
        world.spawn_entity(entity)
    }

    fn mark_chunk_dirty(&self, world: &Arc<World>) {
        let chunk_position = self.position.chunk_position();
        let _ = world.level.read_chunk_sync(&chunk_position, |chunk| {
            chunk.mark_dirty(true);
        });
    }
}
