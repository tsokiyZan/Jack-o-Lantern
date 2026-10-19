use pumpkin_data::Block;
use pumpkin_data::BlockDirection;
use pumpkin_data::world::WorldEvent;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::world::BlockFlags;

use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;

pub struct WeavingMobEffect;

impl MobEffect for WeavingMobEffect {
    /// Vanilla `WeavingMobEffect.onMobRemoved`.
    fn on_mob_death(&self, living: &LivingEntity, _amplifier: u8) {
        let world = living.entity.world.load();
        if !living.is_player() && !world.level_info.load().game_rules.mob_griefing {
            return;
        }
        // `spawnCobwebsRandomlyAround`.
        let center = living.entity.block_pos.load();
        // `MobEffects.WEAVING`'s `maxCobwebs`: `randomBetweenInclusive(mob.getRandom(), 2, 3)`.
        let cobweb_count = (living.entity.random().next_bounded_i32(2) + 2) as usize;
        let mut positions: Vec<BlockPos> = Vec::new();
        // `BlockPos.randomInCube(random, 15, pos, 1)`.
        for _ in 0..15 {
            let pos = {
                let mut random = living.entity.random();
                let x = center.0.x - 1 + random.next_bounded_i32(3);
                let y = center.0.y - 1 + random.next_bounded_i32(3);
                let z = center.0.z - 1 + random.next_bounded_i32(3);
                BlockPos(Vector3::new(x, y, z))
            };
            if !positions.contains(&pos)
                && world.get_block_state(&pos).replaceable()
                && world
                    .get_block_state(&pos.down())
                    .is_side_solid(BlockDirection::Up)
            {
                positions.push(pos);
                if positions.len() >= cobweb_count {
                    break;
                }
            }
        }
        for pos in positions {
            world.set_block_state(&pos, Block::COBWEB.default_state.id, BlockFlags::NOTIFY_ALL);
            world.sync_world_event(WorldEvent::AnimationSpawnCobweb, pos, 0);
        }
    }
}
