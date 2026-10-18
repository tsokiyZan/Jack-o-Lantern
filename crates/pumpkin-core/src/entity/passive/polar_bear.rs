use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use pumpkin_data::tag;

use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::behavior::neutral::apply_targets,
    ai::goal::{
        active_target::{ActiveTargetGoal, TargetCondition},
        escape_danger::EscapeDangerGoal,
        follow_parent::FollowParentGoal,
        look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal,
        melee_attack::MeleeAttackGoal,
        revenge::RevengeGoal,
        swim::SwimGoal,
        wander_around::WanderAroundGoal,
    },
    ai::util::goal_utils,
    mob::{
        Mob, MobEntity,
        neutral::{NeutralData, NeutralMob},
    },
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

fn is_baby(mob: &dyn Mob) -> bool {
    mob.as_ageable().is_some_and(AgeableMob::is_baby)
}

pub struct PolarBearEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub neutral_data: NeutralData,
    pub standing: AtomicBool,
    /// Vanilla `warningSoundTicks`.
    warning_sound_ticks: AtomicI32,
}

impl PolarBearEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        let polar_bear = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            neutral_data: NeutralData::default(),
            standing: AtomicBool::new(false),
            warning_sound_ticks: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(polar_bear);
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

            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            // `PolarBearMeleeAttackGoal`.
            goal_selector.add_goal(
                1,
                Box::new(
                    MeleeAttackGoal::new(1.25, true)
                        .checking_attack_with(polar_bear_check_and_perform_attack)
                        .stopping_with(|mob| {
                            if let Some(bear) = as_polar_bear(mob) {
                                bear.set_standing(false);
                            }
                        }),
                ),
            );
            goal_selector.add_goal(
                1,
                EscapeDangerGoal::with_panic_causes_for(2.0, |mob| {
                    if is_baby(mob) {
                        &tag::DamageType::MINECRAFT_PANIC_CAUSES
                    } else {
                        &tag::DamageType::MINECRAFT_PANIC_ENVIRONMENTAL_CAUSES
                    }
                }),
            );
            goal_selector.add_goal(4, Box::new(FollowParentGoal::new(1.25)));
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::new(1.0)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(7, Box::new(RandomLookAroundGoal::default()));
        };

        {
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            target_selector.add_goal(
                1,
                Box::new(
                    RevengeGoal::new(true)
                        .raising_alarm_when(is_baby)
                        .alerting_only(|other| !is_baby(other)),
                ),
            );
            // `PolarBearAttackPlayersGoal`: adults go for players while a cub is near.
            target_selector.add_goal(
                2,
                Box::new(ActiveTargetGoal::new(
                    &mob_arc.mob_entity,
                    &EntityType::PLAYER,
                    20,
                    true,
                    true,
                    None::<fn(&crate::entity::living::LivingEntity, &crate::world::World) -> bool>,
                ))
                .starting_only_when(|mob| !is_baby(mob))
                .confirmed_by(cub_nearby)
                .with_follow_distance_scale(0.5),
            );
            apply_targets(&mut target_selector, &mob_arc.mob_entity, 3, 5, false);
            target_selector.add_goal(
                4,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::FOX, true)
                    .must_reach()
                    .when(TargetCondition::Adult),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn is_standing(&self) -> bool {
        self.standing.load(Ordering::Relaxed)
    }

    pub fn set_standing(&self, standing: bool) {
        self.standing.store(standing, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::polar_bear::DATA_STANDING_ID,
            standing,
        );
    }
}

impl PolarBearEntity {
    /// Vanilla `PolarBear.playWarningSound`.
    fn play_warning_sound(&self) {
        if self.warning_sound_ticks.load(Ordering::Relaxed) <= 0 {
            self.make_sound(Sound::EntityPolarBearWarning);
            self.warning_sound_ticks.store(40, Ordering::Relaxed);
        }
    }
}

fn as_polar_bear(mob: &dyn Mob) -> Option<&PolarBearEntity> {
    mob.cast_any().downcast_ref::<PolarBearEntity>()
}

/// `PolarBearAttackPlayersGoal.canUse`: any polar bear cub within 8 blocks (4 vertically).
fn cub_nearby(mob: &dyn Mob) -> bool {
    let entity = mob.get_entity();
    let area = entity.bounding_box.load().expand(8.0, 4.0, 8.0);
    goal_utils::get_entities_of_class(&entity.world.load(), Some(&EntityType::POLAR_BEAR), &area)
        .iter()
        .any(|bear| bear.get_mob().is_some_and(is_baby))
}

/// Vanilla `PolarBearMeleeAttackGoal.checkAndPerformAttack`: rear up and growl when close,
/// swipe (without a swing) when in reach.
fn polar_bear_check_and_perform_attack(
    goal: &mut MeleeAttackGoal,
    mob: &dyn Mob,
    target: &dyn EntityBase,
) {
    let Some(bear) = as_polar_bear(mob) else {
        return;
    };
    if goal.can_perform_attack(mob, target) {
        goal.reset_attack_cooldown();
        let attacker: &dyn EntityBase = mob;
        if mob.get_mob_entity().try_attack(attacker, target) {
            mob.on_attack(target);
        }
        bear.set_standing(false);
        return;
    }
    let width = target.get_entity().entity_dimension.load().width + 3.0;
    let dist_sq = mob
        .get_entity()
        .pos
        .load()
        .squared_distance_to_vec(&target.get_entity().pos.load());
    if dist_sq < f64::from(width * width) {
        if goal.is_time_to_attack() {
            bear.set_standing(false);
            goal.reset_attack_cooldown();
        }
        if goal.ticks_until_next_attack() <= 10 {
            bear.set_standing(true);
            bear.play_warning_sound();
        }
    } else {
        goal.reset_attack_cooldown();
        bear.set_standing(false);
    }
}

impl AgeableMob for PolarBearEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for PolarBearEntity {
    fn is_food(&self, _item_stack: &ItemStack) -> bool {
        false
    }
}

impl NeutralMob for PolarBearEntity {
    fn get_neutral_data(&self) -> &NeutralData {
        &self.neutral_data
    }
}

/// Vanilla `PolarBear.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    pumpkin_data::entity_dimensions::POLAR_BEAR_BABY_DIMENSIONS;

impl Mob for PolarBearEntity {
    /// Vanilla `PolarBear.getAmbientSound`.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        Some(if self.get_entity().is_baby() {
            pumpkin_data::sound::Sound::EntityPolarBearAmbientBaby
        } else {
            pumpkin_data::sound::Sound::EntityPolarBearAmbient
        })
    }

    /// Vanilla `PolarBear.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn as_neutral(&self) -> Option<&dyn NeutralMob> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();
        // Vanilla `PolarBear.tick`.
        let ticks = self.warning_sound_ticks.load(Ordering::Relaxed);
        if ticks > 0 {
            self.warning_sound_ticks.store(ticks - 1, Ordering::Relaxed);
        }
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::polar_bear::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::polar_bear::DATA_STANDING_ID,
            self.is_standing(),
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        self.animal_interact(player, item_stack, Sound::EntityPolarBearAmbient)
    }
}
