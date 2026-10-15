use std::sync::{Arc, Weak};

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::Difficulty;

use crate::entity::{
    Entity,
    ai::goal::{
        Goal, active_target::ActiveTargetGoal, avoid_entity::AvoidEntityGoal,
        bow_attack::BowAttackGoal, flee_sun::FleeSunGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, melee_attack::MeleeAttackGoal,
        restrict_sun::RestrictSunGoal, revenge::RevengeGoal, wander_around::WanderAroundGoal,
    },
    living::LivingEntity,
    mob::{Mob, MobEntity, equipment::RegionalDifficulty, zombie::baby_turtle_on_land_target},
};
use crate::world::World;

pub mod bogged;
pub mod parched;
#[allow(clippy::module_inception)]
pub mod skeleton;
pub mod stray;
pub mod wither;

pub struct SkeletonEntityBase {
    pub mob_entity: MobEntity,
}

impl SkeletonEntityBase {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let mob = Self { mob_entity };
        let mob_arc = Arc::new(mob);
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

            // Vanilla `AbstractSkeleton.registerGoals`; the weapon goal (priority 4) comes from
            // `reassess_weapon_goal`.
            goal_selector.add_goal(2, Box::new(RestrictSunGoal::new()));
            goal_selector.add_goal(3, Box::new(FleeSunGoal::new(1.0)));
            goal_selector.add_goal(
                3,
                Box::new(AvoidEntityGoal::new(&EntityType::WOLF, 6.0, 1.0, 1.2)),
            );
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::avoiding_water(1.0)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(6, Box::new(RandomLookAroundGoal::default()));

            let mob_entity = &mob_arc.mob_entity;
            if mob_entity.living_entity.entity.entity_type == &EntityType::WITHER_SKELETON {
                // Vanilla `WitherSkeleton.registerGoals`, added before the shared targets.
                target_selector.add_goal(
                    3,
                    ActiveTargetGoal::predicated(mob_entity, 10, true, is_abstract_piglin),
                );
            }
            target_selector.add_goal(1, Box::new(RevengeGoal::new(true)));
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(mob_entity, &EntityType::PLAYER, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(mob_entity, &EntityType::IRON_GOLEM, true),
            );
            target_selector.add_goal(3, Box::new(baby_turtle_on_land_target(mob_entity)));
        };
        // The `AbstractSkeleton` constructor already picks a weapon goal.
        reassess_weapon_goal(mob_arc.as_ref());

        mob_arc
    }
}

/// Vanilla `AbstractPiglin`: piglins and piglin brutes.
fn is_abstract_piglin(target: &LivingEntity, _world: &World) -> bool {
    let entity_type = target.entity.entity_type;
    entity_type == &EntityType::PIGLIN || entity_type == &EntityType::PIGLIN_BRUTE
}

/// Vanilla `AbstractSkeleton.reassessWeaponGoal`: a bow in either hand gives the ranged goal,
/// anything else the melee one, both at priority 4. Runs on spawn, on load and when the mob
/// equips a new item.
pub fn reassess_weapon_goal(mob: &dyn Mob) {
    let mob_entity = mob.get_mob_entity();
    let living = &mob_entity.living_entity;
    let holds_bow = {
        let equipment = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // `ProjectileUtil.getWeaponHoldingHand` prefers the main hand.
        equipment.get(&EquipmentSlot::MAIN_HAND).item == &Item::BOW
            || equipment.get(&EquipmentSlot::OFF_HAND).item == &Item::BOW
    };
    let entity_type = living.entity.entity_type;
    let slow_bow = entity_type == &EntityType::BOGGED || entity_type == &EntityType::PARCHED;
    let hard = living.entity.world.load().level_info.load().difficulty == Difficulty::Hard;
    // `getHardAttackInterval` / `getAttackInterval`.
    let attack_interval = match (slow_bow, hard) {
        (false, true) => 20,
        (false, false) => 40,
        (true, true) => 50,
        (true, false) => 70,
    };

    let mut stopped = {
        let mut goal_selector = mob_entity
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut stopped = goal_selector.remove_goals::<MeleeAttackGoal>();
        stopped.extend(goal_selector.remove_goals::<BowAttackGoal>());
        if holds_bow {
            goal_selector.add_goal(4, Box::new(BowAttackGoal::new(1.0, attack_interval, 15.0)));
        } else {
            goal_selector.add_goal(4, Box::new(MeleeAttackGoal::new(1.2, false)));
        }
        stopped
    };
    for goal in &mut stopped {
        goal.stop(mob);
    }
}

impl Mob for SkeletonEntityBase {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn populate_default_equipment_slots(
        &self,
        _world: &Arc<World>,
        difficulty: &RegionalDifficulty,
    ) {
        // Default armor slots (super.populateDefaultEquipmentSlots)
        if rand::random::<f32>()
            < MobEntity::MAX_WEARING_ARMOR_CHANCE * difficulty.special_multiplier
        {
            let mut armor_type = rand::random_range(0..3);
            for _ in 1..=3 {
                if rand::random::<f32>() < MobEntity::WEARING_ARMOR_UPGRADE_MATERIAL_CHANCE {
                    armor_type += 1;
                }
            }

            let partial_chance = if difficulty.base_difficulty == Difficulty::Hard {
                0.1f32
            } else {
                0.25f32
            };

            let living = &self.mob_entity.living_entity;
            let mut equipment = living
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut first = true;

            for slot in &MobEntity::EQUIPMENT_POPULATION_ORDER {
                let current = equipment.get(slot);
                if !first && rand::random::<f32>() < partial_chance {
                    break;
                }
                first = false;
                if current.is_empty()
                    && let Some(item) = MobEntity::get_equipment_for_slot(slot, armor_type)
                {
                    equipment.put(slot, ItemStack::new(1, item));
                }
            }
        }

        // AbstractSkeleton sets BOW on MAIN_HAND
        let living = &self.mob_entity.living_entity;
        let mut equipment = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        equipment.put(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, &Item::BOW));
    }
}
