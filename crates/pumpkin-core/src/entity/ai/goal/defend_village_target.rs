//! Vanilla `DefendVillageTargetGoal`: an iron golem goes for players the nearby villagers hate.

use std::sync::Arc;

use pumpkin_data::entity::EntityType;

use super::track_target::TrackTargetGoal;
use super::{Controls, Goal};
use crate::entity::EntityBase;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::ai::util::goal_utils;
use crate::entity::mob::Mob;
use crate::entity::passive::villager::VillagerEntity;

/// The reputation at or below which villagers want a player attacked.
const HOSTILE_REPUTATION: i32 = -100;

pub struct DefendVillageTargetGoal {
    track_target_goal: TrackTargetGoal,
    /// Never cleared once set, as in vanilla.
    potential_target: Option<Arc<dyn EntityBase>>,
    /// `TargetingConditions.forCombat().range(64.0)`.
    attack_targeting: TargetPredicate,
}

impl DefendVillageTargetGoal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            track_target_goal: TrackTargetGoal::new(false, true),
            potential_target: None,
            attack_targeting: TargetPredicate::create_attackable().set_base_max_distance(64.0),
        }
    }
}

impl Default for DefendVillageTargetGoal {
    fn default() -> Self {
        Self::new()
    }
}

impl Goal for DefendVillageTargetGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let entity = mob.get_entity();
        let world = entity.world.load();
        let golem: &dyn EntityBase = mob;
        let grow = entity.bounding_box.load().expand(10.0, 8.0, 10.0);
        let villagers: Vec<_> =
            goal_utils::get_entities_of_class(&world, Some(&EntityType::VILLAGER), &grow)
                .into_iter()
                .filter(|v| self.attack_targeting.test(&world, Some(golem), v.as_ref()))
                .collect();
        // `getNearbyPlayers`: players whose position is inside the box.
        let players: Vec<_> = world
            .players
            .load()
            .iter()
            .filter(|player| {
                let pos = player.get_entity().pos.load();
                pos.x >= grow.min.x
                    && pos.x < grow.max.x
                    && pos.y >= grow.min.y
                    && pos.y < grow.max.y
                    && pos.z >= grow.min.z
                    && pos.z < grow.max.z
            })
            .filter(|player| {
                self.attack_targeting
                    .test(&world, Some(golem), player.as_ref() as &dyn EntityBase)
            })
            .cloned()
            .collect();

        for villager in &villagers {
            let Some(villager) = villager.cast_any().downcast_ref::<VillagerEntity>() else {
                continue;
            };
            for player in &players {
                if villager.get_player_reputation(&player.get_entity().entity_uuid)
                    <= HOSTILE_REPUTATION
                {
                    self.potential_target = Some(player.clone() as Arc<dyn EntityBase>);
                }
            }
        }

        self.potential_target.as_ref().is_some_and(|target| {
            target
                .get_player()
                .is_none_or(|player| !player.is_spectator() && !player.is_creative())
        })
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.track_target_goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.set_mob_target(self.potential_target.clone());
        self.track_target_goal.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.track_target_goal.stop(mob);
    }

    fn controls(&self) -> Controls {
        Controls::TARGET
    }
}
