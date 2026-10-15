use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

use pumpkin_data::entity::EntityStatus;
use pumpkin_data::sound::Sound;
use uuid::Uuid;

use crate::entity::mob::Mob;

/// `AbstractHorse` data flag: tamed.
pub const FLAG_TAME: u8 = 2;
/// `AbstractHorse` data flag: eating.
pub const FLAG_EATING: u8 = 16;
/// `AbstractHorse` data flag: standing (rearing).
pub const FLAG_STANDING: u8 = 32;

/// Vanilla `AbstractHorse`'s taming by riding and rearing, shared by horses, donkeys, mules,
/// llamas and the undead horses.
pub trait AbstractHorse: Mob {
    fn horse_flag(&self, flag: u8) -> bool;
    fn set_horse_flag(&self, flag: u8, value: bool);
    fn set_horse_tamed(&self, tamed: bool);
    fn set_horse_owner(&self, owner: Uuid);
    fn horse_temper(&self) -> &AtomicI32;
    fn stand_counter(&self) -> &AtomicI32;

    /// Vanilla `getAngrySound`; plain `AbstractHorse` has none.
    fn angry_sound(&self) -> Option<Sound> {
        None
    }

    /// Vanilla `getMaxTemper`.
    fn max_temper(&self) -> i32 {
        100
    }

    /// Vanilla `canPerformRearing`.
    fn can_perform_rearing(&self) -> bool {
        true
    }

    fn is_horse_tamed(&self) -> bool {
        self.horse_flag(FLAG_TAME)
    }

    fn is_standing(&self) -> bool {
        self.horse_flag(FLAG_STANDING)
    }

    /// Vanilla `modifyTemper`.
    fn modify_temper(&self, amount: i32) -> i32 {
        let temper = (self.horse_temper().load(Relaxed) + amount).clamp(0, self.max_temper());
        self.horse_temper().store(temper, Relaxed);
        temper
    }

    /// Vanilla `setStanding`.
    fn set_standing(&self, ticks: i32) {
        self.set_horse_flag(FLAG_EATING, false);
        self.set_horse_flag(FLAG_STANDING, true);
        self.stand_counter().store(ticks, Relaxed);
    }

    /// Vanilla `clearStanding`.
    fn clear_standing(&self) {
        self.set_horse_flag(FLAG_STANDING, false);
        self.stand_counter().store(0, Relaxed);
    }

    /// Vanilla `standIfPossible`.
    fn stand_if_possible(&self) {
        if self.can_perform_rearing() {
            self.set_standing(20);
        }
    }

    /// Vanilla `makeMad`: rears and complains, unless already standing.
    fn make_mad(&self) {
        if !self.is_standing() {
            self.stand_if_possible();
            if let Some(sound) = self.angry_sound() {
                self.make_sound(sound);
            }
        }
    }

    /// Vanilla `tameWithName`.
    fn tame_with_name(&self, owner: Uuid) {
        self.set_horse_owner(owner);
        self.set_horse_tamed(true);
        let entity = self.get_entity();
        entity
            .world
            .load()
            .send_entity_status(entity, EntityStatus::TamingSucceeded, None);
    }

    /// The standing countdown from vanilla `AbstractHorse.tick`.
    fn tick_standing(&self) {
        let counter = self.stand_counter().load(Relaxed);
        if counter > 0 {
            self.stand_counter().store(counter - 1, Relaxed);
            if counter - 1 <= 0 {
                self.clear_standing();
            }
        }
    }
}

/// Vanilla `AbstractHorse.registerGoals`, before each kind's `addBehaviourGoals`.
pub fn add_abstract_horse_goals(
    goal_selector: &mut crate::entity::ai::goal::goal_selector::GoalSelector,
    mob: &dyn Mob,
    mob_weak: std::sync::Weak<dyn Mob>,
    can_perform_rearing: bool,
) {
    use crate::entity::ai::goal::{
        ambient_stand::AmbientStandGoal, breed::BreedGoal, follow_parent::FollowParentGoal,
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal,
        run_around_like_crazy::RunAroundLikeCrazyGoal, wander_around::WanderAroundGoal,
    };
    use crate::entity::ai::util::goal_utils;
    use pumpkin_data::entity::EntityType;

    goal_selector.add_goal(1, Box::new(RunAroundLikeCrazyGoal::new(1.2)));
    goal_selector.add_goal(
        2,
        BreedGoal::with_partner_class(1.0, goal_utils::is_abstract_horse),
    );
    goal_selector.add_goal(4, Box::new(FollowParentGoal::new(1.0)));
    goal_selector.add_goal(6, Box::new(WanderAroundGoal::avoiding_water(0.7)));
    goal_selector.add_goal(
        7,
        LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
    );
    goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));
    if can_perform_rearing {
        goal_selector.add_goal(9, Box::new(AmbientStandGoal::new(mob)));
    }
}
