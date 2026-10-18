use std::{
    path::PathBuf,
    sync::{
        RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use bytes::{Buf, BufMut, Bytes};
use pumpkin_data::{Block, BlockId, BlockStateId, chunk::ChunkStatus, fluid::Fluid};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::resource_location::{FromResourceLocation, ToResourceLocation};
use rustc_hash::FxHashMap;

use crate::{
    chunk::{
        ChunkEntityData, ChunkReadingError, ChunkSerializingError,
        format::anvil::SingleChunkDataSerializer,
        io::{Dirtiable, file_manager::PathFromLevelFolder},
    },
    data_fix,
    generation::section_coords,
    level::LevelFolder,
    tick::{SavedTick, TickPriority, scheduler::ChunkTickScheduler},
    world_info::WORLD_VERSION,
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;

use super::{
    ChunkData, ChunkHeightmaps, ChunkLight, ChunkParsingError, ChunkSections,
    palette::{BiomePalette, BlockPalette},
};
pub mod anvil;
pub mod linear;
pub mod pump;

/// Chunk tags Pumpkin reads into `ChunkData` and writes back itself; the rest are kept as they
/// were.
const MODELLED_CHUNK_KEYS: [&str; 16] = [
    "DataVersion",
    "xPos",
    "zPos",
    "yPos",
    "Status",
    "sections",
    "Heightmaps",
    "block_ticks",
    "fluid_ticks",
    "block_entities",
    "isLightOn",
    "InhabitedTime",
    "LastUpdate",
    "PumpkinCustomData",
    "BukkitValues",
    TERRAIN_COMPLETE_TAG,
];

/// Marker that Pumpkin's 26.2-stamped saves put on chunks with finished terrain. Earlier builds
/// also saved half-built terrain as `minecraft:terrain`.
const TERRAIN_COMPLETE_TAG: &str = "pumpkin:terrain_complete";

impl SingleChunkDataSerializer for ChunkData {
    #[inline]
    fn from_bytes(bytes: &Bytes, pos: Vector2<i32>) -> Result<Self, ChunkReadingError> {
        Self::internal_from_bytes(bytes, pos).map_err(ChunkReadingError::ParsingError)
    }

    #[inline]
    fn to_bytes(&self) -> Result<Bytes, ChunkSerializingError> {
        Ok(self.internal_to_bytes())
    }

    #[inline]
    fn to_pump_bytes(&self) -> Result<Bytes, ChunkSerializingError> {
        Ok(self.internal_to_pump_bytes())
    }

    #[inline]
    fn position(&self) -> (i32, i32) {
        (self.x, self.z)
    }
}

impl PathFromLevelFolder for ChunkData {
    #[inline]
    fn file_path(folder: &LevelFolder, file_name: &str) -> PathBuf {
        folder.region_folder.join(file_name)
    }
}

impl Dirtiable for ChunkData {
    #[inline]
    fn mark_dirty(&self, flag: bool) {
        self.dirty.store(flag, Ordering::Relaxed);
    }

    #[inline]
    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }
}

/// The section stores `Y` as a byte, short, int or long depending on who wrote
/// the file. The datafixer writes ints. Reading only bytes would map every int
/// section to `Y = 0`, and they would overwrite each other.
fn section_y(section: &NbtCompound) -> i32 {
    use pumpkin_nbt::tag::NbtTag;

    match section.get("Y") {
        Some(NbtTag::Byte(value)) => i32::from(*value),
        Some(NbtTag::Short(value)) => i32::from(*value),
        Some(NbtTag::Int(value)) => *value,
        Some(NbtTag::Long(value)) => i32::try_from(*value).unwrap_or(0),
        _ => 0,
    }
}

/// What vanilla does when `yPos` is missing, for example after it upgrades a
/// world in place: the lowest section that stores biomes, and never above 0.
/// That gives `-4` in the Overworld and `0` in the Nether and End, which are
/// the section minimums of those dimensions.
fn lowest_biome_section_y(root_tag: &NbtCompound) -> Option<i32> {
    let sections = root_tag.get_list("sections")?;
    sections
        .iter()
        .filter_map(|tag| match tag {
            pumpkin_nbt::tag::NbtTag::Compound(compound) if compound.has("biomes") => {
                Some(section_y(compound))
            }
            _ => None,
        })
        .min()
        .map(|y| y.min(0))
}

fn extract_u16_array(tag: &pumpkin_nbt::tag::NbtTag) -> Option<Box<[BlockStateId]>> {
    match tag {
        pumpkin_nbt::tag::NbtTag::IntArray(arr) => Some(
            arr.iter()
                .map(|&x| BlockStateId::new_or_air(x as u16))
                .collect(),
        ),
        pumpkin_nbt::tag::NbtTag::ByteArray(arr) => Some(
            arr.iter()
                .map(|&x| BlockStateId::new_or_air(x as u16))
                .collect(),
        ),
        pumpkin_nbt::tag::NbtTag::LongArray(arr) => Some(
            arr.iter()
                .map(|&x| BlockStateId::new_or_air(x as u16))
                .collect(),
        ),
        pumpkin_nbt::tag::NbtTag::List(list) => {
            let ids: Box<[BlockStateId]> = list
                .iter()
                .map(|t| match t {
                    pumpkin_nbt::tag::NbtTag::Int(x) => BlockStateId::new_or_air(*x as u16),
                    pumpkin_nbt::tag::NbtTag::Short(x) => BlockStateId::new_or_air(*x as u16),
                    pumpkin_nbt::tag::NbtTag::Byte(x) => BlockStateId::new_or_air(*x as u16),
                    pumpkin_nbt::tag::NbtTag::Long(x) => BlockStateId::new_or_air(*x as u16),
                    pumpkin_nbt::tag::NbtTag::Compound(_) | pumpkin_nbt::tag::NbtTag::String(_) => {
                        Block::state_from_nbt(t).unwrap_or(BlockStateId::AIR)
                    }
                    _ => BlockStateId::AIR,
                })
                .collect();
            Some(ids)
        }
        _ => None,
    }
}

fn extract_u8_array(tag: &pumpkin_nbt::tag::NbtTag) -> Option<Box<[u8]>> {
    match tag {
        pumpkin_nbt::tag::NbtTag::ByteArray(arr) => Some(arr.iter().map(|&x| x as u8).collect()),
        pumpkin_nbt::tag::NbtTag::IntArray(arr) => Some(arr.iter().map(|&x| x as u8).collect()),
        pumpkin_nbt::tag::NbtTag::List(list) => {
            let bytes: Box<[u8]> = list
                .iter()
                .map(|t| match t {
                    pumpkin_nbt::tag::NbtTag::Byte(x) => *x as u8,
                    pumpkin_nbt::tag::NbtTag::Int(x) => *x as u8,
                    pumpkin_nbt::tag::NbtTag::Short(x) => *x as u8,
                    pumpkin_nbt::tag::NbtTag::String(s) => {
                        let name = s.strip_prefix("minecraft:").unwrap_or(s);
                        pumpkin_data::biome::Biome::from_name(name).map_or(0, |b| b.id)
                    }
                    _ => 0,
                })
                .collect();
            Some(bytes)
        }
        _ => None,
    }
}

/// Vanilla `SavedTick.filterTickListForChunk` applied to the chunk's saved tick list.
fn read_saved_ticks<T>(
    root_tag: &NbtCompound,
    key: &str,
    chunk_pos: Vector2<i32>,
) -> Vec<SavedTick<T>>
where
    T: FromResourceLocation,
{
    root_tag.get_list(key).map_or_else(Vec::new, |list| {
        list.iter()
            .filter_map(|tag| match tag {
                pumpkin_nbt::tag::NbtTag::Compound(compound) => {
                    SavedTick::from_nbt_compound(compound)
                }
                _ => None,
            })
            .filter(|tick| tick.position.chunk_position() == chunk_pos)
            .collect()
    })
}

fn saved_ticks_to_nbt<T>(ticks: &ChunkTickScheduler<T>) -> Vec<pumpkin_nbt::tag::NbtTag>
where
    T: ToResourceLocation + Copy + std::hash::Hash + Eq,
{
    ticks
        .to_saved_ticks()
        .iter()
        .map(|tick| pumpkin_nbt::tag::NbtTag::Compound(tick.to_nbt_compound()))
        .collect()
}

impl ChunkData {
    #[allow(clippy::too_many_lines)]
    pub fn internal_from_bytes(
        chunk_data: &[u8],
        position: Vector2<i32>,
    ) -> Result<Self, ChunkParsingError> {
        if chunk_data.starts_with(&pump::PUMP_CHUNK_MAGIC) {
            return Self::internal_from_pump_bytes(chunk_data, position);
        }

        let is_named = chunk_data.len() >= 3
            && chunk_data[0] == 0x0a
            && chunk_data[1] == 0x00
            && chunk_data[2] == 0x00;

        let mut cursor = std::io::Cursor::new(chunk_data);
        let mut reader = pumpkin_nbt::deserializer::NbtReadHelperJava::new(&mut cursor);
        let nbt = if is_named {
            pumpkin_nbt::Nbt::read(&mut reader)
        } else {
            pumpkin_nbt::Nbt::read_unnamed(&mut reader)
        }
        .map_err(|e| ChunkParsingError::ErrorDeserializingChunk(e.to_string()))?;

        let root_tag = nbt.root_tag;

        let x_pos = root_tag.get_int("xPos").ok_or_else(|| {
            ChunkParsingError::ErrorDeserializingChunk("Missing xPos".to_string())
        })?;
        let z_pos = root_tag.get_int("zPos").ok_or_else(|| {
            ChunkParsingError::ErrorDeserializingChunk("Missing zPos".to_string())
        })?;

        if x_pos != position.x || z_pos != position.y {
            return Err(ChunkParsingError::ErrorDeserializingChunk(format!(
                "Expected data for chunk {},{} but got it for {},{}!",
                position.x, position.y, x_pos, z_pos,
            )));
        }

        // Vanilla omits yPos when it upgrades a world in place. It uses the
        // dimension minimum for such chunks, which is the lowest section that
        // stores biomes. Do the same instead of rejecting the chunk.
        let min_y_section = match root_tag.get_int("yPos") {
            Some(y_pos) => y_pos,
            None => lowest_biome_section_y(&root_tag).ok_or_else(|| {
                ChunkParsingError::ErrorDeserializingChunk("Missing yPos".to_string())
            })?,
        };

        // Section indices are bytes in this format, and the loops below already
        // treat them as such. Reject a yPos outside that range instead of
        // truncating it, because the untruncated value sizes the vectors below.
        let Ok(min_y_section) = i8::try_from(min_y_section) else {
            return Err(ChunkParsingError::ErrorDeserializingChunk(format!(
                "yPos {min_y_section} is outside the range of a section index"
            )));
        };

        let mut max_y_section = min_y_section;
        if let Some(sections_list) = root_tag.get_list("sections") {
            for section_tag in sections_list {
                if let pumpkin_nbt::tag::NbtTag::Compound(section_compound) = section_tag {
                    let y = section_y(section_compound) as i8;
                    if y > max_y_section {
                        max_y_section = y;
                    }
                }
            }
        }

        let section_count =
            (i32::from(max_y_section) - i32::from(min_y_section) + 1).max(0) as usize;
        let mut block_lights = vec![LightContainer::Empty(0); section_count];
        let mut sky_lights = vec![LightContainer::Empty(0); section_count];
        let mut block_palettes = vec![BlockPalette::default(); section_count];
        let mut biome_palettes = vec![BiomePalette::default(); section_count];

        if let Some(sections_list) = root_tag.get_list("sections") {
            for section_tag in sections_list {
                if let pumpkin_nbt::tag::NbtTag::Compound(section_compound) = section_tag {
                    let y = section_y(section_compound);
                    let Ok(index) = usize::try_from(y.saturating_sub(i32::from(min_y_section)))
                    else {
                        continue;
                    };
                    if index >= section_count {
                        continue;
                    }

                    let block_light = section_compound
                        .get("BlockLight")
                        .and_then(|tag| tag.extract_byte_array())
                        .map(|arr| -> Box<[u8]> {
                            // SAFETY: `arr` is an `i8` slice (`&[i8]`). `u8` and `i8` have identical memory layout, alignment (1 byte), and lifetime.
                            unsafe {
                                Box::from(std::slice::from_raw_parts(
                                    arr.as_ptr().cast::<u8>(),
                                    arr.len(),
                                ))
                            }
                        });

                    let sky_light = section_compound
                        .get("SkyLight")
                        .and_then(|tag| tag.extract_byte_array())
                        .map(|arr| -> Box<[u8]> {
                            // SAFETY: `arr` is an `i8` slice (`&[i8]`). `u8` and `i8` have identical memory layout, alignment (1 byte), and lifetime.
                            unsafe {
                                Box::from(std::slice::from_raw_parts(
                                    arr.as_ptr().cast::<u8>(),
                                    arr.len(),
                                ))
                            }
                        });

                    // `Full` skips the length check `LightContainer::new` makes,
                    // and every reader indexes it up to `ARRAY_SIZE`, so a
                    // short array from disk is dropped instead of stored.
                    block_lights[index] = block_light
                        .filter(|data| data.len() == LightContainer::ARRAY_SIZE)
                        .map_or(LightContainer::Empty(0), LightContainer::Full);
                    sky_lights[index] = sky_light
                        .filter(|data| data.len() == LightContainer::ARRAY_SIZE)
                        .map_or(LightContainer::Empty(0), LightContainer::Full);

                    if let Some(bs_compound) = section_compound.get_compound("block_states") {
                        let data = bs_compound
                            .get_long_array("data")
                            .map(|arr| arr.to_vec().into_boxed_slice());
                        let palette = bs_compound
                            .get("palette")
                            .and_then(extract_u16_array)
                            .unwrap_or_else(|| vec![BlockStateId::AIR].into_boxed_slice());

                        block_palettes[index] =
                            BlockPalette::from_disk_nbt(ChunkSectionBlockStates { data, palette });
                    } else {
                        block_palettes[index] = BlockPalette::default();
                    }

                    if let Some(b_compound) = section_compound.get_compound("biomes") {
                        let data = b_compound
                            .get_long_array("data")
                            .map(|arr| arr.to_vec().into_boxed_slice());
                        let palette = b_compound
                            .get("palette")
                            .and_then(extract_u8_array)
                            .unwrap_or_else(|| vec![0].into_boxed_slice());

                        biome_palettes[index] =
                            BiomePalette::from_disk_nbt(ChunkSectionBiomes { data, palette });
                    } else {
                        biome_palettes[index] = BiomePalette::default();
                    }
                }
            }
        }

        // Assemble the LightEngine
        let light_engine = ChunkLight {
            block_light: block_lights.into_boxed_slice(),
            sky_light: sky_lights.into_boxed_slice(),
        };

        // Assemble the ChunkSections
        let min_y = section_coords::section_to_block(i32::from(min_y_section));
        let (random_tick_sections, randomly_ticking_mask) =
            ChunkSections::build_random_tick_sections_cache(&block_palettes);
        let section = ChunkSections {
            count: block_palettes.len(),
            block_sections: RwLock::new(block_palettes.into_boxed_slice()),
            random_tick_sections: RwLock::new(random_tick_sections),
            randomly_ticking_mask: std::sync::atomic::AtomicU32::new(randomly_ticking_mask),
            biome_sections: RwLock::new(biome_palettes.into_boxed_slice()),
            min_y,
        };

        let heightmaps = root_tag.get_compound("Heightmaps").map_or(
            ChunkHeightmaps {
                world_surface: None,
                motion_blocking: None,
                motion_blocking_no_leaves: None,
            },
            // A heightmap of the wrong length is dropped rather than stored:
            // every reader indexes it by column and would run off the end.
            |h_compound| ChunkHeightmaps {
                world_surface: h_compound
                    .get_long_array("WORLD_SURFACE")
                    .filter(|a| a.len() == ChunkHeightmaps::LONGS)
                    .map(|a| a.to_vec().into_boxed_slice()),
                motion_blocking: h_compound
                    .get_long_array("MOTION_BLOCKING")
                    .filter(|a| a.len() == ChunkHeightmaps::LONGS)
                    .map(|a| a.to_vec().into_boxed_slice()),
                motion_blocking_no_leaves: h_compound
                    .get_long_array("MOTION_BLOCKING_NO_LEAVES")
                    .filter(|a| a.len() == ChunkHeightmaps::LONGS)
                    .map(|a| a.to_vec().into_boxed_slice()),
            },
        );
        let block_ticks = read_saved_ticks::<&'static Block>(&root_tag, "block_ticks", position);
        let fluid_ticks = read_saved_ticks::<&'static Fluid>(&root_tag, "fluid_ticks", position);

        let data_version = data_fix::data_version(&root_tag);
        let mut block_entities = FxHashMap::default();
        if let Some(list) = root_tag.get_list("block_entities") {
            for tag in list {
                if let pumpkin_nbt::tag::NbtTag::Compound(nbt) = tag
                    && let Some(x) = nbt.get_int("x")
                    && let Some(y) = nbt.get_int("y")
                    && let Some(z) = nbt.get_int("z")
                {
                    let mut nbt = nbt.clone();
                    data_fix::upgrade_block_entity(&mut nbt, data_version);
                    block_entities.insert(BlockPos::new(x, y, z), nbt);
                }
            }
        }

        let light_correct = root_tag.get_bool("isLightOn").unwrap_or(false);

        let status_str = root_tag.get_string("Status").unwrap_or("minecraft:empty");
        let status = match status_str {
            "minecraft:structure_starts" => ChunkStatus::StructureStarts,
            "minecraft:structure_references" => ChunkStatus::StructureReferences,
            // MergeTerrainChunkStatusFix: carvers -> terrain, noise/surface -> biomes.
            // Vanilla never saved `terrain` before that fix, but Pumpkin did, for its
            // half-built noise and surface stages too; those saves (no completeness marker)
            // start over from biomes rather than load as finished terrain.
            "minecraft:terrain"
                if data_version < data_fix::MERGE_TERRAIN_CHUNK_STATUS_FIX_VERSION
                    && root_tag.get_bool(TERRAIN_COMPLETE_TAG) != Some(true) =>
            {
                ChunkStatus::Biomes
            }
            "minecraft:terrain" | "minecraft:carvers" => ChunkStatus::Terrain,
            "minecraft:biomes" | "minecraft:noise" | "minecraft:surface" => ChunkStatus::Biomes,
            "minecraft:features" => ChunkStatus::Features,
            "minecraft:initialize_light" => ChunkStatus::InitializeLight,
            "minecraft:light" => ChunkStatus::Light,
            "minecraft:spawn" => ChunkStatus::Spawn,
            "minecraft:full" => ChunkStatus::Full,
            _ => ChunkStatus::Empty,
        };

        let mut unmodelled_nbt = NbtCompound::new();
        for (key, tag) in &root_tag.child_tags {
            if !MODELLED_CHUNK_KEYS.contains(&key.as_ref()) {
                unmodelled_nbt.put(key, tag.clone());
            }
        }

        let custom_data = root_tag
            .get_compound("PumpkinCustomData")
            .or_else(|| root_tag.get_compound("BukkitValues"))
            .cloned()
            .unwrap_or_default();

        Ok(Self {
            section,
            heightmap: std::sync::Mutex::new(heightmaps),
            x: position.x,
            z: position.y,
            // This chunk is read from disk, so it has not been modified
            dirty: AtomicBool::new(false),
            block_ticks: ChunkTickScheduler::from_saved(block_ticks),
            fluid_ticks: ChunkTickScheduler::from_saved(fluid_ticks),
            pending_block_entities: std::sync::Mutex::new(block_entities),
            light_engine: std::sync::Mutex::new(light_engine),
            light_populated: AtomicBool::new(light_correct),
            status,
            blending_data: None,
            inhabited_time: AtomicU64::new(root_tag.get_long("InhabitedTime").unwrap_or(0) as u64),
            custom_data: std::sync::Mutex::new(custom_data),
            last_update: std::sync::atomic::AtomicI64::new(
                root_tag.get_long("LastUpdate").unwrap_or(0),
            ),
            unmodelled_nbt,
        })
    }

    #[allow(clippy::too_many_lines)]
    pub fn internal_to_bytes(&self) -> Bytes {
        use pumpkin_nbt::tag::NbtTag;

        fn extract_light_ref(light: Option<&LightContainer>) -> Option<&[u8]> {
            match light {
                Some(LightContainer::Full(data)) => Some(data.as_ref()),
                _ => None,
            }
        }

        let is_light_correct = self
            .light_populated
            .load(std::sync::atomic::Ordering::Relaxed);

        let block_entities_nbt = {
            let entities_guard = self
                .pending_block_entities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            entities_guard.values().cloned().collect::<Vec<_>>()
        };

        let light_lock = self
            .light_engine
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let heightmap_lock = self
            .heightmap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let block_lock = self
            .section
            .block_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let biome_lock = self
            .section
            .biome_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let min_section_y = (self.section.min_y >> 4) as i8;

        let mut root_compound = NbtCompound::new();
        root_compound.put_int("DataVersion", WORLD_VERSION);
        root_compound.put_int("xPos", self.x);
        root_compound.put_int("zPos", self.z);
        root_compound.put_int("yPos", section_coords::block_to_section(self.section.min_y));

        let status_str = match self.status {
            ChunkStatus::Empty => "minecraft:empty",
            ChunkStatus::StructureStarts => "minecraft:structure_starts",
            ChunkStatus::StructureReferences => "minecraft:structure_references",
            ChunkStatus::Biomes => "minecraft:biomes",
            ChunkStatus::Terrain => "minecraft:terrain",
            ChunkStatus::Features => "minecraft:features",
            ChunkStatus::InitializeLight => "minecraft:initialize_light",
            ChunkStatus::Light => "minecraft:light",
            ChunkStatus::Spawn => "minecraft:spawn",
            ChunkStatus::Full => "minecraft:full",
        };
        root_compound.put_string("Status", status_str.to_string());

        let mut heightmaps_compound = NbtCompound::new();
        if let Some(ref arr) = heightmap_lock.world_surface {
            heightmaps_compound.put("WORLD_SURFACE", NbtTag::LongArray(arr.to_vec()));
        }
        if let Some(ref arr) = heightmap_lock.motion_blocking {
            heightmaps_compound.put("MOTION_BLOCKING", NbtTag::LongArray(arr.to_vec()));
        }
        if let Some(ref arr) = heightmap_lock.motion_blocking_no_leaves {
            heightmaps_compound.put("MOTION_BLOCKING_NO_LEAVES", NbtTag::LongArray(arr.to_vec()));
        }
        root_compound.put_compound("Heightmaps", heightmaps_compound);

        let mut sections_list = Vec::new();
        for i in 0..self.section.count {
            let mut section_comp = NbtCompound::new();
            let y_val = i as i8 + min_section_y;
            section_comp.put_byte("Y", y_val);

            // block_states
            let block_states_nbt = block_lock[i].to_disk_nbt();
            let mut bs_comp = NbtCompound::new();
            if let Some(ref data_arr) = block_states_nbt.data {
                bs_comp.put("data", NbtTag::LongArray(data_arr.to_vec()));
            }
            let palette_tags: Vec<NbtTag> = block_states_nbt
                .palette
                .iter()
                .map(|&id| Block::state_to_nbt(id))
                .collect();
            bs_comp.put_list("palette", palette_tags);
            section_comp.put_compound("block_states", bs_comp);

            // biomes
            let biomes_nbt = biome_lock[i].to_disk_nbt();
            let mut b_comp = NbtCompound::new();
            if let Some(ref data_arr) = biomes_nbt.data {
                b_comp.put("data", NbtTag::LongArray(data_arr.to_vec()));
            }
            let biome_palette_tags: Vec<NbtTag> = biomes_nbt
                .palette
                .iter()
                .map(|&val| {
                    let name = pumpkin_data::biome::Biome::from_id(val)
                        .map_or("plains", |b| b.registry_id);
                    let full_name = if name.starts_with("minecraft:") {
                        name.to_string()
                    } else {
                        format!("minecraft:{name}")
                    };
                    NbtTag::String(full_name.into())
                })
                .collect();
            b_comp.put_list("palette", biome_palette_tags);
            section_comp.put_compound("biomes", b_comp);

            // block_light
            if let Some(light_data) = extract_light_ref(light_lock.block_light.get(i)) {
                let bytes: Box<[i8]> = light_data.iter().map(|&x| x as i8).collect();
                section_comp.put("BlockLight", NbtTag::ByteArray(bytes));
            }

            // sky_light
            if let Some(light_data) = extract_light_ref(light_lock.sky_light.get(i)) {
                let bytes: Box<[i8]> = light_data.iter().map(|&x| x as i8).collect();
                section_comp.put("SkyLight", NbtTag::ByteArray(bytes));
            }

            sections_list.push(NbtTag::Compound(section_comp));
        }
        root_compound.put_list("sections", sections_list);

        root_compound.put_list("block_ticks", saved_ticks_to_nbt(&self.block_ticks));
        root_compound.put_list("fluid_ticks", saved_ticks_to_nbt(&self.fluid_ticks));

        let mut block_entities_list = Vec::new();
        for entity_comp in block_entities_nbt {
            block_entities_list.push(NbtTag::Compound(entity_comp));
        }
        root_compound.put_list("block_entities", block_entities_list);

        root_compound.put_bool("isLightOn", is_light_correct);
        root_compound.put_long(
            "InhabitedTime",
            self.inhabited_time.load(Ordering::Relaxed) as i64,
        );

        let custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !custom_data.is_empty() {
            root_compound.put_compound("PumpkinCustomData", custom_data.clone());
        }
        root_compound.put_long("LastUpdate", self.last_update.load(Ordering::Relaxed));
        for (key, tag) in &self.unmodelled_nbt.child_tags {
            root_compound.put(key, tag.clone());
        }

        let nbt = pumpkin_nbt::Nbt::from(root_compound);
        nbt.write()
    }

    #[allow(clippy::too_many_lines)]
    pub fn internal_to_pump_bytes(&self) -> Bytes {
        // 2: tick delays are vanilla `SavedTick`'s full `i32` (1 stored a byte).
        const PUMP_CHUNK_VERSION: u8 = 2;

        let is_light_correct = self.light_populated.load(Ordering::Relaxed);
        let block_entities_nbt = {
            let entities_guard = self
                .pending_block_entities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            entities_guard.values().cloned().collect::<Vec<_>>()
        };

        let light_lock = self
            .light_engine
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let heightmap_lock = self
            .heightmap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let block_lock = self
            .section
            .block_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let biome_lock = self
            .section
            .biome_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let ws = heightmap_lock
            .world_surface
            .as_deref()
            .filter(|a| a.len() == ChunkHeightmaps::LONGS);
        let mb = heightmap_lock
            .motion_blocking
            .as_deref()
            .filter(|a| a.len() == ChunkHeightmaps::LONGS);
        let mbnl = heightmap_lock
            .motion_blocking_no_leaves
            .as_deref()
            .filter(|a| a.len() == ChunkHeightmaps::LONGS);
        let has_custom = !custom_data.is_empty();

        let mut flags = u8::from(is_light_correct);
        if ws.is_some() {
            flags |= 1 << 1;
        }
        if mb.is_some() {
            flags |= 1 << 2;
        }
        if mbnl.is_some() {
            flags |= 1 << 3;
        }
        if has_custom {
            flags |= 1 << 4;
        }

        let status_u8 = match self.status {
            ChunkStatus::Empty => 0,
            ChunkStatus::StructureStarts => 1,
            ChunkStatus::StructureReferences => 2,
            ChunkStatus::Biomes => 3,
            ChunkStatus::Terrain => 4,
            ChunkStatus::Features => 5,
            ChunkStatus::InitializeLight => 6,
            ChunkStatus::Light => 7,
            ChunkStatus::Spawn => 8,
            ChunkStatus::Full => 9,
        };

        let mut out = Vec::with_capacity(16 * 1024);
        out.extend_from_slice(&pump::PUMP_CHUNK_MAGIC);
        out.put_u8(PUMP_CHUNK_VERSION);
        out.put_i32(WORLD_VERSION);
        out.put_i32(self.x);
        out.put_i32(self.z);
        out.put_i32(self.section.min_y);
        out.put_u8(status_u8);
        out.put_u8(flags);
        out.put_u64(self.inhabited_time.load(Ordering::Relaxed));

        for hm in [ws, mb, mbnl].into_iter().flatten() {
            for &val in hm {
                out.put_i64(val);
            }
        }

        out.put_u16(self.section.count as u16);
        for i in 0..self.section.count {
            block_lock[i].write_pump_binary(&mut out);
            biome_lock[i].write_pump_binary(&mut out);

            let bl = match light_lock.block_light.get(i) {
                Some(LightContainer::Full(data)) if data.len() == LightContainer::ARRAY_SIZE => {
                    Some(data.as_ref())
                }
                _ => None,
            };
            let sl = match light_lock.sky_light.get(i) {
                Some(LightContainer::Full(data)) if data.len() == LightContainer::ARRAY_SIZE => {
                    Some(data.as_ref())
                }
                _ => None,
            };
            let mut light_flags = 0u8;
            if bl.is_some() {
                light_flags |= 1;
            }
            if sl.is_some() {
                light_flags |= 2;
            }
            out.put_u8(light_flags);
            if let Some(data) = bl {
                out.extend_from_slice(data);
            }
            if let Some(data) = sl {
                out.extend_from_slice(data);
            }
        }

        let block_ticks = self.block_ticks.to_saved_ticks();
        out.put_u32(block_ticks.len() as u32);
        for tick in block_ticks {
            out.put_i32(tick.position.0.x);
            out.put_i32(tick.position.0.y);
            out.put_i32(tick.position.0.z);
            out.put_i32(tick.delay);
            out.put_i8(tick.priority as i32 as i8);
            out.put_u16(tick.value.id.as_u16());
        }

        let fluid_ticks = self.fluid_ticks.to_saved_ticks();
        out.put_u32(fluid_ticks.len() as u32);
        for tick in fluid_ticks {
            out.put_i32(tick.position.0.x);
            out.put_i32(tick.position.0.y);
            out.put_i32(tick.position.0.z);
            out.put_i32(tick.delay);
            out.put_i8(tick.priority as i32 as i8);
            out.put_u16(tick.value.id);
        }

        out.put_u32(block_entities_nbt.len() as u32);
        for entity_comp in block_entities_nbt {
            let bytes = pumpkin_nbt::Nbt::new(String::new(), entity_comp).write_unnamed();
            out.put_u32(bytes.len() as u32);
            out.extend_from_slice(&bytes);
        }

        if has_custom {
            let bytes = pumpkin_nbt::Nbt::new(String::new(), custom_data.clone()).write_unnamed();
            out.put_u32(bytes.len() as u32);
            out.extend_from_slice(&bytes);
        }

        out.into()
    }

    #[allow(clippy::too_many_lines)]
    pub fn internal_from_pump_bytes(
        chunk_data: &[u8],
        position: Vector2<i32>,
    ) -> Result<Self, ChunkParsingError> {
        const HEADER_MIN_LEN: usize = 4 + 1 + 4 + 4 + 4 + 4 + 1 + 1 + 8;
        const MAX_SECTIONS: usize = 256;
        const MAX_TICKS: usize = 65_536;
        const MAX_BLOCK_ENTITIES: usize = 65_536;

        if chunk_data.len() < HEADER_MIN_LEN || chunk_data[..4] != pump::PUMP_CHUNK_MAGIC {
            return Err(ChunkParsingError::ErrorDeserializingChunk(
                "Invalid pump chunk header".into(),
            ));
        }

        let mut buf = &chunk_data[4..];
        let version = buf.get_u8();
        if version != 1 && version != 2 {
            return Err(ChunkParsingError::ErrorDeserializingChunk(format!(
                "Unsupported pump chunk version {version}"
            )));
        }

        let _data_version = buf.get_i32();
        let x_pos = buf.get_i32();
        let z_pos = buf.get_i32();
        if x_pos != position.x || z_pos != position.y {
            return Err(ChunkParsingError::ErrorDeserializingChunk(format!(
                "Expected data for chunk {},{} but got it for {},{}!",
                position.x, position.y, x_pos, z_pos,
            )));
        }

        let min_y = buf.get_i32();
        let status_u8 = buf.get_u8();
        let flags = buf.get_u8();
        let inhabited_time = buf.get_u64();

        let status = match status_u8 {
            1 => ChunkStatus::StructureStarts,
            2 => ChunkStatus::StructureReferences,
            3 => ChunkStatus::Biomes,
            4 => ChunkStatus::Terrain,
            5 => ChunkStatus::Features,
            6 => ChunkStatus::InitializeLight,
            7 => ChunkStatus::Light,
            8 => ChunkStatus::Spawn,
            9 => ChunkStatus::Full,
            _ => ChunkStatus::Empty,
        };
        let light_correct = (flags & 1) != 0;

        let read_heightmap =
            |buf: &mut &[u8], present: bool| -> Result<Option<Box<[i64]>>, ChunkParsingError> {
                if !present {
                    return Ok(None);
                }
                if buf.remaining() < ChunkHeightmaps::LONGS * 8 {
                    return Err(ChunkParsingError::ErrorDeserializingChunk(
                        "Unexpected EOF reading heightmap".into(),
                    ));
                }
                let mut hm = Vec::with_capacity(ChunkHeightmaps::LONGS);
                for _ in 0..ChunkHeightmaps::LONGS {
                    hm.push(buf.get_i64());
                }
                Ok(Some(hm.into_boxed_slice()))
            };

        let heightmaps = ChunkHeightmaps {
            world_surface: read_heightmap(&mut buf, (flags & (1 << 1)) != 0)?,
            motion_blocking: read_heightmap(&mut buf, (flags & (1 << 2)) != 0)?,
            motion_blocking_no_leaves: read_heightmap(&mut buf, (flags & (1 << 3)) != 0)?,
        };

        if buf.remaining() < 2 {
            return Err(ChunkParsingError::ErrorDeserializingChunk(
                "Unexpected EOF reading section count".into(),
            ));
        }
        let section_count = buf.get_u16() as usize;
        if section_count > MAX_SECTIONS {
            return Err(ChunkParsingError::ErrorDeserializingChunk(format!(
                "Section count {section_count} exceeds maximum {MAX_SECTIONS}"
            )));
        }

        let mut block_palettes = Vec::with_capacity(section_count);
        let mut biome_palettes = Vec::with_capacity(section_count);
        let mut block_lights = Vec::with_capacity(section_count);
        let mut sky_lights = Vec::with_capacity(section_count);

        for _ in 0..section_count {
            block_palettes.push(BlockPalette::read_pump_binary(&mut buf)?);
            biome_palettes.push(BiomePalette::read_pump_binary(&mut buf)?);

            if !buf.has_remaining() {
                return Err(ChunkParsingError::ErrorDeserializingChunk(
                    "Unexpected EOF reading section light flags".into(),
                ));
            }
            let light_flags = buf.get_u8();
            let block_light = if (light_flags & 1) != 0 {
                if buf.remaining() < LightContainer::ARRAY_SIZE {
                    return Err(ChunkParsingError::ErrorDeserializingChunk(
                        "Unexpected EOF reading block light".into(),
                    ));
                }
                let data: Box<[u8]> = buf[..LightContainer::ARRAY_SIZE].into();
                buf.advance(LightContainer::ARRAY_SIZE);
                LightContainer::Full(data)
            } else {
                LightContainer::Empty(0)
            };
            let sky_light = if (light_flags & 2) != 0 {
                if buf.remaining() < LightContainer::ARRAY_SIZE {
                    return Err(ChunkParsingError::ErrorDeserializingChunk(
                        "Unexpected EOF reading sky light".into(),
                    ));
                }
                let data: Box<[u8]> = buf[..LightContainer::ARRAY_SIZE].into();
                buf.advance(LightContainer::ARRAY_SIZE);
                LightContainer::Full(data)
            } else {
                LightContainer::Empty(0)
            };
            block_lights.push(block_light);
            sky_lights.push(sky_light);
        }

        let (random_tick_sections, randomly_ticking_mask) =
            ChunkSections::build_random_tick_sections_cache(&block_palettes);
        let section = ChunkSections {
            count: block_palettes.len(),
            block_sections: RwLock::new(block_palettes.into_boxed_slice()),
            random_tick_sections: RwLock::new(random_tick_sections),
            randomly_ticking_mask: std::sync::atomic::AtomicU32::new(randomly_ticking_mask),
            biome_sections: RwLock::new(biome_palettes.into_boxed_slice()),
            min_y,
        };
        let light_engine = ChunkLight {
            block_light: block_lights.into_boxed_slice(),
            sky_light: sky_lights.into_boxed_slice(),
        };

        if buf.remaining() < 4 {
            return Err(ChunkParsingError::ErrorDeserializingChunk(
                "Unexpected EOF reading block tick count".into(),
            ));
        }
        let tick_record_len = if version == 1 { 16 } else { 19 };
        let block_tick_count = buf.get_u32() as usize;
        if block_tick_count > MAX_TICKS || buf.remaining() < block_tick_count * tick_record_len {
            return Err(ChunkParsingError::ErrorDeserializingChunk(
                "Invalid block tick count".into(),
            ));
        }
        let mut block_ticks = Vec::with_capacity(block_tick_count);
        for _ in 0..block_tick_count {
            let x = buf.get_i32();
            let y = buf.get_i32();
            let z = buf.get_i32();
            let delay = if version == 1 {
                i32::from(buf.get_u8())
            } else {
                buf.get_i32()
            };
            let priority = TickPriority::by_value(i32::from(buf.get_i8()));
            let raw_id = buf.get_u16();
            if let Some(block_id) = BlockId::new(raw_id) {
                block_ticks.push(SavedTick {
                    delay,
                    priority,
                    position: BlockPos::new(x, y, z),
                    value: Block::from_id(block_id),
                });
            }
        }

        if buf.remaining() < 4 {
            return Err(ChunkParsingError::ErrorDeserializingChunk(
                "Unexpected EOF reading fluid tick count".into(),
            ));
        }
        let fluid_tick_count = buf.get_u32() as usize;
        if fluid_tick_count > MAX_TICKS || buf.remaining() < fluid_tick_count * tick_record_len {
            return Err(ChunkParsingError::ErrorDeserializingChunk(
                "Invalid fluid tick count".into(),
            ));
        }
        let mut fluid_ticks = Vec::with_capacity(fluid_tick_count);
        for _ in 0..fluid_tick_count {
            let x = buf.get_i32();
            let y = buf.get_i32();
            let z = buf.get_i32();
            let delay = if version == 1 {
                i32::from(buf.get_u8())
            } else {
                buf.get_i32()
            };
            let priority = TickPriority::by_value(i32::from(buf.get_i8()));
            let raw_id = buf.get_u16();
            if let Some(fluid) = Fluid::from_id(raw_id) {
                fluid_ticks.push(SavedTick {
                    delay,
                    priority,
                    position: BlockPos::new(x, y, z),
                    value: fluid,
                });
            }
        }

        if buf.remaining() < 4 {
            return Err(ChunkParsingError::ErrorDeserializingChunk(
                "Unexpected EOF reading block entity count".into(),
            ));
        }
        let be_count = buf.get_u32() as usize;
        if be_count > MAX_BLOCK_ENTITIES {
            return Err(ChunkParsingError::ErrorDeserializingChunk(
                "Invalid block entity count".into(),
            ));
        }
        let mut block_entities = FxHashMap::default();
        for _ in 0..be_count {
            if buf.remaining() < 4 {
                return Err(ChunkParsingError::ErrorDeserializingChunk(
                    "Unexpected EOF reading block entity length".into(),
                ));
            }
            let len = buf.get_u32() as usize;
            if buf.remaining() < len {
                return Err(ChunkParsingError::ErrorDeserializingChunk(
                    "Unexpected EOF reading block entity payload".into(),
                ));
            }
            let mut cursor = std::io::Cursor::new(&buf[..len]);
            let mut reader = pumpkin_nbt::deserializer::NbtReadHelperJava::new(&mut cursor);
            let nbt = pumpkin_nbt::Nbt::read_unnamed(&mut reader)
                .map_err(|e| ChunkParsingError::ErrorDeserializingChunk(e.to_string()))?
                .root_tag;
            buf.advance(len);
            if let Some(x) = nbt.get_int("x")
                && let Some(y) = nbt.get_int("y")
                && let Some(z) = nbt.get_int("z")
            {
                block_entities.insert(BlockPos::new(x, y, z), nbt);
            }
        }

        let custom_data = if (flags & (1 << 4)) != 0 {
            if buf.remaining() < 4 {
                return Err(ChunkParsingError::ErrorDeserializingChunk(
                    "Unexpected EOF reading custom data length".into(),
                ));
            }
            let len = buf.get_u32() as usize;
            if buf.remaining() < len {
                return Err(ChunkParsingError::ErrorDeserializingChunk(
                    "Unexpected EOF reading custom data payload".into(),
                ));
            }
            let mut cursor = std::io::Cursor::new(&buf[..len]);
            let mut reader = pumpkin_nbt::deserializer::NbtReadHelperJava::new(&mut cursor);
            let nbt = pumpkin_nbt::Nbt::read_unnamed(&mut reader)
                .map_err(|e| ChunkParsingError::ErrorDeserializingChunk(e.to_string()))?
                .root_tag;
            buf.advance(len);
            nbt
        } else {
            NbtCompound::default()
        };

        Ok(Self {
            section,
            heightmap: std::sync::Mutex::new(heightmaps),
            x: position.x,
            z: position.y,
            dirty: AtomicBool::new(false),
            block_ticks: ChunkTickScheduler::from_saved(block_ticks),
            fluid_ticks: ChunkTickScheduler::from_saved(fluid_ticks),
            pending_block_entities: std::sync::Mutex::new(block_entities),
            light_engine: std::sync::Mutex::new(light_engine),
            light_populated: AtomicBool::new(light_correct),
            status,
            blending_data: None,
            inhabited_time: AtomicU64::new(inhabited_time),
            custom_data: std::sync::Mutex::new(custom_data),
            // Pump stores neither: `LastUpdate` restarts at 0 and only modelled tags are kept.
            last_update: std::sync::atomic::AtomicI64::new(0),
            unmodelled_nbt: NbtCompound::new(),
        })
    }

    pub fn set_custom_data(&self, namespace: &str, key: &str, value: pumpkin_nbt::tag::NbtTag) {
        let mut custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut namespace_data = custom_data
            .child_tags
            .remove(namespace)
            .and_then(|tag| match tag {
                pumpkin_nbt::tag::NbtTag::Compound(compound) => Some(compound),
                _ => None,
            })
            .unwrap_or_default();

        namespace_data.child_tags.insert(key.into(), value);
        custom_data.child_tags.insert(
            namespace.into(),
            pumpkin_nbt::tag::NbtTag::Compound(namespace_data),
        );
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub fn get_custom_data(&self, namespace: &str, key: &str) -> Option<pumpkin_nbt::tag::NbtTag> {
        let custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        custom_data
            .get(namespace)?
            .extract_compound()?
            .get(key)
            .cloned()
    }

    pub fn remove_custom_data(&self, namespace: &str, key: &str) {
        let mut custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let Some(pumpkin_nbt::tag::NbtTag::Compound(mut namespace_data)) =
            custom_data.child_tags.remove(namespace)
        else {
            return;
        };

        namespace_data.child_tags.remove(key);
        if !namespace_data.is_empty() {
            custom_data.child_tags.insert(
                namespace.into(),
                pumpkin_nbt::tag::NbtTag::Compound(namespace_data),
            );
        }
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub fn has_custom_data(&self, namespace: &str, key: &str) -> bool {
        self.get_custom_data(namespace, key).is_some()
    }
}

impl PathFromLevelFolder for ChunkEntityData {
    #[inline]
    fn file_path(folder: &LevelFolder, file_name: &str) -> PathBuf {
        folder.entities_folder.join(file_name)
    }
}

impl Dirtiable for ChunkEntityData {
    #[inline]
    fn mark_dirty(&self, flag: bool) {
        self.dirty.store(flag, Ordering::Relaxed);
    }

    #[inline]
    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }
}

impl SingleChunkDataSerializer for ChunkEntityData {
    #[inline]
    fn from_bytes(bytes: &Bytes, pos: Vector2<i32>) -> Result<Self, ChunkReadingError> {
        Self::internal_from_bytes(bytes, pos).map_err(ChunkReadingError::ParsingError)
    }

    #[inline]
    fn to_bytes(&self) -> Result<Bytes, ChunkSerializingError> {
        Ok(self.internal_to_bytes())
    }

    #[inline]
    fn position(&self) -> (i32, i32) {
        (self.x, self.z)
    }
}

impl ChunkEntityData {
    fn internal_from_bytes(
        chunk_data: &[u8],
        position: Vector2<i32>,
    ) -> Result<Self, ChunkParsingError> {
        let is_named = chunk_data.len() >= 3
            && chunk_data[0] == 0x0a
            && chunk_data[1] == 0x00
            && chunk_data[2] == 0x00;
        let mut cursor = std::io::Cursor::new(chunk_data);
        let mut reader = pumpkin_nbt::deserializer::NbtReadHelperJava::new(
            pumpkin_nbt::deserializer::NbtStreamReader(&mut cursor),
        );
        let nbt = if is_named {
            pumpkin_nbt::Nbt::read(&mut reader)
        } else {
            pumpkin_nbt::Nbt::read_unnamed(&mut reader)
        }
        .map_err(|e| ChunkParsingError::ErrorDeserializingChunk(e.to_string()))?;

        let pos_array = match (nbt.get_int("Position-X"), nbt.get_int("Position-Z")) {
            (Some(x), Some(z)) => [x, z],
            _ => {
                if let Some(pumpkin_nbt::tag::NbtTag::IntArray(pos)) = nbt.get("Position") {
                    if pos.len() >= 2 {
                        [pos[0], pos[1]]
                    } else {
                        [0, 0]
                    }
                } else {
                    [0, 0]
                }
            }
        };

        if pos_array[0] != position.x || pos_array[1] != position.y {
            return Err(ChunkParsingError::ErrorDeserializingChunk(format!(
                "Expected data for entity chunk {},{} but got it for {},{}!",
                position.x, position.y, pos_array[0], pos_array[1],
            )));
        }

        let data_version = data_fix::data_version(&nbt.root_tag);
        let entities = match nbt.get("Entities") {
            Some(pumpkin_nbt::tag::NbtTag::List(list)) => list
                .iter()
                .filter_map(|t| match t {
                    pumpkin_nbt::tag::NbtTag::Compound(c) => {
                        let mut c = c.clone();
                        data_fix::upgrade_item_stacks(&mut c, data_version);
                        Some(c)
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };

        Ok(Self {
            x: position.x,
            z: position.y,
            data: std::sync::Mutex::new(entities),
            live: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
        })
    }

    fn internal_to_bytes(&self) -> Bytes {
        let mut root = NbtCompound::new();
        root.put_int("DataVersion", WORLD_VERSION);
        root.put(
            "Position",
            pumpkin_nbt::tag::NbtTag::IntArray(vec![self.x, self.z]),
        );
        let entities_tag: Vec<pumpkin_nbt::tag::NbtTag> = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|c| pumpkin_nbt::tag::NbtTag::Compound(c.clone()))
            .collect();
        root.put_list("Entities", entities_tag);

        let nbt = pumpkin_nbt::Nbt::from(root);
        nbt.write()
    }
}

#[derive(Clone)]
pub struct ChunkSectionBiomes {
    pub(crate) data: Option<Box<[i64]>>,
    pub(crate) palette: Box<[u8]>,
}

#[derive(Clone)]
pub struct ChunkSectionBlockStates {
    pub(crate) data: Option<Box<[i64]>>,
    pub(crate) palette: Box<[BlockStateId]>,
}

#[derive(Debug, Clone)]
pub enum LightContainer {
    Empty(u8),
    Full(Box<[u8]>),
}

impl LightContainer {
    pub const DIM: usize = 16;
    pub const ARRAY_SIZE: usize = Self::DIM * Self::DIM * Self::DIM / 2;

    #[must_use]
    pub fn new_empty(default: u8) -> Self {
        assert!(default <= 15, "Default value must be between 0 and 15");
        Self::Empty(default)
    }

    #[must_use]
    pub fn new(data: Box<[u8]>) -> Self {
        assert!(
            data.len() == Self::ARRAY_SIZE,
            "Data length must be {}",
            Self::ARRAY_SIZE
        );
        Self::Full(data)
    }

    #[must_use]
    pub fn new_filled(default: u8) -> Self {
        assert!(default <= 15, "Default value must be between 0 and 15");
        let value = default << 4 | default;
        Self::Full([value; Self::ARRAY_SIZE].into())
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty(_))
    }

    #[inline]
    const fn index(x: usize, y: usize, z: usize) -> usize {
        y * 16 * 16 + z * 16 + x
    }

    #[inline]
    #[must_use]
    pub fn get(&self, x: usize, y: usize, z: usize) -> u8 {
        match self {
            Self::Full(data) => {
                let index = Self::index(x, y, z);
                (data[index >> 1] >> (4 * (index & 1))) & 0x0F
            }
            Self::Empty(default) => *default,
        }
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, value: u8) {
        match self {
            Self::Full(data) => {
                let index = Self::index(x, y, z);
                let shift = 4 * (index & 1);
                let mask = 0x0F << shift;
                data[index >> 1] = (data[index >> 1] & !mask) | (value << shift);
            }
            Self::Empty(default) => {
                if value != *default {
                    *self = Self::new_filled(*default);
                    self.set(x, y, z, value);
                }
            }
        }
    }

    #[inline]
    pub fn set_column_y_range(
        &mut self,
        x: usize,
        z: usize,
        y_start: usize,
        y_end: usize,
        value: u8,
    ) {
        if y_start >= y_end {
            return;
        }
        match self {
            Self::Full(data) => {
                let shift = 4 * (x & 1);
                let mask = 0x0F << shift;
                let val = (value & 0x0F) << shift;
                let mut byte_idx = (y_start * 256 + z * 16 + x) >> 1;
                for _ in y_start..y_end {
                    data[byte_idx] = (data[byte_idx] & !mask) | val;
                    byte_idx += 128;
                }
            }
            Self::Empty(default) => {
                if value != *default {
                    *self = Self::new_filled(*default);
                    self.set_column_y_range(x, z, y_start, y_end, value);
                }
            }
        }
    }

    #[inline]
    pub fn fill(&mut self, value: u8) {
        *self = Self::new_filled(value);
    }
}

impl Default for LightContainer {
    fn default() -> Self {
        Self::new_empty(15)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::Block;
    use pumpkin_nbt::compound::NbtCompound;
    use pumpkin_nbt::tag::NbtTag;

    fn test_section(y: i32, block: &str, with_biomes: bool) -> NbtCompound {
        let mut block_states = NbtCompound::new();
        let mut air = NbtCompound::new();
        air.put_string("Name", "minecraft:air".to_string());
        let mut solid = NbtCompound::new();
        solid.put_string("Name", block.to_string());
        block_states.put(
            "palette",
            NbtTag::List(vec![NbtTag::Compound(air), NbtTag::Compound(solid)]),
        );
        block_states.put("data", NbtTag::LongArray(vec![1; 256]));

        let mut section = NbtCompound::new();
        section.put_int("Y", y);
        section.put("block_states", NbtTag::Compound(block_states));
        if with_biomes {
            let mut biomes = NbtCompound::new();
            biomes.put(
                "palette",
                NbtTag::List(vec![NbtTag::String("minecraft:plains".into())]),
            );
            section.put("biomes", NbtTag::Compound(biomes));
        }
        section
    }

    fn test_chunk(sections: Vec<NbtCompound>) -> pumpkin_nbt::Nbt {
        let mut root = NbtCompound::new();
        root.put_int("DataVersion", 5023);
        root.put_int("xPos", 0);
        root.put_int("zPos", 0);
        root.put_string("Status", "minecraft:full".to_string());
        root.put(
            "sections",
            NbtTag::List(sections.into_iter().map(NbtTag::Compound).collect()),
        );
        pumpkin_nbt::Nbt::new(String::new(), root)
    }

    fn terrain_chunk_status(data_version: i32, complete_marker: bool) -> ChunkStatus {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        let mut nbt = test_chunk(vec![test_section(-4, "minecraft:stone", true)]);
        nbt.root_tag.put_int("DataVersion", data_version);
        nbt.root_tag
            .put_string("Status", "minecraft:terrain".to_string());
        if complete_marker {
            nbt.root_tag.put_bool(TERRAIN_COMPLETE_TAG, true);
        }
        ChunkData::from_bytes(&nbt.write(), Vector2::new(0, 0))
            .expect("terrain chunk parses")
            .status
    }

    // Before the marker, Pumpkin saved its half-built noise and surface stages as `terrain`
    // too; only chunks it marked complete, or saved at 26.3's version, are carved terrain.
    #[test]
    fn unmarked_pumpkin_terrain_chunk_restarts_from_biomes() {
        assert_eq!(terrain_chunk_status(4903, false), ChunkStatus::Biomes);
        assert_eq!(terrain_chunk_status(4903, true), ChunkStatus::Terrain);
        assert_eq!(
            terrain_chunk_status(WORLD_VERSION, false),
            ChunkStatus::Terrain
        );
    }

    #[test]
    fn block_entities_get_the_fixes_for_their_chunk_version() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        let sign_allows_op_features = |data_version: i32| {
            let mut sign = NbtCompound::new();
            sign.put_string("id", "minecraft:sign".to_string());
            sign.put_int("x", 1);
            sign.put_int("y", 2);
            sign.put_int("z", 3);
            let mut nbt = test_chunk(vec![test_section(-4, "minecraft:stone", true)]);
            nbt.root_tag.put_int("DataVersion", data_version);
            nbt.root_tag
                .put_list("block_entities", vec![NbtTag::Compound(sign)]);
            let chunk = ChunkData::from_bytes(&nbt.write(), Vector2::new(0, 0)).expect("parses");
            let block_entities = chunk
                .pending_block_entities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            block_entities
                .get(&BlockPos::new(1, 2, 3))
                .and_then(|sign| sign.get_bool("allow_op_features"))
        };
        assert_eq!(sign_allows_op_features(4903), Some(true));
        assert_eq!(sign_allows_op_features(5002), None);
    }

    #[test]
    fn unmodelled_chunk_tags_survive_a_save() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        let mut structures = NbtCompound::new();
        structures.put_compound("starts", NbtCompound::new());
        let mut nbt = test_chunk(vec![test_section(-4, "minecraft:stone", true)]);
        nbt.root_tag.put_compound("structures", structures.clone());
        nbt.root_tag.put_long("LastUpdate", 77);
        let chunk = ChunkData::from_bytes(&nbt.write(), Vector2::new(0, 0)).expect("parses");
        let saved = chunk.to_bytes().expect("serializes");
        let saved = pumpkin_nbt::Nbt::read(&mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(
            &mut std::io::Cursor::new(saved.as_ref()),
        ))
        .expect("saved chunk is NBT");
        assert_eq!(saved.root_tag.get_compound("structures"), Some(&structures));
        assert_eq!(saved.root_tag.get_long("LastUpdate"), Some(77));
    }

    #[test]
    fn saved_terrain_chunk_keeps_its_status() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        let mut nbt = test_chunk(vec![test_section(-4, "minecraft:stone", true)]);
        nbt.root_tag
            .put_string("Status", "minecraft:carvers".to_string());
        let chunk = ChunkData::from_bytes(&nbt.write(), Vector2::new(0, 0)).expect("parses");
        let saved = chunk.to_bytes().expect("serializes");
        let reloaded = ChunkData::from_bytes(&saved, Vector2::new(0, 0)).expect("reparses");
        assert_eq!(reloaded.status, ChunkStatus::Terrain);
    }

    #[test]
    fn chunk_without_y_pos_uses_the_lowest_biome_section() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        // The datafixer writes Y as an int, so the test must too.
        let bytes = test_chunk(vec![test_section(-4, "minecraft:stone", true)]).write();
        let chunk =
            ChunkData::from_bytes(&bytes, Vector2::new(0, 0)).expect("chunk without yPos parses");
        assert_eq!(
            chunk.section.get_block_absolute_y(0, -64, 0),
            Some(Block::STONE.default_state.id)
        );
    }

    #[test]
    fn chunk_with_int_y_sections_keeps_every_section() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        let bytes = test_chunk(vec![
            test_section(-4, "minecraft:stone", true),
            test_section(0, "minecraft:dirt", true),
        ])
        .write();
        let chunk = ChunkData::from_bytes(&bytes, Vector2::new(0, 0)).expect("int Y chunk parses");
        assert_eq!(
            chunk.section.get_block_absolute_y(0, -64, 0),
            Some(Block::STONE.default_state.id)
        );
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 0, 0),
            Some(Block::DIRT.default_state.id)
        );
    }

    #[test]
    fn chunk_with_mixed_numeric_y_tags_keeps_sections() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        // What a real upgraded chunk looks like: the map's own sections store Y
        // as a byte, the datafixer writes ints. The int section must still land
        // at its own height instead of collapsing to Y = 0.
        let mut byte_section = test_section(-4, "minecraft:air", true);
        byte_section.put_byte("Y", -4);

        let bytes = test_chunk(vec![
            byte_section,
            test_section(0, "minecraft:stone", true),
            test_section(4, "minecraft:dirt", true),
        ])
        .write();
        let chunk =
            ChunkData::from_bytes(&bytes, Vector2::new(0, 0)).expect("mixed Y chunk parses");
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 0, 0),
            Some(Block::STONE.default_state.id)
        );
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 64, 0),
            Some(Block::DIRT.default_state.id)
        );
    }

    #[test]
    fn fallback_ignores_light_only_sections_below_zero() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        // Old worlds keep a light grid at Y = -1 after the upgrade. It has no
        // biomes and must not decide where the chunk starts.
        let mut light_section = NbtCompound::new();
        light_section.put_byte("Y", -1);
        light_section.put(
            "BlockLight",
            NbtTag::ByteArray(vec![0x0Fi8; 2048].into_boxed_slice()),
        );

        let bytes = test_chunk(vec![
            light_section,
            test_section(0, "minecraft:stone", true),
        ])
        .write();
        let chunk =
            ChunkData::from_bytes(&bytes, Vector2::new(0, 0)).expect("light section ignored");
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 0, 0),
            Some(Block::STONE.default_state.id)
        );
        assert_eq!(chunk.section.get_block_absolute_y(0, -1, 0), None);
    }

    #[test]
    fn fallback_never_goes_above_zero() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        let bytes = test_chunk(vec![test_section(1, "minecraft:stone", true)]).write();
        let chunk = ChunkData::from_bytes(&bytes, Vector2::new(0, 0)).expect("caps at zero");
        // The only section sits at Y=1, so the cap has to bring the chunk down
        // to Y=0. Without the cap the chunk would start at Y=1 and a block at
        // Y=0 would be out of range.
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 0, 0),
            Some(Block::AIR.default_state.id)
        );
        assert_eq!(
            chunk.section.get_block_absolute_y(0, 16, 0),
            Some(Block::STONE.default_state.id)
        );
    }

    #[test]
    fn chunk_without_y_pos_or_biomes_still_fails() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        let mut light_section = NbtCompound::new();
        light_section.put_byte("Y", -1);
        light_section.put(
            "BlockLight",
            NbtTag::ByteArray(vec![0x0Fi8; 2048].into_boxed_slice()),
        );

        let bytes = test_chunk(vec![light_section]).write();
        let Err(error) = ChunkData::from_bytes(&bytes, Vector2::new(0, 0)) else {
            panic!("chunk without yPos and without biomes must fail");
        };
        assert!(format!("{error:?}").contains("Missing yPos"));
    }

    #[test]
    fn extract_u16_array_from_vanilla_compound_palette() {
        let mut entry1 = NbtCompound::new();
        entry1.put_string("Name", "minecraft:stone".to_string());

        let mut entry2 = NbtCompound::new();
        entry2.put_string("Name", "minecraft:repeater".to_string());
        let mut props = NbtCompound::new();
        props.put_string("facing", "north".to_string());
        props.put_string("delay", "2".to_string());
        props.put_string("locked", "false".to_string());
        props.put_string("powered", "false".to_string());
        entry2.put_compound("Properties", props);

        let list_tag = NbtTag::List(vec![NbtTag::Compound(entry1), NbtTag::Compound(entry2)]);
        let result = extract_u16_array(&list_tag).expect("should extract palette");

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], Block::STONE.default_state.id);

        let repeater_state = Block::REPEATER
            .from_properties(&[
                pumpkin_data::Property::Facing(pumpkin_data::block_properties::Facing::North),
                pumpkin_data::Property::Delay(2),
                pumpkin_data::Property::Locked(false),
                pumpkin_data::Property::Powered(false),
            ])
            .to_state_id(&Block::REPEATER);
        assert_eq!(result[1], repeater_state);
    }

    // 26.3's `BlockState.CODEC` saves a default state as the bare block id; a palette mixing
    // both forms is a list of compounds with the ids wrapped as `{"": id}`.
    #[test]
    fn palette_round_trips_26_3_block_states() {
        use crate::chunk::ChunkData;
        use pumpkin_util::math::vector2::Vector2;

        let repeater_properties: Vec<pumpkin_data::Property> = [
            ("facing", "east"),
            ("delay", "3"),
            ("locked", "false"),
            ("powered", "true"),
        ]
        .into_iter()
        .map(|(key, value)| {
            pumpkin_data::Property::from_key_value_str(
                pumpkin_data::PropertyKey::from_str(key).unwrap(),
                value,
            )
            .unwrap()
        })
        .collect();
        let repeater = Block::REPEATER
            .from_properties(&repeater_properties)
            .to_state_id(&Block::REPEATER);
        let mut repeater_nbt = NbtCompound::new();
        repeater_nbt.put_string("id", "minecraft:repeater".to_string());
        let mut props = NbtCompound::new();
        for (key, value) in [
            ("facing", "east"),
            ("delay", "3"),
            ("locked", "false"),
            ("powered", "true"),
        ] {
            props.put_string(key, value.to_string());
        }
        repeater_nbt.put_compound("properties", props);

        let mut block_states = NbtCompound::new();
        block_states.put(
            "palette",
            NbtTag::List(vec![
                NbtTag::String("minecraft:stone".into()),
                NbtTag::Compound(repeater_nbt),
            ]),
        );
        let mut data = vec![0; 256];
        data[0] = 1;
        block_states.put("data", NbtTag::LongArray(data));
        let mut section = test_section(-4, "minecraft:air", true);
        section.put("block_states", NbtTag::Compound(block_states));
        let mut nbt = test_chunk(vec![section]);
        nbt.root_tag.put_int("yPos", -4);

        let chunk = ChunkData::from_bytes(&nbt.write(), Vector2::new(0, 0)).expect("parses");
        assert_eq!(
            chunk.section.get_block_absolute_y(0, -64, 0),
            Some(repeater)
        );
        assert_eq!(
            chunk.section.get_block_absolute_y(1, -64, 0),
            Some(Block::STONE.default_state.id)
        );

        let saved = chunk.to_bytes().expect("serializes");
        let saved = pumpkin_nbt::Nbt::read(&mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(
            &mut std::io::Cursor::new(saved.as_ref()),
        ))
        .expect("saved chunk is NBT");
        let sections = saved.root_tag.get_list("sections").expect("sections");
        let NbtTag::Compound(section) = &sections[0] else {
            panic!("section is a compound");
        };
        let palette = section
            .get_compound("block_states")
            .and_then(|states| states.get_list("palette"))
            .expect("palette");
        let mut ids = Vec::new();
        for entry in palette {
            match entry {
                NbtTag::String(id) => ids.push((id.to_string(), None)),
                NbtTag::Compound(entry) => ids.push((
                    entry.get_string("id").expect("id").to_string(),
                    entry
                        .get_compound("properties")
                        .and_then(|props| props.get_string("delay"))
                        .map(str::to_string),
                )),
                _ => panic!("unexpected palette entry"),
            }
        }
        ids.sort();
        assert_eq!(
            ids,
            vec![
                ("minecraft:repeater".to_string(), Some("3".to_string())),
                ("minecraft:stone".to_string(), None),
            ]
        );
    }

    #[test]
    fn extract_u8_array_from_vanilla_string_palette() {
        let list_tag = NbtTag::List(vec![
            NbtTag::String("minecraft:plains".to_string().into()),
            NbtTag::String("minecraft:the_void".to_string().into()),
        ]);
        let result = extract_u8_array(&list_tag).expect("should extract biome palette");

        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0],
            pumpkin_data::biome::Biome::from_name("plains").unwrap().id
        );
        assert_eq!(
            result[1],
            pumpkin_data::biome::Biome::from_name("the_void")
                .unwrap()
                .id
        );
    }
}
