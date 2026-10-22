use crate::entity::player::Player;
use dashmap::DashMap;
use pumpkin_data::dimension::Dimension;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use std::sync::{Arc, Mutex};

pub struct MapManager {
    pub maps: DashMap<i32, Arc<Mutex<MapData>>>,
}

impl Default for MapManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MapManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            maps: DashMap::new(),
        }
    }

    #[must_use]
    pub fn get_map(&self, id: i32) -> Option<Arc<Mutex<MapData>>> {
        self.maps.get(&id).map(|m| m.clone())
    }

    #[must_use]
    pub fn create_map(
        &self,
        id: i32,
        dimension: Dimension,
        x: i32,
        z: i32,
        scale: i8,
    ) -> Arc<Mutex<MapData>> {
        let map = Arc::new(Mutex::new(MapData::new(dimension, x, z, scale)));
        self.maps.insert(id, map.clone());
        map
    }
}

pub struct MapData {
    pub scale: i8,
    pub locked: bool,
    pub dimension: Dimension,
    pub center_x: i32,
    pub center_z: i32,
    pub colors: Box<[u8; 128 * 128]>,
    pub decorations: Vec<MapDecoration>,
    pub dirty: bool,
    pub fully_updated: bool,
}

impl MapData {
    #[must_use]
    pub fn new(dimension: Dimension, x: i32, z: i32, scale: i8) -> Self {
        Self {
            scale,
            locked: false,
            dimension,
            center_x: x,
            center_z: z,
            colors: Box::new([0; 128 * 128]),
            decorations: Vec::new(),
            dirty: true,
            fully_updated: false,
        }
    }

    /// Vanilla `MapItemSavedData.addDecoration` for a decoration the map item carries in its
    /// `map_decorations` (`tickCarriedBy`): where it lands on this map, if it does.
    #[must_use]
    pub fn item_decoration(
        &self,
        entry: &pumpkin_data::data_component_impl::MapDecorationEntry,
        game_time: i64,
    ) -> Option<MapDecoration> {
        let decoration_type = pumpkin_data::map_decoration::MapDecorationType::from_name(
            entry
                .r#type
                .strip_prefix("minecraft:")
                .unwrap_or(&entry.r#type),
        )?;
        let scaling = (1 << self.scale) as f32;
        let x_delta = (entry.x - f64::from(self.center_x)) as f32 / scaling;
        let z_delta = (entry.z - f64::from(self.center_z)) as f32 / scaling;
        // `calculateDecorationLocationAndType`, for a type other than the player.
        if !is_inside_map(x_delta, z_delta) {
            return None;
        }
        Some(MapDecoration {
            icon_type: decoration_type.id as i32,
            x: clamp_map_coordinate(x_delta),
            z: clamp_map_coordinate(z_delta),
            direction: self.calculate_rotation(f64::from(entry.rotation), game_time),
            display_name: None,
        })
    }

    /// Vanilla `MapItemSavedData.calculateRotation`.
    fn calculate_rotation(&self, y_rot: f64, game_time: i64) -> i8 {
        if self.dimension == Dimension::THE_NETHER {
            let s = (game_time / 10) as i32;
            return ((s.wrapping_mul(s).wrapping_mul(34_187_121) + s.wrapping_mul(121)) >> 15 & 15)
                as i8;
        }
        let adjusted = if y_rot < 0.0 {
            y_rot - 8.0
        } else {
            y_rot + 8.0
        };
        (adjusted * 16.0 / 360.0) as i8
    }

    /// Vanilla `MapItemSavedData.isTrackedCountOverLimit`: more than `limit` decorations whose
    /// type counts towards the limit (`trackCount`).
    #[must_use]
    pub fn is_tracked_count_over_limit(&self, limit: usize) -> bool {
        self.decorations
            .iter()
            .filter(|decoration| {
                u32::try_from(decoration.icon_type)
                    .ok()
                    .and_then(pumpkin_data::map_decoration::MapDecorationType::from_id)
                    .is_some_and(|decoration_type| decoration_type.track_count)
            })
            .count()
            > limit
    }

    pub fn set_color(&mut self, x: usize, z: usize, color: u8) {
        if x < 128 && z < 128 {
            let idx = z * 128 + x;
            if self.colors[idx] != color {
                self.colors[idx] = color;
                self.dirty = true;
            }
        }
    }

    pub fn update(&mut self, player: &Player) {
        let world = player.world();
        let scale = 1 << self.scale;
        let center_x = self.center_x;
        let center_z = self.center_z;

        let player_pos = player.position();
        let player_x = player_pos.x as i32;
        let player_z = player_pos.z as i32;

        let start_img_x = ((player_x - center_x) / scale + 64).clamp(0, 127) as usize;
        let start_img_z = ((player_z - center_z) / scale + 64).clamp(0, 127) as usize;

        let radius = 16;
        let (range_x, range_z) = if self.fully_updated {
            (
                (start_img_x.saturating_sub(radius))..(start_img_x + radius).min(128),
                (start_img_z.saturating_sub(radius))..(start_img_z + radius).min(128),
            )
        } else {
            self.fully_updated = true;
            (0..128, 0..128)
        };

        for img_x in range_x {
            let mut prev_y = -1;
            for img_z in range_z.clone() {
                let world_x = (img_x as i32 - 64) * scale + center_x;
                let world_z = (img_z as i32 - 64) * scale + center_z;

                let top_y = world.get_top_block(Vector2::new(world_x, world_z));
                let block = world.get_block(&BlockPos::new(world_x, top_y, world_z));

                let color_base = block.map_color;

                let mut brightness = 2; // Normal
                if prev_y != -1 {
                    if top_y > prev_y {
                        brightness = 3; // High
                    } else if top_y < prev_y {
                        brightness = 1; // Low
                    }
                }
                prev_y = top_y;

                let color = color_base * 4 + brightness;
                self.set_color(img_x, img_z, color);
            }
        }
    }
}

pub struct MapDecoration {
    pub icon_type: i32,
    pub x: i8,
    pub z: i8,
    pub direction: i8,
    pub display_name: Option<String>,
}

/// Vanilla `MapItemSavedData.isInsideMap`.
fn is_inside_map(x_delta: f32, z_delta: f32) -> bool {
    (-63.0..=63.0).contains(&x_delta) && (-63.0..=63.0).contains(&z_delta)
}

/// Vanilla `MapItemSavedData.clampMapCoordinate`.
fn clamp_map_coordinate(delta: f32) -> i8 {
    if delta <= -63.0 {
        -128
    } else if delta >= 63.0 {
        127
    } else {
        (f64::from(delta * 2.0) + 0.5) as i8
    }
}
