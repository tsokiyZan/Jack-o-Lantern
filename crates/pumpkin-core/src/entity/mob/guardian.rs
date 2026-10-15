use crate::entity::ai::pathfinder::node::PathType;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use pumpkin_data::entity::EntityType;
use pumpkin_data::{damage::DamageType, sound::Sound};

use crate::entity::custom_sound::CustomSound;
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::ActiveTargetGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
};

pub struct GuardianEntity {
    pub mob_entity: MobEntity,
}

impl GuardianEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `Guardian`'s constructor.
        mob_entity.set_pathfinding_malus(PathType::Water, 0.0);
        let guardian = Self { mob_entity };
        let mob_arc = Arc::new(guardian);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            register_guardian_goals(
                &mut goal_selector,
                &mut target_selector,
                &mob_arc.mob_entity,
                &mob_weak,
                80,
            );
        };

        mob_arc
    }
}

impl CustomSound for GuardianEntity {
    fn hurt_sound(&self) -> Option<Sound> {
        let entity = self.get_entity();
        let is_water = entity.touching_water.load(Ordering::Relaxed);
        Some(if is_water {
            Sound::EntityGuardianHurt
        } else {
            Sound::EntityGuardianHurtLand
        })
    }

    fn death_sound(&self) -> Option<Sound> {
        let entity = self.get_entity();
        let is_water = entity.touching_water.load(Ordering::Relaxed);
        Some(if is_water {
            Sound::EntityGuardianDeath
        } else {
            Sound::EntityGuardianDeathLand
        })
    }
}

impl Mob for GuardianEntity {
    /// Vanilla `Guardian.getAmbientSound`.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        Some(if self.get_entity().is_in_water() {
            pumpkin_data::sound::Sound::EntityGuardianAmbient
        } else {
            pumpkin_data::sound::Sound::EntityGuardianAmbientLand
        })
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn on_damage(&self, _damage_type: DamageType, source: Option<&dyn EntityBase>) {
        if let Some(src) = source
            && let Some(living) = src.get_living_entity()
        {
            let _ = living.damage(src, 2.0, DamageType::THORNS);
        }
    }
}

/// Vanilla `Guardian.registerGoals`, also used by elder guardians, whose constructor sets the
/// stroll interval to 400.
pub fn register_guardian_goals(
    goal_selector: &mut crate::entity::ai::goal::goal_selector::GoalSelector,
    target_selector: &mut crate::entity::ai::goal::goal_selector::GoalSelector,
    mob_entity: &MobEntity,
    mob_weak: &Weak<dyn Mob>,
    stroll_interval: i32,
) {
    use crate::entity::ai::goal::{Controls, move_towards_restriction::MoveTowardsRestrictionGoal};
    let move_and_look = Controls::MOVE.union(Controls::LOOK);
    // TODO: 4 GuardianAttackGoal (the laser) is not ported.
    goal_selector.add_goal(
        5,
        Box::new(MoveTowardsRestrictionGoal::new(1.0).with_controls(move_and_look)),
    );
    goal_selector.add_goal(
        7,
        Box::new(
            WanderAroundGoal::with_interval(1.0, stroll_interval).with_controls(move_and_look),
        ),
    );
    goal_selector.add_goal(
        8,
        LookAtEntityGoal::with_default(mob_weak.clone(), &EntityType::PLAYER, 8.0),
    );
    goal_selector.add_goal(
        8,
        Box::new(LookAtEntityGoal::new(
            mob_weak.clone(),
            &EntityType::GUARDIAN,
            12.0,
            0.01,
            false,
        )),
    );
    goal_selector.add_goal(9, Box::new(RandomLookAroundGoal::default()));

    // `GuardianAttackSelector`: players, squid and axolotls more than 3 blocks away.
    let guardian = mob_weak.clone();
    target_selector.add_goal(
        1,
        ActiveTargetGoal::predicated(mob_entity, 10, true, move |target, _world| {
            let entity_type = target.entity.entity_type;
            (entity_type == &EntityType::PLAYER
                || entity_type == &EntityType::SQUID
                || entity_type == &EntityType::GLOW_SQUID
                || entity_type == &EntityType::AXOLOTL)
                && guardian.upgrade().is_some_and(|guardian| {
                    target
                        .entity
                        .pos
                        .load()
                        .squared_distance_to_vec(&guardian.get_entity().pos.load())
                        > 9.0
                })
        }),
    );
}
