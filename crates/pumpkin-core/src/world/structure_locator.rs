use pumpkin_data::structures::{StructureKeys, StructurePlacementType, StructureSet};
use pumpkin_data::tag::Tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::generation::generator::structure_finder::{
    StructureSearch, find_nearest_structure, find_nearest_structure_start,
};

use super::World;

/// Index into [`StructureSet::ALL`] of the set a structure belongs to.
///
/// That set's placement decides where it may generate. Vanilla keeps this as a prepared
/// reverse map (`ChunkGeneratorStructureState.placementsForStructure`); there
/// are ~21 sets, so a scan is cheap enough. An index rather than a reference
/// because the sets are consts, whose addresses are not stable.
#[must_use]
pub fn structure_set_containing(key: StructureKeys) -> Option<usize> {
    StructureSet::ALL
        .iter()
        .position(|set| set.structures.iter().any(|entry| entry.structure == key))
}

/// Unfloored horizontal distance, for picking the nearest of several hits
/// without two candidates collapsing onto the same integer.
#[must_use]
pub fn horizontal_distance_exact(origin: &BlockPos, target: &BlockPos) -> f64 {
    let dx = f64::from(target.0.x - origin.0.x);
    let dz = f64::from(target.0.z - origin.0.z);
    dx.hypot(dz)
}

/// The structures in a `worldgen/structure` tag.
#[must_use]
pub fn structures_in_tag(tag: &Tag) -> Vec<StructureKeys> {
    tag.0
        .iter()
        .filter_map(|name| StructureKeys::from_name(name))
        .collect()
}

impl World {
    /// Vanilla `ChunkGenerator.findNearestMapStructure`: the nearest of `wanted`
    /// within `search_radius` chunk-region rings of `origin`, and which one it is.
    #[must_use]
    pub fn find_nearest_map_structure(
        &self,
        wanted: &[StructureKeys],
        origin: BlockPos,
        search_radius: i32,
    ) -> Option<(BlockPos, StructureKeys)> {
        // A structure is located through the placement of the set that holds
        // it, but only that structure counts as a hit: `minecraft:fortress`
        // must not report a bastion just because both share
        // `minecraft:nether_complexes`. Mirrors the `placementScans` map in
        // vanilla's `ChunkGenerator.findNearestMapStructure`.
        let mut scans: Vec<(usize, Vec<StructureKeys>)> = Vec::new();
        for &key in wanted {
            let Some(set_index) = structure_set_containing(key) else {
                continue;
            };
            if let Some((_, keys)) = scans.iter_mut().find(|(index, _)| *index == set_index) {
                keys.push(key);
            } else {
                scans.push((set_index, vec![key]));
            }
        }

        let seed = self.level.seed.0;
        let world_gen = self.level.world_gen.load_full();

        let mut found: Option<(BlockPos, StructureKeys)> = None;
        for (set_index, keys) in scans {
            let set = &StructureSet::ALL[set_index];
            let nearest = match &set.placement.placement_type {
                // Strongholds come out of the pre-computed ring cache, which
                // already holds positions they really occupy. A concentric-ring
                // set holds exactly one structure, so the hit is unambiguous.
                StructurePlacementType::ConcentricRings(_) => world_gen
                    .global_structure_cache()
                    .and_then(|global_cache| {
                        let search = || {
                            find_nearest_structure(
                                origin,
                                &[&set.placement],
                                search_radius,
                                seed as i64,
                                global_cache,
                            )
                        };
                        // Vanilla `getRingPositionsFor` joins the ring future: wait for the
                        // background search.
                        let result = match search() {
                            StructureSearch::Pending => {
                                global_cache.get_stronghold_chunks();
                                search()
                            }
                            result => result,
                        };
                        match result {
                            StructureSearch::Found(pos) => Some(pos),
                            StructureSearch::NotFound | StructureSearch::Pending => None,
                        }
                    })
                    .map(|pos| (pos, keys[0])),
                // Everything else is spread over a grid whose candidate chunks
                // are only *possible* sites: the biome at a candidate can still
                // reject every structure in the set. Resolving the start makes
                // sure the reported position actually holds one, and says which.
                StructurePlacementType::RandomSpread(_) => {
                    find_nearest_structure_start(origin, set, &keys, search_radius, &world_gen)
                }
            };

            if let Some((pos, key)) = nearest
                && found.as_ref().is_none_or(|(best, _)| {
                    horizontal_distance_exact(&origin, &pos)
                        < horizontal_distance_exact(&origin, best)
                })
            {
                found = Some((pos, key));
            }
        }
        found
    }
}
