use pumpkin_data::{
    Block, BlockState, BlockStateId,
    block_properties::{DriedGhastLikeProperties, HorizontalFacing},
    entity::EntityType,
    fluid::Fluid,
    game_event::GameEvent,
    sound::{Sound, SoundCategory},
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::{tick::TickPriority, world::BlockFlags};
use uuid::Uuid;

use crate::{
    block::{
        BlockBehaviour, GetStateForNeighborUpdateArgs, OnPlaceArgs, OnScheduledTickArgs,
        PathComputationType, PlaceLiquidArgs, PlayerPlacedArgs, RandomTickArgs,
        place_liquid_waterlogged,
    },
    entity::{EntityBase, r#type::from_type},
    world::{World, game_event::GameEventContext},
};

/// Vanilla `DriedGhastBlock.MAX_HYDRATION_LEVEL`.
const MAX_HYDRATION_LEVEL: u8 = 3;
/// Vanilla `DriedGhastBlock.HYDRATION_TICK_DELAY`.
const HYDRATION_TICK_DELAY: i32 = 5000;

#[pumpkin_block("minecraft:dried_ghast")]
pub struct DriedGhastBlock;

impl DriedGhastBlock {
    /// Vanilla `DriedGhastBlock.tickWaterlogged`.
    fn tick_waterlogged(world: &std::sync::Arc<World>, pos: &BlockPos, state_id: BlockStateId) {
        let mut props = DriedGhastLikeProperties::from_state_id(state_id);
        if props.hydration == MAX_HYDRATION_LEVEL {
            Self::spawn_ghastling(world, pos, props.facing);
            return;
        }
        world.play_sound(
            Sound::BlockDriedGhastTransition,
            SoundCategory::Blocks,
            &pos.to_centered_f64(),
        );
        props.hydration += 1;
        world.set_block_state(
            pos,
            props.to_state_id(&Block::DRIED_GHAST),
            BlockFlags::NOTIFY_LISTENERS,
        );
        world.game_event(
            GameEvent::BlockChange,
            pos.to_centered_f64(),
            GameEventContext::of_state(BlockState::from_id(state_id)),
        );
    }

    /// Vanilla `DriedGhastBlock.spawnGhastling`.
    fn spawn_ghastling(world: &std::sync::Arc<World>, pos: &BlockPos, facing: HorizontalFacing) {
        // `removeBlock(pos, false)`: the block's fluid stays, so a waterlogged block leaves
        // water behind.
        let waterlogged =
            DriedGhastLikeProperties::from_state_id(world.get_block_state_id(pos)).waterlogged;
        let left = if waterlogged {
            Block::WATER.default_state.id
        } else {
            Block::AIR.default_state.id
        };
        world.set_block_state(pos, left, BlockFlags::NOTIFY_ALL);
        // `EntityTypes.HAPPY_GHAST.create(level, BREEDING)`: no `finalizeSpawn`.
        let spawn_at = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y),
            f64::from(pos.0.z) + 0.5,
        );
        let ghastling = from_type(&EntityType::HAPPY_GHAST, spawn_at, world, Uuid::new_v4());
        if let Some(ageable) = ghastling.get_mob().and_then(|mob| mob.as_ageable()) {
            ageable.set_baby(true);
        }
        let rotation = to_y_rot(facing);
        let entity = ghastling.get_entity();
        entity.head_yaw.store(rotation);
        entity.snap_to(spawn_at, Some(rotation), Some(0.0));
        world.spawn_entity(ghastling.clone());
        world.play_sound_from_entity(
            ghastling.get_entity(),
            Sound::EntityGhastlingSpawn,
            SoundCategory::Blocks,
            1.0,
            1.0,
        );
    }
}

impl BlockBehaviour for DriedGhastBlock {
    /// Vanilla `DriedGhastBlock.getStateForPlacement`: waterlogged in a water source, facing
    /// the player.
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = DriedGhastLikeProperties::default(args.block);
        props.waterlogged = args.replacing.water_source();
        props.facing = args.player.get_entity().get_horizontal_facing().opposite();
        props.to_state_id(args.block)
    }

    /// Vanilla `DriedGhastBlock.setPlacedBy`.
    fn player_placed(&self, args: PlayerPlacedArgs<'_>) {
        let props = DriedGhastLikeProperties::from_state_id(args.state_id);
        let sound = if props.waterlogged {
            Sound::BlockDriedGhastPlaceInWater
        } else {
            Sound::BlockDriedGhastPlace
        };
        args.world.play_sound(
            sound,
            SoundCategory::Blocks,
            &args.position.to_centered_f64(),
        );
    }

    /// Vanilla `DriedGhastBlock.updateShape`.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if DriedGhastLikeProperties::from_state_id(args.state_id).waterlogged {
            args.world.schedule_fluid_tick(
                &Fluid::WATER,
                *args.position,
                Fluid::WATER.flow_speed as i32,
                TickPriority::Normal,
            );
        }
        args.state_id
    }

    /// Vanilla `DriedGhastBlock.tick`: in water it hydrates (or hatches at the last level), out
    /// of water it dries a level.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        let mut props = DriedGhastLikeProperties::from_state_id(state_id);
        if props.waterlogged {
            Self::tick_waterlogged(args.world, args.position, state_id);
        } else if props.hydration > 0 {
            props.hydration -= 1;
            args.world.set_block_state(
                args.position,
                props.to_state_id(args.block),
                BlockFlags::NOTIFY_LISTENERS,
            );
            args.world.game_event(
                GameEvent::BlockChange,
                args.position.to_centered_f64(),
                GameEventContext::of_state(BlockState::from_id(state_id)),
            );
        }
    }

    /// Vanilla `DriedGhastBlock.randomTick`: a waterlogged or hydrated block schedules its next
    /// step 5000 ticks out, unless one is already scheduled.
    fn random_tick(&self, args: RandomTickArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        let props = DriedGhastLikeProperties::from_state_id(state_id);
        if (props.waterlogged || props.hydration > 0)
            && !args
                .world
                .is_block_tick_scheduled(args.position, args.block)
        {
            args.world.schedule_block_tick(
                args.block,
                *args.position,
                HYDRATION_TICK_DELAY,
                TickPriority::Normal,
            );
        }
    }

    /// Vanilla `DriedGhastBlock.placeLiquid`: `SimpleWaterloggedBlock`'s, with
    /// `setBlockAndUpdate` and the in-water sound.
    fn place_liquid(&self, args: PlaceLiquidArgs<'_>) -> bool {
        if !place_liquid_waterlogged(&args) {
            return false;
        }
        args.world.play_sound(
            Sound::BlockDriedGhastPlaceInWater,
            SoundCategory::Blocks,
            &args.position.to_centered_f64(),
        );
        true
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

/// Vanilla `Direction.getYRot`.
const fn to_y_rot(facing: HorizontalFacing) -> f32 {
    match facing {
        HorizontalFacing::South => 0.0,
        HorizontalFacing::West => 90.0,
        HorizontalFacing::North => 180.0,
        HorizontalFacing::East => 270.0,
    }
}
