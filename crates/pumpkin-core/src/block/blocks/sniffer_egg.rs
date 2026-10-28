use std::sync::Arc;

use pumpkin_data::block_properties::SnifferEggLikeProperties;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockState};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::wrap_degrees;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::{BlockAccessor, BlockFlags};
use rand::RngExt;
use uuid::Uuid;

use crate::block::{BlockBehaviour, OnScheduledTickArgs, PathComputationType, PlacedArgs};
use crate::entity::mob::Mob;
use crate::entity::r#type::from_type;
use crate::world::World;

/// Vanilla `SnifferEggBlock.MAX_HATCH_LEVEL`.
const MAX_HATCH_LEVEL: u8 = 2;
/// Vanilla `SnifferEggBlock.REGULAR_HATCH_TIME_TICKS`.
const REGULAR_HATCH_TIME_TICKS: i32 = 24000;
/// Vanilla `SnifferEggBlock.BOOSTED_HATCH_TIME_TICKS`.
const BOOSTED_HATCH_TIME_TICKS: i32 = 12000;
/// Vanilla `SnifferEggBlock.RANDOM_HATCH_OFFSET_TICKS`.
const RANDOM_HATCH_OFFSET_TICKS: i32 = 300;
/// How many cracks a hatch time is split into: vanilla `onPlace`'s `hatchTime / 3`.
const HATCH_STAGES: i32 = 3;
/// Volume of vanilla `SnifferEggBlock.tick`'s crack and hatch sounds.
const EGG_SOUND_VOLUME: f32 = 0.7;

#[pumpkin_block("minecraft:sniffer_egg")]
pub struct SnifferEggBlock;

impl SnifferEggBlock {
    /// Vanilla `SnifferEggBlock.hatchBoost`.
    fn hatch_boost(world: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        world
            .get_block(&pos.down())
            .has_tag(&tag::Block::MINECRAFT_SNIFFER_EGG_HATCH_BOOST)
    }

    /// Vanilla `SnifferEggBlock.onPlace`, which vanilla's `setBlock` also runs for each crack.
    fn on_place_in_world(world: &Arc<World>, block: &Block, pos: &BlockPos) {
        let boosted = Self::hatch_boost(world.as_ref(), pos);
        if boosted {
            world.sync_world_event(WorldEvent::ParticlesEggCrack, *pos, 0);
        }
        let hatch_time = if boosted {
            BOOSTED_HATCH_TIME_TICKS
        } else {
            REGULAR_HATCH_TIME_TICKS
        };
        let progression_tick_delay = hatch_time / HATCH_STAGES;
        world.game_event(
            GameEvent::BlockPlace,
            pos.to_centered_f64(),
            crate::world::game_event::GameEventContext::of_state(world.get_block_state(pos)),
        );
        let delay = progression_tick_delay + rand::rng().random_range(0..RANDOM_HATCH_OFFSET_TICKS);
        world.schedule_block_tick(block, *pos, delay, TickPriority::Normal);
    }
}

impl BlockBehaviour for SnifferEggBlock {
    /// Vanilla `SnifferEggBlock.onPlace`: every state change (each crack) reschedules.
    fn placed(&self, args: PlacedArgs<'_>) {
        Self::on_place_in_world(args.world, args.block, args.position);
    }

    /// Vanilla `SnifferEggBlock.tick`.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        let mut props = SnifferEggLikeProperties::from_state_id(state_id);
        let pitch = 0.9 + rand::rng().random::<f32>() * 0.2;
        let center = args.position.to_centered_f64();
        if props.hatch == MAX_HATCH_LEVEL {
            args.world.play_sound_fine(
                Sound::BlockSnifferEggHatch,
                SoundCategory::Blocks,
                &center,
                EGG_SOUND_VOLUME,
                pitch,
            );
            args.world.break_block(
                args.position,
                None,
                BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
            );
            let sniffer = from_type(&EntityType::SNIFFER, center, args.world, Uuid::new_v4());
            if let Some(ageable) = sniffer.get_mob().and_then(Mob::as_ageable) {
                ageable.set_baby(true);
            }
            let entity = sniffer.get_entity();
            entity
                .yaw
                .store(wrap_degrees(rand::rng().random::<f32>() * 360.0));
            entity.set_pitch(0.0);
            args.world.spawn_entity(sniffer);
        } else {
            args.world.play_sound_fine(
                Sound::BlockSnifferEggCrack,
                SoundCategory::Blocks,
                &center,
                EGG_SOUND_VOLUME,
                pitch,
            );
            props.hatch += 1;
            args.world.set_block_state(
                args.position,
                props.to_state_id(args.block),
                BlockFlags::NOTIFY_LISTENERS,
            );
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
