use std::sync::Arc;

use crate::block::entities::mob_spawner::MobSpawnerBlockEntity;
use crate::entity::experience_orb::ExperienceOrbEntity;
use pumpkin_macros::pumpkin_block;
use pumpkin_util::random::RandomImpl;

use crate::block::{BlockBehaviour, OnSyncedBlockEventArgs, PlacedArgs, SpawnAfterBreakArgs};

#[pumpkin_block("minecraft:spawner")]
pub struct SpawnerBlock;

impl BlockBehaviour for SpawnerBlock {
    fn on_synced_block_event(&self, _args: OnSyncedBlockEventArgs<'_>) -> bool {
        true
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.is_new_block() {
            return;
        }
        {
            let spawner_block_entity = MobSpawnerBlockEntity::new(*args.position, None);
            args.world.add_block_entity(Arc::new(spawner_block_entity));
        }
    }

    /// Vanilla `SpawnerBlock.spawnAfterBreak`: 15 + nextInt(15) + nextInt(15) experience from
    /// the level random, through `popExperience` (which needs `block_drops`).
    fn spawn_after_break(&self, args: SpawnAfterBreakArgs<'_>) {
        if !args.world.level_info.load().game_rules.block_drops {
            return;
        }
        let magic_count = {
            let mut random = args
                .world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            15 + random.next_bounded_i32(15) + random.next_bounded_i32(15)
        };
        ExperienceOrbEntity::spawn(
            args.world,
            args.position.to_centered_f64(),
            magic_count as u32,
        );
    }
}
