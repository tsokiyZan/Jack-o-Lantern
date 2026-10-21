//! Vanilla `InsideBlockEffectType` and `InsideBlockEffectApplier.StepBasedCollector`: the
//! effects blocks and fluids queue while an entity sweeps through them, applied afterwards in
//! a fixed order.

use std::cell::RefCell;

use crate::entity::{Entity, EntityBase};

/// Vanilla `InsideBlockEffectType`, in its apply order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InsideBlockEffectType {
    Freeze,
    ClearFreeze,
    FireIgnite,
    LavaIgnite,
    Extinguish,
}

impl InsideBlockEffectType {
    const APPLY_ORDER: [Self; 5] = [
        Self::Freeze,
        Self::ClearFreeze,
        Self::FireIgnite,
        Self::LavaIgnite,
        Self::Extinguish,
    ];

    /// Vanilla `InsideBlockEffectType.effect`.
    fn apply(self, caller: &dyn EntityBase) {
        let entity = caller.get_entity();
        match self {
            Self::Freeze => {
                entity
                    .is_in_powder_snow
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if entity.can_freeze(caller) {
                    entity.set_frozen_ticks(
                        Entity::MAX_FROZEN_TICKS.min(entity.get_frozen_ticks() + 1),
                    );
                }
            }
            Self::ClearFreeze => entity.clear_freeze(),
            Self::FireIgnite => crate::block::blocks::fire::FireBlockBase::fire_ignite(caller),
            Self::LavaIgnite => Entity::lava_ignite(caller),
            Self::Extinguish => entity.extinguish(),
        }
    }
}

type Effect = Box<dyn FnOnce(&dyn EntityBase)>;

enum FinalEffect {
    Type(InsideBlockEffectType),
    Custom(Effect),
}

#[derive(Default)]
struct Collector {
    effects_in_step: [bool; 5],
    before_effects_in_step: [Vec<Effect>; 5],
    after_effects_in_step: [Vec<Effect>; 5],
    final_effects: Vec<FinalEffect>,
    last_step: Option<i32>,
}

impl Collector {
    fn flush_step(&mut self) {
        for effect_type in InsideBlockEffectType::APPLY_ORDER {
            let index = effect_type as usize;
            let before = std::mem::take(&mut self.before_effects_in_step[index]);
            self.final_effects
                .extend(before.into_iter().map(FinalEffect::Custom));
            if std::mem::take(&mut self.effects_in_step[index]) {
                self.final_effects.push(FinalEffect::Type(effect_type));
            }
            let after = std::mem::take(&mut self.after_effects_in_step[index]);
            self.final_effects
                .extend(after.into_iter().map(FinalEffect::Custom));
        }
    }
}

/// Vanilla `InsideBlockEffectApplier.StepBasedCollector`.
#[derive(Default)]
pub struct InsideBlockEffectApplier {
    collector: RefCell<Collector>,
}

impl InsideBlockEffectApplier {
    /// Vanilla `apply`: the effect runs once per step, however many blocks queue it.
    pub fn apply(&self, effect_type: InsideBlockEffectType) {
        self.collector.borrow_mut().effects_in_step[effect_type as usize] = true;
    }

    /// Vanilla `runBefore`.
    pub fn run_before(
        &self,
        effect_type: InsideBlockEffectType,
        effect: impl FnOnce(&dyn EntityBase) + 'static,
    ) {
        self.collector.borrow_mut().before_effects_in_step[effect_type as usize]
            .push(Box::new(effect));
    }

    /// Vanilla `runAfter`.
    pub fn run_after(
        &self,
        effect_type: InsideBlockEffectType,
        effect: impl FnOnce(&dyn EntityBase) + 'static,
    ) {
        self.collector.borrow_mut().after_effects_in_step[effect_type as usize]
            .push(Box::new(effect));
    }

    /// Vanilla `advanceStep`.
    pub fn advance_step(&self, step: i32) {
        let mut collector = self.collector.borrow_mut();
        if collector.last_step != Some(step) {
            collector.last_step = Some(step);
            collector.flush_step();
        }
    }

    /// Vanilla `applyAndClear`: stops once the entity dies.
    pub fn apply_and_clear(&self, caller: &dyn EntityBase) {
        let final_effects = {
            let mut collector = self.collector.borrow_mut();
            collector.flush_step();
            collector.last_step = None;
            std::mem::take(&mut collector.final_effects)
        };
        for effect in final_effects {
            if !is_alive(caller) {
                break;
            }
            match effect {
                FinalEffect::Type(effect_type) => effect_type.apply(caller),
                FinalEffect::Custom(effect) => effect(caller),
            }
        }
    }
}

/// Vanilla `Entity.isAlive`, with `LivingEntity`'s override.
pub fn is_alive(caller: &dyn EntityBase) -> bool {
    caller.get_living_entity().map_or_else(
        || caller.get_entity().is_alive(),
        super::living::LivingEntity::is_alive,
    )
}

#[cfg(test)]
mod test {
    use super::{Collector, FinalEffect, InsideBlockEffectType};

    #[test]
    fn flush_orders_effects_by_type_with_hooks() {
        let mut collector = Collector::default();
        let extinguish = InsideBlockEffectType::Extinguish as usize;
        let freeze = InsideBlockEffectType::Freeze as usize;
        collector.effects_in_step[extinguish] = true;
        collector.effects_in_step[freeze] = true;
        collector.before_effects_in_step[extinguish].push(Box::new(|_| {}));
        collector.after_effects_in_step[freeze].push(Box::new(|_| {}));
        collector.flush_step();
        let kinds: Vec<_> = collector
            .final_effects
            .iter()
            .map(|effect| match effect {
                FinalEffect::Type(effect_type) => format!("{effect_type:?}"),
                FinalEffect::Custom(_) => "custom".to_owned(),
            })
            .collect();
        assert_eq!(kinds, ["Freeze", "custom", "custom", "Extinguish"]);
        assert!(collector.effects_in_step.iter().all(|set| !set));
    }
}
