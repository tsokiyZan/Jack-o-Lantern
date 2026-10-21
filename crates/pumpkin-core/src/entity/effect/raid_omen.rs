use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;

pub struct RaidOmenMobEffect;

impl MobEffect for RaidOmenMobEffect {
    fn should_apply_effect_tick(&self, tick_count: i32, _amplifier: u8) -> bool {
        tick_count == 1
    }

    fn apply_effect_tick(&self, living: &LivingEntity, _amplifier: u8) -> bool {
        let world = living.entity.world.load();
        if let Some(entity) = world.get_entity_by_id(living.entity.entity_id)
            && let Some(player) = entity.get_player()
            && !player.is_spectator()
            && let Some(raid_pos) = player.get_raid_omen_position()
        {
            // `Raids.createOrExtendRaid`, which refuses where raids can't start.
            if world.dimension.can_start_raid {
                let mut raids = world
                    .raids
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                raids.create_or_extend_raid(raid_pos, &world);
                player.increment_stat(
                    pumpkin_data::statistic::StatisticCategory::Custom,
                    pumpkin_data::statistic::CustomStatistic::RaidTrigger as i32,
                    1,
                );
            }
            player.clear_raid_omen_position();
            return false;
        }
        true
    }
}
