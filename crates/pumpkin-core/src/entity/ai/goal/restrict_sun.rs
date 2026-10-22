use super::{Controls, Goal};
use crate::entity::EquipmentSlot;
use crate::entity::mob::Mob;

/// Vanilla `RestrictSunGoal`: while the sun is up and the mob has nothing on its head, its
/// ground navigation cuts paths short before they reach open sky.
#[derive(Default)]
pub struct RestrictSunGoal;

impl RestrictSunGoal {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    fn has_ground_path_navigation(mob: &dyn Mob) -> bool {
        mob.get_mob_entity()
            .with_navigation(|navigation, _| navigation.can_navigate_ground())
    }

    fn set_avoid_sun(mob: &dyn Mob, avoid_sun: bool) {
        mob.get_mob_entity().with_navigation(|navigation, _| {
            if navigation.can_navigate_ground() {
                navigation.set_avoid_sun(avoid_sun);
            }
        });
    }
}

impl Goal for RestrictSunGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.get_entity().world.load().is_bright_outside()
            && mob
                .get_mob_entity()
                .living_entity
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&EquipmentSlot::HEAD)
                .is_empty()
            && Self::has_ground_path_navigation(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_start(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        Self::set_avoid_sun(mob, true);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        Self::set_avoid_sun(mob, false);
    }

    fn controls(&self) -> Controls {
        Controls::empty()
    }
}
