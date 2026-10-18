use std::any::TypeId;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::Difficulty;
use rand::RngExt;

use crate::entity::item::ItemEntity;
use crate::entity::mob::spawn::{SpawnGroupData, is_halloween};
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        Goal, PrioritizedGoal,
        active_target::ActiveTargetGoal,
        avoid_entity::AvoidEntityGoal,
        bow_attack::{BowAttackGoal, is_plain_arrow, perform_bow_attack, weapon_holding_hand},
        flee_sun::FleeSunGoal,
        look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal,
        melee_attack::MeleeAttackGoal,
        restrict_sun::RestrictSunGoal,
        revenge::RevengeGoal,
        wander_around::WanderAroundGoal,
    },
    living::LivingEntity,
    mob::{
        Mob, MobEntity,
        equipment::{RegionalDifficulty, equip_item_if_possible},
        zombie::baby_turtle_on_land_target,
    },
};
use crate::world::World;

pub mod bogged;
pub mod parched;
#[allow(clippy::module_inception)]
pub mod skeleton;
pub mod stray;
pub mod wither;

/// The weapon goals' priority in vanilla `reassessWeaponGoal`.
const WEAPON_GOAL_PRIORITY: u8 = 4;

/// Vanilla `AbstractSkeleton`, shared by every skeleton type.
pub struct SkeletonEntityBase {
    pub mob_entity: MobEntity,
    /// Vanilla `bowGoal` and `meleeGoal`, kept here while out of the goal selector so their
    /// state carries over, like the fields vanilla keeps them in.
    bow_goal: Mutex<Option<PrioritizedGoal>>,
    melee_goal: Mutex<Option<PrioritizedGoal>>,
    bow_goal_min_attack_interval: Arc<AtomicI32>,
}

impl SkeletonEntityBase {
    pub fn new(entity: Entity) -> Arc<Self> {
        let bow_goal = BowAttackGoal::new(1.0, 20, 15.0).shooting_with(perform_ranged_attack);
        let bow_goal_min_attack_interval = bow_goal.min_attack_interval();
        let mob_arc = Arc::new(Self {
            mob_entity: MobEntity::new(entity),
            bow_goal: Mutex::new(Some(PrioritizedGoal::new(
                TypeId::of::<BowAttackGoal>(),
                WEAPON_GOAL_PRIORITY,
                Box::new(bow_goal),
            ))),
            melee_goal: Mutex::new(Some(PrioritizedGoal::new(
                TypeId::of::<MeleeAttackGoal>(),
                WEAPON_GOAL_PRIORITY,
                Box::new(MeleeAttackGoal::new(1.2, false)),
            ))),
            bow_goal_min_attack_interval,
        });
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

            // Vanilla `AbstractSkeleton.registerGoals`; the weapon goal comes from
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
        mob_arc.reassess_weapon_goal(mob_arc.as_ref());

        mob_arc
    }

    /// Vanilla `AbstractSkeleton.reassessWeaponGoal`: a bow in the weapon hand gives the
    /// ranged goal, anything else the melee one.
    pub fn reassess_weapon_goal(&self, mob: &dyn Mob) {
        let (melee_goals, bow_goals) = {
            let mut goal_selector = self
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                goal_selector.take_goals::<MeleeAttackGoal>(),
                goal_selector.take_goals::<BowAttackGoal>(),
            )
        };
        for (goals, stash) in [(melee_goals, &self.melee_goal), (bow_goals, &self.bow_goal)] {
            for mut goal in goals {
                if goal.running {
                    goal.stop(mob);
                }
                *stash
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(goal);
            }
        }

        let living = &self.mob_entity.living_entity;
        let (_, used_weapon) = weapon_holding_hand(mob, &Item::BOW);
        let stash = if used_weapon.item == &Item::BOW {
            let interval =
                if living.entity.world.load().level_info.load().difficulty == Difficulty::Hard {
                    hard_attack_interval(living.entity.entity_type)
                } else {
                    attack_interval(living.entity.entity_type)
                };
            self.bow_goal_min_attack_interval
                .store(interval, Ordering::Relaxed);
            &self.bow_goal
        } else {
            &self.melee_goal
        };
        if let Some(goal) = stash
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            self.mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .add_prioritized_goal(goal);
        }
    }

    /// Vanilla `AbstractSkeleton.finalizeSpawn`.
    pub fn finalize_spawn(
        &self,
        mob: &dyn Mob,
        world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        let mob_entity = &self.mob_entity;
        mob_entity.finalize_spawn_base();
        let living = &mob_entity.living_entity;
        let difficulty = RegionalDifficulty::at(world, living.entity.pos.load());
        mob.populate_default_equipment_slots(world, &difficulty);
        mob.populate_default_equipment_enchantments(&difficulty);
        self.reassess_weapon_goal(mob);
        let mut random = mob.get_random();
        mob_entity.set_can_pick_up_loot(
            random.random::<f32>()
                < MobEntity::MAX_PICKUP_LOOT_CHANCE * difficulty.special_multiplier,
        );
        if mob_entity.get_item_slot(&EquipmentSlot::HEAD).is_empty()
            && is_halloween()
            && random.random::<f32>() < 0.25
        {
            let head = if random.random::<f32>() < 0.1 {
                &Item::JACK_O_LANTERN
            } else {
                &Item::CARVED_PUMPKIN
            };
            living
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .put(&EquipmentSlot::HEAD, ItemStack::new(1, head));
            living
                .equipment_drop_chances
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(EquipmentSlot::HEAD, 0.0);
        }
        // The spawn equipment is in place before the mob enters the world, so only its
        // attribute modifiers still need applying.
        living.apply_current_equipment_attribute_modifiers();
        group_data
    }

    /// The looting part of vanilla `Mob.aiStep`, with `AbstractSkeleton.wantsToPickUp`.
    pub fn pick_up_loot(&self, mob: &dyn Mob) {
        let entity_type = self.mob_entity.living_entity.entity.entity_type;
        self.mob_entity.for_each_item_in_pickup_reach(|item| {
            let stack = item
                .get_item_stack()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if wants_to_pick_up(entity_type, &stack) {
                pick_up_item(mob, item, stack);
            }
        });
    }
}

/// Vanilla `Mob.pickUpItem`.
fn pick_up_item(mob: &dyn Mob, item: &ItemEntity, stack: ItemStack) {
    let equipped = equip_item_if_possible(mob, stack);
    if equipped.is_empty() {
        return;
    }
    let count = equipped.item_count;
    // `take` tells the clients before the item goes.
    if !mob
        .get_mob_entity()
        .living_entity
        .pickup(item.get_entity(), u32::from(count))
    {
        return;
    }
    let emptied = {
        let mut item_stack = item
            .get_item_stack()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        item_stack.decrement(count);
        item_stack.is_empty()
    };
    if emptied {
        item.get_entity().remove();
    } else {
        item.init_data_tracker();
    }
}

/// Vanilla `AbstractSkeleton.wantsToPickUp`: anything but spears that the mob can hold.
fn wants_to_pick_up(entity_type: &EntityType, stack: &ItemStack) -> bool {
    !stack.item.has_tag(&tag::Item::MINECRAFT_SPEARS) && can_hold_item(entity_type, stack)
}

/// Vanilla `Mob.canHoldItem`, which `WitherSkeleton` narrows to the weapons it likes.
fn can_hold_item(entity_type: &EntityType, stack: &ItemStack) -> bool {
    entity_type != &EntityType::WITHER_SKELETON
        || !stack
            .item
            .has_tag(&tag::Item::MINECRAFT_WITHER_SKELETON_DISLIKED_WEAPONS)
}

/// Vanilla `AbstractSkeleton.getHardAttackInterval`, which bogged and parched raise.
fn hard_attack_interval(entity_type: &EntityType) -> i32 {
    if entity_type == &EntityType::BOGGED || entity_type == &EntityType::PARCHED {
        50
    } else {
        20
    }
}

/// Vanilla `AbstractSkeleton.getAttackInterval`, which bogged and parched raise.
fn attack_interval(entity_type: &EntityType) -> i32 {
    if entity_type == &EntityType::BOGGED || entity_type == &EntityType::PARCHED {
        70
    } else {
        40
    }
}

/// Vanilla `AbstractSkeleton.performRangedAttack`, with each type's `getArrow`.
fn perform_ranged_attack(mob: &dyn Mob, target: &Arc<dyn EntityBase>, power: f32) {
    let entity_type = mob.get_entity().entity_type;
    perform_bow_attack(mob, target, power, |arrow| {
        if entity_type == &EntityType::WITHER_SKELETON {
            // `igniteForSeconds(100.0F)`.
            arrow.set_flame(true);
            return;
        }
        let effect = if entity_type == &EntityType::STRAY {
            Some((&StatusEffect::SLOWNESS, 600))
        } else if entity_type == &EntityType::BOGGED {
            Some((&StatusEffect::POISON, 100))
        } else if entity_type == &EntityType::PARCHED {
            Some((&StatusEffect::WEAKNESS, 600))
        } else {
            None
        };
        if let Some((effect, duration)) = effect
            && is_plain_arrow(arrow)
        {
            arrow.add_effect(effect, duration);
        }
    });
}

/// Vanilla `AbstractPiglin`: piglins and piglin brutes.
fn is_abstract_piglin(target: &LivingEntity, _world: &World) -> bool {
    let entity_type = target.entity.entity_type;
    entity_type == &EntityType::PIGLIN || entity_type == &EntityType::PIGLIN_BRUTE
}

impl Mob for SkeletonEntityBase {
    /// Vanilla `rideTick`.
    fn mob_after_ride_tick(&self) {
        crate::entity::mob::turn_body_with_controlled_vehicle(self);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `AbstractSkeleton.populateDefaultEquipmentSlots`.
    fn populate_default_equipment_slots(
        &self,
        _world: &Arc<World>,
        difficulty: &RegionalDifficulty,
    ) {
        // `super.populateDefaultEquipmentSlots`: the `Mob` armor roll.
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

        let living = &self.mob_entity.living_entity;
        let mut equipment = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        equipment.put(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, &Item::BOW));
    }
}
