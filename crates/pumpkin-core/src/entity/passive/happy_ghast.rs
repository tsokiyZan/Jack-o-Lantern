use crate::entity::ai::brain::BrainTick;
use crate::entity::ai::brain::{Brain, memory::PackedMemories};
use crate::entity::ai::control::flying_move_control::FlyingMoveControl;
use crate::entity::ai::control::look_control::LookControl;
use crate::entity::ai::pathfinder::{FlyingPathNavigation, Navigator};
use crate::entity::mob::equipment::is_equippable_in_slot;
use crate::entity::mob::ghast::{
    GhastMoveControl, RAD_TO_DEG, RandomFloatAroundGoal, face_movement_direction,
};
use crate::entity::mob::{first_passenger_player, mob_controlling_passenger, set_ridden_rotation};
use crate::entity::passive::happy_ghast_ai;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::{atan2, wrap_degrees, wrap_degrees_90};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{Controls, Goal, swim::SwimGoal, tempt::TemptGoal},
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

pub const HAPPY_GHAST_FOOD: &[&Item] = &[&Item::SNOWBALL];
/// Vanilla `HappyGhast.WANDER_GROUND_DISTANCE`.
const WANDER_GROUND_DISTANCE: i32 = 16;
/// Vanilla `HappyGhast.SMALL_RESTRICTION_RADIUS`.
const SMALL_RESTRICTION_RADIUS: i32 = 32;
/// Vanilla `HappyGhast.LARGE_RESTRICTION_RADIUS`.
const LARGE_RESTRICTION_RADIUS: i32 = 64;
/// Vanilla `HappyGhast.RESTRICTION_RADIUS_BUFFER`.
const RESTRICTION_RADIUS_BUFFER: i32 = 16;
/// Vanilla `HappyGhast.STILL_TIMEOUT_ON_LOAD_GRACE_PERIOD`.
const STILL_TIMEOUT_ON_LOAD_GRACE_PERIOD: i32 = 60;
/// Vanilla `HappyGhast.MAX_STILL_TIMEOUT`.
const MAX_STILL_TIMEOUT: i32 = 10;
/// The `maxTurn` of the ghastling's `FlyingMoveControl`.
const BABY_MAX_TURN: i32 = 180;
/// `BabyFlyingPathNavigation.setRequiredPathLength`.
const BABY_REQUIRED_PATH_LENGTH: f32 = 48.0;

/// Represents a Happy Ghast, a passive flying mob.
///
/// Wiki: <https://minecraft.wiki/w/Happy_Ghast>
pub struct HappyGhastEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub server_still_timeout: AtomicI32,
    pub leash_holder_time: AtomicI32,
    pub is_leash_holder: AtomicBool,
    pub stays_still: AtomicBool,
}

impl HappyGhastEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(GhastMoveControl::new(true, is_on_still_timeout));
        let happy_ghast = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            server_still_timeout: AtomicI32::new(0),
            leash_holder_time: AtomicI32::new(0),
            is_leash_holder: AtomicBool::new(false),
            stays_still: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(happy_ghast);
        mob_arc.register_goals();
        mob_arc.mob_entity.init_brain(mob_arc.as_ref());
        mob_arc
    }

    /// Vanilla `adultGhastSetup`.
    fn adult_ghast_setup(&self) {
        let mob_entity = &self.mob_entity;
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(GhastMoveControl::new(true, is_on_still_timeout));
        // `HappyGhastLookControl`, through `custom_look_control_tick`.
        *mob_entity
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = LookControl::default();
        *mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Navigator::default();
        // Pumpkin keeps the path costs in the navigation, vanilla on the mob.
        set_animal_pathfinding_malus(mob_entity);
        mob_entity.clear_ai_goals(self);
        self.register_goals();
        mob_entity.with_brain(self, |tick| {
            tick.brain.stop_all(tick.world, tick.mob, tick.time);
            tick.brain.clear_memories();
        });
    }

    /// Vanilla `babyGhastSetup`.
    fn baby_ghast_setup(&self) {
        let mob_entity = &self.mob_entity;
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(FlyingMoveControl::new(BABY_MAX_TURN, true));
        *mob_entity
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = LookControl::default();
        // Vanilla `BabyFlyingPathNavigation`.
        let mut navigator =
            Navigator::new(FlyingPathNavigation::new().moves_directly_through_fluids());
        navigator.set_can_open_doors(false);
        navigator.set_can_float(true);
        *mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = navigator;
        mob_entity.set_required_path_length(BABY_REQUIRED_PATH_LENGTH);
        set_animal_pathfinding_malus(mob_entity);
        self.set_server_still_timeout(0);
        mob_entity.clear_ai_goals(self);
    }

    /// Vanilla `HappyGhast.registerGoals`.
    fn register_goals(&self) {
        let mob_entity = &self.mob_entity;
        mob_entity.add_goal(3, HappyGhastFloatGoal(*SwimGoal::new(mob_entity)));
        mob_entity.add_goal(
            4,
            TemptGoal::for_non_pathfinders(1.0, is_tempt_item, false, 7.0),
        );
        mob_entity.add_goal(5, RandomFloatAroundGoal::new(WANDER_GROUND_DISTANCE));
    }

    /// Vanilla `setServerStillTimeout`. Pumpkin does not send the position sync vanilla
    /// sends when the timeout starts.
    pub fn set_server_still_timeout(&self, timeout: i32) {
        self.server_still_timeout.store(timeout, Ordering::Relaxed);
        self.sync_stay_still_flag();
    }

    /// Vanilla `getHappyGhastRestrictionRadius`.
    fn get_happy_ghast_restriction_radius(&self) -> i32 {
        let body_empty = self
            .mob_entity
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::BODY)
            .is_empty();
        if !self.is_baby() && body_empty {
            LARGE_RESTRICTION_RADIUS
        } else {
            SMALL_RESTRICTION_RADIUS
        }
    }

    /// Vanilla `checkRestriction`: keeps the home on the ghast while it is free.
    fn check_restriction(&self) {
        let mob_entity = &self.mob_entity;
        let entity = self.get_entity();
        if entity.is_leashed() || entity.has_passengers() {
            return;
        }
        let radius = self.get_happy_ghast_restriction_radius();
        let block_pos = entity.block_pos.load();
        let home = mob_entity.position_target.load();
        let buffered = f64::from(radius + RESTRICTION_RADIUS_BUFFER);
        if !mob_entity.has_position_target()
            || dist_sqr(home, block_pos) >= buffered * buffered
            || radius != mob_entity.position_target_range.load(Ordering::Relaxed)
        {
            mob_entity.position_target.store(block_pos);
            mob_entity
                .position_target_range
                .store(radius, Ordering::Relaxed);
        }
    }

    /// Vanilla `scanPlayerAboveGhast`: a player, or what it rides, standing on top.
    fn scan_player_above_ghast(&self) -> bool {
        let bb = self.get_entity().bounding_box.load();
        let detection_box = BoundingBox::new(
            Vector3::new(
                bb.min.x - 1.0,
                bb.max.y - f64::from(1.0E-5f32),
                bb.min.z - 1.0,
            ),
            Vector3::new(
                bb.max.x + 1.0,
                bb.max.y + (bb.max.y - bb.min.y) / 2.0,
                bb.max.z + 1.0,
            ),
        );
        let world = self.get_entity().world.load();
        world.players.load().iter().any(|player| {
            if player.is_spectator() {
                return false;
            }
            let mut root_vehicle: Arc<dyn EntityBase> = player.clone();
            while let Some(vehicle) = root_vehicle.get_entity().get_vehicle() {
                root_vehicle = vehicle;
            }
            root_vehicle.get_entity().entity_type != &EntityType::HAPPY_GHAST
                && detection_box.contains(root_vehicle.get_entity().pos.load())
        })
    }

    /// Vanilla `Mob.isWearingBodyArmor`.
    fn is_wearing_body_armor(&self) -> bool {
        let living = &self.mob_entity.living_entity;
        let stack = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::BODY);
        !stack.is_empty()
            && is_equippable_in_slot(living.entity.entity_type, &stack, &EquipmentSlot::BODY)
    }

    #[must_use]
    pub fn is_on_still_timeout(&self) -> bool {
        self.stays_still.load(Ordering::Relaxed)
            || self.server_still_timeout.load(Ordering::Relaxed) > 0
    }

    fn set_leash_holder(&self, is_leash_holder: bool) {
        self.is_leash_holder
            .store(is_leash_holder, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::happy_ghast::IS_LEASH_HOLDER,
            is_leash_holder,
        );
    }

    fn sync_stay_still_flag(&self) {
        let stays_still = self.server_still_timeout.load(Ordering::Relaxed) > 0;
        self.stays_still.store(stays_still, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::happy_ghast::STAYS_STILL,
            stays_still,
        );
    }
}

impl AgeableMob for HappyGhastEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }

    /// Vanilla `HappyGhast.ageBoundaryReached`.
    fn age_boundary_reached(&self) {
        if self.is_baby() {
            self.baby_ghast_setup();
        } else {
            self.adult_ghast_setup();
        }
    }
}

impl Animal for HappyGhastEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack
            .item
            .has_tag(&tag::Item::MINECRAFT_HAPPY_GHAST_FOOD)
            || HAPPY_GHAST_FOOD.iter().any(|i| i.id == item_stack.item.id)
    }
}

/// Vanilla `HappyGhast.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = Entity::type_dimensions(&EntityType::HAPPY_GHAST)
    .scale(0.2375)
    .with_eye_height(0.468_75);

impl Mob for HappyGhastEntity {
    /// Vanilla `HappyGhast.getWalkTargetValue`: open air just above the ground is best.
    fn get_walk_target_value(&self, pos: &pumpkin_util::math::position::BlockPos) -> f32 {
        let world = self.get_entity().world.load();
        if !world.get_block_state(pos).is_air() {
            0.0
        } else if world.get_block_state(&pos.down()).is_air()
            && !world.get_block_state(&pos.down_height(2)).is_air()
        {
            10.0
        } else {
            5.0
        }
    }

    /// Vanilla `HappyGhast.getAmbientSound`.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        Some(if self.get_entity().is_baby() {
            pumpkin_data::sound::Sound::EntityGhastlingAmbient
        } else {
            pumpkin_data::sound::Sound::EntityHappyGhastAmbient
        })
    }

    /// Vanilla `HappyGhast.getAmbientSoundInterval`: six times as long while ridden.
    fn ambient_sound_interval(&self) -> i32 {
        let interval = super::super::mob::sounds::DEFAULT_AMBIENT_SOUND_INTERVAL;
        if self.get_entity().has_passengers() {
            interval * 6
        } else {
            interval
        }
    }

    /// Vanilla `HappyGhast.getSoundVolume`.
    fn sound_volume(&self) -> f32 {
        if self.get_entity().is_baby() {
            1.0
        } else {
            4.0
        }
    }

    /// Vanilla `HappyGhast.getVoicePitch`.
    fn voice_pitch(&self) -> f32 {
        1.0
    }

    /// Vanilla `HappyGhast.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    /// Vanilla `HappyGhast.canBreatheUnderwater`: ghastlings always can.
    fn can_breathe_underwater(&self) -> bool {
        self.is_baby()
            || self
                .get_entity()
                .entity_type
                .has_tag(&pumpkin_data::tag::EntityType::MINECRAFT_CAN_BREATHE_UNDER_WATER)
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int(
            "still_timeout",
            self.server_still_timeout.load(Ordering::Relaxed),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if let Some(timeout) = nbt.get_int("still_timeout") {
            self.set_server_still_timeout(timeout);
        }
    }

    /// Vanilla `HappyGhast.tickRidden`: turns part of the way to where the rider looks.
    fn tick_ridden(&self, controller: &Player) {
        let entity = self.get_entity();
        let controller_entity = controller.get_entity();
        let y_rot = entity.yaw.load();
        let y_rot = y_rot + wrap_degrees(controller_entity.yaw.load() - y_rot) * 0.08;
        set_ridden_rotation(entity, y_rot, controller_entity.pitch.load() * 0.5);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `HappyGhast.getControllingPassenger`.
    fn get_controlling_passenger(&self) -> Option<Arc<dyn EntityBase>> {
        if self.is_wearing_body_armor()
            && !self.is_on_still_timeout()
            && let Some(player) = first_passenger_player(&self.mob_entity)
        {
            return Some(player);
        }
        mob_controlling_passenger(&self.mob_entity)
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();

        // Continuous healing
        let entity = self.get_entity();
        if entity.is_alive() {
            let living = &self.mob_entity.living_entity;
            let current_health = living.health.load();
            let max_health = living.get_max_health();
            if current_health < max_health {
                let world = entity.world.load();
                let ticks = world.get_world_age();
                let heal_interval = 600;
                if ticks % heal_interval == 0 {
                    living.set_health(current_health + 1.0);
                }
            }
        }
    }

    /// The part of vanilla `HappyGhast.tick` after `super.tick()`.
    fn post_tick(&self) {
        if self.leash_holder_time.load(Ordering::Relaxed) > 0 {
            self.leash_holder_time.fetch_sub(1, Ordering::Relaxed);
        }
        self.set_leash_holder(self.leash_holder_time.load(Ordering::Relaxed) > 0);

        let still_timeout = self.server_still_timeout.load(Ordering::Relaxed);
        if still_timeout > 0 {
            let entity = self.get_entity();
            if entity.tick_count.load(Ordering::Relaxed) > STILL_TIMEOUT_ON_LOAD_GRACE_PERIOD {
                self.server_still_timeout.fetch_sub(1, Ordering::Relaxed);
            }
            self.sync_stay_still_flag();
        }

        if self.scan_player_above_ghast() {
            self.set_server_still_timeout(MAX_STILL_TIMEOUT);
        }
    }

    /// Vanilla `HappyGhast.customServerAiStep`.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if self.is_baby() {
            self.mob_entity.tick_brain(self);
        }
        self.check_restriction();
    }

    fn make_brain(&self, packed: &PackedMemories) -> Brain {
        happy_ghast_ai::HAPPY_GHAST_PROVIDER.make_brain(self, packed)
    }

    /// The `HappyGhastAi.updateActivity` of vanilla `customServerAiStep`.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        happy_ghast_ai::update_activity(tick);
    }

    /// Vanilla `HappyGhast.travel`.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let living = &self.mob_entity.living_entity;
        let speed = living.get_attribute_value(&Attributes::FLYING_SPEED) as f32 * 5.0 / 3.0;
        living.travel_flying_with_speeds(caller, speed, speed, speed);
        true
    }

    /// Vanilla `HappyGhast.checkFallDamage` does nothing.
    fn mob_check_fall_damage(
        &self,
        _caller: &dyn crate::entity::EntityBase,
        _ya: f64,
        _on_ground: bool,
        _state: &'static pumpkin_data::BlockState,
        _pos: pumpkin_util::math::position::BlockPos,
    ) {
    }

    /// Vanilla `HappyGhastLookControl.tick`.
    fn custom_look_control_tick(&self, look_control: &mut LookControl) -> bool {
        // A ghastling has the plain `LookControl`.
        if self.is_baby() {
            return false;
        }
        let entity = self.get_entity();
        if self.is_on_still_timeout() {
            let close_angle = wrap_degrees_90(entity.yaw.load());
            entity.yaw.store(entity.yaw.load() - close_angle);
            entity.head_yaw.store(entity.yaw.load());
        } else if let Some(wanted) = look_control.take_look_at() {
            let pos = entity.pos.load();
            let y_rot = -(atan2(wanted.x - pos.x, wanted.z - pos.z) as f32) * RAD_TO_DEG;
            entity.yaw.store(y_rot);
            entity.body_yaw.store(y_rot);
            entity.head_yaw.store(y_rot);
        } else {
            face_movement_direction(self);
        }
        true
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::happy_ghast::BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::happy_ghast::IS_LEASH_HOLDER,
            self.is_leash_holder.load(Ordering::Relaxed),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::happy_ghast::STAYS_STILL,
            self.stays_still.load(Ordering::Relaxed),
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if self.is_baby() {
            return self.animal_interact(player, item_stack, Sound::EntityGhastlingAmbient);
        }

        self.animal_interact(player, item_stack, Sound::EntityHappyGhastAmbient)
    }
}

/// Vanilla `HappyGhast::isOnStillTimeout`, for the move control's `shouldBeStopped`.
fn is_on_still_timeout(mob: &dyn Mob) -> bool {
    mob.cast_any()
        .downcast_ref::<HappyGhastEntity>()
        .is_some_and(HappyGhastEntity::is_on_still_timeout)
}

/// The item test of the happy ghast's `TemptGoal.ForNonPathfinders`.
fn is_tempt_item(mob: &dyn Mob, stack: &ItemStack) -> bool {
    let Some(happy_ghast) = mob.cast_any().downcast_ref::<HappyGhastEntity>() else {
        return false;
    };
    if !happy_ghast.is_wearing_body_armor() && !happy_ghast.is_baby() {
        stack
            .item
            .has_tag(&tag::Item::MINECRAFT_HAPPY_GHAST_TEMPT_ITEMS)
    } else {
        stack.item.has_tag(&tag::Item::MINECRAFT_HAPPY_GHAST_FOOD)
    }
}

/// Vanilla `Vec3i.distSqr`.
fn dist_sqr(a: BlockPos, b: BlockPos) -> f64 {
    let dx = f64::from(a.0.x) - f64::from(b.0.x);
    let dy = f64::from(a.0.y) - f64::from(b.0.y);
    let dz = f64::from(a.0.z) - f64::from(b.0.z);
    dx * dx + dy * dy + dz * dz
}

/// Vanilla `HappyGhast.HappyGhastFloatGoal`: a `FloatGoal` that rests while the ghast
/// stays still.
struct HappyGhastFloatGoal(SwimGoal);

impl Goal for HappyGhastFloatGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        !is_on_still_timeout(mob) && self.0.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_start(mob)
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.0.tick(mob);
    }

    fn should_run_every_tick(&self) -> bool {
        self.0.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        self.0.controls()
    }
}
