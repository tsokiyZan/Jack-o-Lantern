use std::sync::atomic::Ordering;

use pumpkin_data::game_event::GameEvent;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockDirection};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use super::super::memory::{MemoryStatus, types};
use super::one_shot::OneShot;
use crate::world::World;

/// Vanilla `TryLaySpawnOnFluidNearLand`: a pregnant mob standing on land beside open
/// water lays `spawn_block` on the water and is no longer pregnant.
#[must_use]
pub fn try_lay_spawn_on_fluid_near_land(spawn_block: &'static Block) -> OneShot {
    OneShot::new(
        "TryLaySpawnOnFluidNearLand",
        vec![
            (types::ATTACK_TARGET.id(), MemoryStatus::ValueAbsent),
            (types::WALK_TARGET.id(), MemoryStatus::ValuePresent),
            (types::IS_PREGNANT.id(), MemoryStatus::ValuePresent),
        ],
        move |tick| {
            let entity = tick.mob.get_entity();
            if entity.is_in_water() || !entity.on_ground.load(Ordering::Relaxed) {
                return false;
            }
            let below_pos = entity.block_pos.load().down();
            for direction in BlockDirection::horizontal() {
                let relative_pos = below_pos.offset(direction.to_offset());
                if !has_empty_top_face(tick.world, &relative_pos)
                    || !(tick
                        .world
                        .get_fluid_type(&relative_pos)
                        .has_tag(&tag::Fluid::MINECRAFT_SUPPORTS_FROGSPAWN)
                        || tick
                            .world
                            .get_block(&relative_pos)
                            .has_tag(&tag::Block::MINECRAFT_SUPPORTS_FROGSPAWN))
                {
                    continue;
                }
                let spawn_pos = relative_pos.up();
                if tick.world.get_block_state(&spawn_pos).is_air() {
                    tick.world.set_block_state(
                        &spawn_pos,
                        spawn_block.default_state.id,
                        BlockFlags::NOTIFY_ALL,
                    );
                    tick.world
                        .emit_game_event(GameEvent::BlockPlace.name(), spawn_pos.to_f64());
                    tick.world.play_sound_fine(
                        Sound::EntityFrogLaySpawn,
                        SoundCategory::Blocks,
                        &entity.pos.load(),
                        1.0,
                        1.0,
                    );
                    tick.brain.erase(types::IS_PREGNANT.id());
                    return true;
                }
            }
            true
        },
    )
}

/// Vanilla `getCollisionShape(level, pos).getFaceShape(Direction.UP).isEmpty()`: no
/// collision box reaches the top of the block.
fn has_empty_top_face(world: &World, pos: &BlockPos) -> bool {
    !world
        .get_block_state(pos)
        .get_block_collision_shapes()
        .any(|shape| shape.max.y >= 1.0 - 1.0e-7 && shape.min.y < 1.0 - 1.0e-7)
}
