use crate::{
    ServerPacket,
    ser::{NetworkReadExt, ReadingError},
};
use pumpkin_data::packet::serverbound::play::MOVE_PLAYER_ROT;
use pumpkin_macros::java_packet;
use pumpkin_util::version::JavaMinecraftVersion;

use super::player_position_rotation::{FLAG_HORIZONTAL_COLLISION, FLAG_ON_GROUND, pack_flags};

#[java_packet(MOVE_PLAYER_ROT)]
pub struct SPlayerRotation {
    pub yaw: f32,
    pub pitch: f32,
    pub ground: bool,
    pub horizontal_collision: bool,
}

impl<'a> ServerPacket<'a> for SPlayerRotation {
    fn read(bytebuf: &mut &'a [u8], _version: &JavaMinecraftVersion) -> Result<Self, ReadingError> {
        let yaw = bytebuf.get_f32_be()?;
        let pitch = bytebuf.get_f32_be()?;
        let flags = bytebuf.get_u8()?;
        Ok(Self {
            yaw,
            pitch,
            ground: flags & FLAG_ON_GROUND != 0,
            horizontal_collision: flags & FLAG_HORIZONTAL_COLLISION != 0,
        })
    }
}

impl crate::ClientPacket for SPlayerRotation {
    fn write_packet_data(
        &self,
        mut write: impl std::io::Write,
        _version: &JavaMinecraftVersion,
    ) -> Result<(), crate::ser::WritingError> {
        use crate::ser::NetworkWriteExt;
        write.write_f32_be(self.yaw)?;
        write.write_f32_be(self.pitch)?;
        write.write_u8(pack_flags(self.ground, self.horizontal_collision))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_flags_carry_the_horizontal_collision() -> Result<(), ReadingError> {
        // Vanilla `packFlags`: bit 0 on ground, bit 1 horizontal collision.
        let bytes = [0x43, 0x34, 0, 0, 0x41, 0x20, 0, 0, 0x02];
        let packet = SPlayerRotation::read(&mut &bytes[..], &JavaMinecraftVersion::V_1_21_4)?;
        assert!((packet.yaw - 180.0).abs() < f32::EPSILON);
        assert!(!packet.ground);
        assert!(packet.horizontal_collision);
        Ok(())
    }
}
