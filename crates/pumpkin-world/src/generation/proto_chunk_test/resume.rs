#![allow(clippy::print_stdout, clippy::needless_pass_by_value)]

use crate::chunk_system::{
    chunk_state::{Chunk, StagedChunkEnum},
    generation_cache::SurfaceBiomeNeighborhood,
};
use crate::generation::{
    biome_coords, generator::WorldGenerator, get_world_gen, proto_chunk::ProtoChunk,
};
use pumpkin_data::chunk::ChunkStatus;
use pumpkin_data::dimension::Dimension;
use pumpkin_util::world_seed::Seed;

use super::surface_biomes;

#[test]
fn terrain_biome_lookup_crosses_chunk_boundary() {
    use pumpkin_data::chunk::Biome;

    let seed = Seed(1_786_192_857_164_469_025);
    let world_gen = get_world_gen(seed, Dimension::OVERWORLD, false, Vec::new(), String::new());
    let WorldGenerator::Noise(generator) = &*world_gen else {
        unreachable!()
    };

    let mut north = ProtoChunk::new(84, 599, &world_gen);
    let mut south = ProtoChunk::new(84, 600, &world_gen);
    north.step_to_biomes(generator);
    south.step_to_biomes(generator);

    // Biome zoom selects absolute quart (338, 17, 2399) on both sides of z=9600.
    // Vanilla resolves that quart through the owning chunk. It must not wrap the quart
    // coordinate into the chunk whose surface is currently being generated.
    let expected = north.get_biome_id(338, 17, 2399);
    assert_eq!(expected, Biome::SAVANNA.id);
    assert_eq!(north.get_terrain_gen_biome_id(1354, 68, 9599), expected);

    let mut surface_biomes = SurfaceBiomeNeighborhood::new(south.x, south.z);
    for chunk_x in south.x - 1..=south.x + 1 {
        for chunk_z in south.z - 1..=south.z + 1 {
            let mut chunk = ProtoChunk::new(chunk_x, chunk_z, &world_gen);
            chunk.step_to_biomes(generator);
            if (chunk_x, chunk_z) == (north.x, north.z) {
                // Make stored authority observably differ from a fresh biome-source sample.
                let index = chunk.local_biome_pos_to_biome_index(
                    338i32.rem_euclid(4),
                    17 - biome_coords::from_block(chunk.bottom_y() as i32),
                    2399i32.rem_euclid(4),
                );
                chunk.flat_biome_map[index] = Biome::DESERT.id;
            }
            assert!(surface_biomes.push_chunk(&Chunk::Proto(Box::new(chunk))));
        }
    }
    assert_eq!(
        south.get_terrain_gen_biome_id_from_neighborhood(&surface_biomes, 1354, 68, 9600),
        Some(Biome::DESERT.id)
    );
}

#[test]
fn generation_cache_resolves_blended_biome_through_owning_chunk() {
    use crate::chunk_system::{Chunk, generation_cache::Cache};
    use crate::generation::proto_chunk::GenerationCache;
    use pumpkin_data::chunk::Biome;

    let seed = Seed(1_786_192_857_164_469_025);
    let world_gen = get_world_gen(seed, Dimension::OVERWORLD, false, Vec::new(), String::new());
    let WorldGenerator::Noise(generator) = &*world_gen else {
        unreachable!()
    };

    let mut cache = Cache::new(83, 599, 3);
    for chunk_x in 83..=85 {
        for chunk_z in 599..=601 {
            let mut chunk = ProtoChunk::new(chunk_x, chunk_z, &world_gen);
            chunk.step_to_biomes(generator);
            cache.chunks.push(Chunk::Proto(Box::new(chunk)));
        }
    }

    assert_eq!(
        cache.get_biome_for_terrain_gen(1354, 68, 9600).id,
        Biome::SAVANNA.id
    );
}

/// Verifies that references use the structure selected for the shared Nether start.
#[test]
fn nether_complex_references_use_selected_structure() {
    use crate::generation::structure::placement::get_structure_chunk_in_region;
    use pumpkin_data::structures::{StructureKeys, StructurePlacementType, StructureSet};

    let seed = Seed(0);
    let world_gen = get_world_gen(
        seed,
        Dimension::THE_NETHER,
        false,
        Vec::new(),
        String::new(),
    );
    let WorldGenerator::Noise(generator) = &*world_gen else {
        unreachable!()
    };
    let set = &StructureSet::NETHER_COMPLEXES;
    let StructurePlacementType::RandomSpread(spread) = &set.placement.placement_type else {
        unreachable!()
    };

    for region_x in 0..8 {
        for region_z in 0..8 {
            let (chunk_x, chunk_z) = get_structure_chunk_in_region(
                spread,
                seed.0 as i64,
                region_x,
                region_z,
                set.placement.salt,
            );
            let mut proto = ProtoChunk::new(chunk_x, chunk_z, &world_gen);
            proto.step_to_biomes(generator);
            proto.set_structure_starts(generator);

            if !proto.has_structure(StructureKeys::BastionRemnant) {
                continue;
            }

            proto.set_structure_references(generator);
            assert!(proto.has_structure(StructureKeys::BastionRemnant));
            assert!(!proto.has_structure(StructureKeys::Fortress));
            return;
        }
    }

    panic!("no bastion remnant start found in sampled nether complex regions");
}

/// Verifies that structure references survive a partial-generation round trip.
#[test]
fn structure_references_are_rebuilt_when_resuming_generation() {
    use crate::chunk_system::chunk_state::Chunk;
    use pumpkin_config::lighting::LightingEngineConfig;
    use pumpkin_data::structures::StructureKeys;

    let seed = Seed(1_782_124_772_053_846_960);
    let world_gen = get_world_gen(seed, Dimension::OVERWORLD, false, Vec::new(), String::new());
    let WorldGenerator::Noise(generator) = &*world_gen else {
        unreachable!()
    };

    // This chunk contains the far edge of a monument starting at (-553, 173).
    let mut proto = ProtoChunk::new(-553, 174, &world_gen);
    proto.step_to_biomes(generator);
    proto.set_structure_starts(generator);
    proto.set_structure_references(generator);
    assert!(proto.has_structure(StructureKeys::Monument));

    let mut staged = Chunk::Proto(Box::new(proto));
    staged.upgrade_to_level_chunk(&Dimension::OVERWORLD, &LightingEngineConfig::Default);
    let Chunk::Level(chunk_data) = staged else {
        unreachable!()
    };

    let resumed = ProtoChunk::from_chunk_data(&chunk_data, &world_gen);
    assert_eq!(resumed.stage, StagedChunkEnum::StructureReferences);
    assert!(resumed.has_structure(StructureKeys::Monument));
}

#[test]
fn block_entities_survive_chunk_data_resume() {
    use crate::chunk_system::chunk_state::Chunk;
    use pumpkin_config::lighting::LightingEngineConfig;
    use pumpkin_nbt::compound::NbtCompound;

    let seed = Seed(4242);
    let world_gen = get_world_gen(seed, Dimension::OVERWORLD, false, Vec::new(), String::new());

    let mut proto = ProtoChunk::new(3, -2, &world_gen);
    let mut chest = NbtCompound::new();
    chest.put_string("id", "minecraft:chest".to_string());
    chest.put_int("x", 49);
    chest.put_int("y", 64);
    chest.put_int("z", -31);
    proto.add_block_entity(chest);

    let mut staged = Chunk::Proto(Box::new(proto));
    staged.upgrade_to_level_chunk(&Dimension::OVERWORLD, &LightingEngineConfig::Default);
    let Chunk::Level(chunk_data) = staged else {
        unreachable!()
    };

    let resumed = ProtoChunk::from_chunk_data(&chunk_data, &world_gen);
    assert_eq!(resumed.pending_block_entities.len(), 1);
    assert_eq!(
        resumed.pending_block_entities[0].get_string("id"),
        Some("minecraft:chest")
    );

    let mut staged_again = Chunk::Proto(Box::new(resumed));
    staged_again.upgrade_to_level_chunk(&Dimension::OVERWORLD, &LightingEngineConfig::Default);
    let Chunk::Level(chunk_again) = staged_again else {
        unreachable!()
    };
    assert_eq!(chunk_again.pending_block_entities.lock().unwrap().len(), 1);
}

fn chunk_at_stage(
    world_gen: &WorldGenerator,
    cx: i32,
    cz: i32,
    stage: StagedChunkEnum,
) -> ProtoChunk {
    let WorldGenerator::Noise(generator) = world_gen else {
        unreachable!()
    };
    let mut proto = ProtoChunk::new(cx, cz, world_gen);
    proto.step_to_biomes(generator);
    proto.set_structure_starts(generator);
    proto.set_structure_references(generator);
    proto.step_to_noise(generator);
    if stage >= StagedChunkEnum::Surface {
        proto.step_to_surface(generator, &surface_biomes(world_gen, cx, cz));
    }
    if stage >= StagedChunkEnum::Carvers {
        proto.step_to_carvers(generator);
    }
    assert_eq!(proto.stage, stage);
    proto
}

fn save_and_load(proto: ProtoChunk, world_gen: &WorldGenerator) -> (ChunkStatus, ProtoChunk) {
    use pumpkin_config::lighting::LightingEngineConfig;
    let mut staged = Chunk::Proto(Box::new(proto));
    staged.upgrade_to_level_chunk(&Dimension::OVERWORLD, &LightingEngineConfig::Default);
    let Chunk::Level(chunk_data) = staged else {
        unreachable!()
    };
    (
        chunk_data.status,
        ProtoChunk::from_chunk_data(&chunk_data, world_gen),
    )
}

// Vanilla 26.3 builds noise, surface and carvers in one TERRAIN step, so "terrain" on
// disk always means carved terrain. A neighbour's features write into a chunk at this
// stage (write radius 1); resuming it at Noise rebuilt its surface over those blocks and
// turned tree leaves into grass and dirt.
#[test]
fn carved_chunk_resumes_at_carvers_and_keeps_neighbour_blocks() {
    use pumpkin_data::Block;
    use pumpkin_util::math::vector3::Vector3;

    let seed = Seed(1779920288596261407);
    let (cx, cz) = (67i32, 63i32);
    let world_gen = get_world_gen(seed, Dimension::OVERWORLD, false, Vec::new(), String::new());
    let mut proto = chunk_at_stage(&world_gen, cx, cz, StagedChunkEnum::Carvers);

    // A leaf placed on top of the terrain by a neighbouring chunk's tree.
    let (wx, wz) = (cx * 16 + 3, cz * 16 + 5);
    let leaf_y = proto.top_block_height_exclusive(3, 5);
    proto.set_block_state(wx, leaf_y, wz, Block::OAK_LEAVES.default_state);

    let mut expected = [[0i32; 16]; 16];
    for z in 0..16i32 {
        for x in 0..16i32 {
            expected[z as usize][x as usize] = proto.top_block_height_exclusive(x, z);
        }
    }
    let blocks_before: Vec<_> = (proto.bottom_y() as i32
        ..proto.bottom_y() as i32 + proto.height() as i32)
        .flat_map(|y| (0..16).flat_map(move |x| (0..16).map(move |z| (x, y, z))))
        .map(|(x, y, z)| proto.get_block_state(&Vector3::new(cx * 16 + x, y, cz * 16 + z)))
        .collect();

    let (status, resumed) = save_and_load(proto, &world_gen);
    assert_eq!(status, ChunkStatus::Terrain);
    assert_eq!(resumed.stage, StagedChunkEnum::Carvers);
    assert_eq!(
        resumed.get_block_state(&Vector3::new(wx, leaf_y, wz)),
        Block::OAK_LEAVES.default_state.id
    );
    let blocks_after: Vec<_> = (resumed.bottom_y() as i32
        ..resumed.bottom_y() as i32 + resumed.height() as i32)
        .flat_map(|y| (0..16).flat_map(move |x| (0..16).map(move |z| (x, y, z))))
        .map(|(x, y, z)| resumed.get_block_state(&Vector3::new(cx * 16 + x, y, cz * 16 + z)))
        .collect();
    assert!(blocks_before == blocks_after, "blocks changed by save/load");

    // Regression for transposed heightmaps on resume; flat terrain cannot expose it.
    for z in 0..16i32 {
        for x in 0..16i32 {
            assert_eq!(
                resumed.top_block_height_exclusive(x, z),
                expected[z as usize][x as usize],
                "heightmap corrupted by save/load roundtrip (transposed or lost)"
            );
        }
    }
}

// MergeTerrainChunkStatusFix: an intermediate terrain chunk is persisted as "biomes"
// without blocks and regenerates its terrain from scratch.
#[test]
fn intermediate_terrain_is_saved_as_biomes_and_regenerates() {
    use pumpkin_util::math::vector3::Vector3;

    let seed = Seed(1779920288596261407);
    let (cx, cz) = (67i32, 63i32);
    let world_gen = get_world_gen(seed, Dimension::OVERWORLD, false, Vec::new(), String::new());
    let WorldGenerator::Noise(generator) = &*world_gen else {
        unreachable!()
    };
    let fresh = chunk_at_stage(&world_gen, cx, cz, StagedChunkEnum::Carvers);

    for stage in [StagedChunkEnum::Noise, StagedChunkEnum::Surface] {
        let (status, mut resumed) =
            save_and_load(chunk_at_stage(&world_gen, cx, cz, stage), &world_gen);
        assert_eq!(status, ChunkStatus::Biomes);
        assert_eq!(resumed.stage, StagedChunkEnum::Biomes);
        let bottom = resumed.bottom_y() as i32;
        for y in bottom..bottom + resumed.height() as i32 {
            assert!(
                resumed
                    .get_block_state(&Vector3::new(cx * 16, y, cz * 16))
                    .to_state()
                    .is_air()
            );
        }

        resumed.set_structure_starts(generator);
        resumed.set_structure_references(generator);
        resumed.step_to_noise(generator);
        resumed.step_to_surface(generator, &surface_biomes(&world_gen, cx, cz));
        resumed.step_to_carvers(generator);
        for z in 0..16 {
            for x in 0..16 {
                for y in bottom..bottom + resumed.height() as i32 {
                    let pos = Vector3::new(cx * 16 + x, y, cz * 16 + z);
                    assert_eq!(resumed.get_block_state(&pos), fresh.get_block_state(&pos));
                }
            }
        }
    }
}
