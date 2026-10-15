//! Vanilla `Creaking`.
//!
//! Its animation states, eye flicker and walk animation are client side; the client derives
//! them from the synced data and the entity events sent here.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, PoisonError};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::block_properties::{CreakingHeartLikeProperties, CreakingHeartState};
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockStateId, tracked_data};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::{VarInt, ser::NetworkWriteExt};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use crate::block::entities::creaking_heart::CreakingHeartBlockEntity;
use crate::entity::ai::brain::memory::{PackedMemories, types};
use crate::entity::ai::brain::{Brain, BrainTick, behavior::utils::is_alive};
use crate::entity::ai::control::Control;
use crate::entity::ai::control::body_rotation_control::{
    BodyRotationControl, BodyRotationControlTrait,
};
use crate::entity::ai::control::jump_control::{JumpControl, JumpControlTrait};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::pathfinder::node_evaluator::MobData;
use crate::entity::custom_sound::CustomSound;
use crate::entity::player::Player;
use crate::entity::projectile::is_projectile;
use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity, creaking_ai, mob_hurt_server},
};

pub const ATTACK_ANIMATION_DURATION: i32 = 15;
const ACTIVATION_RANGE_SQ: f64 = 144.0;
pub const ATTACK_INTERVAL: i32 = 40;
pub const SPEED_MULTIPLIER_WHEN_IDLING: f32 = 0.3;
pub const CREAKING_ORANGE: i32 = 16_545_810;
pub const CREAKING_GRAY: i32 = 6_250_335;
pub const INVULNERABILITY_ANIMATION_DURATION: i32 = 8;
pub const TWITCH_DEATH_DURATION: u8 = 45;
const MAX_PLAYER_STUCK_COUNTER: i32 = 4;

/// The parts of a vanilla `DamageSource` that the creaking's death reads: its type, the
/// direct entity and the one responsible.
pub struct DamageSource<'a> {
    pub damage_type: DamageType,
    pub direct: Option<&'a dyn EntityBase>,
    pub causing: Option<&'a dyn EntityBase>,
}

pub struct CreakingEntity {
    pub mob_entity: MobEntity,
    can_move: AtomicBool,
    is_active: AtomicBool,
    is_tearing_down: AtomicBool,
    home_pos: AtomicCell<Option<BlockPos>>,
    attack_animation_remaining_ticks: AtomicI32,
    invulnerability_animation_remaining_ticks: AtomicI32,
    player_stuck_counter: AtomicI32,
}

impl CreakingEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_can_float(true);
        *mob_entity
            .body_rotation_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            Box::new(CreakingBodyRotationControl(BodyRotationControl::new()));
        *mob_entity
            .jump_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            Box::new(CreakingJumpControl(JumpControl::default()));
        let mob_arc = Arc::new(Self {
            mob_entity,
            can_move: AtomicBool::new(true),
            is_active: AtomicBool::new(false),
            is_tearing_down: AtomicBool::new(false),
            home_pos: AtomicCell::new(None),
            attack_animation_remaining_ticks: AtomicI32::new(0),
            invulnerability_animation_remaining_ticks: AtomicI32::new(0),
            player_stuck_counter: AtomicI32::new(0),
        });
        mob_arc.mob_entity.init_brain(mob_arc.as_ref());
        mob_arc
    }

    /// Vanilla `Creaking.setTransient`: bound to the heart at `pos`.
    pub fn set_transient(&self, pos: BlockPos) {
        self.set_home_pos(pos);
        let mob_entity = &self.mob_entity;
        mob_entity.set_pathfinding_malus(PathType::DamageOther, 8.0);
        mob_entity.set_pathfinding_malus(PathType::PowderSnow, 8.0);
        mob_entity.set_pathfinding_malus(PathType::Lava, 8.0);
        mob_entity.set_pathfinding_malus(PathType::DamageFire, 0.0);
        mob_entity.set_pathfinding_malus(PathType::DangerFire, 0.0);
    }

    #[must_use]
    pub fn is_heart_bound(&self) -> bool {
        self.get_home_pos().is_some()
    }

    #[must_use]
    pub fn can_move(&self) -> bool {
        self.can_move.load(Ordering::Relaxed)
    }

    fn set_can_move(&self, can_move: bool) {
        self.can_move.store(can_move, Ordering::Relaxed);
        self.get_entity()
            .set_synced_data(tracked_data::creaking::CAN_MOVE, can_move);
    }

    /// Vanilla `Creaking.blameSourceForDamage`: the player now held responsible, if any.
    fn blame_source_for_damage(
        &self,
        damage_type: DamageType,
        attacker: Option<&dyn EntityBase>,
    ) -> Option<Arc<Player>> {
        let living = &self.mob_entity.living_entity;
        living.resolve_mob_responsible_for_damage(damage_type, attacker);
        living.resolve_player_responsible_for_damage(attacker);
        living
            .last_hurt_by_player
            .load()
            .and_then(|uuid| self.get_entity().world.load().get_player_by_uuid(uuid))
    }

    /// The heart at the home position, while it still counts this creaking as its protector.
    fn protecting_heart(
        &self,
        home_pos: &BlockPos,
    ) -> Option<Arc<dyn crate::block::entities::BlockEntity>> {
        let world = self.get_entity().world.load();
        world.get_block_entity(home_pos).filter(|block_entity| {
            block_entity
                .as_any()
                .downcast_ref::<CreakingHeartBlockEntity>()
                .is_some_and(|heart| heart.is_protector(&world, self))
        })
    }

    /// Vanilla `Creaking.tearDown`: crumbles away without dying.
    pub fn tear_down(&self) {
        let entity = self.get_entity();
        let world = entity.world.load();
        let bounding_box = entity.bounding_box.load();
        let center = bounding_box.get_center();
        let size = bounding_box.max - bounding_box.min;
        let spread = Vector3::new(
            (size.x * 0.3) as f32,
            (size.y * 0.3) as f32,
            (size.z * 0.3) as f32,
        );
        let mut heart_props =
            CreakingHeartLikeProperties::from_state_id(Block::CREAKING_HEART.default_state.id);
        heart_props.creaking_heart_state = CreakingHeartState::Awake;
        for (state, count) in [
            (Block::PALE_OAK_WOOD.default_state.id, 100),
            (heart_props.to_state_id(&Block::CREAKING_HEART), 10),
        ] {
            spawn_block_crumble(&world, center, spread, count, state);
        }
        self.make_sound(Sound::EntityCreakingDeath);
        entity.remove();
    }

    /// Vanilla `Creaking.creakingDeathEffects`.
    pub fn creaking_death_effects(&self, source: &DamageSource<'_>) {
        let attacker = source.causing.or(source.direct);
        self.blame_source_for_damage(source.damage_type, attacker);
        self.mob_entity
            .living_entity
            .die(source.damage_type, source.direct, source.causing);
        self.make_sound(Sound::EntityCreakingTwitch);
    }

    /// Vanilla `Creaking.playerIsStuckInYou`.
    pub fn player_is_stuck_in_you(&self) -> bool {
        let players = self.mob_entity.with_brain(self, |tick| {
            tick.brain
                .get(types::NEAREST_PLAYERS)
                .cloned()
                .unwrap_or_default()
        });
        if players.is_empty() {
            self.player_stuck_counter.store(0, Ordering::Relaxed);
            return false;
        }
        let own_box = self.get_entity().bounding_box.load();
        if players
            .iter()
            .any(|player| own_box.contains(player.get_entity().get_eye_pos()))
        {
            return self.player_stuck_counter.fetch_add(1, Ordering::Relaxed) + 1
                > MAX_PLAYER_STUCK_COUNTER;
        }
        self.player_stuck_counter.store(0, Ordering::Relaxed);
        false
    }

    fn set_home_pos(&self, pos: BlockPos) {
        self.home_pos.store(Some(pos));
        let entity = self.get_entity();
        entity.set_synced_data(tracked_data::creaking::HOME_POS, Some(pos));
        // Vanilla `Creaking.fireImmune`: a bound creaking never burns.
        entity.fire_immune.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn get_home_pos(&self) -> Option<BlockPos> {
        self.home_pos.load()
    }

    pub fn set_tearing_down(&self) {
        self.is_tearing_down.store(true, Ordering::Relaxed);
        self.get_entity()
            .set_synced_data(tracked_data::creaking::IS_TEARING_DOWN, true);
    }

    #[must_use]
    pub fn is_tearing_down(&self) -> bool {
        self.is_tearing_down.load(Ordering::Relaxed)
    }

    /// Vanilla `Creaking.checkCanMove`: frozen while a player it could attack looks at it.
    fn check_can_move(&self, brain: &mut Brain) -> bool {
        let players = brain
            .get(types::NEAREST_PLAYERS)
            .cloned()
            .unwrap_or_default();
        let active = self.is_active();
        if players.is_empty() {
            if active {
                self.deactivate(brain);
            }
            return true;
        }

        let entity = self.get_entity();
        let living = &self.mob_entity.living_entity;
        let eye_y = entity.get_eye_y();
        let y = entity.pos.load().y;
        let gaze_heights = [
            eye_y,
            y + 0.5 * f64::from(living.get_scale()),
            f64::midpoint(eye_y, y),
        ];
        let mut has_potential_target = false;
        for player in &players {
            let player_base: &dyn EntityBase = player.as_ref();
            let this: &dyn EntityBase = self;
            if !self.can_attack(player_base) || this.is_allied_to(player_base) {
                continue;
            }
            has_potential_target = true;
            if (!active || player_not_wearing_disguise_item(player))
                && living.is_looking_at_me(player.get_entity(), 0.5, false, true, &gaze_heights)
            {
                if active {
                    return false;
                }
                if player
                    .get_entity()
                    .pos
                    .load()
                    .squared_distance_to_vec(&entity.pos.load())
                    < ACTIVATION_RANGE_SQ
                {
                    self.activate(brain, player);
                    return false;
                }
            }
        }

        if !has_potential_target && active {
            self.deactivate(brain);
        }
        true
    }

    /// Vanilla `Creaking.activate`.
    fn activate(&self, brain: &mut Brain, player: &Arc<Player>) {
        let target: Arc<dyn EntityBase> = player.clone();
        brain.set(types::ATTACK_TARGET, target.clone());
        self.mob_entity.set_target(Some(target));
        self.game_event(GameEvent::EntityAction);
        self.make_sound(Sound::EntityCreakingActivate);
        self.set_is_active(true);
    }

    /// Vanilla `Creaking.deactivate`.
    fn deactivate(&self, brain: &mut Brain) {
        brain.erase(types::ATTACK_TARGET.id());
        self.mob_entity.set_target(None);
        self.game_event(GameEvent::EntityAction);
        self.make_sound(Sound::EntityCreakingDeactivate);
        self.set_is_active(false);
    }

    fn set_is_active(&self, active: bool) {
        self.is_active.store(active, Ordering::Relaxed);
        self.get_entity()
            .set_synced_data(tracked_data::creaking::IS_ACTIVE, active);
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.is_active.load(Ordering::Relaxed)
    }

    fn game_event(&self, event: GameEvent) {
        let entity = self.get_entity();
        entity
            .world
            .load()
            .emit_game_event(event.name(), entity.pos.load());
    }
}

/// Vanilla `LivingEntity.PLAYER_NOT_WEARING_DISGUISE_ITEM`.
fn player_not_wearing_disguise_item(player: &Player) -> bool {
    let helmet = player
        .living_entity
        .entity_equipment
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&EquipmentSlot::HEAD);
    !helmet
        .item
        .has_tag(&tag::Item::MINECRAFT_GAZE_DISGUISE_EQUIPMENT)
}

/// `serverLevel.sendParticles(new BlockParticleOption(BLOCK_CRUMBLE, state), ...)`.
fn spawn_block_crumble(
    world: &crate::world::World,
    center: Vector3<f64>,
    spread: Vector3<f32>,
    count: i32,
    state: BlockStateId,
) {
    let mut data = Vec::new();
    if data
        .write_var_int(&VarInt(i32::from(state.as_u16())))
        .is_ok()
    {
        world.spawn_particle_with_data(center, spread, 0.0, count, Particle::BlockCrumble, &data);
    }
}

fn creaking_can_move(mob: &dyn Mob) -> bool {
    mob.cast_any()
        .downcast_ref::<CreakingEntity>()
        .is_none_or(CreakingEntity::can_move)
}

/// Vanilla `Creaking.CreakingBodyRotationControl`.
struct CreakingBodyRotationControl(BodyRotationControl);

impl Control for CreakingBodyRotationControl {}

impl BodyRotationControlTrait for CreakingBodyRotationControl {
    fn client_tick(&mut self, mob: &dyn Mob) {
        if creaking_can_move(mob) {
            self.0.client_tick(mob);
        }
    }
}

/// Vanilla `Creaking.CreakingJumpControl`: a frozen creaking keeps its pending jump.
struct CreakingJumpControl(JumpControl);

impl Control for CreakingJumpControl {}

impl JumpControlTrait for CreakingJumpControl {
    fn jump(&mut self) {
        self.0.jump();
    }

    fn tick(&mut self, mob: &dyn Mob) {
        if creaking_can_move(mob) {
            self.0.tick(mob);
        } else {
            mob.get_mob_entity()
                .living_entity
                .jumping
                .store(false, Ordering::SeqCst);
        }
    }
}

impl CustomSound for CreakingEntity {
    fn death_sound(&self) -> Option<Sound> {
        None
    }

    /// Vanilla `Creaking.getHurtSound`.
    fn hurt_sound(&self) -> Option<Sound> {
        self.is_heart_bound().then_some(Sound::EntityCreakingSway)
    }
}

impl Mob for CreakingEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    fn make_brain(&self, packed: &PackedMemories) -> Brain {
        creaking_ai::CREAKING_PROVIDER.make_brain(self, packed)
    }

    /// Vanilla `Creaking.customServerAiStep`'s `CreakingAi.updateActivity`. `Creaking.getTarget`
    /// reads the brain; the mirror keeps the shared target code seeing the same target.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        creaking_ai::update_activity(tick);
        self.mob_entity
            .set_target(tick.brain.get(types::ATTACK_TARGET).cloned());
    }

    /// Vanilla `Creaking.customServerAiStep`.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
    }

    /// Vanilla `Creaking.tick`, before `super.tick()`: a creaking its heart no longer
    /// protects dies.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        if let Some(home_pos) = self.get_home_pos()
            && self.protecting_heart(&home_pos).is_none()
        {
            self.mob_entity.living_entity.set_health(0.0);
        }
    }

    /// Vanilla `Creaking.aiStep`, before `super.aiStep()`.
    fn ai_step(&self, _caller: &dyn EntityBase) {
        if self
            .invulnerability_animation_remaining_ticks
            .load(Ordering::Relaxed)
            > 0
        {
            self.invulnerability_animation_remaining_ticks
                .fetch_sub(1, Ordering::Relaxed);
        }
        if self
            .attack_animation_remaining_ticks
            .load(Ordering::Relaxed)
            > 0
        {
            self.attack_animation_remaining_ticks
                .fetch_sub(1, Ordering::Relaxed);
        }

        let can_move = self.can_move();
        let now_can_move = self
            .mob_entity
            .with_brain(self, |tick| self.check_can_move(tick.brain));
        if now_can_move != can_move {
            self.game_event(GameEvent::EntityAction);
            if now_can_move {
                self.make_sound(Sound::EntityCreakingUnfreeze);
            } else {
                self.mob_entity.stop_in_place();
                self.make_sound(Sound::EntityCreakingFreeze);
            }
        }
        self.set_can_move(now_can_move);
    }

    /// Vanilla `Creaking.tickDeath`: a torn down creaking twitches for 45 ticks, then crumbles.
    fn tick_death(&self, _caller: &dyn EntityBase) -> bool {
        if !(self.is_heart_bound() && self.is_tearing_down()) {
            return false;
        }
        let living = &self.mob_entity.living_entity;
        if living.health.load() <= 0.0 {
            let death_time = living.death_time.fetch_add(1, Ordering::Relaxed) + 1;
            if death_time > TWITCH_DEATH_DURATION && !self.get_entity().is_removed() {
                self.tear_down();
            }
        }
        true
    }

    /// Vanilla `Creaking.hurtServer`: while bound to a heart, hits only hurt the heart.
    fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        let Some(home_pos) = self.get_home_pos() else {
            return mob_hurt_server(self, caller, amount, damage_type, position, source, cause);
        };
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_INVULNERABILITY) {
            return mob_hurt_server(self, caller, amount, damage_type, position, source, cause);
        }

        let entity = self.get_entity();
        if entity.is_invulnerable_to(&damage_type, cause)
            || self
                .invulnerability_animation_remaining_ticks
                .load(Ordering::Relaxed)
                > 0
            || self.mob_entity.living_entity.health.load() <= 0.0
        {
            return false;
        }

        let responsible_player = self.blame_source_for_damage(damage_type, cause.or(source));
        let direct_is_attacker = source.is_some_and(|direct| {
            direct.get_living_entity().is_some() || is_projectile(direct.get_entity().entity_type)
        });
        if !direct_is_attacker && responsible_player.is_none() {
            return false;
        }

        self.invulnerability_animation_remaining_ticks
            .store(INVULNERABILITY_ANIMATION_DURATION, Ordering::Relaxed);
        let world = entity.world.load_full();
        world.send_entity_status(entity, EntityStatus::Shake, None);
        self.game_event(GameEvent::EntityAction);
        if let Some(heart) = self.protecting_heart(&home_pos)
            && let Some(heart) = heart.as_any().downcast_ref::<CreakingHeartBlockEntity>()
        {
            if responsible_player.is_some() {
                heart.creaking_hurt(&world);
            }
            // Vanilla `LivingEntity.playHurtSound`.
            self.reset_ambient_sound_time();
            self.make_sound(Sound::EntityCreakingSway);
        }
        true
    }

    /// Vanilla `Creaking.doHurtTarget`.
    fn do_hurt_target(&self, tick: &mut BrainTick<'_>, target: &Arc<dyn EntityBase>) -> bool {
        if target.get_living_entity().is_none() {
            return false;
        }
        self.attack_animation_remaining_ticks
            .store(ATTACK_ANIMATION_DURATION, Ordering::Relaxed);
        tick.world
            .send_entity_status(self.get_entity(), EntityStatus::StartAttacking, None);
        let attacker: &dyn EntityBase = tick.mob;
        let hurt = self.mob_entity.try_attack(attacker, target.as_ref());
        if hurt {
            // Vanilla `Mob.doHurtTarget`'s `playAttackSound`.
            self.make_sound(Sound::EntityCreakingAttack);
        }
        hurt
    }

    /// Vanilla `Creaking.isPushable`.
    fn mob_is_pushable(&self) -> bool {
        self.can_move()
    }

    /// Vanilla `Creaking.knockback`: a frozen creaking takes none.
    fn custom_knockback(
        &self,
        _power: f64,
        _xd: f64,
        _zd: f64,
        _attacker: Option<&Entity>,
        _damage: f32,
        _comes_from_effect: bool,
    ) -> bool {
        !self.can_move()
    }

    /// Vanilla `CreakingMoveControl` and `CreakingLookControl`.
    fn controls_paused(&self) -> bool {
        !self.can_move()
    }

    /// Vanilla `CreakingPathNavigation.tick`.
    fn skips_navigation_tick(&self) -> bool {
        !self.can_move()
    }

    /// Vanilla `Creaking.createNavigation`'s `HomeNodeEvaluator`.
    fn on_pathfinding_start(&self, mob_data: &mut MobData) {
        mob_data.home_pos = self.get_home_pos();
    }

    /// Vanilla `Creaking.canUsePortal`.
    fn can_use_portal(&self) -> bool {
        !self.is_heart_bound()
    }

    /// Vanilla `Creaking.getWalkTargetValue`.
    fn get_walk_target_value(&self, _pos: &BlockPos) -> f32 {
        0.0
    }

    /// Vanilla `Creaking.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        (!self.is_active()).then_some(Sound::EntityCreakingAmbient)
    }

    /// Vanilla `Creaking.addAdditionalSaveData`, which replaces `Mob`'s `home_pos`.
    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        if let Some(pos) = self.get_home_pos() {
            nbt.put(
                "home_pos",
                NbtTag::IntArray(vec![pos.0.x, pos.0.y, pos.0.z]),
            );
        }
    }

    /// Vanilla `Creaking.readAdditionalSaveData`.
    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if let Some(&[x, y, z]) = nbt.get_int_array("home_pos") {
            self.set_transient(BlockPos::new(x, y, z));
        }
    }
}
