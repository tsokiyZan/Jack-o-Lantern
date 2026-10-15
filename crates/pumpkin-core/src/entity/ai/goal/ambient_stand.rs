use crate::entity::ai::goal::{Controls, Goal};
use crate::entity::mob::Mob;
use rand::RngExt;

/// Vanilla `RandomStandGoal`: a horse now and then rears up on its own.
pub struct AmbientStandGoal {
    next_stand: i32,
}

impl AmbientStandGoal {
    #[must_use]
    pub fn new(mob: &dyn Mob) -> Self {
        Self {
            next_stand: -mob.ambient_sound_interval(),
        }
    }

    /// `resetStandInterval`: `getAmbientStandInterval` is the ambient sound interval.
    fn reset_stand_interval(&mut self, mob: &dyn Mob) {
        self.next_stand = -mob.ambient_sound_interval();
    }
}

impl Goal for AmbientStandGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.next_stand += 1;
        if self.next_stand > 0 && mob.get_random().random_range(0..1000) < self.next_stand {
            self.reset_stand_interval(mob);
            // `isImmobile` is `isDeadOrDying` for living entities.
            mob.get_mob_entity().living_entity.health.load() > 0.0
                && mob.get_random().random_range(0..10) == 0
        } else {
            false
        }
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(horse) = mob.as_abstract_horse() {
            horse.stand_if_possible();
        }
        // `playStandSound`: the ambient sound through `Entity.playSound`, at volume and pitch 1.
        let entity = mob.get_entity();
        if let Some(sound) = mob.ambient_sound()
            && !entity.is_silent()
        {
            entity.world.load().play_sound_fine(
                sound,
                mob.sound_source(),
                &entity.pos.load(),
                1.0,
                1.0,
            );
        }
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        Controls::empty()
    }
}
