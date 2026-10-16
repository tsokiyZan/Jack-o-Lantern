use crate::ClientPacket;
use crate::VarInt;
use crate::ser::NetworkWriteExt;
use pumpkin_data::data_component_impl::SwingAnimationImpl;
use pumpkin_data::packet::clientbound::play::{ANIMATE, SWING_ANIMATION};
use pumpkin_macros::java_packet;
use pumpkin_util::version::JavaMinecraftVersion;

/// Triggers a specific animation for an entity that is visible to the client.
///
/// This is primarily used for player-driven animations like swinging an arm
/// or showing damage, but it can apply to other entities as well.
#[java_packet(ANIMATE)]
pub struct CEntityAnimation {
    /// The Entity ID of the entity performing the animation.
    pub entity_id: VarInt,
    /// The ID of the animation to play.
    /// See the table below for standard values.
    pub animation: u8,
}

impl CEntityAnimation {
    #[must_use]
    pub const fn new(entity_id: VarInt, animation: Animation) -> Self {
        Self {
            entity_id,
            animation: animation as u8,
        }
    }
}

impl ClientPacket for CEntityAnimation {
    fn write_packet_data(
        &self,
        mut write: impl std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), crate::ser::WritingError> {
        write.write_var_int(&self.entity_id)?;
        // 26.3 moved the swings into their own packet and renumbered the remaining animations
        let animation = if *version >= JavaMinecraftVersion::V_26_3 {
            match self.animation {
                2 => 0, // Leave bed
                4 => 1, // Critical effect
                5 => 2, // Magic critical effect
                other => other,
            }
        } else {
            self.animation
        };
        write.write_u8(animation)?;
        Ok(())
    }
}

/// Plays the swing animation of an entity's hand.
///
/// Up to 26.2 this was an animation of [`CEntityAnimation`], since 26.3 it is its own packet that
/// also carries the swing animation of the held item.
pub struct CSwingArm {
    pub entity_id: VarInt,
    pub off_hand: bool,
    pub animation: SwingAnimationImpl,
}

impl CSwingArm {
    /// A swing with the default animation, whacking for 6 ticks.
    #[must_use]
    pub const fn new(entity_id: VarInt, off_hand: bool) -> Self {
        Self::with_animation(entity_id, off_hand, SwingAnimationImpl::DEFAULT)
    }

    #[must_use]
    pub const fn with_animation(
        entity_id: VarInt,
        off_hand: bool,
        animation: SwingAnimationImpl,
    ) -> Self {
        Self {
            entity_id,
            off_hand,
            animation,
        }
    }
}

impl crate::packet::MultiVersionJavaPacket for CSwingArm {
    fn to_id(version: JavaMinecraftVersion) -> i32 {
        if version >= JavaMinecraftVersion::V_26_3 {
            SWING_ANIMATION.to_id(version)
        } else {
            ANIMATE.to_id(version)
        }
    }
}

impl ClientPacket for CSwingArm {
    fn write_packet_data(
        &self,
        mut write: impl std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), crate::ser::WritingError> {
        write.write_var_int(&self.entity_id)?;
        if *version >= JavaMinecraftVersion::V_26_3 {
            write.write_var_int(&VarInt(i32::from(self.off_hand)))?;
            write.write_var_int(&VarInt(self.animation.animation_type.to_id()))?;
            write.write_var_int(&VarInt(self.animation.duration))?;
        } else if self.off_hand {
            write.write_u8(Animation::SwingOffhand as u8)?;
        } else {
            write.write_u8(Animation::SwingMainArm as u8)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum Animation {
    SwingMainArm,
    LeaveBed = 2,
    SwingOffhand,
    CriticalEffect,
    MagicCriticaleffect,
}
