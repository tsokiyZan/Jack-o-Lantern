use pumpkin_data::damage::DamageType;
use pumpkin_macros::pumpkin_block;

use crate::block::{BlockBehaviour, FallOnArgs};

#[pumpkin_block("minecraft:hay_block")]
pub struct HayBlock;

impl BlockBehaviour for HayBlock {
    /// Vanilla `HayBlock.fallOn`: a fifth of the fall damage.
    fn fall_on(&self, args: FallOnArgs<'_>) {
        args.entity
            .cause_fall_damage(args.entity, args.fall_distance, 0.2, DamageType::FALL);
    }
}
