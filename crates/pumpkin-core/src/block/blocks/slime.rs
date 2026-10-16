use pumpkin_data::damage::DamageType;
use pumpkin_macros::pumpkin_block;

use crate::block::{BlockBehaviour, FallOnArgs};

#[pumpkin_block("minecraft:slime_block")]
pub struct SlimeBlock;

impl BlockBehaviour for SlimeBlock {
    /// Vanilla `SlimeBlock.fallOn`: no fall damage, unless the entity doesn't bounce.
    fn fall_on(&self, args: FallOnArgs<'_>) {
        if !args.entity.get_entity().is_sneaking() {
            args.entity
                .cause_fall_damage(args.entity, args.fall_distance, 0.0, DamageType::FALL);
        }
    }
}
