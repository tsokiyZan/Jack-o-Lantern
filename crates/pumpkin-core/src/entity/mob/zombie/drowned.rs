use std::sync::Arc;

use crate::entity::ai::goal::active_target::ActiveTargetGoal;
use crate::entity::ai::goal::goal_selector::GoalSelector;
use crate::entity::ai::goal::revenge::RevengeGoal;
use crate::entity::ai::goal::wander_around::WanderAroundGoal;
use crate::entity::ai::goal::zombie_attack::ZombieAttackGoal;
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::living::LivingEntity;
use crate::entity::mob::zombie::{
    ZombieEntityBase, baby_turtle_on_land_target, is_abstract_villager,
};
use crate::entity::{
    Entity,
    mob::{Mob, MobEntity},
};
use crate::world::World;
use pumpkin_data::entity::EntityType;
use pumpkin_nbt::compound::NbtCompound;

pub struct DrownedEntity {
    entity: Arc<ZombieEntityBase>,
}

impl DrownedEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        Self::with_can_break_doors(entity, false)
    }

    #[must_use]
    pub fn with_can_break_doors(entity: Entity, can_break_doors: bool) -> Arc<Self> {
        let entity = ZombieEntityBase::with_behaviour_goals(
            entity,
            can_break_doors,
            Self::add_behaviour_goals,
        );
        // Vanilla `Drowned`'s constructor.
        entity
            .mob_entity
            .set_pathfinding_malus(PathType::Water, 0.0);
        Arc::new(Self { entity })
    }

    /// Vanilla `Drowned.addBehaviourGoals`. Not ported yet: going to water (1), the
    /// trident attack (2), going to the beach (5) and swimming up (6), along with
    /// the amphibious navigation and swimming move control they rely on.
    fn add_behaviour_goals(
        mob: &MobEntity,
        goal_selector: &mut GoalSelector,
        target_selector: &mut GoalSelector,
    ) {
        goal_selector.add_goal(
            2,
            ZombieAttackGoal::new(1.0, false).gated_by(|mob| {
                mob.get_mob_entity()
                    .get_target()
                    .as_deref()
                    .is_some_and(|target| {
                        let world = mob.get_entity().world.load();
                        target
                            .get_living_entity()
                            .is_some_and(|target| ok_target(target, &world))
                    })
            }),
        );
        goal_selector.add_goal(7, Box::new(WanderAroundGoal::new(1.0)));

        target_selector.add_goal(
            1,
            Box::new(
                RevengeGoal::new(true)
                    .ignoring(|entity_type| entity_type == &EntityType::DROWNED)
                    .alerting_others_except(|entity_type| {
                        entity_type == &EntityType::ZOMBIFIED_PIGLIN
                    }),
            ),
        );
        target_selector.add_goal(
            2,
            Box::new(ActiveTargetGoal::new(
                mob,
                &EntityType::PLAYER,
                10,
                true,
                false,
                Some(ok_target),
            )),
        );
        target_selector.add_goal(
            3,
            ActiveTargetGoal::predicated(mob, 10, false, is_abstract_villager),
        );
        target_selector.add_goal(
            3,
            ActiveTargetGoal::with_default(mob, &EntityType::IRON_GOLEM, true),
        );
        target_selector.add_goal(
            3,
            ActiveTargetGoal::with_default(mob, &EntityType::AXOLOTL, true),
        );
        target_selector.add_goal(5, Box::new(baby_turtle_on_land_target(mob)));
    }
}

/// Vanilla `Drowned.okTarget`: by day, drowned only go after targets in water.
fn ok_target(target: &LivingEntity, world: &World) -> bool {
    !world.is_bright_outside() || target.is_in_water()
}

impl Mob for DrownedEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn spawn_as_baby(&self) -> bool {
        self.entity.spawn_as_baby()
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.entity.mob_write_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.entity.mob_read_nbt(nbt);
    }
}
