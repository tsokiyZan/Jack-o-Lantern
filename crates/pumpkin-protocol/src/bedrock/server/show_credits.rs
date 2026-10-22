// Last verified for v2169

use crate::{
    codec::{var_int::VarInt, var_ulong::VarULong},
    serial::PacketRead,
};
use pumpkin_macros::packet;

/// The client's side of the end-game credits: state 1 when the player has finished (or
/// skipped) them.
#[derive(PacketRead)]
#[packet(75)]
pub struct SShowCredits {
    /// Runtime entity identifier of the player viewing the credits.
    pub player_runtime_id: VarULong,
    /// 0: credits started, 1: credits finished.
    pub credits_state: VarInt,
}

impl SShowCredits {
    pub const STATE_END: i32 = 1;
}
