use std::sync::atomic::Ordering;

use pumpkin_data::HorizontalFacingExt;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::rot_lerp;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use super::try_find_water::TryFindWaterGoal;
use super::{Controls, Goal, to_goal_ticks};
use crate::entity::mob::Mob;
use crate::world::World;

/// Vanilla `DolphinJumpGoal.STEPS_TO_CHECK`.
const STEPS_TO_CHECK: [i32; 6] = [0, 1, 4, 5, 6, 7];

/// Vanilla `DolphinJumpGoal`: now and then a dolphin with clear water and open air
/// ahead of it leaps out of the water.
pub struct DolphinJumpGoal {
    interval: i32,
    breached: bool,
}

impl DolphinJumpGoal {
    #[must_use]
    pub const fn new(interval: i32) -> Self {
        Self {
            interval: to_goal_ticks(interval),
            breached: false,
        }
    }

    fn water_is_clear(world: &World, pos: &BlockPos) -> bool {
        TryFindWaterGoal::is_water(world, pos)
            && !world
                .get_block(pos)
                .has_tag(&tag::Block::MINECRAFT_BLOCKS_DOLPHIN_JUMP)
    }

    fn surface_is_clear(world: &World, pos: &BlockPos) -> bool {
        world.get_block_state(&pos.up()).is_air() && world.get_block_state(&pos.up().up()).is_air()
    }

    fn motion_step(mob: &dyn Mob) -> (i32, i32) {
        let offset = mob
            .get_entity()
            .get_horizontal_facing()
            .to_block_direction()
            .to_offset();
        (offset.x, offset.z)
    }
}

impl Goal for DolphinJumpGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if mob
            .get_entity()
            .random()
            .next_bounded_i32(self.interval.max(1))
            != 0
        {
            return false;
        }
        let entity = mob.get_entity();
        let world = entity.world.load();
        let (step_x, step_z) = Self::motion_step(mob);
        let start = entity.block_pos.load();
        STEPS_TO_CHECK.iter().all(|&step| {
            let pos = BlockPos::new(
                start.0.x + step_x * step,
                start.0.y,
                start.0.z + step_z * step,
            );
            Self::water_is_clear(&world, &pos) && Self::surface_is_clear(&world, &pos)
        })
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let entity = mob.get_entity();
        let yd = entity.velocity.load().y;
        let pitch = entity.pitch.load();
        (yd * yd >= 0.03 || pitch == 0.0 || pitch.abs() >= 10.0 || !entity.is_in_water())
            && !entity.on_ground.load(Ordering::Relaxed)
    }

    fn start(&mut self, mob: &dyn Mob) {
        let entity = mob.get_entity();
        let (step_x, step_z) = Self::motion_step(mob);
        let velocity = entity.velocity.load();
        entity.set_velocity(Vector3::new(
            velocity.x + f64::from(step_x) * 0.6,
            velocity.y + 0.7,
            velocity.z + f64::from(step_z) * 0.6,
        ));
        mob.get_mob_entity().own_navigation().stop();
    }

    fn stop(&mut self, mob: &dyn Mob) {
        mob.get_entity().pitch.store(0.0);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let entity = mob.get_entity();
        let already_breached = self.breached;
        if !already_breached {
            let world = entity.world.load();
            self.breached = TryFindWaterGoal::is_water(&world, &entity.block_pos.load());
        }
        if self.breached && !already_breached {
            entity.play_sound(Sound::EntityDolphinJump);
        }

        let movement = entity.velocity.load();
        let pitch = entity.pitch.load();
        if movement.y * movement.y < 0.03 && pitch != 0.0 {
            entity.pitch.store(rot_lerp(0.2, pitch, 0.0));
        } else if movement.length() > 1.0E-5 {
            let rotation = (-movement.y)
                .atan2(movement.horizontal_length())
                .to_degrees();
            entity.pitch.store(rotation as f32);
        }
    }

    fn can_stop(&self) -> bool {
        false
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::JUMP
    }
}
