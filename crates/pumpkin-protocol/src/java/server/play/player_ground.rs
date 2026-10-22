use crate::{
    ServerPacket,
    ser::{NetworkReadExt, ReadingError},
};
use pumpkin_data::packet::serverbound::play::MOVE_PLAYER_STATUS_ONLY;
use pumpkin_macros::java_packet;
use pumpkin_util::version::JavaMinecraftVersion;

use super::player_position_rotation::{FLAG_HORIZONTAL_COLLISION, FLAG_ON_GROUND, pack_flags};

#[java_packet(MOVE_PLAYER_STATUS_ONLY)]
pub struct SSetPlayerGround {
    pub on_ground: bool,
    pub horizontal_collision: bool,
}

impl<'a> ServerPacket<'a> for SSetPlayerGround {
    fn read(bytebuf: &mut &'a [u8], _version: &JavaMinecraftVersion) -> Result<Self, ReadingError> {
        let flags = bytebuf.get_u8()?;
        Ok(Self {
            on_ground: flags & FLAG_ON_GROUND != 0,
            horizontal_collision: flags & FLAG_HORIZONTAL_COLLISION != 0,
        })
    }
}

impl crate::ClientPacket for SSetPlayerGround {
    fn write_packet_data(
        &self,
        mut write: impl std::io::Write,
        _version: &JavaMinecraftVersion,
    ) -> Result<(), crate::ser::WritingError> {
        use crate::ser::NetworkWriteExt;
        write.write_u8(pack_flags(self.on_ground, self.horizontal_collision))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_flags_carry_the_horizontal_collision() -> Result<(), ReadingError> {
        let packet = SSetPlayerGround::read(&mut &[0x03][..], &JavaMinecraftVersion::V_1_21_4)?;
        assert!(packet.on_ground);
        assert!(packet.horizontal_collision);
        Ok(())
    }
}
