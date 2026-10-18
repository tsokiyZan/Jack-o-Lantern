use std::sync::{
    Arc, PoisonError,
    atomic::{AtomicI32, AtomicU64, Ordering},
};

use crate::enchantment::HurtAndBreak;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::GameMode;
use pumpkin_util::math::boundingbox::EntityDimensions;
use rand::RngExt;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::brain::{Brain, BrainTick, behavior::utils::is_alive, memory::types},
    ai::control::Control,
    ai::control::body_rotation_control::{BodyRotationControl, BodyRotationControlTrait},
    custom_sound::CustomSound,
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

use super::armadillo_ai;

pub const ARMADILLO_BABY_START_AGE: i32 = -48000;
/// Vanilla `Armadillo.BABY_SCALE`.
const BABY_SCALE: f32 = 0.6;
/// Vanilla `Armadillo.SCARE_CHECK_INTERVAL`, which vanilla writes out as 80 wherever the
/// danger memory is set.
pub const SCARE_CHECK_INTERVAL: i32 = 80;
/// Vanilla `Armadillo.SCARE_DISTANCE_HORIZONTAL`.
const SCARE_DISTANCE_HORIZONTAL: f64 = 7.0;
/// Vanilla `Armadillo.SCARE_DISTANCE_VERTICAL`.
const SCARE_DISTANCE_VERTICAL: f64 = 2.0;
/// Vanilla `Armadillo.getMaxHeadYRot` while not scared.
const MAX_HEAD_Y_ROT: f32 = 32.0;
/// Vanilla `BuiltInLootTables.ARMADILLO_SHED`.
const ARMADILLO_SHED: &str = "minecraft:gameplay/armadillo_shed";
/// Vanilla `BuiltInLootTables.ARMADILLO_BRUSH`.
const ARMADILLO_BRUSH: &str = "minecraft:brush/armadillo";
/// Durability a brush loses per vanilla `Armadillo.mobInteract`.
const BRUSH_DURABILITY_COST: i32 = 16;
/// Vanilla `20 * TimeUtil.SECONDS_PER_MINUTE * 5`: the shortest wait between sheds, and
/// the size of the random part on top.
const SCUTE_DROP_TIME: i32 = 20 * 60 * 5;

/// Vanilla `Armadillo.ArmadilloState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum ArmadilloState {
    #[default]
    Idle = 0,
    Rolling = 1,
    Scared = 2,
    Unrolling = 3,
}

impl ArmadilloState {
    /// Vanilla `BY_ID`, which reads an unknown id as `IDLE`.
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Rolling,
            2 => Self::Scared,
            3 => Self::Unrolling,
            _ => Self::Idle,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub const fn is_threatened(self) -> bool {
        !matches!(self, Self::Idle)
    }

    #[must_use]
    pub const fn animation_duration(self) -> i32 {
        match self {
            Self::Idle => 0,
            Self::Rolling => 10,
            Self::Scared => 50,
            Self::Unrolling => 30,
        }
    }

    #[must_use]
    pub const fn should_hide_in_shell(self, ticks_in_state: u64) -> bool {
        match self {
            Self::Idle => false,
            Self::Rolling => ticks_in_state > 5,
            Self::Scared => true,
            Self::Unrolling => ticks_in_state < 26,
        }
    }

    /// Vanilla `getSerializedName`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Rolling => "rolling",
            Self::Scared => "scared",
            Self::Unrolling => "unrolling",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "idle" => Some(Self::Idle),
            "rolling" => Some(Self::Rolling),
            "scared" => Some(Self::Scared),
            "unrolling" => Some(Self::Unrolling),
            _ => None,
        }
    }
}

/// Represents an Armadillo, a passive entity that can roll into a ball when threatened.
///
/// Wiki: <https://minecraft.wiki/w/Armadillo>
pub struct ArmadilloEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    state: AtomicI32,
    in_state_ticks: AtomicU64,
    scute_time: AtomicI32,
}

impl ArmadilloEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_can_float(true);
        *mob_entity
            .body_rotation_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            Box::new(ArmadilloBodyRotationControl(BodyRotationControl::new()));
        let armadillo = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            state: AtomicI32::new(ArmadilloState::Idle.id()),
            in_state_ticks: AtomicU64::new(0),
            scute_time: AtomicI32::new(0),
        };
        armadillo
            .scute_time
            .store(armadillo.pick_next_scute_drop_time(), Ordering::Relaxed);
        let mob_arc = Arc::new(armadillo);
        mob_arc.mob_entity.init_brain(mob_arc.as_ref());
        mob_arc
    }

    /// Vanilla `Armadillo.isScared`.
    #[must_use]
    pub fn is_scared(&self) -> bool {
        self.get_state() != ArmadilloState::Idle
    }

    /// Vanilla `Armadillo.shouldHideInShell`.
    #[must_use]
    pub fn should_hide_in_shell(&self) -> bool {
        self.get_state()
            .should_hide_in_shell(self.in_state_ticks.load(Ordering::Relaxed))
    }

    /// Vanilla `Armadillo.shouldSwitchToScaredState`.
    #[must_use]
    pub fn should_switch_to_scared_state(&self) -> bool {
        self.get_state() == ArmadilloState::Rolling
            && self.in_state_ticks.load(Ordering::Relaxed)
                > ArmadilloState::Rolling.animation_duration() as u64
    }

    #[must_use]
    pub fn get_state(&self) -> ArmadilloState {
        ArmadilloState::from_id(self.state.load(Ordering::Relaxed))
    }

    /// Vanilla `Armadillo.switchToState`, with `onSyncedDataUpdated`: only a real change
    /// restarts the state's tick count.
    pub fn switch_to_state(&self, state: ArmadilloState) {
        let previous = self.state.swap(state.id(), Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::armadillo::ARMADILLO_STATE,
            VarInt(state.id()),
        );
        if previous != state.id() {
            self.in_state_ticks.store(0, Ordering::Relaxed);
        }
    }

    /// Vanilla `Armadillo.pickNextScuteDropTime`.
    fn pick_next_scute_drop_time(&self) -> i32 {
        self.get_random().random_range(0..SCUTE_DROP_TIME) + SCUTE_DROP_TIME
    }

    /// Vanilla `Armadillo.isScaredBy`.
    #[must_use]
    pub fn is_scared_by(&self, living_entity: &dyn EntityBase) -> bool {
        let target = living_entity.get_entity();
        if !self
            .get_entity()
            .bounding_box
            .load()
            .expand(
                SCARE_DISTANCE_HORIZONTAL,
                SCARE_DISTANCE_VERTICAL,
                SCARE_DISTANCE_HORIZONTAL,
            )
            .intersects(&target.bounding_box.load())
        {
            return false;
        }
        if target
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_UNDEAD)
        {
            return true;
        }
        if self
            .mob_entity
            .living_entity
            .get_last_hurt_by_mob()
            .is_some_and(|hurt_by| hurt_by.get_entity().entity_id == target.entity_id)
        {
            return true;
        }
        if target.entity_type == &EntityType::PLAYER {
            return !living_entity.is_spectator()
                && (target.is_sprinting() || target.has_vehicle());
        }
        false
    }

    /// Vanilla `Armadillo.canStayRolledUp`, reading `isPanicking` from `brain`.
    #[must_use]
    pub fn can_stay_rolled_up_with(&self, brain: &Brain) -> bool {
        let entity = self.get_entity();
        !brain.has_memory_value(types::IS_PANICKING.id())
            && !entity.is_in_liquid()
            && !entity.is_leashed()
            && !entity.has_vehicle()
            && !entity.has_passengers()
    }

    /// Vanilla `Armadillo.rollUp`.
    pub fn roll_up(&self) {
        if !self.is_scared() {
            self.mob_entity.stop_in_place();
            self.mob_entity.reset_love_ticks();
            self.game_event(GameEvent::EntityAction);
            self.make_sound(Sound::EntityArmadilloRoll);
            self.switch_to_state(ArmadilloState::Rolling);
        }
    }

    /// Vanilla `Armadillo.rollOut`.
    pub fn roll_out(&self) {
        if self.is_scared() {
            self.game_event(GameEvent::EntityAction);
            self.make_sound(Sound::EntityArmadilloUnrollFinish);
            self.switch_to_state(ArmadilloState::Idle);
        }
    }

    /// Vanilla `Armadillo.brushOffScute`.
    pub fn brush_off_scute(&self, tool: &ItemStack) -> bool {
        if self.is_baby() {
            return false;
        }
        self.mob_entity
            .living_entity
            .drop_from_entity_interact_loot_table(ARMADILLO_BRUSH, tool);
        self.play_sound_at_self(Sound::EntityArmadilloBrush, 1.0, 1.0);
        self.game_event(GameEvent::EntityInteract);
        true
    }

    fn game_event(&self, event: GameEvent) {
        let entity = self.get_entity();
        entity
            .world
            .load()
            .emit_game_event(event.name(), entity.pos.load());
    }
}

/// Vanilla `Armadillo.BRAIN_PROVIDER`'s `ARMADILLO_SCARE_DETECTED` mob test.
#[must_use]
pub fn is_scared_by(tick: &BrainTick<'_>, living_entity: &dyn EntityBase) -> bool {
    tick.mob
        .cast_any()
        .downcast_ref::<ArmadilloEntity>()
        .is_some_and(|armadillo| armadillo.is_scared_by(living_entity))
}

/// Vanilla `Armadillo.canStayRolledUp`, from inside the armadillo's brain tick.
#[must_use]
pub fn can_stay_rolled_up(tick: &BrainTick<'_>) -> bool {
    tick.mob
        .cast_any()
        .downcast_ref::<ArmadilloEntity>()
        .is_some_and(|armadillo| armadillo.can_stay_rolled_up_with(tick.brain))
}

/// Vanilla `Armadillo.createBodyControl`: the body stays put while scared.
struct ArmadilloBodyRotationControl(BodyRotationControl);

impl Control for ArmadilloBodyRotationControl {}

impl BodyRotationControlTrait for ArmadilloBodyRotationControl {
    fn client_tick(&mut self, mob: &dyn Mob) {
        let scared = mob
            .cast_any()
            .downcast_ref::<ArmadilloEntity>()
            .is_some_and(ArmadilloEntity::is_scared);
        if !scared {
            self.0.client_tick(mob);
        }
    }
}

impl AgeableMob for ArmadilloEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }

    fn get_baby_start_age(&self) -> i32 {
        ARMADILLO_BABY_START_AGE
    }
}

impl Animal for ArmadilloEntity {
    /// Vanilla `Armadillo.isFood`.
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack
            .item
            .has_tag(&tag::Item::MINECRAFT_ARMADILLO_FOOD)
    }
}

impl CustomSound for ArmadilloEntity {
    /// Vanilla `Armadillo.getDeathSound`.
    fn death_sound(&self) -> Option<Sound> {
        Some(Sound::EntityArmadilloDeath)
    }

    /// Vanilla `Armadillo.getHurtSound`.
    fn hurt_sound(&self) -> Option<Sound> {
        Some(if self.is_scared() {
            Sound::EntityArmadilloHurtReduced
        } else {
            Sound::EntityArmadilloHurt
        })
    }
}

/// Vanilla `Armadillo.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    pumpkin_data::entity_dimensions::ARMADILLO_BABY_DIMENSIONS;

impl Mob for ArmadilloEntity {
    /// Vanilla `Armadillo.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        (!self.is_scared()).then_some(Sound::EntityArmadilloAmbient)
    }

    /// Vanilla `Armadillo.getAgeScale`.
    fn age_scale(&self) -> f32 {
        if self.is_baby() { BABY_SCALE } else { 1.0 }
    }

    /// Vanilla `Armadillo.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose).scale(self.age_scale())
        }
    }

    /// Vanilla `Armadillo.getMaxHeadYRot`.
    fn get_max_head_rotation(&self) -> f32 {
        if self.is_scared() {
            0.0
        } else {
            MAX_HEAD_Y_ROT
        }
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn make_brain(&self, packed: &crate::entity::ai::brain::memory::PackedMemories) -> Brain {
        armadillo_ai::ARMADILLO_PROVIDER.make_brain(self, packed)
    }

    /// Vanilla `Armadillo.customServerAiStep`'s `ArmadilloAi.updateActivity`.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        armadillo_ai::update_activity(tick);
    }

    /// Vanilla `Armadillo.customServerAiStep`: the brain, then shedding a scute.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
        let scute_time = self.scute_time.fetch_sub(1, Ordering::Relaxed) - 1;
        if scute_time <= 0 && self.should_drop_loot() {
            if self
                .mob_entity
                .living_entity
                .drop_from_gift_loot_table(ARMADILLO_SHED)
            {
                let mut random = self.get_random();
                let pitch = (random.random::<f32>() - random.random::<f32>()) * 0.2 + 1.0;
                self.play_sound_at_self(Sound::EntityArmadilloScuteDrop, 1.0, pitch);
                self.game_event(GameEvent::EntityPlace);
            }
            self.scute_time
                .store(self.pick_next_scute_drop_time(), Ordering::Relaxed);
        }
    }

    /// Vanilla `AgeableMob.aiStep`, the end of `Armadillo`'s `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        if self.get_entity().is_alive() {
            self.ageable_ai_step();
        }
    }

    /// The rest of vanilla `Armadillo.tick`, after `super.tick()`.
    fn post_tick(&self) {
        if self.is_scared() {
            self.clamp_head_rotation_to_body();
        }
        self.in_state_ticks.fetch_add(1, Ordering::Relaxed);
    }

    /// Vanilla `Armadillo.hurtServer`; `LivingEntity.hurtServer` then raises a negative
    /// amount to zero.
    fn modify_incoming_damage(&self, amount: f32, _damage_type: DamageType) -> f32 {
        if self.is_scared() {
            ((amount - 1.0) / 2.0).max(0.0)
        } else {
            amount
        }
    }

    /// Vanilla `Armadillo.actuallyHurt`.
    fn actually_hurt(
        &self,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) {
        if self.mob_entity.is_no_ai() || self.mob_entity.living_entity.health.load() <= 0.0 {
            return;
        }
        if cause
            .or(source)
            .is_some_and(|attacker| attacker.get_living_entity().is_some())
        {
            let can_stay_rolled_up = self.mob_entity.with_brain(self, |tick| {
                tick.brain.set_with_expiry(
                    types::DANGER_DETECTED_RECENTLY,
                    true,
                    i64::from(SCARE_CHECK_INTERVAL),
                );
                self.can_stay_rolled_up_with(tick.brain)
            });
            if can_stay_rolled_up {
                self.roll_up();
            }
        } else if damage_type.has_tag(&tag::DamageType::MINECRAFT_PANIC_ENVIRONMENTAL_CAUSES) {
            self.roll_out();
        }
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_string("state", self.get_state().name().to_string());
        nbt.put_int("scute_time", self.scute_time.load(Ordering::Relaxed));
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.switch_to_state(
            nbt.get_string("state")
                .and_then(ArmadilloState::from_name)
                .unwrap_or_default(),
        );
        if let Some(scute_time) = nbt.get_int("scute_time") {
            self.scute_time.store(scute_time, Ordering::Relaxed);
        }
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::armadillo::BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::armadillo::ARMADILLO_STATE,
            VarInt(self.get_state().id()),
        );
    }

    /// Vanilla `Armadillo.mobInteract`.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if item_stack.item == &Item::BRUSH && self.brush_off_scute(item_stack) {
            // Vanilla `hurtAndBreak`: a player with infinite materials keeps the brush whole.
            if player.gamemode.load() != GameMode::Creative {
                let _ = item_stack.hurt_and_break(&player.world(), BRUSH_DURABILITY_COST);
            }
            return true;
        }
        if self.is_scared() {
            return false;
        }
        self.animal_interact(player, item_stack, Sound::EntityArmadilloEat)
    }
}

impl ArmadilloEntity {
    /// Vanilla `LivingEntity.shouldDropLoot`.
    fn should_drop_loot(&self) -> bool {
        !self.is_baby()
            && self
                .get_entity()
                .world
                .load()
                .level_info
                .load()
                .game_rules
                .mob_drops
    }
}

/// Vanilla `armadillo -> !armadillo.isScared()`, the predicate spiders flee armadillos by.
pub fn armadillo_not_scared(target: &crate::entity::living::LivingEntity) -> bool {
    target
        .entity
        .world
        .load()
        .get_entity_by_id(target.entity.entity_id)
        .is_some_and(|entity| {
            entity
                .cast_any()
                .downcast_ref::<ArmadilloEntity>()
                .is_some_and(|armadillo| !armadillo.is_scared())
        })
}
