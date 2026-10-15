use std::sync::Arc;

use pumpkin_data::attributes::Attributes;
use pumpkin_data::item_stack::ItemStack;

use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, types};
use super::Sensor;
use crate::entity::EntityBase;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::mob::Mob;
use crate::entity::player::Player;

const REQUIRES: &[MemoryModuleId] = &[types::TEMPTING_PLAYER.id()];

/// Vanilla `TemptingSensor`: the nearest player within the tempt range holding a
/// temptation in either hand, line of sight ignored.
pub struct TemptingSensor {
    temptations: fn(&dyn Mob, &ItemStack) -> bool,
}

impl TemptingSensor {
    #[must_use]
    pub const fn new(temptations: fn(&dyn Mob, &ItemStack) -> bool) -> Self {
        Self { temptations }
    }

    /// Vanilla `TemptingSensor.forAnimal`: food the animal eats.
    #[must_use]
    pub const fn for_animal() -> Self {
        Self::new(|mob, stack| mob.as_animal().is_some_and(|animal| animal.is_food(stack)))
    }
}

impl Sensor for TemptingSensor {
    fn requires(&self) -> &'static [MemoryModuleId] {
        REQUIRES
    }

    fn do_tick(&mut self, tick: &mut BrainTick<'_>) {
        let temptations = self.temptations;
        let living = &tick.mob.get_mob_entity().living_entity;
        let range = living.get_attribute_value(&Attributes::TEMPT_RANGE);
        let targeting = TargetPredicate::create_non_attackable()
            .set_base_max_distance(range)
            .ignore_visibility();
        let body = &living.entity;
        let body_pos = body.pos.load();
        let holding_temptation = |player: &Player| {
            let inventory = player.inventory();
            temptations(tick.mob, &inventory.held_item())
                || temptations(tick.mob, &inventory.off_hand_item())
        };
        let mut players: Vec<Arc<Player>> = tick
            .world
            .players
            .load()
            .iter()
            .filter(|player| !player.is_spectator())
            .filter(|player| {
                targeting.test(
                    tick.world,
                    Some(tick.mob),
                    player.as_ref() as &dyn EntityBase,
                )
            })
            .filter(|player| holding_temptation(player))
            .filter(|player| !body.has_passenger(player.get_entity().entity_id))
            .cloned()
            .collect();
        players.sort_by(|a, b| {
            a.get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&body_pos)
                .total_cmp(&b.get_entity().pos.load().squared_distance_to_vec(&body_pos))
        });
        tick.brain
            .set_optional(types::TEMPTING_PLAYER, players.into_iter().next());
    }
}
