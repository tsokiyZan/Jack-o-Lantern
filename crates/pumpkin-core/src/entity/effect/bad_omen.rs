use pumpkin_data::effect::StatusEffect;
use pumpkin_data::potion::Effect;
use pumpkin_util::Difficulty;

use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;
use crate::world::raid::{Raid, Raids};

/// Vanilla `BadOmenMobEffect`.
pub struct BadOmenMobEffect;

/// Vanilla `ServerLevel.getRaidAt`'s distance: `Raid.VALID_RAID_RADIUS_SQR`.
const RAID_AT_DISTANCE_SQR: f64 = 9216.0;

/// The raid check of `applyEffectTick`: whether the bad omen turns into a raid omen, which it
/// does unless the raid at the position (`getRaidAt`) already has the most omen it can take.
fn converts_to_raid_omen(raids: &Raids, pos: &pumpkin_util::math::position::BlockPos) -> bool {
    raids
        .get_nearby_raid(pos, RAID_AT_DISTANCE_SQR)
        .is_none_or(|raid| raid.raid_omen_level < Raid::MAX_RAID_OMEN_LEVEL)
}

impl MobEffect for BadOmenMobEffect {
    fn should_apply_effect_tick(&self, _tick_count: i32, _amplifier: u8) -> bool {
        true
    }

    /// A non-spectator player in a village, not on peaceful, with no raid at full omen nearby
    /// gets `RAID_OMEN` for 600 ticks at this amplifier and its raid omen position, and the bad
    /// omen ends.
    fn apply_effect_tick(&self, living: &LivingEntity, amplifier: u8) -> bool {
        let world = living.entity.world.load();
        let Some(entity) = world.get_entity_by_id(living.entity.entity_id) else {
            return true;
        };
        let Some(player) = entity.get_player() else {
            return true;
        };
        let pos = player.living_entity.entity.block_pos.load();
        if player.is_spectator()
            || world.level_info.load().difficulty == Difficulty::Peaceful
            || !world.is_village(&pos)
        {
            return true;
        }
        let converts = converts_to_raid_omen(
            &world
                .raids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            &pos,
        );
        if converts {
            // `new MobEffectInstance(RAID_OMEN, 600, amplification)`.
            player.add_effect(Effect {
                effect_type: &StatusEffect::RAID_OMEN,
                duration: 600,
                amplifier,
                ambient: false,
                show_particles: true,
                show_icon: true,
                blend: false,
            });
            player.set_raid_omen_position(pos);
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_util::Difficulty;
    use pumpkin_util::math::position::BlockPos;

    use super::converts_to_raid_omen;
    use crate::world::raid::{Raid, Raids};

    #[test]
    fn converts_unless_the_raid_there_is_at_full_omen() {
        let pos = BlockPos::new(0, 64, 0);
        let mut raids = Raids::default();
        assert!(converts_to_raid_omen(&raids, &pos));

        let mut raid = Raid::new(1, BlockPos::new(10, 64, 10), Difficulty::Normal);
        raid.raid_omen_level = Raid::MAX_RAID_OMEN_LEVEL - 1;
        raids.raid_map.insert(1, raid);
        assert!(converts_to_raid_omen(&raids, &pos));

        raids.raid_map.get_mut(&1).unwrap().raid_omen_level = Raid::MAX_RAID_OMEN_LEVEL;
        assert!(!converts_to_raid_omen(&raids, &pos));

        // A full raid farther than 96 blocks isn't the raid here.
        raids.raid_map.get_mut(&1).unwrap().center = BlockPos::new(200, 64, 0);
        assert!(converts_to_raid_omen(&raids, &pos));
    }
}
