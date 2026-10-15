use crate::entity::ai::goal::destroy_egg::DestroyEggGoal;
use crate::entity::ai::goal::zombie_attack::ZombieAttackGoal;
use pumpkin_data::entity::EntityPose;
use pumpkin_util::math::boundingbox::EntityDimensions;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;

use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::util::goal_utils;
use crate::entity::{
    Entity, EntityBase,
    ai::behavior::neutral::apply_targets,
    ai::goal::{
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, revenge::RevengeGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{
        Mob, MobEntity,
        equipment::RegionalDifficulty,
        neutral::{NeutralData, NeutralMob},
    },
};
use crate::world::World;

/// Vanilla `ALERT_INTERVAL`: 4 to 6 seconds.
const ALERT_INTERVAL: std::ops::RangeInclusive<i32> = 80..=120;

pub struct ZombifiedPiglinEntity {
    pub mob_entity: MobEntity,
    neutral_data: NeutralData,
    ticks_until_next_alert: AtomicI32,
    /// Detects the target transition that restarts the alert interval.
    had_target: AtomicBool,
}

impl ZombifiedPiglinEntity {
    pub const XP_REWARD: u32 = 5;

    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `ZombifiedPiglin`'s constructor.
        mob_entity.set_pathfinding_malus(PathType::Lava, 8.0);
        let piglin = Self {
            mob_entity,
            neutral_data: NeutralData::default(),
            ticks_until_next_alert: AtomicI32::new(0),
            had_target: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(piglin);
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

            // Vanilla `Zombie.registerGoals`, then this mob's `addBehaviourGoals`.
            goal_selector.add_goal(4, DestroyEggGoal::new(1.0, 3));
            goal_selector.add_goal(
                8,
                LookAtEntityGoal::with_default(mob_weak.clone(), &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));
            // TODO: 1 SpearUseGoal(1.0, 1.0, 10, 2) is not ported.
            goal_selector.add_goal(2, ZombieAttackGoal::new(1.0, false));
            goal_selector.add_goal(7, Box::new(WanderAroundGoal::avoiding_water(1.0)));

            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            target_selector.add_goal(1, Box::new(RevengeGoal::new(true).alerting_others()));
            apply_targets(&mut target_selector, &mob_arc.mob_entity, 2, 3, true);
        };

        mob_arc
    }

    /// Hands the current target to nearby piglins that have none of their own.
    fn maybe_alert_others(&self, target: &Arc<dyn EntityBase>) {
        let remaining = self.ticks_until_next_alert.load(Ordering::Relaxed);
        if remaining > 0 {
            self.ticks_until_next_alert
                .store(remaining - 1, Ordering::Relaxed);
            return;
        }

        if self.has_line_of_sight(target.get_entity()) {
            for other in goal_utils::nearby_same_type(self) {
                let Some(other_mob) = other.get_mob() else {
                    continue;
                };
                if other_mob.get_mob_entity().get_target().is_some()
                    || other.is_allied_to(target.as_ref())
                {
                    continue;
                }
                other_mob.set_mob_target(Some(target.clone()));
            }
        }

        self.ticks_until_next_alert
            .store(rand::random_range(ALERT_INTERVAL), Ordering::Relaxed);
    }
}

crate::impl_neutral_mob!(ZombifiedPiglinEntity, neutral_data);

/// Vanilla `ZombifiedPiglin.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    EntityDimensions::scalable(0.49, 0.98).with_eye_height(0.78);

impl Mob for ZombifiedPiglinEntity {
    /// Vanilla `ZombifiedPiglin.getAmbientSound`.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        Some(
            if self
                .as_neutral()
                .is_some_and(super::neutral::NeutralMob::is_angry)
            {
                pumpkin_data::sound::Sound::EntityZombifiedPiglinAngry
            } else {
                pumpkin_data::sound::Sound::EntityZombifiedPiglinAmbient
            },
        )
    }

    /// Vanilla `ZombifiedPiglin.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn spawn_as_baby(&self) -> bool {
        self.mob_entity.set_baby_by_age();
        // `ZombifiedPiglin extends Zombie`: the baby speed bonus too.
        super::zombie::set_baby_speed(&self.mob_entity.living_entity, true);
        self.refresh_dimensions();
        true
    }

    /// Vanilla `Zombie.getBaseExperienceReward`, which zombified piglins inherit.
    fn get_base_experience_reward(&self) -> u32 {
        super::zombie::zombie_experience_reward(
            self.get_entity().entity_type.experience_reward,
            self.get_entity().is_baby(),
        )
    }

    fn as_neutral(&self) -> Option<&dyn NeutralMob> {
        Some(self)
    }

    fn populate_default_equipment_slots(
        &self,
        _world: &Arc<World>,
        _difficulty: &RegionalDifficulty,
    ) {
        let living = &self.mob_entity.living_entity;
        let mut equipment = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let weapon = if rand::random_range(0..20) == 0 {
            &Item::GOLDEN_SPEAR
        } else {
            &Item::GOLDEN_SWORD
        };
        equipment.put(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, weapon));
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        let entity = &self.mob_entity.living_entity.entity;
        if !entity.is_alive() {
            return;
        }

        let Some(target) = self.mob_entity.get_target() else {
            self.had_target.store(false, Ordering::Relaxed);
            return;
        };

        // Fresh target: wait out a full interval before spreading the word.
        if !self.had_target.swap(true, Ordering::Relaxed) {
            self.ticks_until_next_alert
                .store(rand::random_range(ALERT_INTERVAL), Ordering::Relaxed);
        }

        self.maybe_alert_others(&target);
    }
}
