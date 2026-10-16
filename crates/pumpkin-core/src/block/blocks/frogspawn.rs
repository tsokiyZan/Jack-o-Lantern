use std::sync::Arc;
use std::sync::atomic::Ordering;

use pumpkin_data::entity::EntityType;
use pumpkin_data::fluid::Fluid;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::{BlockAccessor, BlockFlags};
use rand::RngExt;
use uuid::Uuid;

use crate::block::{
    BlockBehaviour, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, OnEntityCollisionArgs,
    OnScheduledTickArgs, PlacedArgs,
};
use crate::entity::r#type::from_type;
use crate::world::World;

/// Vanilla `FrogspawnBlock.MIN_TADPOLES_SPAWN`.
const MIN_TADPOLES_SPAWN: i32 = 2;
/// Vanilla `FrogspawnBlock.MAX_TADPOLES_SPAWN`.
const MAX_TADPOLES_SPAWN: i32 = 5;
/// Vanilla `FrogspawnBlock.DEFAULT_MIN_HATCH_TICK_DELAY`.
const DEFAULT_MIN_HATCH_TICK_DELAY: i32 = 3600;
/// Vanilla `FrogspawnBlock.DEFAULT_MAX_HATCH_TICK_DELAY`.
const DEFAULT_MAX_HATCH_TICK_DELAY: i32 = 12000;

#[pumpkin_block("minecraft:frogspawn")]
pub struct FrogspawnBlock;

impl FrogspawnBlock {
    /// Vanilla `FrogspawnBlock.canSurvive`.
    fn can_survive(world: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        Self::may_place_on(world, &pos.down())
    }

    /// Vanilla `FrogspawnBlock.mayPlaceOn`.
    fn may_place_on(world: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        let state_id = world.get_block_state_id(pos);
        let fluid = World::fluid_type_from_state_id(state_id);
        let fluid_above = World::fluid_type_from_state_id(world.get_block_state_id(&pos.up()));
        (fluid.has_tag(&tag::Fluid::MINECRAFT_SUPPORTS_FROGSPAWN)
            || state_id
                .to_block()
                .has_tag(&tag::Block::MINECRAFT_SUPPORTS_FROGSPAWN))
            && fluid_above == &Fluid::EMPTY
    }

    /// Vanilla `FrogspawnBlock.getFrogspawnHatchDelay`.
    fn get_frogspawn_hatch_delay() -> i32 {
        rand::rng().random_range(DEFAULT_MIN_HATCH_TICK_DELAY..DEFAULT_MAX_HATCH_TICK_DELAY)
    }

    /// Vanilla `FrogspawnBlock.destroyBlock`: `level.destroyBlock(pos, false)`.
    fn destroy_block(world: &Arc<World>, pos: &BlockPos) {
        world.break_block(pos, None, BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS);
    }

    /// Vanilla `FrogspawnBlock.hatchFrogspawn`.
    fn hatch_frogspawn(world: &Arc<World>, pos: &BlockPos) {
        Self::destroy_block(world, pos);
        world.play_block_sound(Sound::BlockFrogspawnHatch, SoundCategory::Blocks, *pos);
        Self::spawn_tadpoles(world, pos);
    }

    /// Vanilla `FrogspawnBlock.spawnTadpoles`.
    fn spawn_tadpoles(world: &Arc<World>, pos: &BlockPos) {
        let tadpole_amount = rand::rng().random_range(MIN_TADPOLES_SPAWN..=MAX_TADPOLES_SPAWN);
        for _ in 1..=tadpole_amount {
            let x_pos = f64::from(pos.0.x) + Self::get_random_tadpole_position_offset();
            let z_pos = f64::from(pos.0.z) + Self::get_random_tadpole_position_offset();
            let y_rot = rand::rng().random_range(1..361);
            let tadpole = from_type(
                &EntityType::TADPOLE,
                Vector3::new(x_pos, f64::from(pos.0.y) - 0.5, z_pos),
                world,
                Uuid::new_v4(),
            );
            let entity = tadpole.get_entity();
            entity.yaw.store(y_rot as f32);
            entity.set_pitch(0.0);
            if let Some(mob) = tadpole.get_mob() {
                mob.get_mob_entity()
                    .persistence_required
                    .store(true, Ordering::Relaxed);
            }
            world.spawn_entity(tadpole);
        }
    }

    /// Vanilla `FrogspawnBlock.getRandomTadpolePositionOffset`.
    fn get_random_tadpole_position_offset() -> f64 {
        rand::rng()
            .random::<f64>()
            .clamp(f64::from(0.2f32), 0.799_999_997_019_767_8)
    }
}

impl BlockBehaviour for FrogspawnBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        Self::can_survive(args.block_accessor, args.position)
    }

    /// Vanilla `FrogspawnBlock.onPlace`.
    fn placed(&self, args: PlacedArgs<'_>) {
        args.world.schedule_block_tick(
            args.block,
            *args.position,
            Self::get_frogspawn_hatch_delay(),
            TickPriority::Normal,
        );
    }

    /// Vanilla `FrogspawnBlock.updateShape`.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if Self::can_survive(args.world, args.position) {
            args.state_id
        } else {
            Block::AIR.default_state.id
        }
    }

    /// Vanilla `FrogspawnBlock.tick`.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        if Self::can_survive(args.world.as_ref(), args.position) {
            Self::hatch_frogspawn(args.world, args.position);
        } else {
            Self::destroy_block(args.world, args.position);
        }
    }

    /// Vanilla `FrogspawnBlock.entityInside`.
    fn on_entity_collision(&self, args: OnEntityCollisionArgs<'_>) {
        if args.entity.get_entity().entity_type == &EntityType::FALLING_BLOCK {
            Self::destroy_block(args.world, args.position);
        }
    }
}
