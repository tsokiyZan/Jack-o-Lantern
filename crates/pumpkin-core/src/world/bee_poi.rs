use pumpkin_data::Block;
use pumpkin_data::tag::PointOfInterestType;
use pumpkin_util::math::position::BlockPos;

use super::portal::PortalPoiStorage;

/// Vanilla `PoiTypes.BEEHIVE` and `PoiTypes.BEE_NEST`, which hold no tickets.
#[must_use]
pub fn poi_type_for_block(block: &Block) -> Option<&'static str> {
    if block == &Block::BEEHIVE {
        Some("minecraft:beehive")
    } else if block == &Block::BEE_NEST {
        Some("minecraft:bee_nest")
    } else {
        None
    }
}

fn is_bee_home(poi_type: &str) -> bool {
    let name = poi_type.strip_prefix("minecraft:").unwrap_or(poi_type);
    PointOfInterestType::MINECRAFT_BEE_HOME.0.contains(&name)
}

/// Vanilla `PoiManager.getInRange` for `PoiTypeTags.BEE_HOME` with any occupancy: every bee
/// home in the square around `center`, kept when within `radius` blocks of it.
pub fn bee_homes_in_range(
    storage: &mut PortalPoiStorage,
    center: BlockPos,
    radius: i32,
) -> Vec<BlockPos> {
    let radius_sq = i64::from(radius) * i64::from(radius);
    storage
        .get_in_square_matching(center, radius, is_bee_home)
        .into_iter()
        .filter(|pos| i64::from(pos.squared_distance(&center)) <= radius_sq)
        .collect()
}
