use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::breath::BreathManager;
use crate::item::interaction_result::InteractionResult;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityPose;
use pumpkin_util::Hand;
use pumpkin_util::math::boundingbox::EntityDimensions;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::random::RandomImpl;
use rand::RngExt;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::brain::{BrainTick, memory::types},
    ai::control::{
        look_control::LookControl, smooth_swimming_move_control::SmoothSwimmingMoveControl,
    },
    ai::pathfinder::{AmphibiousPathNavigation, Navigator},
    mob::{Mob, MobEntity},
    passive::{
        animal::{Animal, set_animal_pathfinding_malus},
        axolotl_ai,
    },
    player::Player,
};
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::potion::Effect;
use std::sync::PoisonError;

/// The `inWaterSpeedModifier` and `outsideWaterSpeedModifier` of vanilla's
/// `AxolotlMoveControl`.
const IN_WATER_SPEED_MODIFIER: f32 = 0.1;
const OUTSIDE_WATER_SPEED_MODIFIER: f32 = 0.5;
/// Vanilla `Axolotl.TOTAL_PLAYDEAD_TIME`.
pub const TOTAL_PLAYDEAD_TIME: i32 = 200;
/// Vanilla `Axolotl.PLAYER_REGEN_DETECTION_RANGE`.
const PLAYER_REGEN_DETECTION_RANGE: f64 = 20.0;
/// Vanilla `Axolotl.REGEN_BUFF_MAX_DURATION`.
const REGEN_BUFF_MAX_DURATION: i32 = 2400;
/// Vanilla `Axolotl.REGEN_BUFF_BASE_DURATION`.
const REGEN_BUFF_BASE_DURATION: i32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum AxolotlVariant {
    #[default]
    Lucy = 0,
    Wild = 1,
    Gold = 2,
    Cyan = 3,
    Blue = 4,
}

impl AxolotlVariant {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Wild,
            2 => Self::Gold,
            3 => Self::Cyan,
            4 => Self::Blue,
            _ => Self::Lucy,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub fn random_variant() -> Self {
        let mut rng = rand::rng();
        if rng.random_range(0..1200) == 0 {
            Self::Blue
        } else {
            match rng.random_range(0..4) {
                1 => Self::Wild,
                2 => Self::Gold,
                3 => Self::Cyan,
                _ => Self::Lucy,
            }
        }
    }
}

pub struct AxolotlEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub variant: AtomicI32,
    pub playing_dead: AtomicBool,
    pub from_bucket: AtomicBool,
}

impl AxolotlEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let variant = AxolotlVariant::random_variant();
        let axolotl = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            variant: AtomicI32::new(variant.id()),
            playing_dead: AtomicBool::new(false),
            from_bucket: AtomicBool::new(false),
        };
        *axolotl
            .mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            Navigator::new(AmphibiousPathNavigation::new(false).steered_by_move_control());
        // Vanilla `Animal`'s and `Axolotl`'s constructors; after the navigator, which
        // holds the maluses.
        set_animal_pathfinding_malus(&axolotl.mob_entity);
        axolotl
            .mob_entity
            .set_pathfinding_malus(PathType::Water, 0.0);
        // Vanilla `AxolotlMoveControl` and `AxolotlLookControl`; both pause while the
        // axolotl plays dead (`controls_paused`).
        *axolotl
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(SmoothSwimmingMoveControl::new(
            85,
            10,
            IN_WATER_SPEED_MODIFIER,
            OUTSIDE_WATER_SPEED_MODIFIER,
            false,
        ));
        *axolotl
            .mob_entity
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = LookControl::smooth_swimming(20.0);
        let mob_arc = Arc::new(axolotl);
        mob_arc.mob_entity.init_brain(mob_arc.as_ref());
        mob_arc
    }

    #[must_use]
    pub fn get_variant(&self) -> AxolotlVariant {
        AxolotlVariant::from_id(self.variant.load(Ordering::Relaxed))
    }

    pub fn set_variant(&self, variant: AxolotlVariant) {
        self.variant.store(variant.id(), Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::axolotl::DATA_VARIANT,
            VarInt(variant.id()),
        );
    }

    #[must_use]
    pub fn is_playing_dead(&self) -> bool {
        self.playing_dead.load(Ordering::Relaxed)
    }

    pub fn set_playing_dead(&self, playing_dead: bool) {
        self.playing_dead.store(playing_dead, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::axolotl::DATA_PLAYING_DEAD,
            playing_dead,
        );
    }

    #[must_use]
    pub fn is_from_bucket(&self) -> bool {
        self.from_bucket.load(Ordering::Relaxed)
    }

    pub fn set_from_bucket(&self, from_bucket: bool) {
        self.from_bucket.store(from_bucket, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::axolotl::FROM_BUCKET,
            from_bucket,
        );
    }
}

impl AxolotlEntity {
    /// Vanilla `Axolotl.rehydrate`.
    pub fn rehydrate(&self) {
        let breath = &self.mob_entity.living_entity.breath;
        breath.set_air_supply(self, breath.air() + 1800);
    }
}

impl AxolotlEntity {
    /// Vanilla `Axolotl.saveToBucketTag`: the default bucket data, the variant component,
    /// the age and the hunting cooldown still to run.
    fn save_to_bucket_tag(&self, bucket: &mut ItemStack) {
        let mut tag = crate::entity::bucketable::save_default_data_to_bucket_tag(self, bucket);
        bucket.set_data_component(pumpkin_data::data_component_impl::AxolotlVariantImpl {
            value: std::borrow::Cow::Borrowed(
                pumpkin_data::data_component_impl::AXOLOTL_VARIANTS
                    [self.get_variant().id() as usize],
            ),
        });
        tag.put_int("Age", self.get_age());
        tag.put_bool("AgeLocked", self.is_age_locked());
        let hunting_cooldown = self.mob_entity.with_brain(self, |tick| {
            tick.brain.time_until_expiry(
                crate::entity::ai::brain::memory::types::HAS_HUNTING_COOLDOWN.id(),
            )
        });
        if let Some(cooldown) = hunting_cooldown {
            tag.put_long("HuntingCooldown", cooldown);
        }
        bucket.set_data_component(pumpkin_data::data_component_impl::BucketEntityDataImpl {
            nbt: Some(tag),
        });
    }
}

impl AgeableMob for AxolotlEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for AxolotlEntity {
    /// Vanilla `Axolotl.getWalkTargetValue`: no spot is better than another.
    fn animal_walk_target_value(&self, _pos: &pumpkin_util::math::position::BlockPos) -> f32 {
        0.0
    }

    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_AXOLOTL_FOOD)
    }
}

/// Vanilla `Axolotl.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::AXOLOTL_BABY_DIMENSIONS;

impl Mob for AxolotlEntity {
    /// Vanilla `Axolotl.loadFromBucketTag`.
    fn load_from_bucket_tag(&self, tag: &NbtCompound) {
        use crate::entity::ai::brain::memory::types::HAS_HUNTING_COOLDOWN;
        crate::entity::bucketable::load_default_data_from_bucket_tag(self, tag);
        self.set_age(tag.get_int("Age").unwrap_or(0));
        self.set_age_locked(tag.get_bool("AgeLocked").unwrap_or(false));
        let cooldown = tag.get_long("HuntingCooldown");
        self.mob_entity.with_brain(self, |tick| match cooldown {
            Some(ttl) => tick.brain.set_with_expiry(HAS_HUNTING_COOLDOWN, true, ttl),
            None => tick.brain.erase(HAS_HUNTING_COOLDOWN.id()),
        });
    }

    fn set_from_bucket(&self, from_bucket: bool) {
        Self::set_from_bucket(self, from_bucket);
    }

    fn mob_set_variant_name(&self, name: &str) {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        if let Some(id) = pumpkin_data::data_component_impl::AXOLOTL_VARIANTS
            .iter()
            .position(|variant| *variant == name)
        {
            self.set_variant(AxolotlVariant::from_id(id as i32));
        }
    }

    /// Vanilla `Axolotl.playAttackSound`.
    fn play_attack_sound(&self) {
        self.play_sound_at_self(pumpkin_data::sound::Sound::EntityAxolotlAttack, 1.0, 1.0);
    }

    /// Vanilla `Axolotl.checkSpawnObstruction`: water doesn't obstruct it.
    fn check_spawn_obstruction(&self, world: &crate::world::World) -> bool {
        let entity = self.get_entity();
        world.is_unobstructed(Some(entity), &entity.bounding_box.load())
    }

    /// Vanilla `Axolotl.getAmbientSound`.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        Some(if self.get_entity().is_in_water() {
            pumpkin_data::sound::Sound::EntityAxolotlIdleWater
        } else {
            pumpkin_data::sound::Sound::EntityAxolotlIdleAir
        })
    }

    /// Vanilla `Axolotl.playAmbientSound`: silent while playing dead.
    fn play_ambient_sound(&self) {
        if !self.is_playing_dead()
            && let Some(sound) = self.ambient_sound()
        {
            self.make_sound(sound);
        }
    }

    /// Vanilla `Axolotl.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    /// Vanilla `Axolotl.handleAirSupply`, which `Axolotl.baseTick` skips for
    /// mobs without AI.
    fn handle_air_supply(&self, pre_tick_air: i32) {
        if self.mob_entity.is_no_ai() {
            return;
        }
        let in_water_or_rain = self.get_entity().is_in_water_or_rain();
        BreathManager::handle_air_out_of_water(
            self,
            pre_tick_air,
            in_water_or_rain,
            self.max_air_supply(),
            DamageType::DRY_OUT,
        );
    }

    /// `Axolotl.getMaxAirSupply`.
    fn max_air_supply(&self) -> i32 {
        6000
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int("Variant", self.get_variant().id());
        nbt.put_bool("FromBucket", self.is_from_bucket());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if let Some(variant) = nbt.get_int("Variant") {
            self.set_variant(AxolotlVariant::from_id(variant));
        }
        if let Some(from_bucket) = nbt.get_bool("FromBucket") {
            self.set_from_bucket(from_bucket);
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();
    }

    fn make_brain(
        &self,
        packed: &crate::entity::ai::brain::memory::PackedMemories,
    ) -> crate::entity::ai::brain::Brain {
        axolotl_ai::AXOLOTL_PROVIDER.make_brain(self, packed)
    }

    /// The rest of vanilla `Axolotl.customServerAiStep`: the activity update, and
    /// playing dead while its ticks last.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        axolotl_ai::update_activity(tick);
        if !self.mob_entity.is_no_ai() {
            let playing_dead = tick
                .brain
                .get(types::PLAY_DEAD_TICKS)
                .is_some_and(|ticks| *ticks > 0);
            if playing_dead != self.is_playing_dead() {
                self.set_playing_dead(playing_dead);
            }
        }
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !crate::entity::ai::brain::behavior::utils::is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
    }

    /// Vanilla `Axolotl.travelInWater`.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        if !entity.is_in_water() {
            return false;
        }
        entity.update_velocity_from_input(living.movement_input.load(), living.get_speed(caller));
        living.make_move(caller);
        entity.velocity.store(entity.velocity.load() * 0.9);
        true
    }

    /// Vanilla `Axolotl.isPushedByFluid`.
    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }

    /// Vanilla `Axolotl.hurtServer`: one hit in three, in water, from an attacker,
    /// that is heavy or lands below half health, makes the axolotl play dead.
    fn before_hurt(
        &self,
        amount: f32,
        _damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) {
        let living = &self.mob_entity.living_entity;
        let current_health = living.health.load();
        let max_health = living.get_max_health();
        // `this.random.nextInt(3) == 0 && (this.random.nextInt(3) < damage || ...)`, short-
        // circuited as in vanilla; one guard for both draws.
        let rolls_pass = !self.mob_entity.is_no_ai() && {
            let mut random = living.entity.random();
            random.next_bounded_i32(3) == 0
                && ((random.next_bounded_i32(3) as f32) < amount
                    || current_health / max_health < 0.5)
        };
        if rolls_pass
            && amount < current_health
            && living.entity.is_in_water()
            && (cause.is_some() || source.is_some())
            && !self.is_playing_dead()
        {
            self.mob_entity.post_to_brain(Box::new(|tick| {
                tick.brain.set(types::PLAY_DEAD_TICKS, TOTAL_PLAYDEAD_TIME);
            }));
        }
    }

    /// Vanilla `Axolotl.canBeSeenAsEnemy`.
    fn can_be_seen_as_enemy(&self) -> bool {
        !self.is_playing_dead()
    }

    fn controls_paused(&self) -> bool {
        self.is_playing_dead()
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::axolotl::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::axolotl::DATA_VARIANT,
            VarInt(self.get_variant().id()),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::axolotl::DATA_PLAYING_DEAD,
            self.is_playing_dead(),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::axolotl::FROM_BUCKET,
            self.is_from_bucket(),
        );
    }

    fn mob_interact(
        &self,
        player: &Arc<Player>,
        _hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        // `Bucketable.bucketMobPickup`, else `Animal.mobInteract`.
        if crate::entity::bucketable::bucket_mob_pickup(
            self,
            player,
            item_stack,
            &Item::AXOLOTL_BUCKET,
            Sound::ItemBucketFillAxolotl,
            |bucket| self.save_to_bucket_tag(bucket),
        ) {
            return InteractionResult::SUCCESS;
        }

        self.animal_interact(player, item_stack, Sound::EntityAxolotlIdleAir)
    }
}

/// Vanilla `Axolotl.onStopAttacking`: when a target the axolotl was fighting dies to a
/// player within 20 blocks, that player gets the axolotl's support.
pub(crate) fn on_stop_attacking(tick: &mut BrainTick<'_>, target: &Arc<dyn EntityBase>) {
    if !crate::entity::ai::brain::behavior::utils::is_dead_or_dying(target.as_ref()) {
        return;
    }
    let Some(living) = target.get_living_entity() else {
        return;
    };
    if living.get_last_damage_type().is_none() {
        return;
    }
    let Some(player) = tick
        .world
        .get_player_by_id(living.last_attacker_id.load(Ordering::Relaxed))
    else {
        return;
    };
    let range = tick.mob.get_entity().bounding_box.load().expand(
        PLAYER_REGEN_DETECTION_RANGE,
        PLAYER_REGEN_DETECTION_RANGE,
        PLAYER_REGEN_DETECTION_RANGE,
    );
    if player.get_entity().bounding_box.load().intersects(&range) {
        apply_supporting_effects(&player);
    }
}

/// Vanilla `Axolotl.applySupportingEffects`: regeneration that stacks to two minutes,
/// and mining fatigue lifted.
fn apply_supporting_effects(player: &Player) {
    let regen = player.living_entity.get_effect(&StatusEffect::REGENERATION);
    let ends_within = regen
        .as_ref()
        .is_none_or(|effect| effect.duration != -1 && effect.duration < REGEN_BUFF_MAX_DURATION);
    if ends_within {
        let previous = regen.map_or(0, |effect| effect.duration);
        player.add_effect(Effect {
            effect_type: &StatusEffect::REGENERATION,
            duration: REGEN_BUFF_MAX_DURATION.min(REGEN_BUFF_BASE_DURATION + previous),
            amplifier: 0,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        });
    }
    player.remove_effect(&StatusEffect::MINING_FATIGUE);
}
