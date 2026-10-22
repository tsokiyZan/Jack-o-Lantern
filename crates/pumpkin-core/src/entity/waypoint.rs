//! Vanilla `Waypoint` (its `Icon`) and `WaypointTransmitter` (the three connection types and
//! the static range checks). Only living entities transmit waypoints.

use pumpkin_data::attributes::Attributes;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::java::client::play::{CWaypoint, WaypointIcon};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::text::color::NamedColor;

use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};

/// Vanilla `WaypointTransmitter.REALLY_FAR_DISTANCE`.
pub const REALLY_FAR_DISTANCE: f32 = 332.0;

/// Vanilla `Waypoint.WAYPOINT_TRANSMIT_RANGE_HIDE_MODIFIER`'s id.
pub const WAYPOINT_TRANSMIT_RANGE_HIDE_MODIFIER_ID: &str = "minecraft:waypoint_transmit_range_hide";

/// The azimuth an update must move by before it is sent (half a degree, as a float).
const AZIMUTH_UPDATE_THRESHOLD: f32 = 0.008_726_646;

/// Vanilla `Waypoint.Icon`: a style key and an optional RGB colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaypointIconState {
    pub style: String,
    pub color: Option<i32>,
}

impl Default for WaypointIconState {
    fn default() -> Self {
        Self {
            style: WaypointIcon::DEFAULT_STYLE.to_string(),
            color: None,
        }
    }
}

impl WaypointIconState {
    /// Vanilla `Icon.hasData`.
    #[must_use]
    pub fn has_data(&self) -> bool {
        self.style != WaypointIcon::DEFAULT_STYLE || self.color.is_some()
    }

    /// Vanilla `Icon.cloneAndAssignStyle`: the own colour, else the colour of the entity's team
    /// (`team_color`), with black lifted to `-13619152` so it shows on the bar.
    #[must_use]
    pub fn clone_and_assign_style(&self, team_color: Option<NamedColor>) -> Self {
        Self {
            style: self.style.clone(),
            color: self.color.or_else(|| {
                team_color.map(|color| {
                    if color == NamedColor::Black {
                        -13_619_152
                    } else {
                        named_color_rgb(color)
                    }
                })
            }),
        }
    }

    /// Vanilla `Icon.copyFrom`.
    pub fn copy_from(&mut self, other: &Self) {
        self.color = other.color;
        self.style.clone_from(&other.style);
    }

    #[must_use]
    pub fn to_packet_icon(&self) -> WaypointIcon {
        WaypointIcon {
            style: self.style.clone(),
            color: self.color,
        }
    }

    /// Vanilla `Icon.CODEC`: `style` (a `waypoint_style_asset` key), optional `color`.
    #[must_use]
    pub fn to_nbt(&self) -> NbtCompound {
        let mut nbt = NbtCompound::new();
        nbt.put_string("style", self.style.clone());
        if let Some(color) = self.color {
            nbt.put_int("color", color);
        }
        nbt
    }

    /// Vanilla `input.read("locator_bar_icon", Icon.CODEC)`: `None` when the data doesn't
    /// parse (the caller falls back to a new icon).
    #[must_use]
    pub fn from_nbt(nbt: &NbtCompound) -> Option<Self> {
        let style = nbt.get_string("style")?;
        let style = if style.contains(':') {
            style.to_string()
        } else {
            format!("minecraft:{style}")
        };
        let color = match nbt.get("color") {
            None => None,
            Some(_) => Some(nbt.get_int("color")?),
        };
        Some(Self { style, color })
    }
}

/// `TeamColor.rgb()` (its `TextColor` value).
#[must_use]
pub fn named_color_rgb(color: NamedColor) -> i32 {
    let rgb = color.to_rgb();
    (i32::from(rgb.red) << 16) | (i32::from(rgb.green) << 8) | i32::from(rgb.blue)
}

/// Vanilla `Entity.distanceTo`: the deltas as floats, then a float square root.
#[must_use]
pub fn distance_to(a: &Entity, b: &Entity) -> f32 {
    let pa = a.pos.load();
    let pb = b.pos.load();
    let xd = (pa.x - pb.x) as f32;
    let yd = (pa.y - pb.y) as f32;
    let zd = (pa.z - pb.z) as f32;
    (xd * xd + yd * yd + zd * zd).sqrt()
}

/// Vanilla `Entity.hasIndirectPassenger`: whether `entity` rides `vehicle`, directly or not.
fn has_indirect_passenger(vehicle: &Entity, entity: &Entity) -> bool {
    let mut ridden = entity.get_vehicle();
    while let Some(current) = ridden {
        if current.get_entity().entity_id == vehicle.entity_id {
            return true;
        }
        ridden = current.get_entity().get_vehicle();
    }
    false
}

/// Vanilla `WaypointTransmitter.doesSourceIgnoreReceiver`.
#[must_use]
pub fn does_source_ignore_receiver(source: &dyn EntityBase, receiver: &Player) -> bool {
    if receiver.is_spectator() {
        return false;
    }
    let source_entity = source.get_entity();
    if source.is_spectator() || has_indirect_passenger(source_entity, receiver.get_entity()) {
        return true;
    }
    let Some(living) = source.get_living_entity() else {
        return true;
    };
    let broadcast_range = living
        .get_attribute_value(&Attributes::WAYPOINT_TRANSMIT_RANGE)
        .min(
            receiver
                .living_entity
                .get_attribute_value(&Attributes::WAYPOINT_RECEIVE_RANGE),
        );
    f64::from(distance_to(source_entity, receiver.get_entity())) >= broadcast_range
}

/// Vanilla `WaypointTransmitter.isChunkVisible`: inside the receiver's chunk tracking view,
/// without the neighbour ring (`isInViewDistance`).
#[must_use]
pub fn is_chunk_visible(chunk: Vector2<i32>, receiver: &Player) -> bool {
    receiver
        .watched_section
        .load()
        .is_in_view_distance(chunk.x, chunk.y)
}

/// Vanilla `WaypointTransmitter.isReallyFar`.
#[must_use]
pub fn is_really_far(source: &Entity, receiver: &Player) -> bool {
    distance_to(source, receiver.get_entity()) > REALLY_FAR_DISTANCE
}

/// `(float)Mth.atan2(direction.z, direction.x)` of
/// `receiver.position().subtract(source.position()).rotateClockwise90()`.
fn azimuth(source: &Entity, receiver: &Player) -> f32 {
    let r = receiver.get_entity().pos.load();
    let s = source.pos.load();
    let (dx, dz) = (r.x - s.x, r.z - s.z);
    // `rotateClockwise90`: `(-z, y, x)`.
    let (rx, rz) = (-dz, dx);
    pumpkin_util::math::atan2(rz, rx) as f32
}

/// What a connection remembers between updates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConnectionKind {
    /// Vanilla `EntityAzimuthConnection`.
    Azimuth { last_angle: f32 },
    /// Vanilla `EntityBlockConnection`.
    Block { last: BlockPos },
    /// Vanilla `EntityChunkConnection`.
    Chunk { last: Vector2<i32> },
}

/// Vanilla `WaypointTransmitter.Connection` from one living source to one receiving player.
/// The source and the receiver are passed to every call instead of being held.
#[derive(Clone, Debug)]
pub struct WaypointConnection {
    pub icon: WaypointIcon,
    pub kind: ConnectionKind,
}

impl WaypointConnection {
    #[must_use]
    pub fn azimuth(source: &Entity, icon: WaypointIcon, receiver: &Player) -> Self {
        Self {
            icon,
            kind: ConnectionKind::Azimuth {
                last_angle: azimuth(source, receiver),
            },
        }
    }

    #[must_use]
    pub fn block(source: &Entity, icon: WaypointIcon) -> Self {
        Self {
            icon,
            kind: ConnectionKind::Block {
                last: source.block_pos.load(),
            },
        }
    }

    #[must_use]
    pub fn chunk(source: &Entity, icon: WaypointIcon) -> Self {
        Self {
            icon,
            kind: ConnectionKind::Chunk {
                last: source.chunk_pos.load(),
            },
        }
    }

    /// Vanilla `Connection.connect`: track the waypoint at the remembered place.
    pub fn connect(&self, source: &Entity, receiver: &Player) {
        let id = source.entity_uuid;
        let icon = self.icon.clone();
        let packet = match self.kind {
            ConnectionKind::Azimuth { last_angle } => {
                CWaypoint::add_waypoint_azimuth(id, icon, last_angle)
            }
            ConnectionKind::Block { last } => CWaypoint::add_waypoint_position(id, icon, last),
            ConnectionKind::Chunk { last } => {
                CWaypoint::add_waypoint_chunk(id, icon, last.x, last.y)
            }
        };
        receiver.try_send_client_packet(&packet);
    }

    /// Vanilla `Connection.disconnect`.
    pub fn disconnect(&self, source: &Entity, receiver: &Player) {
        receiver.try_send_client_packet(&CWaypoint::remove_waypoint(source.entity_uuid));
    }

    /// Vanilla `Connection.update`: send the new place once it differs from the last one sent.
    pub fn update(&mut self, source: &Entity, receiver: &Player) {
        let id = source.entity_uuid;
        match &mut self.kind {
            ConnectionKind::Azimuth { last_angle } => {
                let current = azimuth(source, receiver);
                if (current - *last_angle).abs() > AZIMUTH_UPDATE_THRESHOLD {
                    receiver.try_send_client_packet(&CWaypoint::update_waypoint_azimuth(
                        id,
                        self.icon.clone(),
                        current,
                    ));
                    *last_angle = current;
                }
            }
            ConnectionKind::Block { last } => {
                let current = source.block_pos.load();
                if manhattan(current, *last) > 0 {
                    receiver.try_send_client_packet(&CWaypoint::update_waypoint_position(
                        id,
                        self.icon.clone(),
                        current,
                    ));
                    *last = current;
                }
            }
            ConnectionKind::Chunk { last } => {
                let current = source.chunk_pos.load();
                if chessboard(current, *last) > 0 {
                    receiver.try_send_client_packet(&CWaypoint::update_waypoint_chunk(
                        id,
                        self.icon.clone(),
                        current.x,
                        current.y,
                    ));
                    *last = current;
                }
            }
        }
    }

    /// Vanilla `Connection.isBroken`, per connection type.
    #[must_use]
    pub fn is_broken(&self, source: &dyn EntityBase, receiver: &Player) -> bool {
        let source_entity = source.get_entity();
        match self.kind {
            ConnectionKind::Azimuth { .. } => {
                does_source_ignore_receiver(source, receiver)
                    || is_chunk_visible(source_entity.chunk_pos.load(), receiver)
                    || !is_really_far(source_entity, receiver)
            }
            ConnectionKind::Block { last } => {
                // `BlockConnection.isBroken` (`distanceManhattan() > 1`), then the ignore check.
                manhattan(last, source_entity.block_pos.load()) > 1
                    || does_source_ignore_receiver(source, receiver)
            }
            ConnectionKind::Chunk { last } => {
                // `!ChunkConnection.isBroken && !doesSourceIgnoreReceiver ? isChunkVisible(last)
                // : true`.
                if chessboard(last, source_entity.chunk_pos.load()) <= 1
                    && !does_source_ignore_receiver(source, receiver)
                {
                    is_chunk_visible(last, receiver)
                } else {
                    true
                }
            }
        }
    }
}

/// `Vec3i.distManhattan`.
const fn manhattan(a: BlockPos, b: BlockPos) -> i32 {
    (a.0.x - b.0.x).abs() + (a.0.y - b.0.y).abs() + (a.0.z - b.0.z).abs()
}

/// `ChunkPos.getChessboardDistance`.
fn chessboard(a: Vector2<i32>, b: Vector2<i32>) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

/// Vanilla `LivingEntity.isTransmittingWaypoint`.
#[must_use]
pub fn is_transmitting_waypoint(source: &dyn EntityBase) -> bool {
    source.get_living_entity().is_some_and(|living| {
        living.get_attribute_value(&Attributes::WAYPOINT_TRANSMIT_RANGE) > 0.0
    })
}

/// Vanilla `ServerPlayer.isReceivingWaypoints`.
#[must_use]
pub fn is_receiving_waypoints(player: &Player) -> bool {
    player
        .living_entity
        .get_attribute_value(&Attributes::WAYPOINT_RECEIVE_RANGE)
        > 0.0
}

/// Vanilla `LivingEntity.makeWaypointConnectionWith`. `team_color` gives the source's team
/// colour; it is only asked for once a connection is made.
pub fn make_waypoint_connection_with(
    source: &dyn EntityBase,
    player: &Player,
    team_color: impl FnOnce() -> Option<NamedColor>,
) -> Option<WaypointConnection> {
    let source_entity = source.get_entity();
    let living = source.get_living_entity()?;
    if source_entity
        .first_tick
        .load(std::sync::atomic::Ordering::Relaxed)
        || source_entity.entity_id == player.get_entity().entity_id
    {
        return None;
    }
    if does_source_ignore_receiver(source, player) {
        return None;
    }
    let icon = living
        .locator_bar_icon
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone_and_assign_style(team_color())
        .to_packet_icon();
    Some(if is_really_far(source_entity, player) {
        WaypointConnection::azimuth(source_entity, icon, player)
    } else if !is_chunk_visible(source_entity.chunk_pos.load(), player) {
        WaypointConnection::chunk(source_entity, icon)
    } else {
        WaypointConnection::block(source_entity, icon)
    })
}

/// The team colour `cloneAndAssignStyle` reads (`getTeam().getColor()`).
#[must_use]
pub fn team_color_of(source: &dyn EntityBase) -> Option<NamedColor> {
    source.get_team().and_then(|team| team.color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_data_only_off_the_defaults() {
        let mut icon = WaypointIconState::default();
        assert!(!icon.has_data());
        icon.color = Some(0x55FF55);
        assert!(icon.has_data());
        icon.color = None;
        icon.style = "minecraft:bowtie".to_string();
        assert!(icon.has_data());
    }

    #[test]
    fn team_colour_fills_a_missing_colour_and_black_is_lifted() {
        let icon = WaypointIconState::default();
        assert_eq!(
            icon.clone_and_assign_style(Some(NamedColor::Red)).color,
            Some(0xFF5555)
        );
        assert_eq!(
            icon.clone_and_assign_style(Some(NamedColor::Black)).color,
            Some(-13_619_152)
        );
        assert_eq!(icon.clone_and_assign_style(None).color, None);
        let own = WaypointIconState {
            color: Some(0x123456),
            ..WaypointIconState::default()
        };
        assert_eq!(
            own.clone_and_assign_style(Some(NamedColor::Red)).color,
            Some(0x123456)
        );
    }

    #[test]
    fn icon_nbt_round_trips() {
        let icon = WaypointIconState {
            style: "minecraft:bowtie".to_string(),
            color: Some(0x00AA00),
        };
        assert_eq!(WaypointIconState::from_nbt(&icon.to_nbt()), Some(icon));
        assert_eq!(WaypointIconState::from_nbt(&NbtCompound::new()), None);
    }
}
