use std::sync::atomic::Ordering;

use super::{Controls, Goal};
use crate::entity::mob::{Mob, MobEntity};
use rand::RngExt;

pub struct SwimGoal {
    goal_control: Controls,
}

impl SwimGoal {
    /// Vanilla `FloatGoal`, whose constructor lets the mob's navigation path through water.
    #[must_use]
    pub fn new(mob: &MobEntity) -> Box<Self> {
        mob.navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_can_float(true);
        Box::new(Self {
            goal_control: Controls::JUMP,
        })
    }

    /// Vanilla `FloatGoal.canUse`: deeper in a floatable fluid than the jump threshold,
    /// or in lava.
    pub(crate) fn is_in_fluid(mob: &dyn Mob) -> bool {
        let living = &mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        let in_water = entity.touching_water.load(Ordering::SeqCst)
            && entity.water_height.load() > living.get_swim_height();
        in_water || entity.touching_lava.load(Ordering::SeqCst)
    }
}

impl Goal for SwimGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        Self::is_in_fluid(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        Self::is_in_fluid(mob)
    }

    fn tick(&mut self, mob: &dyn Mob) {
        if mob.get_random().random::<f32>() < 0.8 {
            mob.get_mob_entity().jump();
        }
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
