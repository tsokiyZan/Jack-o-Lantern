use pumpkin_macros::pumpkin_block;

use pumpkin_data::block_sound_type::SoundType;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::sound::Sound;

use crate::block::{BlockBehaviour, FallOnArgs, PathComputationType};

#[pumpkin_block("minecraft:honey_block")]
pub struct HoneyBlock;

impl BlockBehaviour for HoneyBlock {
    /// Vanilla `HoneyBlock.fallOn`: a slide sound and jump particles, and a fifth of the
    /// fall damage.
    fn fall_on(&self, args: FallOnArgs<'_>) {
        args.entity
            .play_sound_at_self(Sound::BlockHoneyBlockSlide, 1.0, 1.0);
        args.world
            .send_entity_status(args.entity.get_entity(), EntityStatus::HoneyJump, None);
        if args
            .entity
            .cause_fall_damage(args.entity, args.fall_distance, 0.2, DamageType::FALL)
        {
            let sound_type = SoundType::HONEY_BLOCK;
            args.entity.play_sound_at_self(
                sound_type.fall_sound,
                sound_type.volume * 0.5,
                sound_type.pitch * 0.75,
            );
        }
    }

    fn is_pathfindable(
        &self,
        _state: &pumpkin_data::BlockState,
        _computation_type: PathComputationType,
    ) -> bool {
        false
    }
}
