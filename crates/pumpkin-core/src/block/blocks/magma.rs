use pumpkin_data::{damage::DamageType, effect::StatusEffect};
use pumpkin_macros::pumpkin_block;

use crate::block::{BlockBehaviour, OnEntityStepArgs};

#[pumpkin_block("minecraft:magma_block")]
pub struct MagmaBlock;

impl BlockBehaviour for MagmaBlock {
    fn on_entity_step(&self, args: OnEntityStepArgs<'_>) {
        {
            // Only living entities take damage
            let Some(living_entity) = args.entity.get_living_entity() else {
                return;
            };

            let ent = args.entity.get_entity();

            if args.entity.is_stepping_carefully() {
                return;
            }

            // Fire immune entities don't take damage
            if ent.is_fire_immune() {
                return;
            }

            if living_entity
                .get_effect(&StatusEffect::FIRE_RESISTANCE)
                .is_some()
            {
                return;
            }

            // Apply damage
            args.entity.damage(args.entity, 1.0, DamageType::HOT_FLOOR);
        }
    }
}
