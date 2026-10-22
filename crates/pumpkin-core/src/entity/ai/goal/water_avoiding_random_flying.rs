use super::{Controls, Goal, to_goal_ticks};
use crate::entity::ai::pathfinder::NavigatorGoal;
use crate::entity::ai::util::{air_and_water_random_pos, hover_random_pos, land_random_pos};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::position::BlockPos;

/// Vanilla `WaterAvoidingRandomStrollGoal.PROBABILITY`.
const WATER_AVOIDING_PROBABILITY: f32 = 0.001;
use crate::entity::mob::Mob;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

pub struct WaterAvoidingRandomFlyingGoal {
    goal_control: Controls,
    speed: f64,
    target: Option<Vector3<f64>>,
    chance: i32,
    /// Vanilla `Parrot.ParrotWanderGoal`: mostly heads for a perch on a tree.
    prefers_trees: bool,
}

impl WaterAvoidingRandomFlyingGoal {
    #[must_use]
    pub const fn new(speed: f64) -> Self {
        Self {
            goal_control: Controls::MOVE,
            speed,
            target: None,
            chance: to_goal_ticks(120),
            prefers_trees: false,
        }
    }

    /// Vanilla `Parrot.ParrotWanderGoal`.
    #[must_use]
    pub const fn preferring_trees(mut self) -> Self {
        self.prefers_trees = true;
        self
    }

    /// `ParrotWanderGoal.getPosition`: out of water onto land, and nearly always to a tree.
    fn parrot_position(mob: &dyn Mob) -> Option<Vector3<f64>> {
        // The land search runs (and rolls) even when a tree spot then replaces it.
        let land_pos = if mob.get_entity().is_in_water() {
            land_random_pos::get_pos(mob, 15, 15)
        } else {
            None
        };
        if mob.get_entity().random().next_f32() >= WATER_AVOIDING_PROBABILITY {
            Self::tree_pos(mob)
        } else {
            land_pos
        }
    }

    /// `ParrotWanderGoal.getTreePos`: the first air block, with air above, resting on leaves
    /// or a log within 3/6/3, in `BlockPos.betweenClosed` order (x fastest, then y, then z).
    fn tree_pos(mob: &dyn Mob) -> Option<Vector3<f64>> {
        let entity = mob.get_entity();
        let world = entity.world.load();
        let mob_pos = entity.block_pos.load();
        let pos = entity.pos.load();
        let (min_x, max_x) = ((pos.x - 3.0).floor() as i32, (pos.x + 3.0).floor() as i32);
        let (min_y, max_y) = ((pos.y - 6.0).floor() as i32, (pos.y + 6.0).floor() as i32);
        let (min_z, max_z) = ((pos.z - 3.0).floor() as i32, (pos.z + 3.0).floor() as i32);
        for z in min_z..=max_z {
            for y in min_y..=max_y {
                for x in min_x..=max_x {
                    let candidate = BlockPos::new(x, y, z);
                    if candidate == mob_pos {
                        continue;
                    }
                    let below = world.get_block(&candidate.down());
                    let can_sit_on = below.has_tag(&tag::Block::MINECRAFT_LEAVES)
                        || below.has_tag(&tag::Block::MINECRAFT_LOGS);
                    if can_sit_on
                        && world.get_block_state(&candidate).is_air()
                        && world.get_block_state(&candidate.up()).is_air()
                    {
                        return Some(Vector3::new(
                            f64::from(x) + 0.5,
                            f64::from(y),
                            f64::from(z) + 0.5,
                        ));
                    }
                }
            }
        }
        None
    }

    /// Prefers a spot to hover over the ground, and falls back to open air or water.
    fn get_position(mob: &dyn Mob) -> Option<Vector3<f64>> {
        let wander_direction = mob.get_looking_vector();

        hover_random_pos::get_pos(
            mob,
            8,
            7,
            wander_direction.x,
            wander_direction.z,
            f64::from(std::f32::consts::FRAC_PI_2),
            3,
            1,
        )
        .or_else(|| {
            air_and_water_random_pos::get_pos(
                mob,
                8,
                4,
                -2,
                wander_direction.x,
                wander_direction.z,
                f64::from(std::f32::consts::FRAC_PI_2),
            )
        })
    }
}

impl Goal for WaterAvoidingRandomFlyingGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if mob.get_controlling_passenger().is_some() {
            return false;
        }

        // `RandomStrollGoal.canUse`: idle mobs (noActionTime >= 100) stop wandering.
        if mob
            .get_mob_entity()
            .living_entity
            .no_action_time
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 100
            || mob.get_entity().random().next_bounded_i32(self.chance) != 0
        {
            return false;
        }

        self.target = if self.prefers_trees {
            Self::parrot_position(mob).or_else(|| Self::get_position(mob))
        } else {
            Self::get_position(mob)
        };
        self.target.is_some()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let is_idle = mob.is_navigator_idle();
        !is_idle && mob.get_controlling_passenger().is_none()
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(target) = self.target {
            mob.get_mob_entity().with_navigation(|navigation, owner| {
                let owner_pos = owner.living_entity.entity.pos.load();
                navigation.set_progress(NavigatorGoal::new(owner_pos, target, self.speed));
            });
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity()
            .with_navigation(|navigation, _| navigation.stop());
        self.target = None;
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
