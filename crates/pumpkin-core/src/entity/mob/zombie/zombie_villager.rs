use pumpkin_util::math::boundingbox::EntityDimensions;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::Block;
use pumpkin_data::Enchantment;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Effect;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_data::world::WorldEvent;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::java::client::play::Metadata;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::version::JavaMinecraftVersion;
use rand::RngExt;
use uuid::Uuid;

use crate::entity::item::ItemEntity;
use crate::entity::mob::conversion::{ConversionParams, convert_to};
use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::zombie::ZombieEntityBase;
use crate::entity::mob::{Mob, MobEntity};
use crate::entity::passive::villager::VillagerEntity;
use crate::entity::passive::villager::data::{GossipType, VillagerData, villager_type_by_biome};
use crate::entity::player::Player;
use crate::entity::player::advancement::trigger::AdvancementTrigger;
use crate::entity::{Entity, EntityBase};
use crate::world::World;

/// Blocks that make a zombie villager's cure tick down faster: iron bars and beds.
fn speeds_up_conversion(block: &Block) -> bool {
    block == &Block::IRON_BARS || block.has_tag(&tag::Block::MINECRAFT_BEDS)
}

/// Vanilla `initializeZombieVillagerData`: a plains villager with a random profession, so
/// a cured villager has trades.
fn initialize_zombie_villager_data() -> VillagerData {
    VillagerData::new(
        pumpkin_data::villager::VillagerType::Plains,
        pumpkin_data::villager::VillagerProfession::from_i32(rand::random_range(0..15))
            .unwrap_or(pumpkin_data::villager::VillagerProfession::None),
        1,
    )
}

pub struct ZombieVillagerEntity {
    pub mob_entity: Arc<ZombieEntityBase>,
    villager_data: std::sync::Mutex<VillagerData>,
    /// Vanilla `DATA_VILLAGER_DATA_FINALIZED`.
    villager_data_finalized: AtomicBool,
    /// `Offers` and `Gossips` kept as the NBT vanilla stores them, so a cure can
    /// hand them straight back to the villager it converts into.
    villager_nbt: std::sync::Mutex<NbtCompound>,
    villager_xp: AtomicI32,
    /// Vanilla `villagerConversionTime`.
    conversion_time: AtomicI32,
    /// Vanilla `DATA_CONVERTING_ID`.
    converting: AtomicBool,
    conversion_starter: AtomicCell<Option<Uuid>>,
}

impl ZombieVillagerEntity {
    const VILLAGER_CONVERSION_WAIT_MIN: i32 = 3600;
    const VILLAGER_CONVERSION_WAIT_MAX: i32 = 6000;
    const MAX_SPECIAL_BLOCKS_COUNT: i32 = 14;
    const SPECIAL_BLOCK_RADIUS: i32 = 4;
    const NOT_CONVERTING: i32 = -1;
    /// Nausea the cured villager wakes up with.
    const NAUSEA_DURATION: i32 = 200;

    pub fn new(entity: Entity) -> Arc<Self> {
        Self::with_can_break_doors(entity, false)
    }

    #[must_use]
    pub fn with_can_break_doors(entity: Entity, can_break_doors: bool) -> Arc<Self> {
        let mob_entity =
            ZombieEntityBase::with_can_break_doors(entity, can_break_doors, BABY_DIMENSIONS);
        let zombie_villager = Arc::new(Self {
            mob_entity,
            villager_data: std::sync::Mutex::new(initialize_zombie_villager_data()),
            villager_data_finalized: AtomicBool::new(false),
            villager_nbt: std::sync::Mutex::new(NbtCompound::new()),
            villager_xp: AtomicI32::new(0),
            conversion_time: AtomicI32::new(0),
            converting: AtomicBool::new(false),
            conversion_starter: AtomicCell::new(None),
        });
        let data = zombie_villager.get_villager_data();
        zombie_villager
            .get_entity()
            .set_synced_data(tracked_data::zombie_villager::VILLAGER_DATA, data);
        zombie_villager
    }

    #[must_use]
    pub fn get_villager_data(&self) -> VillagerData {
        *self
            .villager_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Vanilla `ZombieVillager.setVillagerData`: a new profession forgets the old trades.
    pub fn set_villager_data(&self, data: VillagerData) {
        let old_profession = std::mem::replace(
            &mut *self
                .villager_data
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            data,
        )
        .profession;
        if old_profession != data.profession {
            self.villager_nbt
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .child_tags
                .remove("Offers");
        }
        self.get_entity()
            .set_synced_data(tracked_data::zombie_villager::VILLAGER_DATA, data);
    }

    #[must_use]
    pub fn get_villager_data_finalized(&self) -> bool {
        self.villager_data_finalized.load(Ordering::Relaxed)
    }

    pub fn set_villager_data_finalized(&self, finalized: bool) {
        self.villager_data_finalized
            .store(finalized, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            tracked_data::zombie_villager::DATA_VILLAGER_DATA_FINALIZED,
            finalized,
        );
    }

    /// Vanilla `VillagerDataHolder.finalizeVillagerType`.
    fn finalize_villager_type(&self, world: &World, pos: &BlockPos) {
        if !self.get_villager_data_finalized() {
            let mut data = self.get_villager_data();
            data.r#type = VarInt(villager_type_by_biome(world.get_biome(pos)) as i32);
            self.set_villager_data(data);
            self.set_villager_data_finalized(true);
        }
    }

    #[must_use]
    pub fn is_converting(&self) -> bool {
        self.converting.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_baby(&self) -> bool {
        self.mob_entity.is_baby()
    }

    /// Vanilla `startConverting`: begin the cure, swapping Weakness for Strength
    /// and telling clients to play the cure sound.
    pub fn start_converting(&self, starter: Option<Uuid>, ticks: i32) {
        let living = &self.mob_entity.mob_entity.living_entity;
        let entity = &living.entity;

        self.conversion_starter.store(starter);
        self.conversion_time.store(ticks, Ordering::Relaxed);
        self.converting.store(true, Ordering::Relaxed);
        entity.set_synced_data(tracked_data::zombie_villager::DATA_CONVERTING_ID, true);
        living.remove_effect(&StatusEffect::WEAKNESS);
        living.add_effect(Effect {
            effect_type: &StatusEffect::STRENGTH,
            duration: ticks,
            // Vanilla computes `min(difficulty - 1, 0)`, which `MobEffectInstance` clamps
            // to 0 on every difficulty.
            amplifier: 0,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        });
        entity
            .world
            .load()
            .send_entity_status(entity, EntityStatus::ZombieConverting, None);
    }

    /// Vanilla `getConversionProgress`: iron bars and beds nearby speed the cure up.
    #[expect(clippy::cast_possible_truncation)]
    fn conversion_progress(&self) -> i32 {
        let mut progress = 1;
        let mut random = self.get_random();
        if random.random::<f32>() >= 0.01 {
            return progress;
        }

        let entity = self.get_entity();
        let world = entity.world.load();
        let pos = entity.pos.load();
        let (x, y, z) = (pos.x as i32, pos.y as i32, pos.z as i32);
        let mut special_blocks = 0;

        for block_x in x - Self::SPECIAL_BLOCK_RADIUS..x + Self::SPECIAL_BLOCK_RADIUS {
            for block_y in y - Self::SPECIAL_BLOCK_RADIUS..y + Self::SPECIAL_BLOCK_RADIUS {
                for block_z in z - Self::SPECIAL_BLOCK_RADIUS..z + Self::SPECIAL_BLOCK_RADIUS {
                    if special_blocks >= Self::MAX_SPECIAL_BLOCKS_COUNT {
                        return progress;
                    }
                    let block = world.get_block(&BlockPos::new(block_x, block_y, block_z));
                    if speeds_up_conversion(block) {
                        if random.random::<f32>() < 0.3 {
                            progress += 1;
                        }
                        special_blocks += 1;
                    }
                }
            }
        }

        progress
    }

    /// The `dropPreservedEquipment` part of vanilla `finishConversion`: guaranteed drops fall
    /// to the ground. Items with Curse of Binding would move into the villager's inventory.
    fn drop_preserved_equipment(&self) {
        let mob_entity = &self.mob_entity.mob_entity;
        let entity = &mob_entity.living_entity.entity;
        let world = entity.world.load();
        // Vanilla `EquipmentSlot.VALUES` order.
        for slot in [
            EquipmentSlot::MAIN_HAND,
            EquipmentSlot::OFF_HAND,
            EquipmentSlot::FEET,
            EquipmentSlot::LEGS,
            EquipmentSlot::CHEST,
            EquipmentSlot::HEAD,
            EquipmentSlot::BODY,
            EquipmentSlot::SADDLE,
        ] {
            let stack = mob_entity.get_item_slot(&slot);
            if stack.is_empty() {
                continue;
            }
            if stack.get_enchantment_level(&Enchantment::BINDING_CURSE) > 0 {
                // TODO: vanilla moves it into the villager's inventory (`SlotAccess` 300 +
                // slot index).
                continue;
            }
            if mob_entity.drop_chance(&slot) > 1.0 {
                mob_entity.set_item_slot(&slot, ItemStack::EMPTY.clone());
                let item_entity = ItemEntity::new(
                    Entity::new(world.clone(), entity.pos.load(), &EntityType::ITEM),
                    stack,
                );
                world.spawn_entity(Arc::new(item_entity));
            }
        }
    }

    /// Vanilla `finishConversion`: the zombie villager turns back into the villager it
    /// remembers being.
    fn finish_conversion(&self) {
        let entity = self.get_entity();
        let silent = entity.is_silent();
        let block_pos = entity.block_pos.load();
        let data = self.get_villager_data();
        let mut carried = self
            .villager_nbt
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        carried.put_int("Xp", self.villager_xp.load(Ordering::Relaxed));
        let starter = self.conversion_starter.load();
        convert_to(
            self,
            &EntityType::VILLAGER,
            ConversionParams {
                keep_equipment: false,
                preserve_can_pick_up_loot: false,
            },
            |converted| {
                self.drop_preserved_equipment();
                let Some(villager) = converted.cast_any().downcast_ref::<VillagerEntity>() else {
                    return;
                };
                let world = converted.get_entity().world.load_full();
                // `Villager` has no `villagerDataFinalized` in Pumpkin yet.
                villager.set_villager_data(data);
                // Gossips, trades and trading experience.
                villager.mob_read_nbt(&carried);
                villager.finalize_spawn(&world, None);

                if let Some(starter) = starter
                    && let Some(player) = world.get_player_by_uuid(starter)
                {
                    player.trigger_advancement(AdvancementTrigger::CuredZombieVillager);
                    // `ReputationEventType.ZOMBIE_VILLAGER_CURED`.
                    let mut gossips = villager
                        .gossips
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let entries = gossips.entry(starter).or_default();
                    for (gossip_type, amount) in [
                        (GossipType::MajorPositive, 20),
                        (GossipType::MinorPositive, 25),
                    ] {
                        let value = entries.entry(gossip_type).or_default();
                        *value = (*value + amount).min(gossip_type.max_value());
                    }
                }

                villager.mob_entity.living_entity.add_effect(Effect {
                    effect_type: &StatusEffect::NAUSEA,
                    duration: Self::NAUSEA_DURATION,
                    amplifier: 0,
                    ambient: false,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                });
                if !silent {
                    world.sync_world_event(WorldEvent::SoundZombieConverted, block_pos, 0);
                }
            },
        );
    }

    /// Vanilla `ZombieVillager.finalizeSpawn`. `conversion` stands for
    /// `EntitySpawnReason.CONVERSION`.
    fn finalize_zombie_villager_spawn(
        &self,
        world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
        conversion: bool,
    ) -> Option<SpawnGroupData> {
        self.finalize_villager_type(world, &self.get_entity().block_pos.load());
        self.mob_entity
            .finalize_zombie_spawn(self, world, group_data, conversion)
    }

    /// The `afterConversion` step of vanilla `Zombie.convertVillagerToZombieVillager`, from
    /// the killed villager's saved data.
    pub(crate) fn convert_from_villager(&self, world: &Arc<World>, villager_nbt: &NbtCompound) {
        // Pumpkin villagers do not track `villagerDataFinalized`; the ones vanilla spawns
        // always end up finalized.
        self.set_villager_data_finalized(true);
        self.finalize_zombie_villager_spawn(
            world,
            Some(SpawnGroupData::Zombie {
                is_baby: false,
                can_spawn_jockey: true,
            }),
            true,
        );
        if let Some(villager_data_nbt) = villager_nbt.get_compound("VillagerData") {
            self.set_villager_data(read_villager_data(
                villager_data_nbt,
                self.get_villager_data(),
            ));
        }
        let mut carried = NbtCompound::new();
        if let Some(gossips) = villager_nbt.get_list("Gossips") {
            carried.put_list("Gossips", gossips.to_vec());
        }
        if let Some(offers) = villager_nbt.get_compound("Offers") {
            carried.put_compound("Offers", offers.clone());
        }
        *self
            .villager_nbt
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = carried;
        self.villager_xp
            .store(villager_nbt.get_int("Xp").unwrap_or(0), Ordering::Relaxed);
    }
}

fn read_villager_data(nbt: &NbtCompound, mut data: VillagerData) -> VillagerData {
    if let Some(r#type) = nbt.get_int("Type") {
        data.r#type = VarInt(r#type);
    }
    if let Some(profession) = nbt.get_int("Profession") {
        data.profession = VarInt(profession);
    }
    if let Some(level) = nbt.get_int("Level") {
        data.level = VarInt(level);
    }
    data
}

/// Vanilla `ZombieVillager.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    EntityDimensions::scalable(0.49, 0.98).with_eye_height(0.67);

impl Mob for ZombieVillagerEntity {
    crate::impl_zombie_mob!(mob_entity);

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity.mob_entity
    }

    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.finalize_zombie_villager_spawn(world, group_data, false)
    }

    fn populate_default_equipment_slots(
        &self,
        world: &Arc<World>,
        difficulty: &RegionalDifficulty,
    ) {
        self.mob_entity
            .populate_default_equipment_slots(world, difficulty);
    }

    fn on_attack(&self, target: &dyn EntityBase) {
        self.mob_entity.on_attack(self, target);
    }

    /// Vanilla `ZombieVillager.getVoicePitch`: babies squeak an octave up.
    fn voice_pitch(&self) -> f32 {
        let base = if self.is_baby() { 2.0 } else { 1.0 };
        let mut random = self.get_random();
        (random.random::<f32>() - random.random::<f32>()) * 0.2 + base
    }

    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        !self.is_converting() && self.villager_xp.load(Ordering::Relaxed) == 0
    }

    fn mob_java_spawn_metadata(&self, version: JavaMinecraftVersion) -> Option<Box<[u8]>> {
        if version < JavaMinecraftVersion::V_1_9 {
            return None;
        }
        let mut metadata = Vec::new();
        Metadata::new(
            tracked_data::zombie_villager::VILLAGER_DATA,
            self.get_villager_data(),
        )
        .write(&mut metadata, &version)
        .ok()?;
        Metadata::new(
            tracked_data::zombie_villager::DATA_CONVERTING_ID,
            self.is_converting(),
        )
        .write(&mut metadata, &version)
        .ok()?;
        metadata.push(255);
        Some(metadata.into_boxed_slice())
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if item_stack.get_item() == &Item::GOLDEN_APPLE {
            if self
                .mob_entity
                .mob_entity
                .living_entity
                .has_effect(&StatusEffect::WEAKNESS)
            {
                item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                self.start_converting(
                    Some(player.get_entity().entity_uuid),
                    self.get_random().random_range(
                        Self::VILLAGER_CONVERSION_WAIT_MIN..=Self::VILLAGER_CONVERSION_WAIT_MAX,
                    ),
                );
            }
            return true;
        }
        self.mob_entity.mob_interact(player, item_stack)
    }

    /// Vanilla `ZombieVillager.tick` before `super.tick()`.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        if !self.mob_entity.mob_entity.living_entity.is_alive() || !self.is_converting() {
            return;
        }
        let progress = self.conversion_progress();
        if self.conversion_time.fetch_sub(progress, Ordering::Relaxed) - progress <= 0 {
            self.finish_conversion();
        }
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.mob_entity.write_zombie_nbt(nbt);

        let data = self.get_villager_data();
        let mut villager_data_nbt = NbtCompound::new();
        villager_data_nbt.put_int("Type", data.r#type.0);
        villager_data_nbt.put_int("Profession", data.profession.0);
        villager_data_nbt.put_int("Level", data.level.0);
        nbt.put_compound("VillagerData", villager_data_nbt);
        nbt.put_bool("VillagerDataFinalized", self.get_villager_data_finalized());

        {
            let carried = self
                .villager_nbt
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(offers) = carried.get_compound("Offers") {
                nbt.put_compound("Offers", offers.clone());
            }
            if let Some(gossips) = carried.get_list("Gossips") {
                nbt.put_list("Gossips", gossips.to_vec());
            }
        }

        nbt.put_int(
            "ConversionTime",
            if self.is_converting() {
                self.conversion_time.load(Ordering::Relaxed)
            } else {
                Self::NOT_CONVERTING
            },
        );
        if let Some(starter) = self.conversion_starter.load() {
            nbt.put_uuid("ConversionPlayer", starter);
        }
        nbt.put_int("Xp", self.villager_xp.load(Ordering::Relaxed));
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.mob_entity.read_zombie_nbt(self, nbt);

        let villager_data_nbt = nbt.get_compound("VillagerData");
        if nbt.get_bool("VillagerDataFinalized").unwrap_or(false) || villager_data_nbt.is_some() {
            self.set_villager_data_finalized(true);
            let data = villager_data_nbt.map_or_else(initialize_zombie_villager_data, |data_nbt| {
                read_villager_data(data_nbt, self.get_villager_data())
            });
            // Vanilla sets the synced data directly here, keeping any trades.
            *self
                .villager_data
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = data;
            self.get_entity()
                .set_synced_data(tracked_data::zombie_villager::VILLAGER_DATA, data);
        }

        let mut carried = NbtCompound::new();
        if let Some(offers) = nbt.get_compound("Offers") {
            carried.put_compound("Offers", offers.clone());
        }
        if let Some(gossips) = nbt.get_list("Gossips") {
            carried.put_list("Gossips", gossips.to_vec());
        }
        *self
            .villager_nbt
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = carried;

        let conversion_time = nbt
            .get_int("ConversionTime")
            .unwrap_or(Self::NOT_CONVERTING);
        if conversion_time == Self::NOT_CONVERTING {
            self.converting.store(false, Ordering::Relaxed);
            self.get_entity()
                .set_synced_data(tracked_data::zombie_villager::DATA_CONVERTING_ID, false);
            self.conversion_time
                .store(Self::NOT_CONVERTING, Ordering::Relaxed);
        } else {
            self.start_converting(nbt.get_uuid("ConversionPlayer"), conversion_time);
        }

        self.villager_xp
            .store(nbt.get_int("Xp").unwrap_or(0), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::{Block, speeds_up_conversion};

    #[test]
    fn only_iron_bars_and_beds_speed_up_the_cure() {
        assert!(speeds_up_conversion(&Block::IRON_BARS));
        assert!(speeds_up_conversion(&Block::RED_BED));
        assert!(speeds_up_conversion(&Block::WHITE_BED));
        assert!(!speeds_up_conversion(&Block::COPPER_BARS));
        assert!(!speeds_up_conversion(&Block::STONE));
        assert!(!speeds_up_conversion(&Block::RED_WOOL));
    }
}
