use std::sync::{Arc, Weak};

use crate::entity::ai::goal::avoid_entity::AvoidEntityGoal;
use crate::entity::passive::armadillo::armadillo_not_scared;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::potion::Effect;

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::{ActiveTargetGoal, TargetCondition},
        leap_at_target::LeapAtTargetGoal,
        look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal,
        revenge::RevengeGoal,
        spider_attack::SpiderAttackGoal,
        swim::SwimGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
};

pub struct CaveSpiderEntity {
    pub mob_entity: MobEntity,
    /// Vanilla `Spider.isClimbing`, the spider data flag.
    pub is_climbing: std::sync::atomic::AtomicBool,
}

impl CaveSpiderEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let cave_spider = Self {
            mob_entity,
            is_climbing: std::sync::atomic::AtomicBool::new(false),
        };
        let mob_arc = Arc::new(cave_spider);
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

            goal_selector.add_goal(1, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(
                2,
                Box::new(
                    AvoidEntityGoal::new(&EntityType::ARMADILLO, 6.0, 1.0, 1.2)
                        .avoiding_only(armadillo_not_scared),
                ),
            );
            goal_selector.add_goal(3, Box::new(LeapAtTargetGoal::new(0.4)));
            goal_selector.add_goal(4, SpiderAttackGoal::new(1.0, true));
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::avoiding_water(0.8)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(6, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(1, Box::new(RevengeGoal::new(true)));
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true)
                    .when(TargetCondition::NoDaylight),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::IRON_GOLEM, true)
                    .when(TargetCondition::NoDaylight),
            );
        };

        mob_arc
    }
}

impl Mob for CaveSpiderEntity {
    /// Vanilla `Spider.tick`: a spider climbs while it walks into a wall.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        let entity = &self.mob_entity.living_entity.entity;
        if !entity.is_alive() {
            return;
        }
        let climbing = entity
            .horizontal_collision
            .load(std::sync::atomic::Ordering::Relaxed);
        if self
            .is_climbing
            .swap(climbing, std::sync::atomic::Ordering::Relaxed)
            != climbing
        {
            entity.set_synced_data(
                pumpkin_data::tracked_data::spider::DATA_FLAGS_ID,
                i8::from(climbing),
            );
        }
    }

    /// Vanilla `Spider.onClimbable`.
    fn mob_on_climbable(&self) -> Option<bool> {
        Some(self.is_climbing.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn finalize_spawn(
        &self,
        world: &Arc<crate::world::World>,
        group_data: Option<super::spawn::SpawnGroupData>,
    ) -> Option<super::spawn::SpawnGroupData> {
        super::spider::finalize_spider_spawn(&self.mob_entity, world, group_data)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn on_attack(&self, target: &dyn EntityBase) {
        if let Some(living) = target.get_living_entity() {
            let world = self.mob_entity.living_entity.entity.world.load();
            let duration = match world.level_info.load().difficulty {
                pumpkin_util::Difficulty::Normal => 7 * 20,
                pumpkin_util::Difficulty::Hard => 15 * 20,
                _ => 0,
            };
            if duration > 0 {
                living.add_effect(Effect {
                    effect_type: &StatusEffect::POISON,
                    duration,
                    amplifier: 0,
                    ambient: false,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                });
            }
        }
    }
}
