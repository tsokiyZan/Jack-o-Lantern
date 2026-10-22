use std::sync::Arc;

use crate::block::entities::bed::BedBlockEntity;
use pumpkin_data::block_properties::BedPart;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::translation;
use pumpkin_data::{Block, BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::GameMode;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, BrokenArgs, CanPlaceAtArgs, NormalUseArgs, OnPlaceArgs, OnStateReplacedArgs,
    PathComputationType, PlacedArgs,
};
use crate::entity::{EntityBase, player::Player};
use crate::world::World;

type BedProperties = pumpkin_data::block_properties::WhiteBedLikeProperties;

#[pumpkin_block("minecraft:straw_bed")]
pub struct StrawBedBlock;

impl BlockBehaviour for StrawBedBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        if let Some(player) = args.player {
            let facing = player.get_entity().get_horizontal_facing();
            return args
                .block_accessor
                .get_block_state(args.position)
                .replaceable()
                && args
                    .block_accessor
                    .get_block_state(&args.position.offset(facing.to_offset()))
                    .replaceable();
        }
        false
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut bed_props = BedProperties::default(args.block);
        bed_props.facing = args.player.get_entity().get_horizontal_facing();
        bed_props.part = BedPart::Foot;
        bed_props.to_state_id(args.block)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        let bed_entity = BedBlockEntity::new(*args.position);
        args.world.add_block_entity(Arc::new(bed_entity));

        let mut bed_head_props = BedProperties::default(args.block);
        bed_head_props.facing = BedProperties::from_state_id(args.state_id).facing;
        bed_head_props.part = BedPart::Head;

        let bed_head_pos = args.position.offset(bed_head_props.facing.to_offset());
        args.world.set_block_state(
            &bed_head_pos,
            bed_head_props.to_state_id(args.block),
            BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
        );

        let bed_head_entity = BedBlockEntity::new(bed_head_pos);
        args.world.add_block_entity(Arc::new(bed_head_entity));
    }

    fn broken(&self, args: BrokenArgs<'_>) {
        let bed_props = BedProperties::from_state_id(args.state.id);
        let other_half_pos = if bed_props.part == BedPart::Head {
            args.position
                .offset(bed_props.facing.opposite().to_offset())
        } else {
            args.position.offset(bed_props.facing.to_offset())
        };
        let neighbor_state_id = args.world.get_block_state_id(&other_half_pos);
        if neighbor_state_id.to_block_id() != args.block.id {
            args.world.update_neighbors(&other_half_pos, None);
            return;
        }

        let is_creative = args.player.gamemode.load() == GameMode::Creative;
        let flags = if bed_props.part == BedPart::Foot && !is_creative {
            BlockFlags::NOTIFY_ALL
        } else {
            BlockFlags::SKIP_DROPS | BlockFlags::NOTIFY_ALL
        };

        args.world
            .break_block(&other_half_pos, Some(args.player), flags);
    }

    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        if args.moved {
            return;
        }

        let bed_props = BedProperties::from_state_id(args.old_state_id);
        let other_half_pos = if bed_props.part == BedPart::Head {
            args.position
                .offset(bed_props.facing.opposite().to_offset())
        } else {
            args.position.offset(bed_props.facing.to_offset())
        };

        let (other_block, other_state) = args.world.get_block_and_state(&other_half_pos);
        if other_block == args.block {
            let other_props = BedProperties::from_state_id(other_state.id);
            if other_props.part != bed_props.part {
                args.world.break_block(
                    &other_half_pos,
                    None,
                    BlockFlags::SKIP_DROPS | BlockFlags::NOTIFY_ALL,
                );
            }
        }
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        Self::use_bed(args.world, args.player, args.position)
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

impl StrawBedBlock {
    pub fn destroy_after_use(world: &Arc<World>, bed_head_pos: BlockPos) {
        let (block, state) = world.get_block_and_state_id(&bed_head_pos);
        if block != &Block::STRAW_BED {
            return;
        }
        let bed_props = BedProperties::from_state_id(state);
        let (head_pos, foot_pos) = if bed_props.part == BedPart::Head {
            (
                bed_head_pos,
                bed_head_pos.offset(bed_props.facing.opposite().to_offset()),
            )
        } else {
            (
                bed_head_pos.offset(bed_props.facing.to_offset()),
                bed_head_pos,
            )
        };
        world.play_block_sound(
            Sound::BlockStrawBedBreakLeave,
            SoundCategory::Blocks,
            head_pos,
        );
        world.break_block(
            &head_pos,
            None,
            BlockFlags::SKIP_DROPS | BlockFlags::NOTIFY_ALL,
        );
        world.break_block(
            &foot_pos,
            None,
            BlockFlags::SKIP_DROPS | BlockFlags::NOTIFY_ALL,
        );
    }

    fn use_bed(world: &Arc<World>, player: &Arc<Player>, position: &BlockPos) -> BlockActionResult {
        let state_id = world.get_block_state_id(position);
        let bed_props = BedProperties::from_state_id(state_id);

        let bed_head_pos = if bed_props.part == BedPart::Head {
            *position
        } else {
            position.offset(bed_props.facing.to_offset())
        };

        // Vanilla `StrawBedBlock.destroyOnUse`: straw beds never explode, they fall apart
        // (`destroy_on_use` in the same dimensions where beds explode).
        if world.dimension.bed_rule.explodes {
            Self::destroy_after_use(world, bed_head_pos);
            return BlockActionResult::SuccessServer;
        }
        // `AbstractBedBlock.useWithoutItem`: an occupied bed (the head's state) kicks a
        // sleeping villager out, or says it is occupied.
        if BedProperties::from_state_id(world.get_block_state_id(&bed_head_pos)).occupied {
            if !crate::block::blocks::bed::kick_villager_out_of_bed(world, bed_head_pos) {
                player.send_system_message_raw(
                    &pumpkin_macros::translate_cross!(
                        translation::java::BLOCK_MINECRAFT_BED_OCCUPIED,
                        translation::bedrock::TILE_BED_OCCUPIED
                    ),
                    true,
                );
            }
            return BlockActionResult::SuccessServer;
        }
        // `ServerPlayer.startSleepInBed`: asleep already or dead, `OTHER_PROBLEM` (no message).
        if player.is_sleeping() || !crate::entity::inside_block_effect::is_alive(player.as_ref()) {
            return BlockActionResult::SuccessServer;
        }

        // `STRAW_BED_RULE` (`DESTROY_ON_LEAVE`) never sets the spawn.
        if !crate::block::blocks::bed::start_sleep_in_bed_checks(
            world,
            player,
            bed_head_pos,
            bed_props.facing,
            world.dimension.bed_rule.can_sleep(world.is_dark_outside()),
            false,
        ) {
            return BlockActionResult::SuccessServer;
        }

        if let Some(server) = world.server.upgrade() {
            let mut event =
                crate::plugin::api::events::player::player_bed::PlayerBedEnterEvent::new(
                    player.clone(),
                    bed_head_pos,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return BlockActionResult::SuccessServer;
            }
        }

        // `SLEPT_IN_BED` fires only where the bed sets the spawn; `STRAW_BED_RULE`
        // (`DESTROY_ON_LEAVE`) never does.
        if player.start_sleep_in_bed(bed_head_pos) {
            player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::Custom,
                pumpkin_data::statistic::CustomStatistic::SleepInStrawBed as i32,
                1,
            );
        }
        player.after_start_sleep_in_bed();

        BlockActionResult::SuccessServer
    }
}
