use pumpkin_data::Block;
use pumpkin_data::fluid::Fluid;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::{Controls, Goal};
use crate::entity::ai::pathfinder::NavigatorGoal;
use crate::entity::ai::pathfinder::node::PathComputationType;
use crate::entity::mob::Mob;
use crate::world::World;

/// Vanilla `BreathAirGoal.canUse`: below this much air the mob heads for the surface.
const LOW_AIR: i32 = 140;
/// How far up `BreathAirGoal.findAirPosition` looks for air.
const SEARCH_HEIGHT: i32 = 8;

/// Vanilla `BreathAirGoal`: a water mob that is running out of air swims straight
/// for the nearest air above it.
pub struct BreathAirGoal;

impl BreathAirGoal {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Vanilla `BreathAirGoal.givesAir`.
    fn gives_air(world: &World, pos: &BlockPos) -> bool {
        let (block, state) = world.get_block_and_state(pos);
        let no_fluid = world.get_fluid(pos).id == Fluid::EMPTY.id;
        (no_fluid || block == &Block::BUBBLE_COLUMN)
            && world
                .block_registry
                .is_pathfindable(block, state, PathComputationType::Land)
    }

    /// Vanilla `BreathAirGoal.findAirPosition`, searching like `BlockPos.neighborColumn`:
    /// the mob's own column first, then the columns north, east, south and west.
    fn find_air_position(mob: &dyn Mob) {
        let entity = mob.get_entity();
        let world = entity.world.load();
        let start = entity.block_pos.load();
        let columns = [(0, 0), (0, -1), (1, 0), (0, 1), (-1, 0)];
        let destination = columns
            .iter()
            .flat_map(|&(dx, dz)| {
                (0..=SEARCH_HEIGHT)
                    .map(move |dy| BlockPos::new(start.0.x + dx, start.0.y + dy, start.0.z + dz))
            })
            .find(|pos| Self::gives_air(&world, pos))
            .unwrap_or_else(|| BlockPos::new(start.0.x, start.0.y + SEARCH_HEIGHT, start.0.z));

        let to = Vector3::new(
            f64::from(destination.0.x),
            f64::from(destination.0.y + 1),
            f64::from(destination.0.z),
        );
        mob.get_mob_entity()
            .own_navigation()
            .set_progress(NavigatorGoal::new(entity.pos.load(), to, 1.0));
    }
}

impl Default for BreathAirGoal {
    fn default() -> Self {
        Self::new()
    }
}

impl Goal for BreathAirGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.get_mob_entity().living_entity.breath.air() < LOW_AIR
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity().own_navigation().stop();
        Self::find_air_position(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        Self::find_air_position(mob);
        let living = &mob.get_mob_entity().living_entity;
        living
            .entity
            .update_velocity_from_input(living.movement_input.load(), 0.02);
        living.make_move(mob);
    }

    fn can_stop(&self) -> bool {
        false
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}
