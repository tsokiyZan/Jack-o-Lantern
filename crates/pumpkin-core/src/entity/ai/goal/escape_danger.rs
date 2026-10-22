use pumpkin_data::tag::{self, Tag, Taggable};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering::Relaxed;

use super::revenge::MobFilter;
use super::{Controls, Goal};
use crate::entity::ai::goal::try_find_water::TryFindWaterGoal;
use crate::entity::ai::util::{default_random_pos, goal_utils};
use crate::entity::{ai::pathfinder::NavigatorGoal, mob::Mob};

const WATER_SEARCH_XZ: i32 = 5;
const WATER_SEARCH_Y: i32 = 1;
const HORIZONTAL_RANGE: i32 = 5;
const VERTICAL_RANGE: i32 = 4;

pub struct EscapeDangerGoal {
    speed: f64,
    goal_control: Controls,
    target: Option<Vector3<f64>>,
    running: bool,
    /// Vanilla `TurtlePanicGoal`: always looks for water this far first, not only on fire.
    always_seek_water: Option<i32>,
    /// What a vanilla subclass adds to `shouldPanic`, like a defending fox staying put.
    gate: Option<MobFilter>,
    /// Vanilla `panicCausingDamageTypes`.
    panic_causes: &'static Tag,
    /// The same, chosen per mob, like a polar bear cub panicking where an adult would not.
    panic_causes_for: Option<fn(&dyn Mob) -> &'static Tag>,
    /// What a vanilla subclass adds to `canContinueToUse`, like `PandaPanicGoal`.
    continue_while: Option<MobFilter>,
    /// What a vanilla subclass does in `tick`, given the speed, like `RabbitPanicGoal`.
    on_tick: Option<fn(&dyn Mob, f64)>,
}

impl EscapeDangerGoal {
    #[must_use]
    pub fn new(speed: f64) -> Box<Self> {
        Box::new(Self {
            speed,
            goal_control: Controls::MOVE,
            target: None,
            running: false,
            always_seek_water: None,
            gate: None,
            panic_causes: &tag::DamageType::MINECRAFT_PANIC_CAUSES,
            panic_causes_for: None,
            continue_while: None,
            on_tick: None,
        })
    }

    /// Vanilla `PanicGoal(mob, speed, panicCausingDamageTypes)`: only damage in `panic_causes`
    /// makes the mob flee.
    #[must_use]
    pub fn with_panic_causes(speed: f64, panic_causes: &'static Tag) -> Box<Self> {
        let mut goal = Self::new(speed);
        goal.panic_causes = panic_causes;
        goal
    }

    /// Vanilla `PanicGoal(mob, speed, mob -> tag)`: the panic causes depend on the mob.
    #[must_use]
    pub fn with_panic_causes_for(
        speed: f64,
        panic_causes: fn(&dyn Mob) -> &'static Tag,
    ) -> Box<Self> {
        let mut goal = Self::new(speed);
        goal.panic_causes_for = Some(panic_causes);
        goal
    }

    /// Checked before the base `canContinueToUse`; the filter may act on the mob, as
    /// `PandaPanicGoal` stops the navigation when it sits down.
    #[must_use]
    pub fn continuing_while(mut self: Box<Self>, filter: MobFilter) -> Box<Self> {
        self.continue_while = Some(filter);
        self
    }

    /// Vanilla `Turtle.TurtlePanicGoal`: heads for water within `range` blocks first.
    #[must_use]
    pub fn seeking_water(speed: f64, range: i32) -> Box<Self> {
        let mut goal = Self::new(speed);
        goal.always_seek_water = Some(range);
        goal
    }

    /// Runs `on_tick` with the goal's speed each time the goal ticks, like vanilla's
    /// `RabbitPanicGoal.tick`.
    #[must_use]
    pub fn ticking_with(mut self: Box<Self>, on_tick: fn(&dyn Mob, f64)) -> Box<Self> {
        self.on_tick = Some(on_tick);
        self
    }

    /// Only panics while `gate` holds, like vanilla's `FoxPanicGoal.shouldPanic`.
    #[must_use]
    pub fn panics_only_when(mut self: Box<Self>, gate: MobFilter) -> Box<Self> {
        self.gate = Some(gate);
        self
    }

    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.running
    }

    /// Only `panic_causes` damage makes a mob flee, and only while the source is still
    /// remembered.
    fn should_panic(&self, mob: &dyn Mob) -> bool {
        self.gate.is_none_or(|gate| gate(mob))
            && mob
                .get_mob_entity()
                .living_entity
                .get_last_damage_type()
                .is_some_and(|damage_type| {
                    damage_type.has_tag(self.panic_causes_for.map_or(self.panic_causes, |f| f(mob)))
                })
    }

    /// Vanilla `PanicGoal.lookForWater`: the first water in Manhattan order within `range` blocks
    /// horizontally (5 for most mobs) and 1 vertically, only when the mob is not stuck in a block.
    fn look_for_water(mob: &dyn Mob, range: i32) -> Option<BlockPos> {
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

        goal_utils::within_box_by_manhattan_distance(mob_pos, range, WATER_SEARCH_Y, range)
            .into_iter()
            .find(|pos| TryFindWaterGoal::is_water(&world, pos))
    }

    fn find_random_position(mob: &dyn Mob) -> Option<Vector3<f64>> {
        default_random_pos::get_pos(mob, HORIZONTAL_RANGE, VERTICAL_RANGE)
    }
}

impl Goal for EscapeDangerGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if !self.should_panic(mob) {
            return false;
        }

        let water_range = self
            .always_seek_water
            .or_else(|| (mob.get_entity().fire_ticks.load(Relaxed) > 0).then_some(WATER_SEARCH_XZ));
        if let Some(range) = water_range
            && let Some(water) = Self::look_for_water(mob, range)
        {
            self.target = Some(water.to_f64());
            return true;
        }

        self.target = Self::find_random_position(mob);
        self.target.is_some()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.continue_while.is_none_or(|filter| filter(mob)) && !mob.is_navigator_idle()
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(target) = self.target {
            mob.get_mob_entity().with_navigation(|navigation, owner| {
                let owner_pos = owner.living_entity.entity.pos.load();
                navigation.set_progress(NavigatorGoal::new(owner_pos, target, self.speed));
            });
        }
        self.running = true;
    }

    fn stop(&mut self, _mob: &dyn Mob) {
        self.target = None;
        self.running = false;
    }

    fn tick(&mut self, mob: &dyn Mob) {
        if let Some(on_tick) = self.on_tick {
            on_tick(mob, self.speed);
        }
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
