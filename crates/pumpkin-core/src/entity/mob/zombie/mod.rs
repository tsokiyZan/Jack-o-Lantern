use super::{Mob, MobEntity};
use crate::entity::ai::goal::break_door::BreakDoorGoal;
use crate::entity::ai::goal::destroy_egg::DestroyEggGoal;
use crate::entity::ai::goal::goal_selector::GoalSelector;
use crate::entity::ai::goal::look_around::RandomLookAroundGoal;
use crate::entity::ai::goal::move_through_village::MoveThroughVillageGoal;
use crate::entity::ai::goal::revenge::RevengeGoal;
use crate::entity::ai::goal::spear_use::SpearUseGoal;
use crate::entity::ai::goal::wander_around::WanderAroundGoal;
use crate::entity::ai::goal::zombie_attack::ZombieAttackGoal;
use crate::entity::attributes::{Modifier, ModifierOperation};
use crate::entity::living::LivingEntity;
use crate::entity::mob::conversion::{ConversionParams, convert_to};
use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::mob::spawn::{RANDOM_SPAWN_BONUS_ID, SpawnGroupData};
use crate::entity::passive::chicken::ChickenEntity;
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{Goal, active_target::ActiveTargetGoal, look_at_entity::LookAtEntityGoal},
};
use crate::world::World;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tracked_data;
use pumpkin_data::world::WorldEvent;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::Difficulty;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_util::random::legacy_rand::LegacyRand;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Weak};

pub mod drowned;
pub mod husk;
#[allow(clippy::module_inception)]
pub mod zombie;
pub mod zombie_villager;

pub struct ZombieEntityBase {
    pub mob_entity: MobEntity,
    pub can_break_doors: AtomicBool,
    pub is_baby: AtomicBool,
    /// The kind's own `BABY_DIMENSIONS`, which vanilla gives each zombie class.
    baby_dimensions: EntityDimensions,
    /// Vanilla `drowningTracker.afflictionTime`.
    in_water_time: AtomicI32,
    /// Vanilla `drowningTracker.conversionTime`.
    drowned_conversion_time: AtomicI32,
    /// Vanilla `DATA_DROWNED_CONVERSION_ID`.
    drowned_converting: AtomicBool,
}

/// Vanilla `Zombie.BABY_DIMENSIONS`.
pub const BABY_DIMENSIONS: EntityDimensions =
    pumpkin_data::entity_dimensions::ZOMBIE_BABY_DIMENSIONS;

/// Vanilla `Zombie.REINFORCEMENT_CALLER_CHARGE_ID`.
const REINFORCEMENT_CALLER_CHARGE_ID: &str = "minecraft:reinforcement_caller_charge";
/// Vanilla `Zombie.ZOMBIE_REINFORCEMENT_CALLEE_CHARGE`'s id.
const REINFORCEMENT_CALLEE_CHARGE_ID: &str = "minecraft:reinforcement_callee_charge";
/// Vanilla `Zombie.LEADER_ZOMBIE_BONUS_ID`.
const LEADER_ZOMBIE_BONUS_ID: &str = "minecraft:leader_zombie_bonus";
/// Vanilla `Zombie.ZOMBIE_RANDOM_SPAWN_BONUS_ID`.
const ZOMBIE_RANDOM_SPAWN_BONUS_ID: &str = "minecraft:zombie_random_spawn_bonus";
/// Vanilla `Zombie.ZOMBIE_LEADER_CHANCE`.
const ZOMBIE_LEADER_CHANCE: f32 = 0.05;
/// Vanilla `Zombie.REINFORCEMENT_ATTEMPTS`.
const REINFORCEMENT_ATTEMPTS: i32 = 50;
/// Vanilla `Zombie.REINFORCEMENT_RANGE_MAX`.
const REINFORCEMENT_RANGE_MAX: i32 = 40;
/// Vanilla `Zombie.REINFORCEMENT_RANGE_MIN`.
const REINFORCEMENT_RANGE_MIN: i32 = 7;
/// Vanilla `Zombie.BREAK_DOOR_CHANCE`.
const BREAK_DOOR_CHANCE: f32 = 0.1;
/// The drowning tracker's `totalAfflictionTime`: ticks with the eyes under water before
/// the conversion starts.
const DROWNING_AFFLICTION_TIME: i32 = 600;
/// The drowning tracker's `totalConversionTime`.
const DROWNED_CONVERSION_TIME: i32 = 300;
/// The drowning tracker's `NOT_CONVERTING`.
const NOT_CONVERTING: i32 = -1;

impl ZombieEntityBase {
    /// Vanilla `Zombie.canHoldItem`: a baby riding something won't take eggs.
    #[must_use]
    pub fn can_hold_item(&self, stack: &ItemStack) -> bool {
        let entity = &self.mob_entity.living_entity.entity;
        !(pumpkin_data::tag::Taggable::has_tag(
            stack.item,
            &pumpkin_data::tag::Item::MINECRAFT_EGGS,
        ) && entity.is_baby()
            && entity.has_vehicle())
    }

    /// Vanilla `Zombie.wantsToPickUp`: anything it can hold but glow ink sacs.
    pub fn zombie_wants_to_pick_up(mob: &dyn Mob, stack: &ItemStack) -> bool {
        stack.item != &Item::GLOW_INK_SAC && mob.can_hold_item(stack)
    }
    pub fn new(entity: Entity) -> Arc<Self> {
        Self::with_can_break_doors(entity, false, BABY_DIMENSIONS)
    }

    pub fn with_can_break_doors(
        entity: Entity,
        can_break_doors: bool,
        baby_dimensions: EntityDimensions,
    ) -> Arc<Self> {
        Self::with_behaviour_goals(
            entity,
            can_break_doors,
            baby_dimensions,
            Self::add_behaviour_goals,
        )
    }

    /// Vanilla `Zombie.registerGoals`, with `add_behaviour_goals` standing in for
    /// `addBehaviourGoals`. The drowned adds its own once it exists.
    pub fn with_behaviour_goals(
        entity: Entity,
        can_break_doors: bool,
        baby_dimensions: EntityDimensions,
        add_behaviour_goals: fn(&MobEntity, &mut GoalSelector, &mut GoalSelector),
    ) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let zombie = Self {
            mob_entity,
            can_break_doors: AtomicBool::new(false),
            is_baby: AtomicBool::new(false),
            baby_dimensions,
            in_water_time: AtomicI32::new(0),
            drowned_conversion_time: AtomicI32::new(0),
            drowned_converting: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(zombie);
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

            goal_selector.add_goal(4, DestroyEggGoal::new(1.0, 3));
            goal_selector.add_goal(
                8,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));
            add_behaviour_goals(
                &mob_arc.mob_entity,
                &mut goal_selector,
                &mut target_selector,
            );
        };

        mob_arc.set_can_break_doors(can_break_doors, &*mob_arc);
        mob_arc
    }

    /// Vanilla `Zombie.addBehaviourGoals`.
    fn add_behaviour_goals(
        mob: &MobEntity,
        goal_selector: &mut GoalSelector,
        target_selector: &mut GoalSelector,
    ) {
        goal_selector.add_goal(2, SpearUseGoal::new(1.0, 1.0, 10.0, 2.0));
        goal_selector.add_goal(3, ZombieAttackGoal::new(1.0, false));
        goal_selector.add_goal(
            6,
            MoveThroughVillageGoal::new(1.0, true, 4, |mob| {
                mob.as_zombie().is_some_and(Self::can_break_doors)
            }),
        );
        goal_selector.add_goal(7, Box::new(WanderAroundGoal::avoiding_water(1.0)));

        target_selector.add_goal(
            1,
            Box::new(
                RevengeGoal::new(true).alerting_others_except(|entity_type| {
                    entity_type == &EntityType::ZOMBIFIED_PIGLIN
                }),
            ),
        );
        target_selector.add_goal(
            2,
            ActiveTargetGoal::with_default(mob, &EntityType::PLAYER, true),
        );
        target_selector.add_goal(
            3,
            ActiveTargetGoal::predicated(mob, 10, false, is_abstract_villager),
        );
        target_selector.add_goal(
            3,
            ActiveTargetGoal::with_default(mob, &EntityType::IRON_GOLEM, true),
        );
        target_selector.add_goal(5, Box::new(baby_turtle_on_land_target(mob)));
    }

    #[must_use]
    pub fn can_break_doors(&self) -> bool {
        self.can_break_doors.load(Ordering::Relaxed)
    }

    /// Vanilla `Zombie.setCanBreakDoors`: door breaking needs ground navigation, which
    /// is also told it may path through doors.
    pub fn set_can_break_doors(&self, can_break_doors: bool, mob: &dyn Mob) {
        let can_break_doors = {
            let mut navigator = self.mob_entity.own_navigation();
            let can_break_doors = can_break_doors && navigator.can_navigate_ground();
            if navigator.can_navigate_ground() {
                navigator.set_can_open_doors(can_break_doors);
            }
            can_break_doors
        };
        if self
            .can_break_doors
            .swap(can_break_doors, Ordering::Relaxed)
            != can_break_doors
        {
            let mut stopped = {
                let mut goal_selector = self
                    .mob_entity
                    .goals_selector
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if can_break_doors {
                    goal_selector.add_goal(1, Box::new(BreakDoorGoal::default()));
                    Vec::new()
                } else {
                    goal_selector.remove_goals::<BreakDoorGoal>()
                }
            };
            for goal in &mut stopped {
                goal.stop(mob);
            }
        }
    }

    const fn entity_type(&self) -> &'static EntityType {
        self.mob_entity.living_entity.entity.entity_type
    }

    /// Vanilla `convertsInWater`: only zombies and husks drown into something else.
    fn converts_in_water(&self) -> bool {
        let entity_type = self.entity_type();
        entity_type == &EntityType::ZOMBIE || entity_type == &EntityType::HUSK
    }

    /// Vanilla `convertsToWhenDrowning`.
    fn converts_to_when_drowning(&self) -> &'static EntityType {
        if self.entity_type() == &EntityType::HUSK {
            &EntityType::ZOMBIE
        } else {
            &EntityType::DROWNED
        }
    }

    /// Vanilla `getConversionSound`.
    fn conversion_sound(&self) -> WorldEvent {
        if self.entity_type() == &EntityType::HUSK {
            WorldEvent::SoundHuskToZombie
        } else {
            WorldEvent::SoundZombieToDrowned
        }
    }

    /// Vanilla `canSpawnInLiquids`.
    fn can_spawn_in_liquids(&self) -> bool {
        self.entity_type() == &EntityType::DROWNED
    }

    /// Vanilla `isUnderWaterConverting`.
    #[must_use]
    pub fn is_under_water_converting(&self) -> bool {
        self.drowned_converting.load(Ordering::Relaxed)
    }

    fn set_drowned_converting(&self, converting: bool) {
        self.drowned_converting.store(converting, Ordering::Relaxed);
        self.mob_entity
            .living_entity
            .entity
            .set_synced_data(tracked_data::zombie::DATA_DROWNED_CONVERSION_ID, converting);
    }

    /// The drowning tracker's `startConversion`.
    fn start_drowned_conversion(&self, time: i32) {
        self.drowned_conversion_time.store(time, Ordering::Relaxed);
        self.set_drowned_converting(true);
    }

    /// Vanilla `Zombie.tick` after `super.tick()`: the drowning tracker's `tick`.
    pub fn tick_drowning(&self, mob: &dyn Mob) {
        let living = &self.mob_entity.living_entity;
        if !self.converts_in_water() || !living.is_alive() || self.mob_entity.is_no_ai() {
            return;
        }
        if living.entity.eye_in_water.load(Ordering::Relaxed) {
            if self.is_under_water_converting() {
                if self.drowned_conversion_time.fetch_sub(1, Ordering::Relaxed) - 1 < 0 {
                    self.do_underwater_conversion(mob);
                }
            } else if self.in_water_time.fetch_add(1, Ordering::Relaxed) + 1
                >= DROWNING_AFFLICTION_TIME
            {
                self.start_drowned_conversion(DROWNED_CONVERSION_TIME);
            }
        } else {
            self.in_water_time.store(-1, Ordering::Relaxed);
            self.set_drowned_converting(false);
        }
    }

    /// The drowning tracker's `doConversion`.
    fn do_underwater_conversion(&self, mob: &dyn Mob) {
        let sound = self.conversion_sound();
        convert_to(
            mob,
            self.converts_to_when_drowning(),
            &ConversionParams::single(mob, true, true),
            |converted| {
                let entity = converted.get_entity();
                let world = entity.world.load_full();
                if let Some(zombie) = converted.as_zombie() {
                    let difficulty = RegionalDifficulty::at(&world, entity.pos.load());
                    zombie.handle_attributes(converted, difficulty.special_multiplier, true);
                }
                if !entity.is_silent() {
                    world.sync_world_event(sound, entity.block_pos.load(), 0);
                }
            },
        );
    }

    /// Vanilla `Zombie.hurtServer`: on hard difficulty a hurt zombie may call another of its
    /// kind to fight its target. `mob_hurt_server` has already applied the damage.
    pub fn summon_reinforcements(&self, _mob: &dyn Mob, cause: Option<&dyn EntityBase>) {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        let world = entity.world.load_full();
        let Some(target) = self.get_target().or_else(|| {
            cause
                .filter(|cause| cause.get_living_entity().is_some())
                .and_then(|cause| world.get_entity_by_id(cause.get_entity().entity_id))
        }) else {
            return;
        };
        if world.level_info.load().difficulty != Difficulty::Hard
            || f64::from(entity.random().next_f32())
                >= living.get_attribute_value(&Attributes::SPAWN_REINFORCEMENTS)
            || !world.should_spawn_monsters()
        {
            return;
        }

        let pos = entity.pos.load();
        let (x, y, z) = (
            pos.x.floor() as i32,
            pos.y.floor() as i32,
            pos.z.floor() as i32,
        );
        let entity_type = entity.entity_type;
        let reinforcement =
            crate::entity::r#type::from_type(entity_type, pos, &world, uuid::Uuid::new_v4());
        let Some(reinforcement_mob) = reinforcement.get_mob() else {
            return;
        };
        // `Mth.nextInt(this.random, 7, 40) * Mth.nextInt(this.random, -1, 1)`.
        let offset = || {
            let mut random = entity.random();
            random.next_inbetween_i32(REINFORCEMENT_RANGE_MIN, REINFORCEMENT_RANGE_MAX)
                * random.next_inbetween_i32(-1, 1)
        };

        for _ in 0..REINFORCEMENT_ATTEMPTS {
            let xt = x + offset();
            let yt = y + offset();
            let zt = z + offset();
            let spawn_pos = BlockPos::new(xt, yt, zt);
            if !crate::world::natural_spawner::is_spawn_position_ok(&world, &spawn_pos, entity_type)
                || !crate::entity::r#type::check_spawn_rules(
                    entity_type,
                    &world,
                    &spawn_pos,
                    world.is_thundering(),
                    crate::entity::mob::spawn::EntitySpawnReason::Reinforcement,
                    &mut world
                        .random
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                )
            {
                continue;
            }
            let spawn_pos = Vector3::new(f64::from(xt), f64::from(yt), f64::from(zt));
            let reinforcement_entity = reinforcement.get_entity();
            reinforcement_entity.set_pos(spawn_pos);
            let bounding_box = reinforcement_entity.bounding_box.load();
            if world.has_nearby_alive_player(spawn_pos, 7.0)
                || !world.is_unobstructed(Some(reinforcement_entity), &bounding_box)
                || !world.is_space_empty(bounding_box)
                || (!self.can_spawn_in_liquids() && world.contains_any_liquid(bounding_box))
            {
                continue;
            }
            reinforcement_mob.set_mob_target(Some(target));
            reinforcement_mob.finalize_spawn(
                &world,
                crate::entity::mob::spawn::EntitySpawnReason::Reinforcement,
                None,
            );
            world.spawn_entity(reinforcement.clone());
            living.update_attribute(&Attributes::SPAWN_REINFORCEMENTS, |instance| {
                let existing_amount = instance
                    .get_modifier(REINFORCEMENT_CALLER_CHARGE_ID)
                    .map_or(0.0, |modifier| modifier.amount);
                instance.add_or_replace_permanent_modifier(Modifier {
                    id: REINFORCEMENT_CALLER_CHARGE_ID.to_string(),
                    amount: existing_amount - 0.05,
                    operation: ModifierOperation::Add,
                });
            });
            reinforcement_mob
                .get_mob_entity()
                .living_entity
                .update_attribute(&Attributes::SPAWN_REINFORCEMENTS, |instance| {
                    instance.add_or_replace_permanent_modifier(Modifier {
                        id: REINFORCEMENT_CALLEE_CHARGE_ID.to_string(),
                        amount: f64::from(-0.05f32),
                        operation: ModifierOperation::Add,
                    });
                });
            break;
        }
    }

    /// Vanilla `Zombie.doHurtTarget` after a hit lands: a burning, empty-handed zombie
    /// may set its target alight.
    pub fn on_attack(&self, _mob: &dyn Mob, target: &dyn EntityBase) {
        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load_full();
        let difficulty = RegionalDifficulty::at(&world, entity.pos.load()).effective_difficulty;
        if self
            .mob_entity
            .get_item_slot(&EquipmentSlot::MAIN_HAND)
            .is_empty()
            && entity.is_on_fire()
            && entity.random().next_f32() < difficulty * 0.3
        {
            target.set_on_fire_for((2 * difficulty as i32) as f32);
        }
    }

    /// Vanilla `Zombie.killedEntity`: on normal and hard difficulty a killed villager may
    /// rise again as a zombie villager, which leaves no loot.
    pub fn killed_entity(&self, _mob: &dyn Mob, victim: &dyn EntityBase) -> bool {
        let entity = &self.mob_entity.living_entity.entity;
        let difficulty = entity.world.load().level_info.load().difficulty;
        if !matches!(difficulty, Difficulty::Normal | Difficulty::Hard) {
            return true;
        }
        let Some(villager) = victim
            .get_mob()
            .filter(|victim| victim.get_entity().entity_type == &EntityType::VILLAGER)
        else {
            return true;
        };
        if difficulty != Difficulty::Hard && entity.random().next_bool() {
            return true;
        }
        !self.convert_villager_to_zombie_villager(villager)
    }

    /// Vanilla `Zombie.convertVillagerToZombieVillager`.
    fn convert_villager_to_zombie_villager(&self, villager: &dyn Mob) -> bool {
        let mut villager_nbt = NbtCompound::new();
        villager.mob_write_nbt(&mut villager_nbt);
        let entity = &self.mob_entity.living_entity.entity;
        let silent = entity.is_silent();
        let block_pos = entity.block_pos.load();
        convert_to(
            villager,
            &EntityType::ZOMBIE_VILLAGER,
            &ConversionParams::single(villager, true, true),
            |converted| {
                let Some(zombie_villager) = converted
                    .cast_any()
                    .downcast_ref::<zombie_villager::ZombieVillagerEntity>(
                ) else {
                    return;
                };
                let world = converted.get_entity().world.load_full();
                zombie_villager.convert_from_villager(&world, &villager_nbt);
                if !silent {
                    world.sync_world_event(WorldEvent::SoundZombieInfected, block_pos, 0);
                }
            },
        )
        .is_some()
    }

    /// Vanilla `Zombie.finalizeSpawn`. `conversion` stands for `EntitySpawnReason.CONVERSION`.
    pub fn finalize_zombie_spawn(
        &self,
        mob: &dyn Mob,
        world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
        conversion: bool,
    ) -> Option<SpawnGroupData> {
        self.mob_entity.finalize_spawn_base();
        let entity = &self.mob_entity.living_entity.entity;
        let difficulty = RegionalDifficulty::at(world, entity.pos.load());
        let difficulty_modifier = difficulty.special_multiplier;
        // `level.getRandom()`, locked per draw: the jockey and equipment code below draws
        // from it too.
        if !conversion {
            let can_pick_up_loot = level_random(world).next_f32() < 0.55 * difficulty_modifier;
            self.mob_entity.set_can_pick_up_loot(can_pick_up_loot);
        }

        let group_data = group_data.unwrap_or_else(|| SpawnGroupData::Zombie {
            is_baby: get_spawn_as_baby_odds(&mut *level_random(world)),
            can_spawn_jockey: true,
        });
        if let SpawnGroupData::Zombie {
            is_baby,
            can_spawn_jockey,
        } = group_data
        {
            if is_baby {
                self.set_baby(true);
                if can_spawn_jockey {
                    self.ride_chicken_jockey(world);
                }
            }

            let can_break_doors =
                level_random(world).next_f32() < difficulty_modifier * BREAK_DOOR_CHANCE;
            self.set_can_break_doors(can_break_doors, mob);
            if !conversion {
                mob.populate_default_equipment_slots(world, &difficulty);
                mob.populate_default_equipment_enchantments(world, &difficulty);
            }
        }

        if self
            .mob_entity
            .get_item_slot(&EquipmentSlot::HEAD)
            .is_empty()
            && is_halloween()
            && level_random(world).next_f32() < 0.25
        {
            let pumpkin = if level_random(world).next_f32() < 0.1 {
                &Item::JACK_O_LANTERN
            } else {
                &Item::CARVED_PUMPKIN
            };
            self.mob_entity
                .set_item_slot(&EquipmentSlot::HEAD, ItemStack::new(1, pumpkin));
            self.mob_entity
                .living_entity
                .equipment_drop_chances
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(EquipmentSlot::HEAD, 0.0);
        }

        self.handle_attributes(mob, difficulty_modifier, conversion);
        Some(group_data)
    }

    /// The chicken jockey part of vanilla `Zombie.finalizeSpawn`: a baby zombie may take
    /// over a nearby chicken, or bring its own.
    #[expect(
        clippy::same_functions_in_if_condition,
        reason = "vanilla rolls the second chance afresh"
    )]
    fn ride_chicken_jockey(&self, world: &Arc<World>) {
        let entity = &self.mob_entity.living_entity.entity;
        if f64::from(level_random(world).next_f32()) < 0.05 {
            let search_box = entity.bounding_box.load().expand(5.0, 3.0, 5.0);
            // `EntitySelector.ENTITY_NOT_BEING_RIDDEN`.
            let chicken = world
                .get_entities_at_box(&search_box)
                .into_iter()
                .find(|other| {
                    let other_entity = other.get_entity();
                    other_entity.entity_type == &EntityType::CHICKEN
                        && other
                            .get_living_entity()
                            .is_some_and(LivingEntity::is_alive)
                        && !other_entity.has_passengers()
                        && !other_entity.has_vehicle()
                });
            if let Some(chicken) = chicken {
                if let Some(chicken_entity) = chicken.cast_any().downcast_ref::<ChickenEntity>() {
                    chicken_entity.set_chicken_jockey(true);
                }
                self.mob_entity.set_pending_vehicle(chicken);
            }
        } else if f64::from(level_random(world).next_f32()) < 0.05 {
            let chicken = crate::entity::r#type::from_type(
                &EntityType::CHICKEN,
                entity.pos.load(),
                world,
                uuid::Uuid::new_v4(),
            );
            chicken.get_entity().set_rotation(entity.yaw.load(), 0.0);
            crate::entity::mob::spawn::finalize_spawn(
                &chicken,
                world,
                crate::entity::mob::spawn::EntitySpawnReason::Jockey,
                None,
            );
            if let Some(chicken_entity) = chicken.cast_any().downcast_ref::<ChickenEntity>() {
                chicken_entity.set_chicken_jockey(true);
            }
            self.mob_entity.set_pending_vehicle(chicken.clone());
            world.spawn_entity(chicken);
        }
    }

    /// Vanilla `Zombie.handleAttributes`. `conversion` stands for `EntitySpawnReason.CONVERSION`,
    /// the only reason Pumpkin runs it with that keeps the health as it is.
    pub fn handle_attributes(&self, mob: &dyn Mob, difficulty_modifier: f32, conversion: bool) {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        self.randomize_reinforcements_chance();
        let knockback_bonus = entity.random().next_f64() * f64::from(0.05f32);
        living.update_attribute(&Attributes::KNOCKBACK_RESISTANCE, |instance| {
            instance.add_or_replace_permanent_modifier(Modifier {
                id: RANDOM_SPAWN_BONUS_ID.to_string(),
                amount: knockback_bonus,
                operation: ModifierOperation::Add,
            });
        });
        let follow_range_modifier =
            entity.random().next_f64() * 1.5 * f64::from(difficulty_modifier);
        if follow_range_modifier > 1.0 {
            living.update_attribute(&Attributes::FOLLOW_RANGE, |instance| {
                instance.add_or_replace_permanent_modifier(Modifier {
                    id: ZOMBIE_RANDOM_SPAWN_BONUS_ID.to_string(),
                    amount: follow_range_modifier,
                    operation: ModifierOperation::MultiplyTotal,
                });
            });
            // Pumpkin's eager stand-in for the next tick's `refreshDirtyAttributes` →
            // `Mob.onAttributeUpdated` → `getNavigation()` (LivingEntity.java:2860, Mob.java:1449).
            self.mob_entity.with_navigation(|navigation, owner| {
                navigation.update_pathfinder_max_visited_nodes(&owner.living_entity);
            });
        }

        if entity.random().next_f32() < difficulty_modifier * ZOMBIE_LEADER_CHANCE {
            let reinforcement_bonus = entity.random().next_f64() * 0.25 + 0.5;
            living.update_attribute(&Attributes::SPAWN_REINFORCEMENTS, |instance| {
                instance.add_or_replace_permanent_modifier(Modifier {
                    id: LEADER_ZOMBIE_BONUS_ID.to_string(),
                    amount: reinforcement_bonus,
                    operation: ModifierOperation::Add,
                });
            });
            let health_bonus = entity.random().next_f64() * 3.0 + 1.0;
            living.update_attribute(&Attributes::MAX_HEALTH, |instance| {
                instance.add_or_replace_permanent_modifier(Modifier {
                    id: LEADER_ZOMBIE_BONUS_ID.to_string(),
                    amount: health_bonus,
                    operation: ModifierOperation::MultiplyTotal,
                });
            });
            if !conversion {
                living.set_health(living.get_max_health());
            }
            self.set_can_break_doors(true, mob);
        }
    }

    /// Vanilla `randomizeReinforcementsChance`; zombified piglins never call for help on
    /// their own.
    fn randomize_reinforcements_chance(&self) {
        let chance = if self.entity_type() == &EntityType::ZOMBIFIED_PIGLIN {
            0.0
        } else {
            self.mob_entity.living_entity.entity.random().next_f64() * f64::from(0.1f32)
        };
        self.mob_entity
            .living_entity
            .set_attribute_base(&Attributes::SPAWN_REINFORCEMENTS, chance);
    }

    /// Vanilla `Zombie.addAdditionalSaveData`.
    pub fn write_zombie_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("IsBaby", self.is_baby());
        nbt.put_bool("CanBreakDoors", self.can_break_doors());
        // The drowning tracker's `addAdditionalSaveData`.
        nbt.put_int(
            "DrownedConversionTime",
            if self.is_under_water_converting() {
                self.drowned_conversion_time.load(Ordering::Relaxed)
            } else {
                NOT_CONVERTING
            },
        );
        let afflicted = self
            .mob_entity
            .living_entity
            .entity
            .eye_in_water
            .load(Ordering::Relaxed);
        nbt.put_int(
            "InWaterTime",
            if afflicted {
                self.in_water_time.load(Ordering::Relaxed)
            } else {
                -1
            },
        );
    }

    /// Vanilla `Zombie.readAdditionalSaveData`.
    pub fn read_zombie_nbt(&self, mob: &dyn Mob, nbt: &NbtCompound) {
        self.set_baby(nbt.get_bool("IsBaby").unwrap_or(false));
        self.set_can_break_doors(nbt.get_bool("CanBreakDoors").unwrap_or(false), mob);
        self.in_water_time
            .store(nbt.get_int("InWaterTime").unwrap_or(0), Ordering::Relaxed);
        let conversion_time = nbt
            .get_int("DrownedConversionTime")
            .unwrap_or(NOT_CONVERTING);
        if conversion_time == NOT_CONVERTING {
            self.set_drowned_converting(false);
        } else {
            self.start_drowned_conversion(conversion_time);
        }
    }
}

/// Vanilla `Zombie.getSpawnAsBabyOdds`.
fn get_spawn_as_baby_odds(random: &mut impl RandomImpl) -> bool {
    random.next_f32() < 0.05
}

/// `level.getRandom()`. Lock it for one draw at a time: spawn code it calls draws from it too.
fn level_random(world: &World) -> std::sync::MutexGuard<'_, LegacyRand> {
    world
        .random
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Vanilla `SpecialDates.isHalloween`, in the server's local time.
fn is_halloween() -> bool {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    now.month() == time::Month::October && now.day() == 31
}

/// Vanilla targets `AbstractVillager`: villagers and wandering traders.
pub(crate) fn is_abstract_villager(target: &LivingEntity, _world: &World) -> bool {
    let entity_type = target.entity.entity_type;
    entity_type == &EntityType::VILLAGER || entity_type == &EntityType::WANDERING_TRADER
}

/// Vanilla's turtle target for zombies and drowned: only baby turtles on land
/// (`Turtle.BABY_ON_LAND_SELECTOR`), checked one tick in ten.
pub(crate) fn baby_turtle_on_land_target(mob: &MobEntity) -> ActiveTargetGoal {
    ActiveTargetGoal::new(
        mob,
        &EntityType::TURTLE,
        10,
        true,
        false,
        Some(|target: &LivingEntity, _world: &World| {
            target.entity.age.load(Ordering::Relaxed) < 0 && !target.is_in_water()
        }),
    )
}

/// Vanilla `Zombie.SPEED_MODIFIER_BABY_ID`.
const SPEED_MODIFIER_BABY_ID: &str = "minecraft:baby";

/// The speed half of vanilla `Zombie.setBaby`: +50% of base speed while a baby.
pub fn set_baby_speed(living: &LivingEntity, baby: bool) {
    living.update_attribute(
        &pumpkin_data::attributes::Attributes::MOVEMENT_SPEED,
        |inst| {
            inst.remove_modifier(SPEED_MODIFIER_BABY_ID);
            if baby {
                inst.add_or_replace_modifier(crate::entity::attributes::Modifier {
                    id: SPEED_MODIFIER_BABY_ID.to_string(),
                    amount: 0.5,
                    operation: crate::entity::attributes::ModifierOperation::MultiplyBase,
                });
            }
        },
    );
    crate::entity::attributes::send_attribute_updates_for_living(
        living,
        vec![pumpkin_data::attributes::Attributes::MOVEMENT_SPEED],
    );
}

/// Vanilla `Zombie.getBaseExperienceReward`'s `xpReward`: babies are worth two and a half times
/// as much.
#[must_use]
pub fn zombie_experience_reward(base: u32, baby: bool) -> u32 {
    if baby {
        (f64::from(base) * 2.5) as u32
    } else {
        base
    }
}

impl ZombieEntityBase {
    /// Vanilla `Zombie.setBaby`: the baby state is only the synced flag, it never ages up,
    /// and babies are half again as fast.
    pub fn set_baby(&self, baby: bool) {
        self.mob_entity.set_baby_flag(
            &self.is_baby,
            pumpkin_data::tracked_data::zombie::BABY,
            baby,
        );
        set_baby_speed(&self.mob_entity.living_entity, baby);
        self.refresh_dimensions();
    }

    #[must_use]
    pub fn is_baby(&self) -> bool {
        self.is_baby.load(Ordering::Relaxed)
    }
}

impl Mob for ZombieEntityBase {
    /// Vanilla `Zombie.getDefaultDimensions`, with the kind's own baby size.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            self.baby_dimensions
        } else {
            entity.get_dimensions(pose)
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_zombie(&self) -> Option<&ZombieEntityBase> {
        Some(self)
    }

    fn spawn_as_baby(&self) -> bool {
        self.set_baby(true);
        true
    }

    /// Vanilla `Zombie.populateDefaultEquipmentSlots`.
    fn populate_default_equipment_slots(
        &self,
        world: &Arc<World>,
        difficulty: &RegionalDifficulty,
    ) {
        crate::entity::mob::populate_default_armor(self, world, difficulty);

        let weapon_chance = if difficulty.base_difficulty == Difficulty::Hard {
            0.05f32
        } else {
            0.01f32
        };
        if level_random(world).next_f32() < weapon_chance {
            let r = level_random(world).next_bounded_i32(6);
            let weapon_item = match r {
                0 => &Item::IRON_SWORD,
                1 => &Item::IRON_SPEAR,
                _ => &Item::IRON_SHOVEL,
            };
            let living = &self.mob_entity.living_entity;
            let mut equipment = living
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            equipment.put(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, weapon_item));
        }
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_zombie_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_zombie_nbt(self, nbt);
    }
}

/// The vanilla `Zombie` overrides every zombie-family mob shares, delegated to its
/// `ZombieEntityBase` field.
#[macro_export]
macro_rules! impl_zombie_mob {
    ($field:ident) => {
        fn as_zombie(&self) -> Option<&$crate::entity::mob::zombie::ZombieEntityBase> {
            Some(&self.$field)
        }

        fn default_dimensions(
            &self,
            pose: pumpkin_data::entity::EntityPose,
        ) -> pumpkin_util::math::boundingbox::EntityDimensions {
            self.$field.default_dimensions(pose)
        }

        fn spawn_as_baby(&self) -> bool {
            self.$field.spawn_as_baby()
        }

        /// Vanilla `Zombie.getBaseExperienceReward`, which raises `xpReward` before `super`.
        fn xp_reward(&self) -> u32 {
            $crate::entity::mob::zombie::zombie_experience_reward(
                self.get_entity().entity_type.experience_reward,
                self.$field.is_baby(),
            )
        }

        /// Vanilla `Zombie.tick` after `super.tick()`.
        fn post_tick(&self) {
            self.$field.tick_drowning(self);
        }

        /// Vanilla `Zombie.hurtServer`.
        fn hurt_server(
            &self,
            caller: &dyn $crate::entity::EntityBase,
            amount: f32,
            damage_type: pumpkin_data::damage::DamageType,
            position: Option<pumpkin_util::math::vector3::Vector3<f64>>,
            source: Option<&dyn $crate::entity::EntityBase>,
            cause: Option<&dyn $crate::entity::EntityBase>,
        ) -> bool {
            if !$crate::entity::mob::mob_hurt_server(
                self,
                caller,
                amount,
                damage_type,
                position,
                source,
                cause,
            ) {
                return false;
            }
            self.$field.summon_reinforcements(self, cause);
            true
        }

        fn killed_entity(&self, victim: &dyn $crate::entity::EntityBase) -> bool {
            self.$field.killed_entity(self, victim)
        }

        fn can_hold_item(&self, stack: &pumpkin_data::item_stack::ItemStack) -> bool {
            self.$field.can_hold_item(stack)
        }
    };
}
