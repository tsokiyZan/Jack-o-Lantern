use crate::entity::ai::control::Control;
use crate::entity::mob::Mob;
use std::any::Any;
use std::sync::atomic::Ordering;

/// Vanilla `JumpControl`, which mobs like the rabbit replace with their own.
/// `Any` lets a mob reach its own control's extra state, as vanilla casts `mob.jumpControl`.
pub trait JumpControlTrait: Control + Any {
    /// Vanilla `JumpControl.jump`.
    fn jump(&mut self);

    /// Vanilla `JumpControl.tick`.
    fn tick(&mut self, mob: &dyn Mob);
}

#[derive(Default)]
pub struct JumpControl {
    jump: bool,
}

impl Control for JumpControl {}

impl JumpControlTrait for JumpControl {
    fn jump(&mut self) {
        self.jump = true;
    }

    fn tick(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity()
            .living_entity
            .jumping
            .store(self.jump, Ordering::SeqCst);
        self.jump = false;
    }
}
