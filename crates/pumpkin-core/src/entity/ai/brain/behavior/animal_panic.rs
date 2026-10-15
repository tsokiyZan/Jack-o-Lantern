use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::vector3::Vector3;

use super::super::BrainTick;
use super::super::memory::walk_target::WalkTarget;
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::timed::Behavior;
use crate::entity::ai::goal::try_find_water::TryFindWaterGoal;
use crate::entity::ai::util::{air_and_water_random_pos, goal_utils, land_random_pos};
use crate::entity::mob::Mob;
use pumpkin_util::math::position::BlockPos;

/// Vanilla `AnimalPanic.PANIC_MIN_DURATION`.
const PANIC_MIN_DURATION: i32 = 100;
/// Vanilla `AnimalPanic.PANIC_MAX_DURATION`.
const PANIC_MAX_DURATION: i32 = 120;
/// Vanilla `AnimalPanic.PANIC_DISTANCE_HORIZONTAL`.
const PANIC_DISTANCE_HORIZONTAL: i32 = 5;
/// Vanilla `AnimalPanic.PANIC_DISTANCE_VERTICAL`.
const PANIC_DISTANCE_VERTICAL: i32 = 4;

const ENTRY_CONDITIONS: [(MemoryModuleId, MemoryStatus); 2] = [
    (types::IS_PANICKING.id(), MemoryStatus::Registered),
    (types::HURT_BY.id(), MemoryStatus::Registered),
];

/// Vanilla `AnimalPanic`: after a panic-causing hit, runs (or with a fly height, flies) to
/// random spots nearby, or to water when on fire.
pub struct AnimalPanic {
    speed_multiplier: f32,
    fly_height: Option<i32>,
}

impl AnimalPanic {
    /// Vanilla `AnimalPanic(speedMultiplier)`, which runs to `LandRandomPos` spots.
    #[must_use]
    pub const fn new(speed_multiplier: f32) -> Self {
        Self {
            speed_multiplier,
            fly_height: None,
        }
    }

    /// Vanilla `AnimalPanic(speedMultiplier, flyHeight)`.
    #[must_use]
    pub const fn flying(speed_multiplier: f32, fly_height: i32) -> Self {
        Self {
            speed_multiplier,
            fly_height: Some(fly_height),
        }
    }

    fn get_panic_pos(&self, mob: &dyn Mob) -> Option<Vector3<f64>> {
        if mob.get_entity().is_on_fire()
            && let Some(water) = Self::look_for_water(mob)
        {
            return Some(water.to_f64() + Vector3::new(0.5, 0.0, 0.5));
        }
        let Some(fly_height) = self.fly_height else {
            return land_random_pos::get_pos(
                mob,
                PANIC_DISTANCE_HORIZONTAL,
                PANIC_DISTANCE_VERTICAL,
            );
        };
        // `getViewVector(0.0F)` is last tick's head rotation, which Pumpkin doesn't keep.
        let entity = mob.get_entity();
        let view = crate::entity::Entity::calculate_view_vector(
            entity.pitch.load(),
            entity.head_yaw.load(),
        );
        air_and_water_random_pos::get_pos(
            mob,
            PANIC_DISTANCE_HORIZONTAL,
            PANIC_DISTANCE_VERTICAL,
            fly_height,
            view.x,
            view.z,
            f64::from(std::f32::consts::FRAC_PI_2),
        )
    }

    /// Vanilla `AnimalPanic.lookForWater`: a two-wide mob needs the whole 2x2 wet.
    fn look_for_water(mob: &dyn Mob) -> Option<BlockPos> {
        let entity = mob.get_entity();
        let world = entity.world.load();
        let mob_pos = entity.block_pos.load();
        if world
            .get_block_state(&mob_pos)
            .get_block_collision_shapes_at(&mob_pos)
            .next()
            .is_some()
        {
            return None;
        }
        let two_wide = entity.entity_dimension.load().width.ceil() as i32 == 2;
        goal_utils::within_box_by_manhattan_distance(mob_pos, 5, 1, 5)
            .into_iter()
            .find(|pos| {
                TryFindWaterGoal::is_water(&world, pos)
                    && (!two_wide
                        || TryFindWaterGoal::is_water(&world, &pos.south())
                            && TryFindWaterGoal::is_water(&world, &pos.east())
                            && TryFindWaterGoal::is_water(&world, &pos.south().east()))
            })
    }
}

impl Behavior for AnimalPanic {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &ENTRY_CONDITIONS
    }

    fn min_duration(&self) -> i32 {
        PANIC_MIN_DURATION
    }

    fn max_duration(&self) -> i32 {
        PANIC_MAX_DURATION
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        tick.brain.get(types::HURT_BY).is_some_and(|hurt_by| {
            hurt_by
                .damage_type
                .has_tag(&tag::DamageType::MINECRAFT_PANIC_CAUSES)
        }) || tick.brain.has_memory_value(types::IS_PANICKING.id())
    }

    fn can_still_use(&mut self, _tick: &BrainTick<'_>) -> bool {
        true
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        tick.brain.set(types::IS_PANICKING, true);
        tick.brain.erase(types::WALK_TARGET.id());
        tick.mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        tick.brain.erase(types::IS_PANICKING.id());
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let done = tick
            .mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_done();
        if done && let Some(pos) = self.get_panic_pos(tick.mob) {
            tick.brain.set(
                types::WALK_TARGET,
                WalkTarget::from_vec(pos, self.speed_multiplier, 0),
            );
        }
    }

    fn debug_name(&self) -> &'static str {
        "AnimalPanic"
    }
}
