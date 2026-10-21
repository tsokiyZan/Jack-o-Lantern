use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering},
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::block_properties::{DoubleBlockHalf, TallSeagrassLikeProperties};
use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockState};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::world::BlockFlags;

use crate::block::BonemealArgs;
use crate::block::blocks::plant::crop::next_age_state;
use crate::block::entities::beehive::BeehiveBlockEntity;
use crate::entity::ai::control::flying_move_control::FlyingMoveControl;
use crate::entity::ai::goal::{Controls, Goal};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::pathfinder::path::Path;
use crate::entity::ai::pathfinder::{FlyingPathNavigation, Navigator};
use crate::entity::ai::util::{air_and_water_random_pos, air_random_pos, hover_random_pos};
use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        active_target::{ActiveTargetGoal, TargetCondition},
        breed::BreedGoal,
        follow_parent::FollowParentGoal,
        melee_attack::MeleeAttackGoal,
        reset_universal_anger::ResetUniversalAngerGoal,
        revenge::RevengeGoal,
        swim::SwimGoal,
        tempt::TemptGoal,
    },
    mob::{
        Mob, MobEntity,
        neutral::{NeutralData, NeutralMob},
    },
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};
use crate::world::World;
use crate::world::bee_poi;

pub const FLAG_ROLL: u8 = 2;
pub const FLAG_HAS_STUNG: u8 = 4;
pub const FLAG_HAS_NECTAR: u8 = 8;
const STING_DEATH_COUNTDOWN: i32 = 1200;
const TICKS_BEFORE_GOING_TO_KNOWN_FLOWER: i32 = 600;
const TICKS_WITHOUT_NECTAR_BEFORE_GOING_HOME: i32 = 3600;
/// How close, squared, the target has to be for an angry bee to roll.
const ROLL_RANGE_SQR: f64 = 4.0;
const MAX_CROPS_GROWABLE: i32 = 10;
const POISON_SECONDS_NORMAL: i32 = 10;
const POISON_SECONDS_HARD: i32 = 18;
const TOO_FAR_DISTANCE: i32 = 48;
const HIVE_CLOSE_ENOUGH_DISTANCE: i32 = 2;
const RESTRICTED_WANDER_DISTANCE_REDUCTION: i32 = 24;
const DEFAULT_WANDER_DISTANCE_REDUCTION: i32 = 16;
const PATHFIND_TO_HIVE_WHEN_CLOSER_THAN: i32 = 16;
const HIVE_SEARCH_DISTANCE: i32 = 20;
const COOLDOWN_BEFORE_LOCATING_NEW_HIVE: i32 = 200;
const COOLDOWN_BEFORE_LOCATING_NEW_FLOWER: i32 = 200;
const MIN_FIND_FLOWER_RETRY_COOLDOWN: i32 = 20;
const MAX_FIND_FLOWER_RETRY_COOLDOWN: i32 = 60;
/// Vanilla `Bee.createNavigation`'s `setRequiredPathLength`.
const REQUIRED_PATH_LENGTH: f32 = 48.0;

fn as_bee(mob: &dyn Mob) -> Option<&BeeEntity> {
    mob.cast_any().downcast_ref::<BeeEntity>()
}

fn is_angry(mob: &dyn Mob) -> bool {
    mob.as_neutral().is_some_and(NeutralMob::is_angry)
}

/// Angry and still holding its stinger.
fn can_sting(mob: &dyn Mob) -> bool {
    is_angry(mob) && !as_bee(mob).is_some_and(BeeEntity::has_stung)
}

pub struct BeeEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub neutral_data: NeutralData,
    flags: AtomicU8,
    time_since_sting: AtomicI32,
    ticks_without_nectar_since_exiting_hive: AtomicI32,
    stay_out_of_hive_countdown: AtomicI32,
    num_crops_grown_since_pollination: AtomicI32,
    remaining_cooldown_before_locating_new_hive: AtomicI32,
    remaining_cooldown_before_locating_new_flower: AtomicI32,
    saved_flower_pos: AtomicCell<Option<BlockPos>>,
    hive_pos: AtomicCell<Option<BlockPos>>,
    under_water_ticks: AtomicI32,
    /// `BeePollinateGoal.pollinating`, which the navigation, the look control and
    /// `hurtServer` read too.
    pollinating: AtomicBool,
    /// `BeeGoToHiveGoal.blacklistedTargets`, which `BeeLocateHiveGoal` reads too.
    blacklisted_hives: Mutex<Vec<BlockPos>>,
}

impl BeeEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `Bee.createNavigation`, made first as in `Mob`'s constructor.
        let mut navigator = Navigator::new(FlyingPathNavigation::new().stable_over_any_block());
        navigator.set_can_open_doors(false);
        navigator.set_can_float(false);
        *mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = navigator;
        mob_entity.set_required_path_length(REQUIRED_PATH_LENGTH);
        set_animal_pathfinding_malus(&mob_entity);
        // Vanilla `Bee`'s constructor.
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(FlyingMoveControl::new(20, true));
        mob_entity.set_pathfinding_malus(PathType::DamageFire, -1.0);
        mob_entity.set_pathfinding_malus(PathType::Water, -1.0);
        mob_entity.set_pathfinding_malus(PathType::WaterBorder, 16.0);
        mob_entity.set_pathfinding_malus(PathType::Cocoa, -1.0);
        mob_entity.set_pathfinding_malus(PathType::Fence, -1.0);
        let bee = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            neutral_data: NeutralData::default(),
            flags: AtomicU8::new(0),
            time_since_sting: AtomicI32::new(0),
            ticks_without_nectar_since_exiting_hive: AtomicI32::new(0),
            stay_out_of_hive_countdown: AtomicI32::new(0),
            num_crops_grown_since_pollination: AtomicI32::new(0),
            remaining_cooldown_before_locating_new_hive: AtomicI32::new(0),
            // Drawn below, after the goals: a field initializer runs after `Mob`'s
            // constructor has called `registerGoals`.
            remaining_cooldown_before_locating_new_flower: AtomicI32::new(0),
            saved_flower_pos: AtomicCell::new(None),
            hive_pos: AtomicCell::new(None),
            under_water_ticks: AtomicI32::new(0),
            pollinating: AtomicBool::new(false),
            blacklisted_hives: Mutex::new(Vec::new()),
        };
        let mob_arc = Arc::new(bee);

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            // Vanilla BeeAttackGoal.
            goal_selector.add_goal(
                0,
                Box::new(MeleeAttackGoal::new(1.4, true).gated_by(can_sting)),
            );
            goal_selector.add_goal(1, Box::new(BeeEnterHiveGoal));
            goal_selector.add_goal(2, BreedGoal::new(1.0));
            goal_selector.add_goal(
                3,
                Box::new(TemptGoal::with_tag(
                    1.25,
                    &tag::Item::MINECRAFT_BEE_FOOD,
                    false,
                )),
            );
            goal_selector.add_goal(3, Box::new(ValidateHiveGoal::new(&mob_arc.mob_entity)));
            goal_selector.add_goal(3, Box::new(ValidateFlowerGoal::new(&mob_arc.mob_entity)));
            goal_selector.add_goal(4, Box::new(BeePollinateGoal::default()));
            goal_selector.add_goal(5, Box::new(FollowParentGoal::new(1.25)));
            goal_selector.add_goal(5, Box::new(BeeLocateHiveGoal));
            goal_selector.add_goal(5, Box::new(BeeGoToHiveGoal::default()));
            goal_selector.add_goal(6, Box::new(BeeGoToKnownFlowerGoal::default()));
            goal_selector.add_goal(7, Box::new(BeeGrowCropGoal));
            goal_selector.add_goal(8, Box::new(BeeWanderGoal));
            goal_selector.add_goal(9, SwimGoal::new(&mob_arc.mob_entity));
        };

        {
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            // Vanilla BeeHurtByOtherGoal.
            target_selector.add_goal(
                1,
                Box::new(
                    RevengeGoal::new(true)
                        .alerting_others()
                        .alerting_in_sight()
                        .continuing_while(is_angry),
                ),
            );
            // Vanilla BeeBecomeAngryTargetGoal.
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(
                    &mob_arc.mob_entity,
                    &pumpkin_data::entity::EntityType::PLAYER,
                    true,
                )
                .when(TargetCondition::AngryAt)
                .gated_by(can_sting),
            );
            target_selector.add_goal(3, ResetUniversalAngerGoal::new(true));
        };

        // The field's `Mth.nextInt(this.random, 20, 60)`, the same draw `dropFlower` makes.
        mob_arc.drop_flower();

        mob_arc
    }

    fn has_flag(&self, flag: u8) -> bool {
        (self.flags.load(Ordering::Relaxed) & flag) != 0
    }

    fn set_flag(&self, flag: u8, val: bool) {
        let current = self.flags.load(Ordering::Relaxed);
        let new_flags = if val { current | flag } else { current & !flag };
        self.flags.store(new_flags, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::bee::DATA_FLAGS_ID,
            new_flags as i8,
        );
    }

    #[must_use]
    pub fn has_nectar(&self) -> bool {
        self.has_flag(FLAG_HAS_NECTAR)
    }

    fn set_has_nectar(&self, has_nectar: bool) {
        if has_nectar {
            self.reset_ticks_without_nectar_since_exiting_hive();
        }
        self.set_flag(FLAG_HAS_NECTAR, has_nectar);
    }

    #[must_use]
    pub fn has_stung(&self) -> bool {
        self.has_flag(FLAG_HAS_STUNG)
    }

    fn set_has_stung(&self, has_stung: bool) {
        self.set_flag(FLAG_HAS_STUNG, has_stung);
    }

    fn set_rolling(&self, rolling: bool) {
        self.set_flag(FLAG_ROLL, rolling);
    }

    #[must_use]
    pub fn get_saved_flower_pos(&self) -> Option<BlockPos> {
        self.saved_flower_pos.load()
    }

    #[must_use]
    pub fn has_saved_flower_pos(&self) -> bool {
        self.get_saved_flower_pos().is_some()
    }

    pub fn set_saved_flower_pos(&self, pos: BlockPos) {
        self.saved_flower_pos.store(Some(pos));
    }

    #[must_use]
    pub fn get_hive_pos(&self) -> Option<BlockPos> {
        self.hive_pos.load()
    }

    #[must_use]
    pub fn has_hive(&self) -> bool {
        self.get_hive_pos().is_some()
    }

    pub fn set_hive_pos(&self, pos: Option<BlockPos>) {
        self.hive_pos.store(pos);
    }

    pub fn set_stay_out_of_hive_countdown(&self, ticks: i32) {
        self.stay_out_of_hive_countdown
            .store(ticks, Ordering::Relaxed);
    }

    fn reset_ticks_without_nectar_since_exiting_hive(&self) {
        self.ticks_without_nectar_since_exiting_hive
            .store(0, Ordering::Relaxed);
    }

    fn is_tired_of_looking_for_nectar(&self) -> bool {
        self.ticks_without_nectar_since_exiting_hive
            .load(Ordering::Relaxed)
            > TICKS_WITHOUT_NECTAR_BEFORE_GOING_HOME
    }

    fn is_pollinating(&self) -> bool {
        self.pollinating.load(Ordering::Relaxed)
    }

    fn drop_hive(&self) {
        self.set_hive_pos(None);
        self.remaining_cooldown_before_locating_new_hive
            .store(COOLDOWN_BEFORE_LOCATING_NEW_HIVE, Ordering::Relaxed);
    }

    fn drop_flower(&self) {
        self.saved_flower_pos.store(None);
        self.remaining_cooldown_before_locating_new_flower.store(
            self.get_entity().random().next_inbetween_i32(
                MIN_FIND_FLOWER_RETRY_COOLDOWN,
                MAX_FIND_FLOWER_RETRY_COOLDOWN,
            ),
            Ordering::Relaxed,
        );
    }

    /// Vanilla `dropOffNectar`, when a bee leaves a hive it brought honey to.
    pub fn drop_off_nectar(&self) {
        self.set_has_nectar(false);
        self.num_crops_grown_since_pollination
            .store(0, Ordering::Relaxed);
    }

    fn wants_to_enter_hive(&self) -> bool {
        if self.stay_out_of_hive_countdown.load(Ordering::Relaxed) <= 0
            && !self.is_pollinating()
            && !self.has_stung()
            && self.get_target().is_none()
        {
            let entity = self.get_entity();
            let wants_to_enter_hive = self.has_nectar()
                || self.is_tired_of_looking_for_nectar()
                || entity
                    .world
                    .load()
                    .bees_stay_in_hive(&BlockPos::floored_v(entity.pos.load()));
            wants_to_enter_hive && !self.is_hive_near_fire()
        } else {
            false
        }
    }

    fn world(&self) -> Arc<World> {
        self.get_entity().world.load_full()
    }

    /// Vanilla `getBeehiveBlockEntity`, handed to `f` since the hive lives behind an `Arc`.
    fn with_beehive<R>(&self, f: impl FnOnce(&BeehiveBlockEntity) -> R) -> Option<R> {
        let hive_pos = self.get_hive_pos()?;
        if self.is_too_far_away(hive_pos) {
            return None;
        }
        let block_entity = self.world().get_block_entity(&hive_pos)?;
        block_entity
            .as_any()
            .downcast_ref::<BeehiveBlockEntity>()
            .map(f)
    }

    fn is_hive_valid(&self) -> bool {
        self.with_beehive(|_| ()).is_some()
    }

    fn is_hive_near_fire(&self) -> bool {
        let world = self.world();
        self.with_beehive(|hive| hive.is_fire_nearby(&world))
            .unwrap_or(false)
    }

    fn does_hive_have_space(&self, hive_pos: BlockPos) -> bool {
        self.world()
            .get_block_entity(&hive_pos)
            .is_some_and(|block_entity| {
                block_entity
                    .as_any()
                    .downcast_ref::<BeehiveBlockEntity>()
                    .is_some_and(|hive| !hive.is_full())
            })
    }

    fn is_too_far_away(&self, target_pos: BlockPos) -> bool {
        !self.closer_than(target_pos, TOO_FAR_DISTANCE)
    }

    /// Vanilla `closerThan`: block distance from the bee's block.
    fn closer_than(&self, target_pos: BlockPos, distance: i32) -> bool {
        target_pos.squared_distance(&self.get_entity().block_pos.load()) < distance * distance
    }

    fn navigation_move_to(&self, destination: Vector3<f64>, reach_range: i32, speed: f64) {
        let living = &self.mob_entity.living_entity;
        let mut navigator = self
            .mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let path = navigator.create_path(living, destination, reach_range);
        navigator.move_to_path(path, speed, living);
    }

    /// Vanilla `pathfindRandomlyTowards`.
    fn pathfind_randomly_towards(&self, target_pos: BlockPos) {
        let target_vec = Vector3::new(
            f64::from(target_pos.0.x) + 0.5,
            f64::from(target_pos.0.y),
            f64::from(target_pos.0.z) + 0.5,
        );
        let bee_pos = self.get_entity().block_pos.load();
        let y_delta = target_vec.y as i32 - bee_pos.0.y;
        let y_adjust = if y_delta > 2 {
            4
        } else if y_delta < -2 {
            -4
        } else {
            0
        };

        let mut xz_dist = 6;
        let mut y_dist = 8;
        let dist = bee_pos.manhattan_distance(target_pos);
        if dist < 15 {
            xz_dist = dist / 2;
            y_dist = dist / 2;
        }

        if let Some(next_pos_towards) = air_random_pos::get_pos_towards(
            self,
            xz_dist,
            y_dist,
            y_adjust,
            target_vec,
            f64::from(std::f32::consts::PI / 10.0),
        ) {
            self.mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .set_max_visited_nodes_multiplier(0.5);
            self.navigation_move_to(next_pos_towards, 1, 1.0);
        }
    }

    /// Vanilla `Bee.attractsBees`.
    #[must_use]
    pub fn attracts_bees(block: &Block, state: &BlockState) -> bool {
        if !block.has_tag(&tag::Block::MINECRAFT_BEE_ATTRACTIVE) || state.is_waterlogged() {
            return false;
        }
        block != &Block::SUNFLOWER
            || TallSeagrassLikeProperties::from_state_id(state.id).half == DoubleBlockHalf::Upper
    }

    fn attracts_bees_at(world: &World, pos: &BlockPos) -> bool {
        let (block, state) = world.get_block_and_state(pos);
        Self::attracts_bees(block, state)
    }

    fn is_blacklisted(&self, pos: BlockPos) -> bool {
        self.blacklisted_hives
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&pos)
    }
}

impl AgeableMob for BeeEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for BeeEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_BEE_FOOD)
    }
}

impl NeutralMob for BeeEntity {
    fn get_neutral_data(&self) -> &NeutralData {
        &self.neutral_data
    }

    /// Bees calm down once the target is gone, unlike the other neutral mobs.
    fn stays_angry_with_target(&self) -> bool {
        false
    }
}

fn put_block_pos(nbt: &mut NbtCompound, key: &str, pos: Option<BlockPos>) {
    if let Some(pos) = pos {
        nbt.put(key, NbtTag::IntArray(vec![pos.0.x, pos.0.y, pos.0.z]));
    }
}

fn get_block_pos(nbt: &NbtCompound, key: &str) -> Option<BlockPos> {
    nbt.get_int_array(key)
        .and_then(|arr| (arr.len() == 3).then(|| BlockPos::new(arr[0], arr[1], arr[2])))
}

impl Mob for BeeEntity {
    /// Vanilla `Bee.checkFallDamage` does nothing.
    fn mob_check_fall_damage(
        &self,
        _caller: &dyn crate::entity::EntityBase,
        _ya: f64,
        _on_ground: bool,
        _state: &'static pumpkin_data::BlockState,
        _pos: pumpkin_util::math::position::BlockPos,
    ) {
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    /// Vanilla `Bee.jumpInLiquid`.
    fn jump_in_liquid(&self, _lava: bool) {
        let entity = &self.mob_entity.living_entity.entity;
        let mut velocity = entity.velocity.load();
        velocity.y += 0.01;
        entity.velocity.store(velocity);
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn as_neutral(&self) -> Option<&dyn NeutralMob> {
        Some(self)
    }

    fn omnidirectional_air_mover(&self) -> bool {
        true
    }

    fn get_walk_target_value(&self, pos: &BlockPos) -> f32 {
        if self.get_entity().world.load().get_block_state(pos).is_air() {
            10.0
        } else {
            0.0
        }
    }

    /// Vanilla `Bee.createNavigation`'s `tick`.
    fn skips_navigation_tick(&self) -> bool {
        self.is_pollinating()
    }

    /// Vanilla `BeeLookControl.tick`.
    fn skips_look_control_tick(&self) -> bool {
        self.is_angry()
    }

    /// Vanilla `BeeLookControl.resetXRotOnTick`.
    fn look_control_resets_x_rot(&self) -> bool {
        !self.is_pollinating()
    }

    fn melee_damage_type(&self) -> DamageType {
        DamageType::STING
    }

    /// The part of vanilla `Bee.doHurtTarget` after the sting landed.
    fn on_attack(&self, target: &dyn EntityBase) {
        if let Some(living) = target.get_living_entity() {
            living.set_stinger_count(living.get_stinger_count() + 1);
            let poison_time = match self.world().level_info.load().difficulty {
                pumpkin_util::Difficulty::Normal => POISON_SECONDS_NORMAL,
                pumpkin_util::Difficulty::Hard => POISON_SECONDS_HARD,
                _ => 0,
            };
            if poison_time > 0 {
                living.add_effect(pumpkin_data::potion::Effect {
                    effect_type: &StatusEffect::POISON,
                    duration: poison_time * 20,
                    amplifier: 0,
                    ambient: false,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                });
            }
        }
        self.set_has_stung(true);
        self.stop_being_angry();
        self.play_sound_at_self(Sound::EntityBeeSting, 1.0, 1.0);
    }

    /// Vanilla `Bee.hurtServer`: any hurt it isn't immune to stops the pollination.
    fn before_hurt(
        &self,
        _amount: f32,
        damage_type: DamageType,
        _source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) {
        if !self.get_entity().is_invulnerable_to(&damage_type, cause) {
            self.pollinating.store(false, Ordering::Relaxed);
        }
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
        put_block_pos(nbt, "hive_pos", self.get_hive_pos());
        put_block_pos(nbt, "flower_pos", self.get_saved_flower_pos());
        nbt.put_bool("HasNectar", self.has_nectar());
        nbt.put_bool("HasStung", self.has_stung());
        nbt.put_int(
            "TicksSincePollination",
            self.ticks_without_nectar_since_exiting_hive
                .load(Ordering::Relaxed),
        );
        nbt.put_int(
            "CannotEnterHiveTicks",
            self.stay_out_of_hive_countdown.load(Ordering::Relaxed),
        );
        nbt.put_int(
            "CropsGrownSincePollination",
            self.num_crops_grown_since_pollination
                .load(Ordering::Relaxed),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        self.set_has_nectar(nbt.get_bool("HasNectar").unwrap_or(false));
        self.set_has_stung(nbt.get_bool("HasStung").unwrap_or(false));
        self.ticks_without_nectar_since_exiting_hive.store(
            nbt.get_int("TicksSincePollination").unwrap_or(0),
            Ordering::Relaxed,
        );
        self.stay_out_of_hive_countdown.store(
            nbt.get_int("CannotEnterHiveTicks").unwrap_or(0),
            Ordering::Relaxed,
        );
        self.num_crops_grown_since_pollination.store(
            nbt.get_int("CropsGrownSincePollination").unwrap_or(0),
            Ordering::Relaxed,
        );
        self.set_hive_pos(get_block_pos(nbt, "hive_pos"));
        self.saved_flower_pos
            .store(get_block_pos(nbt, "flower_pos"));
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();
    }

    /// Vanilla `Bee.customServerAiStep`.
    fn custom_server_ai_step(&self, caller: &dyn EntityBase) {
        let has_stung = self.has_stung();
        if self.get_entity().is_in_water() {
            self.under_water_ticks.fetch_add(1, Ordering::Relaxed);
        } else {
            self.under_water_ticks.store(0, Ordering::Relaxed);
        }

        if self.under_water_ticks.load(Ordering::Relaxed) > 20 {
            caller.damage(caller, 1.0, DamageType::DROWN);
        }

        if has_stung {
            let time_since_sting = self.time_since_sting.fetch_add(1, Ordering::Relaxed) + 1;
            if time_since_sting % 5 == 0
                && self.get_entity().random().next_bounded_i32(
                    (STING_DEATH_COUNTDOWN - time_since_sting).clamp(1, STING_DEATH_COUNTDOWN),
                ) == 0
            {
                let health = self.mob_entity.living_entity.health.load();
                caller.damage(caller, health, DamageType::GENERIC);
            }
        }

        if !self.has_nectar() {
            self.ticks_without_nectar_since_exiting_hive
                .fetch_add(1, Ordering::Relaxed);
        }

        self.update_persistent_anger();
    }

    fn updates_anger_in_ai_step(&self) -> bool {
        true
    }

    /// The ends of vanilla `Bee.aiStep` and `Bee.tick`, after the movement.
    fn post_tick(&self) {
        for counter in [
            &self.stay_out_of_hive_countdown,
            &self.remaining_cooldown_before_locating_new_hive,
            &self.remaining_cooldown_before_locating_new_flower,
        ] {
            if counter.load(Ordering::Relaxed) > 0 {
                counter.fetch_sub(1, Ordering::Relaxed);
            }
        }

        let entity = self.get_entity();
        let should_roll = self.is_angry()
            && !self.has_stung()
            && self.get_target().is_some_and(|target| {
                target
                    .get_entity()
                    .pos
                    .load()
                    .squared_distance_to_vec(&entity.pos.load())
                    < ROLL_RANGE_SQR
            });
        self.set_rolling(should_roll);
        if entity.tick_count.load(Ordering::Relaxed) % 20 == 0 && !self.is_hive_valid() {
            self.set_hive_pos(None);
        }

        // The rest of vanilla `Bee.tick`: falling-nectar drips. `level.addParticle` does
        // nothing on a server level, but the random numbers are drawn there too.
        if self.has_nectar()
            && self
                .num_crops_grown_since_pollination
                .load(Ordering::Relaxed)
                < MAX_CROPS_GROWABLE
            && entity.random().next_f32() < 0.05
        {
            let world = entity.world.load();
            let mut i = 0;
            // `i < this.random.nextInt(2) + 1` draws again on every check.
            while i < entity.random().next_bounded_i32(2) + 1 {
                // `spawnFluidParticle`: two `level.getRandom().nextDouble()`s.
                let mut level_random = world
                    .random
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                level_random.next_f64();
                level_random.next_f64();
                drop(level_random);
                i += 1;
            }
        }
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::bee::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::bee::DATA_FLAGS_ID,
            self.flags.load(Ordering::Relaxed) as i8,
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        self.animal_interact(player, item_stack, Sound::EntityBeePollinate)
    }
}

/// Vanilla `BeeEnterHiveGoal`.
struct BeeEnterHiveGoal;

impl BeeEnterHiveGoal {
    fn can_bee_use(bee: &BeeEntity) -> bool {
        if let Some(hive_pos) = bee.get_hive_pos()
            && bee.wants_to_enter_hive()
            && {
                let pos = bee.get_entity().pos.load();
                hive_pos.dist_to_center_sqr(pos.x, pos.y, pos.z)
                    < f64::from(HIVE_CLOSE_ENOUGH_DISTANCE * HIVE_CLOSE_ENOUGH_DISTANCE)
            }
            && let Some(full) = bee.with_beehive(BeehiveBlockEntity::is_full)
        {
            if !full {
                return true;
            }
            bee.set_hive_pos(None);
        }
        false
    }
}

impl Goal for BeeEnterHiveGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_bee(mob).is_some_and(|bee| Self::can_bee_use(bee) && !bee.is_angry())
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(bee) = as_bee(mob) else {
            return;
        };
        let world = bee.world();
        bee.with_beehive(|hive| hive.add_occupant(&world, bee));
    }
}

/// Vanilla `ValidateHiveGoal`.
struct ValidateHiveGoal {
    validate_hive_cooldown: i32,
    last_validate_tick: i64,
}

impl ValidateHiveGoal {
    /// `VALIDATE_HIVE_COOLDOWN = Mth.nextInt(Bee.this.random, 20, 40)`.
    fn new(mob_entity: &MobEntity) -> Self {
        Self {
            validate_hive_cooldown: mob_entity
                .living_entity
                .entity
                .random()
                .next_inbetween_i32(20, 40),
            last_validate_tick: -1,
        }
    }
}

impl Goal for ValidateHiveGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_bee(mob).is_some_and(|bee| {
            bee.world().get_world_age()
                > self.last_validate_tick + i64::from(self.validate_hive_cooldown)
                && !bee.is_angry()
        })
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(bee) = as_bee(mob) else {
            return;
        };
        let world = bee.world();
        if let Some(hive_pos) = bee.get_hive_pos()
            && world.is_loaded(&hive_pos)
            && !bee.is_hive_valid()
        {
            bee.drop_hive();
        }
        self.last_validate_tick = world.get_world_age();
    }
}

/// Vanilla `ValidateFlowerGoal`.
struct ValidateFlowerGoal {
    validate_flower_cooldown: i32,
    last_validate_tick: i64,
}

impl ValidateFlowerGoal {
    /// `validateFlowerCooldown = Mth.nextInt(Bee.this.random, 20, 40)`.
    fn new(mob_entity: &MobEntity) -> Self {
        Self {
            validate_flower_cooldown: mob_entity
                .living_entity
                .entity
                .random()
                .next_inbetween_i32(20, 40),
            last_validate_tick: -1,
        }
    }
}

impl Goal for ValidateFlowerGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_bee(mob).is_some_and(|bee| {
            bee.world().get_world_age()
                > self.last_validate_tick + i64::from(self.validate_flower_cooldown)
                && !bee.is_angry()
        })
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(bee) = as_bee(mob) else {
            return;
        };
        let world = bee.world();
        if let Some(flower_pos) = bee.get_saved_flower_pos()
            && world.is_loaded(&flower_pos)
            && !BeeEntity::attracts_bees_at(&world, &flower_pos)
        {
            bee.drop_flower();
        }
        self.last_validate_tick = world.get_world_age();
    }
}

/// Vanilla `BeePollinateGoal`. Its `pollinating` flag lives on the bee.
#[derive(Default)]
struct BeePollinateGoal {
    successful_pollinating_ticks: i32,
    last_sound_played_tick: i32,
    hover_pos: Option<Vector3<f64>>,
    pollinating_ticks: i32,
    unreachable_flower_cache: HashMap<i64, i64>,
}

impl BeePollinateGoal {
    const MIN_POLLINATION_TICKS: i32 = 400;
    const ARRIVAL_THRESHOLD: f64 = 0.1;
    const POSITION_CHANGE_CHANCE: i32 = 25;
    const SPEED_MODIFIER: f32 = 0.35;
    const HOVER_HEIGHT_WITHIN_FLOWER: f32 = 0.6;
    const HOVER_POS_OFFSET: f32 = 0.333_333_34;
    const FLOWER_SEARCH_RADIUS: i32 = 5;
    const MAX_POLLINATING_TICKS: i32 = 600;
    /// How long a flower no path reached is skipped.
    const UNREACHABLE_FLOWER_TICKS: i64 = 600;

    fn can_bee_use(&mut self, bee: &BeeEntity) -> bool {
        if bee
            .remaining_cooldown_before_locating_new_flower
            .load(Ordering::Relaxed)
            > 0
            || bee.has_nectar()
            || bee.world().is_raining()
        {
            return false;
        }
        let Some(flower_pos) = self.find_nearby_flower(bee) else {
            bee.remaining_cooldown_before_locating_new_flower.store(
                bee.get_entity().random().next_inbetween_i32(
                    MIN_FIND_FLOWER_RETRY_COOLDOWN,
                    MAX_FIND_FLOWER_RETRY_COOLDOWN,
                ),
                Ordering::Relaxed,
            );
            return false;
        };
        bee.set_saved_flower_pos(flower_pos);
        bee.navigation_move_to(
            Vector3::new(
                f64::from(flower_pos.0.x) + 0.5,
                f64::from(flower_pos.0.y) + 0.5,
                f64::from(flower_pos.0.z) + 0.5,
            ),
            1,
            f64::from(1.2f32),
        );
        true
    }

    fn can_bee_continue_to_use(&self, bee: &BeeEntity) -> bool {
        if !bee.is_pollinating() || !bee.has_saved_flower_pos() || bee.world().is_raining() {
            return false;
        }
        if self.has_pollinated_long_enough() {
            bee.get_entity().random().next_f32() < 0.2
        } else {
            true
        }
    }

    const fn has_pollinated_long_enough(&self) -> bool {
        self.successful_pollinating_ticks > Self::MIN_POLLINATION_TICKS
    }

    fn set_wanted_pos(&self, bee: &BeeEntity) {
        if let Some(hover_pos) = self.hover_pos {
            bee.mob_entity
                .move_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .set_wanted_position(
                    hover_pos.x,
                    hover_pos.y,
                    hover_pos.z,
                    f64::from(Self::SPEED_MODIFIER),
                );
        }
    }

    fn get_offset(bee: &BeeEntity) -> f32 {
        (bee.get_entity().random().next_f32() * 2.0 - 1.0) * Self::HOVER_POS_OFFSET
    }

    /// Vanilla `findNearbyFlower`: the nearest flower by Manhattan order that a path
    /// reaches, remembering for 600 ticks the ones that can't be reached.
    fn find_nearby_flower(&mut self, bee: &BeeEntity) -> Option<BlockPos> {
        let world = bee.world();
        let game_time = world.get_world_age();
        let mut temp_cache = HashMap::new();
        let living = &bee.mob_entity.living_entity;
        let result = crate::entity::ai::util::goal_utils::within_box_by_manhattan_distance(
            bee.get_entity().block_pos.load(),
            Self::FLOWER_SEARCH_RADIUS,
            Self::FLOWER_SEARCH_RADIUS,
            Self::FLOWER_SEARCH_RADIUS,
        )
        .into_iter()
        .filter(|pos| BeeEntity::attracts_bees_at(&world, pos))
        .find(|pos| {
            let key = pos.as_long();
            let unreachable_until_time = self
                .unreachable_flower_cache
                .get(&key)
                .copied()
                .unwrap_or(i64::MIN);
            if game_time < unreachable_until_time {
                temp_cache.insert(key, unreachable_until_time);
                return false;
            }
            let path = bee
                .mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .create_path(living, pos.to_f64(), 1);
            if path.as_ref().is_some_and(Path::can_reach) {
                return true;
            }
            temp_cache.insert(key, game_time + Self::UNREACHABLE_FLOWER_TICKS);
            false
        });
        self.unreachable_flower_cache = temp_cache;
        result
    }
}

impl Goal for BeePollinateGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_bee(mob).is_some_and(|bee| self.can_bee_use(bee) && !bee.is_angry())
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        as_bee(mob).is_some_and(|bee| self.can_bee_continue_to_use(bee) && !bee.is_angry())
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(bee) = as_bee(mob) else {
            return;
        };
        self.successful_pollinating_ticks = 0;
        self.pollinating_ticks = 0;
        self.last_sound_played_tick = 0;
        bee.pollinating.store(true, Ordering::Relaxed);
        bee.reset_ticks_without_nectar_since_exiting_hive();
    }

    fn stop(&mut self, mob: &dyn Mob) {
        let Some(bee) = as_bee(mob) else {
            return;
        };
        if self.has_pollinated_long_enough() {
            bee.set_has_nectar(true);
        }
        bee.pollinating.store(false, Ordering::Relaxed);
        bee.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
        bee.remaining_cooldown_before_locating_new_flower
            .store(COOLDOWN_BEFORE_LOCATING_NEW_FLOWER, Ordering::Relaxed);
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(bee) = as_bee(mob) else {
            return;
        };
        let Some(saved_flower_pos) = bee.get_saved_flower_pos() else {
            return;
        };
        self.pollinating_ticks += 1;
        if self.pollinating_ticks > Self::MAX_POLLINATING_TICKS {
            bee.drop_flower();
            bee.pollinating.store(false, Ordering::Relaxed);
            bee.remaining_cooldown_before_locating_new_flower
                .store(COOLDOWN_BEFORE_LOCATING_NEW_FLOWER, Ordering::Relaxed);
            return;
        }

        let flower_pos = Vector3::new(
            f64::from(saved_flower_pos.0.x) + 0.5,
            f64::from(saved_flower_pos.0.y) + f64::from(Self::HOVER_HEIGHT_WITHIN_FLOWER),
            f64::from(saved_flower_pos.0.z) + 0.5,
        );
        let bee_pos = bee.get_entity().pos.load();
        if flower_pos.squared_distance_to_vec(&bee_pos).sqrt() > 1.0 {
            self.hover_pos = Some(flower_pos);
            self.set_wanted_pos(bee);
            return;
        }

        let hover_pos = *self.hover_pos.get_or_insert(flower_pos);
        let arrived_at_hover_pos =
            bee_pos.squared_distance_to_vec(&hover_pos).sqrt() <= Self::ARRIVAL_THRESHOLD;
        let mut should_set_wanted_pos = true;
        // Unreachable: the 600 ticks were already checked above, as in vanilla.
        if !arrived_at_hover_pos && self.pollinating_ticks > Self::MAX_POLLINATING_TICKS {
            bee.drop_flower();
            return;
        }
        if arrived_at_hover_pos {
            let should_change_hover_positions = bee
                .get_entity()
                .random()
                .next_bounded_i32(Self::POSITION_CHANGE_CHANCE)
                == 0;
            if should_change_hover_positions {
                self.hover_pos = Some(Vector3::new(
                    flower_pos.x + f64::from(Self::get_offset(bee)),
                    flower_pos.y,
                    flower_pos.z + f64::from(Self::get_offset(bee)),
                ));
                bee.mob_entity
                    .navigator
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .stop();
            } else {
                should_set_wanted_pos = false;
            }
            bee.mob_entity
                .look_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .look_at(bee, flower_pos.x, flower_pos.y, flower_pos.z);
        }

        if should_set_wanted_pos {
            self.set_wanted_pos(bee);
        }

        self.successful_pollinating_ticks += 1;
        if bee.get_entity().random().next_f32() < 0.05
            && self.successful_pollinating_ticks > self.last_sound_played_tick + 60
        {
            self.last_sound_played_tick = self.successful_pollinating_ticks;
            bee.play_sound_at_self(Sound::EntityBeePollinate, 1.0, 1.0);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `BeeLocateHiveGoal`.
struct BeeLocateHiveGoal;

impl BeeLocateHiveGoal {
    /// Vanilla `findNearbyHivesWithSpace`: nearest first.
    fn find_nearby_hives_with_space(bee: &BeeEntity) -> Vec<BlockPos> {
        let bee_pos = bee.get_entity().block_pos.load();
        let nearby_hives = {
            let world = bee.world();
            let mut poi = world
                .portal_poi
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            bee_poi::bee_homes_in_range(&mut poi, bee_pos, HIVE_SEARCH_DISTANCE)
        };
        let mut hives: Vec<BlockPos> = nearby_hives
            .into_iter()
            .filter(|pos| bee.does_hive_have_space(*pos))
            .collect();
        hives.sort_by_key(|pos| pos.squared_distance(&bee_pos));
        hives
    }
}

impl Goal for BeeLocateHiveGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_bee(mob).is_some_and(|bee| {
            bee.remaining_cooldown_before_locating_new_hive
                .load(Ordering::Relaxed)
                == 0
                && !bee.has_hive()
                && bee.wants_to_enter_hive()
                && !bee.is_angry()
        })
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(bee) = as_bee(mob) else {
            return;
        };
        bee.remaining_cooldown_before_locating_new_hive
            .store(COOLDOWN_BEFORE_LOCATING_NEW_HIVE, Ordering::Relaxed);
        let hives_with_space = Self::find_nearby_hives_with_space(bee);
        let Some(&nearest) = hives_with_space.first() else {
            return;
        };
        for pos_to_check in &hives_with_space {
            if !bee.is_blacklisted(*pos_to_check) {
                bee.set_hive_pos(Some(*pos_to_check));
                return;
            }
        }
        bee.blacklisted_hives
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        bee.set_hive_pos(Some(nearest));
    }
}

/// Vanilla `BeeGoToHiveGoal`. Its blacklist lives on the bee.
#[derive(Default)]
struct BeeGoToHiveGoal {
    travelling_ticks: i32,
    last_path: Option<Path>,
    ticks_stuck: i32,
}

impl BeeGoToHiveGoal {
    const MAX_TRAVELLING_TICKS: i32 = 2400;
    const MAX_BLACKLISTED_TARGETS: usize = 3;
    const TICKS_BEFORE_HIVE_DROP: i32 = 60;

    fn can_bee_use(bee: &BeeEntity) -> bool {
        let Some(hive_pos) = bee.get_hive_pos() else {
            return false;
        };
        !bee.is_too_far_away(hive_pos)
            && !bee.mob_entity.has_position_target()
            && bee.wants_to_enter_hive()
            && !Self::has_reached_target(bee, hive_pos)
            && bee
                .world()
                .get_block(&hive_pos)
                .has_tag(&tag::Block::MINECRAFT_BEEHIVES)
    }

    fn has_reached_target(bee: &BeeEntity, target_pos: BlockPos) -> bool {
        if bee.closer_than(target_pos, HIVE_CLOSE_ENOUGH_DISTANCE) {
            return true;
        }
        bee.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_path()
            .is_some_and(|path| {
                path.get_target() == target_pos && path.can_reach() && path.is_done()
            })
    }

    /// Vanilla `pathfindDirectlyTowards`: whether the path reaches the hive.
    fn pathfind_directly_towards(bee: &BeeEntity, target_pos: BlockPos) -> bool {
        let close_enough = if bee.closer_than(target_pos, 3) { 1 } else { 2 };
        bee.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_max_visited_nodes_multiplier(10.0);
        bee.navigation_move_to(target_pos.to_f64(), close_enough, 1.0);
        bee.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_path()
            .is_some_and(Path::can_reach)
    }

    fn blacklist_target(bee: &BeeEntity, target_pos: BlockPos) {
        let mut blacklist = bee
            .blacklisted_hives
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        blacklist.push(target_pos);
        while blacklist.len() > Self::MAX_BLACKLISTED_TARGETS {
            blacklist.remove(0);
        }
    }

    fn drop_and_blacklist_hive(bee: &BeeEntity) {
        if let Some(hive_pos) = bee.get_hive_pos() {
            Self::blacklist_target(bee, hive_pos);
        }
        bee.drop_hive();
    }
}

impl Goal for BeeGoToHiveGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_bee(mob).is_some_and(|bee| Self::can_bee_use(bee) && !bee.is_angry())
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_start(mob)
    }

    fn start(&mut self, _mob: &dyn Mob) {
        self.travelling_ticks = 0;
        self.ticks_stuck = 0;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.travelling_ticks = 0;
        self.ticks_stuck = 0;
        let mut navigator = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        navigator.stop();
        navigator.reset_max_visited_nodes_multiplier();
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(bee) = as_bee(mob) else {
            return;
        };
        let Some(hive_pos) = bee.get_hive_pos() else {
            return;
        };
        self.travelling_ticks += 1;
        if self.travelling_ticks > self.get_tick_count(Self::MAX_TRAVELLING_TICKS) {
            Self::drop_and_blacklist_hive(bee);
            return;
        }
        if bee.is_navigator_in_progress() {
            return;
        }
        if !bee.closer_than(hive_pos, PATHFIND_TO_HIVE_WHEN_CLOSER_THAN) {
            if bee.is_too_far_away(hive_pos) {
                bee.drop_hive();
            } else {
                bee.pathfind_randomly_towards(hive_pos);
            }
            return;
        }
        let can_reach_all_the_way_to_target = Self::pathfind_directly_towards(bee, hive_pos);
        if !can_reach_all_the_way_to_target {
            Self::drop_and_blacklist_hive(bee);
            return;
        }
        let navigator = bee
            .mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let path = navigator.get_path();
        if let Some(last_path) = &self.last_path
            && path.is_some_and(|path| path.same_as_path(last_path))
        {
            drop(navigator);
            self.ticks_stuck += 1;
            if self.ticks_stuck > Self::TICKS_BEFORE_HIVE_DROP {
                bee.drop_hive();
                self.ticks_stuck = 0;
            }
        } else {
            self.last_path = path.cloned();
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `BeeGoToKnownFlowerGoal`.
#[derive(Default)]
struct BeeGoToKnownFlowerGoal {
    travelling_ticks: i32,
}

impl BeeGoToKnownFlowerGoal {
    const MAX_TRAVELLING_TICKS: i32 = 2400;

    fn can_bee_use(bee: &BeeEntity) -> bool {
        bee.get_saved_flower_pos().is_some_and(|flower_pos| {
            !bee.mob_entity.has_position_target()
                && Self::wants_to_go_to_known_flower(bee)
                && !bee.closer_than(flower_pos, HIVE_CLOSE_ENOUGH_DISTANCE)
        })
    }

    fn wants_to_go_to_known_flower(bee: &BeeEntity) -> bool {
        bee.ticks_without_nectar_since_exiting_hive
            .load(Ordering::Relaxed)
            > TICKS_BEFORE_GOING_TO_KNOWN_FLOWER
    }
}

impl Goal for BeeGoToKnownFlowerGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_bee(mob).is_some_and(|bee| Self::can_bee_use(bee) && !bee.is_angry())
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_start(mob)
    }

    fn start(&mut self, _mob: &dyn Mob) {
        self.travelling_ticks = 0;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.travelling_ticks = 0;
        let mut navigator = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        navigator.stop();
        navigator.reset_max_visited_nodes_multiplier();
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(bee) = as_bee(mob) else {
            return;
        };
        let Some(flower_pos) = bee.get_saved_flower_pos() else {
            return;
        };
        self.travelling_ticks += 1;
        if self.travelling_ticks > self.get_tick_count(Self::MAX_TRAVELLING_TICKS) {
            bee.drop_flower();
        } else if !bee.is_navigator_in_progress() {
            if bee.is_too_far_away(flower_pos) {
                bee.drop_flower();
            } else {
                bee.pathfind_randomly_towards(flower_pos);
            }
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `BeeGrowCropGoal`.
struct BeeGrowCropGoal;

impl BeeGrowCropGoal {
    const GROW_CHANCE: i32 = 30;

    fn can_bee_use(bee: &BeeEntity) -> bool {
        if bee
            .num_crops_grown_since_pollination
            .load(Ordering::Relaxed)
            >= MAX_CROPS_GROWABLE
        {
            return false;
        }
        if bee.get_entity().random().next_f32() < 0.3 {
            return false;
        }
        bee.has_nectar() && bee.is_hive_valid()
    }
}

impl Goal for BeeGrowCropGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_bee(mob).is_some_and(|bee| Self::can_bee_use(bee) && !bee.is_angry())
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_start(mob)
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(bee) = as_bee(mob) else {
            return;
        };
        if bee
            .get_entity()
            .random()
            .next_bounded_i32(self.get_tick_count(Self::GROW_CHANCE))
            != 0
        {
            return;
        }
        let world = bee.world();
        for i in 1..=2 {
            let below_pos = bee.get_entity().block_pos.load().down_height(i);
            let (below_block, below_state) = world.get_block_and_state_id(&below_pos);
            if !below_block.has_tag(&tag::Block::MINECRAFT_BEE_GROWABLES) {
                continue;
            }
            let grow_state =
                if below_block == &Block::CAVE_VINES || below_block == &Block::CAVE_VINES_PLANT {
                    world
                        .block_registry
                        .get_pumpkin_block(below_block.id)
                        .and_then(|behaviour| {
                            let args = || BonemealArgs {
                                world: &world,
                                block: below_block,
                                position: &below_pos,
                                state_id: below_state,
                            };
                            behaviour.is_valid_bonemeal_target(args()).then(|| {
                                behaviour.perform_bonemeal(args());
                                world.get_block_state_id(&below_pos)
                            })
                        })
                } else {
                    next_age_state(below_block, below_state)
                };

            if let Some(grow_state) = grow_state {
                world.sync_world_event(WorldEvent::ParticlesBeeGrowth, below_pos, 15);
                world.set_block_state(&below_pos, grow_state, BlockFlags::NOTIFY_ALL);
                bee.num_crops_grown_since_pollination
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Vanilla `BeeWanderGoal`, which keeps flying while the bee is angry.
struct BeeWanderGoal;

impl BeeWanderGoal {
    const WANDER_XZ_DIST: i32 = 8;

    fn find_pos(bee: &BeeEntity) -> Option<Vector3<f64>> {
        let wander_direction = match bee.get_hive_pos() {
            Some(hive_pos)
                if bee.is_hive_valid()
                    && !bee.closer_than(hive_pos, Self::get_wander_threshold(bee)) =>
            {
                hive_pos
                    .to_centered_f64()
                    .sub(&bee.get_entity().pos.load())
                    .normalize()
            }
            _ => bee.get_looking_vector(),
        };

        hover_random_pos::get_pos(
            bee,
            Self::WANDER_XZ_DIST,
            7,
            wander_direction.x,
            wander_direction.z,
            f64::from(std::f32::consts::FRAC_PI_2),
            3,
            1,
        )
        .or_else(|| {
            air_and_water_random_pos::get_pos(
                bee,
                Self::WANDER_XZ_DIST,
                4,
                -2,
                wander_direction.x,
                wander_direction.z,
                f64::from(std::f32::consts::FRAC_PI_2),
            )
        })
    }

    fn get_wander_threshold(bee: &BeeEntity) -> i32 {
        let distance_reduction = if !bee.has_hive() && !bee.has_saved_flower_pos() {
            DEFAULT_WANDER_DISTANCE_REDUCTION
        } else {
            RESTRICTED_WANDER_DISTANCE_REDUCTION
        };
        TOO_FAR_DISTANCE - distance_reduction
    }
}

impl Goal for BeeWanderGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.is_navigator_idle() && mob.get_entity().random().next_bounded_i32(10) == 0
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        !mob.is_navigator_idle()
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(bee) = as_bee(mob) else {
            return;
        };
        if let Some(target_pos) = Self::find_pos(bee) {
            bee.navigation_move_to(target_pos, 1, 1.0);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

impl BeeEntity {
    fn is_navigator_in_progress(&self) -> bool {
        self.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_in_progress()
    }
}
