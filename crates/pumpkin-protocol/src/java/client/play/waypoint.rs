use std::io::Write;

use crate::ser::NetworkWriteExt;
use crate::{ClientPacket, VarInt, WritingError};
use pumpkin_data::packet::clientbound::play::WAYPOINT;
use pumpkin_macros::java_packet;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::version::JavaMinecraftVersion;
use uuid::Uuid;

/// Vanilla `ClientboundTrackedWaypointPacket.Operation`, written as its ordinal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum WaypointOperation {
    Track = 0,
    Untrack = 1,
    Update = 2,
}

/// Vanilla `TrackedWaypoint` and its `Type` (`EMPTY` 0, `VEC3I` 1, `CHUNK` 2, `AZIMUTH` 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WaypointTarget {
    Empty,
    Vec3i(BlockPos),
    Chunk { x: i32, z: i32 },
    Azimuth(f32),
}

impl WaypointTarget {
    #[must_use]
    pub const fn type_id(&self) -> i32 {
        match self {
            Self::Empty => 0,
            Self::Vec3i(_) => 1,
            Self::Chunk { .. } => 2,
            Self::Azimuth(_) => 3,
        }
    }
}

/// Vanilla `Waypoint.Icon` on the wire: the style's identifier, then an optional RGB colour
/// as three bytes (`ByteBufCodecs.RGB_COLOR`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaypointIcon {
    pub style: String,
    pub color: Option<i32>,
}

impl WaypointIcon {
    /// Vanilla `WaypointStyleAssets.DEFAULT`.
    pub const DEFAULT_STYLE: &'static str = "minecraft:default";
}

impl Default for WaypointIcon {
    /// Vanilla `Waypoint.Icon.NULL` / `new Waypoint.Icon()`.
    fn default() -> Self {
        Self {
            style: Self::DEFAULT_STYLE.to_string(),
            color: None,
        }
    }
}

/// Vanilla `TrackedWaypoint`; the server only ever sends the UUID side of the identifier.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackedWaypoint {
    pub identifier: Uuid,
    pub icon: WaypointIcon,
    pub target: WaypointTarget,
}

/// Syncs tracked waypoints (`ClientboundTrackedWaypointPacket`) to the client.
#[java_packet(WAYPOINT)]
pub struct CWaypoint {
    pub operation: WaypointOperation,
    pub waypoint: TrackedWaypoint,
}

impl CWaypoint {
    #[must_use]
    pub const fn new(operation: WaypointOperation, waypoint: TrackedWaypoint) -> Self {
        Self {
            operation,
            waypoint,
        }
    }

    const fn with(
        operation: WaypointOperation,
        identifier: Uuid,
        icon: WaypointIcon,
        target: WaypointTarget,
    ) -> Self {
        Self::new(
            operation,
            TrackedWaypoint {
                identifier,
                icon,
                target,
            },
        )
    }

    /// Vanilla `removeWaypoint`: `UNTRACK` of `TrackedWaypoint.empty`, which carries
    /// `Icon.NULL`.
    #[must_use]
    pub fn remove_waypoint(identifier: Uuid) -> Self {
        Self::with(
            WaypointOperation::Untrack,
            identifier,
            WaypointIcon::default(),
            WaypointTarget::Empty,
        )
    }

    #[must_use]
    pub const fn add_waypoint_position(
        identifier: Uuid,
        icon: WaypointIcon,
        position: BlockPos,
    ) -> Self {
        Self::with(
            WaypointOperation::Track,
            identifier,
            icon,
            WaypointTarget::Vec3i(position),
        )
    }

    #[must_use]
    pub const fn update_waypoint_position(
        identifier: Uuid,
        icon: WaypointIcon,
        position: BlockPos,
    ) -> Self {
        Self::with(
            WaypointOperation::Update,
            identifier,
            icon,
            WaypointTarget::Vec3i(position),
        )
    }

    #[must_use]
    pub const fn add_waypoint_chunk(identifier: Uuid, icon: WaypointIcon, x: i32, z: i32) -> Self {
        Self::with(
            WaypointOperation::Track,
            identifier,
            icon,
            WaypointTarget::Chunk { x, z },
        )
    }

    #[must_use]
    pub const fn update_waypoint_chunk(
        identifier: Uuid,
        icon: WaypointIcon,
        x: i32,
        z: i32,
    ) -> Self {
        Self::with(
            WaypointOperation::Update,
            identifier,
            icon,
            WaypointTarget::Chunk { x, z },
        )
    }

    #[must_use]
    pub const fn add_waypoint_azimuth(identifier: Uuid, icon: WaypointIcon, angle: f32) -> Self {
        Self::with(
            WaypointOperation::Track,
            identifier,
            icon,
            WaypointTarget::Azimuth(angle),
        )
    }

    #[must_use]
    pub const fn update_waypoint_azimuth(identifier: Uuid, icon: WaypointIcon, angle: f32) -> Self {
        Self::with(
            WaypointOperation::Update,
            identifier,
            icon,
            WaypointTarget::Azimuth(angle),
        )
    }
}

impl ClientPacket for CWaypoint {
    fn write_packet_data(
        &self,
        mut write: impl Write,
        _version: &JavaMinecraftVersion,
    ) -> Result<(), WritingError> {
        // `Operation.STREAM_CODEC`: the ordinal.
        write.write_var_int(&VarInt(self.operation as i32))?;

        // `TrackedWaypoint.write`: `writeEither(identifier, UUIDUtil.STREAM_CODEC, writeUtf)`,
        // a `true` for the left (UUID) side.
        write.write_bool(true)?;
        write.write_uuid(&self.waypoint.identifier)?;

        // `Waypoint.Icon.STREAM_CODEC`: the style key's identifier, then
        // `optional(RGB_COLOR)`.
        write.write_string(&self.waypoint.icon.style)?;
        match self.waypoint.icon.color {
            Some(color) => {
                write.write_bool(true)?;
                write.write_u8((color >> 16) as u8)?;
                write.write_u8((color >> 8) as u8)?;
                write.write_u8(color as u8)?;
            }
            None => write.write_bool(false)?,
        }

        // `writeEnum(type)`, then `writeContents`.
        write.write_var_int(&VarInt(self.waypoint.target.type_id()))?;
        match &self.waypoint.target {
            WaypointTarget::Empty => {}
            WaypointTarget::Vec3i(pos) => {
                write.write_var_int(&VarInt(pos.0.x))?;
                write.write_var_int(&VarInt(pos.0.y))?;
                write.write_var_int(&VarInt(pos.0.z))?;
            }
            WaypointTarget::Chunk { x, z } => {
                write.write_var_int(&VarInt(*x))?;
                write.write_var_int(&VarInt(*z))?;
            }
            WaypointTarget::Azimuth(angle) => {
                write.write_f32_be(*angle)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: Uuid = Uuid::from_u128(0x0011_2233_4455_6677_8899_aabb_ccdd_eeff);

    fn bytes(packet: &CWaypoint) -> Vec<u8> {
        let mut buf = Vec::new();
        packet
            .write_packet_data(&mut buf, &JavaMinecraftVersion::V_26_3)
            .unwrap();
        buf
    }

    fn uuid_bytes() -> Vec<u8> {
        ID.as_u128().to_be_bytes().to_vec()
    }

    /// Vanilla's reading side (`ClientboundTrackedWaypointPacket.STREAM_CODEC.decode`):
    /// operation, `readEither`, `Icon.STREAM_CODEC`, `readEnum(Type)`, the type's contents.
    fn decode(data: &[u8]) -> (i32, TrackedWaypoint) {
        fn var_int(data: &[u8], at: &mut usize) -> i32 {
            let mut value = 0i32;
            for shift in 0..5 {
                let byte = data[*at];
                *at += 1;
                value |= i32::from(byte & 0x7F) << (shift * 7);
                if byte & 0x80 == 0 {
                    break;
                }
            }
            value
        }
        let mut at = 0;
        let op = var_int(data, &mut at);
        assert_eq!(data[at], 1, "identifier is Either.left");
        at += 1;
        let identifier = Uuid::from_slice(&data[at..at + 16]).unwrap();
        at += 16;
        let len = var_int(data, &mut at) as usize;
        let style = String::from_utf8(data[at..at + len].to_vec()).unwrap();
        at += len;
        let color = if data[at] == 1 {
            at += 1;
            let rgb = (i32::from(data[at]) << 16)
                | (i32::from(data[at + 1]) << 8)
                | i32::from(data[at + 2]);
            at += 3;
            // `ARGB.color(r, g, b)`: alpha 255.
            Some(rgb | 0xFF00_0000u32 as i32)
        } else {
            at += 1;
            None
        };
        let target = match var_int(data, &mut at) {
            0 => WaypointTarget::Empty,
            1 => {
                let x = var_int(data, &mut at);
                let y = var_int(data, &mut at);
                let z = var_int(data, &mut at);
                WaypointTarget::Vec3i(BlockPos::new(x, y, z))
            }
            2 => {
                let x = var_int(data, &mut at);
                let z = var_int(data, &mut at);
                WaypointTarget::Chunk { x, z }
            }
            3 => {
                let angle = f32::from_be_bytes(data[at..at + 4].try_into().unwrap());
                at += 4;
                WaypointTarget::Azimuth(angle)
            }
            other => panic!("unknown type {other}"),
        };
        assert_eq!(at, data.len(), "trailing bytes");
        (
            op,
            TrackedWaypoint {
                identifier,
                icon: WaypointIcon { style, color },
                target,
            },
        )
    }

    #[test]
    fn untrack_is_empty_with_the_null_icon() {
        let data = bytes(&CWaypoint::remove_waypoint(ID));
        let mut expected = vec![1, 1];
        expected.extend(uuid_bytes());
        expected.push(17);
        expected.extend(b"minecraft:default");
        expected.extend([0, 0]);
        assert_eq!(data, expected);
        let (op, waypoint) = decode(&data);
        assert_eq!(op, 1);
        assert_eq!(waypoint.target, WaypointTarget::Empty);
        assert_eq!(waypoint.icon, WaypointIcon::default());
    }

    #[test]
    fn vec3i_track_writes_var_ints_and_rgb_bytes() {
        let icon = WaypointIcon {
            style: "minecraft:bowtie".to_string(),
            color: Some(0x00FF_5555),
        };
        let data = bytes(&CWaypoint::add_waypoint_position(
            ID,
            icon,
            BlockPos::new(-1, 300, 5),
        ));
        let mut expected = vec![0, 1];
        expected.extend(uuid_bytes());
        expected.push(16);
        expected.extend(b"minecraft:bowtie");
        expected.extend([1, 0xFF, 0x55, 0x55]);
        expected.push(1);
        expected.extend([0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
        expected.extend([0xAC, 0x02]);
        expected.push(5);
        assert_eq!(data, expected);
        let (op, waypoint) = decode(&data);
        assert_eq!(op, 0);
        assert_eq!(waypoint.identifier, ID);
        assert_eq!(waypoint.icon.color, Some(0xFFFF_5555u32 as i32));
        assert_eq!(
            waypoint.target,
            WaypointTarget::Vec3i(BlockPos::new(-1, 300, 5))
        );
    }

    #[test]
    fn chunk_and_azimuth_updates_round_trip() {
        let data = bytes(&CWaypoint::update_waypoint_chunk(
            ID,
            WaypointIcon::default(),
            -3,
            70,
        ));
        let (op, waypoint) = decode(&data);
        assert_eq!(op, 2);
        assert_eq!(waypoint.target, WaypointTarget::Chunk { x: -3, z: 70 });

        let data = bytes(&CWaypoint::add_waypoint_azimuth(
            ID,
            WaypointIcon::default(),
            -1.25,
        ));
        assert_eq!(&data[data.len() - 5..], &[3, 0xBF, 0xA0, 0, 0]);
        let (op, waypoint) = decode(&data);
        assert_eq!(op, 0);
        assert_eq!(waypoint.target, WaypointTarget::Azimuth(-1.25));
    }
}
