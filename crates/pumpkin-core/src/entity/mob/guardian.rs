use crate::entity::ai::control::guardian_move_control::GuardianMoveControl;
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::pathfinder::{Navigator, WaterBoundPathNavigation};
use crate::entity::mob::elder_guardian::ElderGuardianEntity;
use crate::entity::movement_emission::MovementEmission;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, PoisonError, Weak};

use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_data::{damage::DamageType, sound::Sound};
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::Difficulty;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::entity::custom_sound::CustomSound;
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        Controls, Goal, active_target::ActiveTargetGoal, goal_selector::GoalSelector,
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal,
        move_towards_restriction::MoveTowardsRestrictionGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity, monster_walk_target_value},
};

/// Vanilla `Guardian.ATTACK_TIME`, the ticks a laser charges.
const ATTACK_TIME: i32 = 80;
/// Vanilla `ElderGuardian.getAttackDuration`.
const ELDER_ATTACK_TIME: i32 = 60;

/// What vanilla `Guardian` adds to a mob, shared by guardians and elder guardians.
pub struct GuardianData {
    elder: bool,
    /// Vanilla `DATA_ID_MOVING`.
    moving: AtomicBool,
    /// Vanilla `DATA_ID_ATTACK_TARGET`: the laser's target, or 0.
    active_attack_target: AtomicI32,
    /// Vanilla `randomStrollGoal.trigger()`, read by the stroll goal.
    stroll_trigger: Arc<AtomicBool>,
}

impl GuardianData {
    #[must_use]
    pub fn new(elder: bool) -> Self {
        Self {
            elder,
            moving: AtomicBool::new(false),
            active_attack_target: AtomicI32::new(0),
            stroll_trigger: Arc::new(AtomicBool::new(false)),
        }
    }

    #[must_use]
    pub fn is_moving(&self) -> bool {
        self.moving.load(Ordering::Relaxed)
    }

    pub fn set_moving(&self, entity: &Entity, moving: bool) {
        self.moving.store(moving, Ordering::Relaxed);
        entity.set_synced_data(tracked_data::guardian::DATA_ID_MOVING, moving);
    }

    fn set_active_attack_target(&self, entity: &Entity, entity_id: i32) {
        self.active_attack_target
            .store(entity_id, Ordering::Relaxed);
        entity.set_synced_data(
            tracked_data::guardian::DATA_ID_ATTACK_TARGET,
            VarInt(entity_id),
        );
    }

    #[must_use]
    pub fn has_active_attack_target(&self) -> bool {
        self.active_attack_target.load(Ordering::Relaxed) != 0
    }

    /// Vanilla `getAttackDuration`.
    const fn attack_duration(&self) -> i32 {
        if self.elder {
            ELDER_ATTACK_TIME
        } else {
            ATTACK_TIME
        }
    }

    /// Vanilla `Guardian`'s constructor and `registerGoals`; elder guardians stroll every
    /// 400 ticks instead of 80.
    pub fn init(&self, mob_entity: &MobEntity, mob_weak: &Weak<dyn Mob>) {
        let mut navigator =
            Navigator::new(WaterBoundPathNavigation::new(false).steered_by_move_control());
        navigator.set_pathfinding_malus(PathType::Water, 0.0);
        *mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = navigator;
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(GuardianMoveControl::default());
        // The constructor's `clientSideTailAnimation = this.random.nextFloat()`: the value is
        // the client's, but the server draws it too.
        mob_entity.living_entity.entity.random().next_f32();

        let mut goal_selector = mob_entity
            .goals_selector
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut target_selector = mob_entity
            .target_selector
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.register_goals(
            &mut goal_selector,
            &mut target_selector,
            mob_entity,
            mob_weak,
        );
    }

    fn register_goals(
        &self,
        goal_selector: &mut GoalSelector,
        target_selector: &mut GoalSelector,
        mob_entity: &MobEntity,
        mob_weak: &Weak<dyn Mob>,
    ) {
        let move_and_look = Controls::MOVE.union(Controls::LOOK);
        let stroll_interval = if self.elder { 400 } else { 80 };
        goal_selector.add_goal(4, Box::new(GuardianAttackGoal { attack_time: 0 }));
        goal_selector.add_goal(
            5,
            Box::new(MoveTowardsRestrictionGoal::new(1.0).with_controls(move_and_look)),
        );
        goal_selector.add_goal(
            7,
            Box::new(
                WanderAroundGoal::with_interval(1.0, stroll_interval)
                    .with_controls(move_and_look)
                    .with_shared_trigger(self.stroll_trigger.clone()),
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

    /// The server half of vanilla `Guardian.aiStep`: breathe in water, flop about on land,
    /// and face the laser's target.
    pub fn ai_step(&self, mob: &dyn Mob, caller: &dyn EntityBase) {
        let living = &mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        if !living.is_alive() {
            return;
        }
        if entity.is_in_water() {
            living.breath.set_air_supply(caller, 300);
        } else if entity.on_ground.load(Ordering::Relaxed) {
            let (x, z, y_rot) = {
                let mut random = entity.random();
                let x = (random.next_f32() * 2.0 - 1.0) * 0.4;
                let z = (random.next_f32() * 2.0 - 1.0) * 0.4;
                (x, z, random.next_f32() * 360.0)
            };
            let velocity = entity.velocity.load();
            entity.velocity.store(Vector3::new(
                velocity.x + f64::from(x),
                velocity.y + 0.5,
                velocity.z + f64::from(z),
            ));
            entity.yaw.store(y_rot);
            entity.on_ground.store(false, Ordering::Relaxed);
            entity.velocity_dirty.store(true, Ordering::SeqCst);
        }
        if self.has_active_attack_target() {
            entity.yaw.store(entity.head_yaw.load());
        }
    }

    /// Vanilla `Guardian.travelInWater`, which skips the usual water drag and climbing out.
    pub fn travel(&self, mob: &dyn Mob, caller: &dyn EntityBase) -> bool {
        let mob_entity = mob.get_mob_entity();
        let living = &mob_entity.living_entity;
        let entity = &living.entity;
        if !entity.is_in_water() {
            return false;
        }
        entity.update_velocity_from_input(living.movement_input.load(), f64::from(0.1f32));
        living.make_move(caller);
        let mut velocity = entity.velocity.load() * 0.9;
        if !self.is_moving() && mob.get_target().is_none() {
            velocity.y -= 0.005;
        }
        entity.velocity.store(velocity);
        true
    }

    /// The part of vanilla `Guardian.hurtServer` before the damage: spikes hurt whoever
    /// hit a guardian that is keeping still, and getting hit makes it swim off.
    pub fn before_hurt(
        &self,
        guardian: &dyn EntityBase,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
    ) {
        if !self.is_moving()
            && !damage_type.has_tag(&tag::DamageType::MINECRAFT_AVOIDS_GUARDIAN_THORNS)
            && damage_type != DamageType::THORNS
            && let Some(source) = source
            && source.get_living_entity().is_some()
        {
            source.damage_with_context(
                source,
                2.0,
                DamageType::THORNS,
                None,
                Some(guardian),
                Some(guardian),
            );
        }
        self.stroll_trigger.store(true, Ordering::Relaxed);
    }
}

/// The guardian state of a guardian or an elder guardian.
pub fn guardian_data(mob: &dyn Mob) -> Option<&GuardianData> {
    let any = mob.cast_any();
    if let Some(guardian) = any.downcast_ref::<GuardianEntity>() {
        return Some(&guardian.guardian);
    }
    any.downcast_ref::<ElderGuardianEntity>()
        .map(|elder| &elder.guardian)
}

/// Vanilla `Guardian.getWalkTargetValue`: water first, then the darkest spots.
pub fn guardian_walk_target_value(mob: &dyn Mob, pos: &BlockPos) -> f32 {
    let world = mob.get_entity().world.load();
    if world.get_fluid(pos).has_tag(&tag::Fluid::MINECRAFT_WATER) {
        10.0 + (world.get_light_level_dependent_magic_value(pos) - 0.5)
    } else {
        monster_walk_target_value(&world, pos)
    }
}

pub struct GuardianEntity {
    pub mob_entity: MobEntity,
    pub guardian: GuardianData,
}

impl GuardianEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_arc = Arc::new(Self {
            mob_entity: MobEntity::new(entity),
            guardian: GuardianData::new(false),
        });
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };
        mob_arc.guardian.init(&mob_arc.mob_entity, &mob_weak);
        mob_arc
    }
}

impl CustomSound for GuardianEntity {
    fn hurt_sound(&self) -> Option<Sound> {
        Some(if self.get_entity().is_in_water() {
            Sound::EntityGuardianHurt
        } else {
            Sound::EntityGuardianHurtLand
        })
    }

    fn death_sound(&self) -> Option<Sound> {
        Some(if self.get_entity().is_in_water() {
            Sound::EntityGuardianDeath
        } else {
            Sound::EntityGuardianDeathLand
        })
    }
}

impl Mob for GuardianEntity {
    /// Vanilla `Guardian.checkSpawnObstruction`: water doesn't obstruct it.
    fn check_spawn_obstruction(&self, world: &crate::world::World) -> bool {
        let entity = self.get_entity();
        world.is_unobstructed(Some(entity), &entity.bounding_box.load())
    }

    /// Vanilla `Guardian.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        Some(if self.get_entity().is_in_water() {
            Sound::EntityGuardianAmbient
        } else {
            Sound::EntityGuardianAmbientLand
        })
    }

    /// Vanilla `Guardian.getAmbientSoundInterval`.
    fn ambient_sound_interval(&self) -> i32 {
        160
    }

    /// Vanilla `Guardian.getMovementEmission`.
    fn movement_emission(&self) -> MovementEmission {
        MovementEmission::Events
    }

    /// Vanilla `Guardian.getMaxHeadXRot`.
    fn get_max_look_pitch_change(&self) -> f32 {
        180.0
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    fn get_walk_target_value(&self, pos: &BlockPos) -> f32 {
        guardian_walk_target_value(self, pos)
    }

    fn ai_step(&self, caller: &dyn EntityBase) {
        self.guardian.ai_step(self, caller);
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        self.guardian.travel(self, caller)
    }

    fn before_hurt(
        &self,
        _amount: f32,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) {
        self.guardian.before_hurt(self, damage_type, source);
    }
}

/// Vanilla `Guardian.GuardianAttackGoal`: holds still, charges the laser for the attack
/// duration while it can see the target, then zaps it with magic and a melee hit.
struct GuardianAttackGoal {
    attack_time: i32,
}

impl Goal for GuardianAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.get_target().is_some_and(|target| {
            target
                .get_living_entity()
                .is_some_and(crate::entity::living::LivingEntity::is_alive)
        })
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        if !self.can_start(mob) {
            return false;
        }
        let Some(guardian) = guardian_data(mob) else {
            return false;
        };
        guardian.elder
            || mob.get_target().is_some_and(|target| {
                mob.get_entity()
                    .pos
                    .load()
                    .squared_distance_to_vec(&target.get_entity().pos.load())
                    > 9.0
            })
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.attack_time = -10;
        let mob_entity = mob.get_mob_entity();
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
        if let Some(target) = mob.get_target() {
            mob_entity
                .look_control
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .look_at_entity_with_range(&target, 90.0, 90.0);
        }
        mob_entity
            .living_entity
            .entity
            .velocity_dirty
            .store(true, Ordering::SeqCst);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        let Some(guardian) = guardian_data(mob) else {
            return;
        };
        guardian.set_active_attack_target(mob.get_entity(), 0);
        mob.set_mob_target(None);
        guardian.stroll_trigger.store(true, Ordering::Relaxed);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(guardian) = guardian_data(mob) else {
            return;
        };
        let mob_entity = mob.get_mob_entity();
        let entity = &mob_entity.living_entity.entity;
        let Some(target) = mob.get_target() else {
            return;
        };
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
        mob_entity
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .look_at_entity_with_range(&target, 90.0, 90.0);
        if !entity.has_line_of_sight(target.get_entity()) {
            mob.set_mob_target(None);
            return;
        }

        self.attack_time += 1;
        if self.attack_time == 0 {
            guardian.set_active_attack_target(entity, target.get_entity().entity_id);
            if !entity.is_silent() {
                entity.world.load().send_entity_status(
                    entity,
                    EntityStatus::GuardianAttackSound,
                    None,
                );
            }
        } else if self.attack_time >= guardian.attack_duration() {
            let mut magic_damage = 1.0;
            if entity.world.load().level_info.load().difficulty == Difficulty::Hard {
                magic_damage += 2.0;
            }
            if guardian.elder {
                magic_damage += 2.0;
            }
            let caller: &dyn EntityBase = mob;
            target.damage_with_context(
                target.as_ref(),
                magic_damage,
                DamageType::INDIRECT_MAGIC,
                None,
                Some(caller),
                Some(caller),
            );
            mob_entity.try_attack(caller, target.as_ref());
            mob.set_mob_target(None);
        }
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        Controls::MOVE.union(Controls::LOOK)
    }
}
