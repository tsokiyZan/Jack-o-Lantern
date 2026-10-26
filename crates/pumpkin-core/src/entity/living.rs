use crate::enchantment::HurtAndBreak;
use pumpkin_data::item::Item;
use pumpkin_data::particle::Particle;
use pumpkin_data::potion::Effect;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_inventory::build_equipment_slots;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_protocol::bedrock::client::take_item_actor::CTakeItemActor;
use pumpkin_protocol::bedrock::server::actor_event::{ActorEventID, SActorEvent};
use pumpkin_protocol::codec::var_ulong::VarULong;
use pumpkin_util::Difficulty;
use pumpkin_util::GameMode;
use pumpkin_util::Hand;
use pumpkin_util::math::position::BlockPos;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicU8,
    Ordering::{Relaxed, SeqCst},
};

use super::breath::BreathManager;
use super::experience_orb::ExperienceOrbEntity;
use super::{Entity, EntityBase, NBTStorageInit};
use crate::enchantment::EnchantmentHelper;
use crate::entity::NBTStorage;
use crate::entity::ageable::AgeableMob;
use crate::entity::attributes::AttributeInstance;
use crate::entity::attributes::Modifier;
use crate::entity::attributes::ModifierOperation;
use crate::entity::combat::{CombatRules, CombatTracker, FallLocation, knockback_after_resistance};
use crate::entity::damage_source::DamageSource;
use crate::entity::effect::MobEffectInstance;
use crate::entity::mob::equipment::DEFAULT_EQUIPMENT_DROP_CHANCE;
use crate::entity::player::advancement::trigger::AdvancementTrigger;
use crate::entity::player::cooldown_group;
use crate::entity::player::statistics::{CustomStatistic, StatisticCategory};
use crate::server::Server;
use crate::world::loot::LootContextParameters;
use crossbeam::atomic::AtomicCell;
use pumpkin_data::AttributeModifierSlot;
use pumpkin_data::Block;
use pumpkin_data::BlockState;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::block_sound_type::SoundType;
use pumpkin_data::data_component_impl::IdOr;
use pumpkin_data::data_component_impl::SwingAnimationImpl;
use pumpkin_data::data_component_impl::food::{ConsumableImpl, ConsumeAnimation, ConsumeEffect};
use pumpkin_data::data_component_impl::{
    AttributeModifiersImpl, BlocksAttacksImpl, DeathProtectionImpl, EquipmentSlot, EquippableImpl,
    FoodImpl, IDSet, MobVisibilityImpl, WeaponImpl,
};
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::{EntityPose, EntityStatus, EntityType};
use pumpkin_data::fluid::Fluid;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::game_rules::{GameRule, GameRuleValue};
use pumpkin_data::item_stack::{DamageResult, ItemStack};
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::{Enchantment, damage::DamageType, sound::Sound};
use pumpkin_inventory::entity_equipment::EntityEquipment;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::java::client::play::{
    CEntityStatus, CHurtAnimation, CRemoveMobEffect, CTakeItemEntity,
};
use pumpkin_protocol::{
    codec::item_stack_seralizer::ItemStackSerializer,
    java::client::play::{CSetEquipment, MetadataSerializer},
    ser::{NetworkWriteExt, WritingError},
};
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_util::text::TextComponent;
use std::sync::RwLock;

/// Represents a living entity within the game world.
///
/// This struct encapsulates the core properties and behaviors of living entities, including players, mobs, and other creatures.
/// The entity types whose classes extend vanilla `Projectile`.
const PROJECTILE_TYPES: [&EntityType; 19] = [
    &EntityType::ARROW,
    &EntityType::SPECTRAL_ARROW,
    &EntityType::TRIDENT,
    &EntityType::EGG,
    &EntityType::SNOWBALL,
    &EntityType::ENDER_PEARL,
    &EntityType::EXPERIENCE_BOTTLE,
    &EntityType::SPLASH_POTION,
    &EntityType::LINGERING_POTION,
    &EntityType::FIREBALL,
    &EntityType::SMALL_FIREBALL,
    &EntityType::DRAGON_FIREBALL,
    &EntityType::WITHER_SKULL,
    &EntityType::WIND_CHARGE,
    &EntityType::BREEZE_WIND_CHARGE,
    &EntityType::FIREWORK_ROCKET,
    &EntityType::LLAMA_SPIT,
    &EntityType::SHULKER_BULLET,
    &EntityType::FISHING_BOBBER,
];

/// Vanilla `LivingEntity.SPEED_MODIFIER_POWDER_SNOW_ID`.
const SPEED_MODIFIER_POWDER_SNOW_ID: &str = "minecraft:powder_snow";

pub struct LivingEntity {
    /// The underlying entity object, providing basic entity information and functionality.
    pub entity: Entity,
    /// Air supply, drowning and dry-out damage (`LivingEntity.baseTick`).
    pub breath: BreathManager,
    /// Tracks the remaining time until the entity can regenerate health.
    pub hurt_cooldown: AtomicI32,
    /// Vanilla `LivingEntity.hurtTime`: counts down from `HURT_DURATION` after a hit that
    /// was not softened by the damage cooldown.
    pub hurt_time: AtomicI32,
    /// Vanilla `LivingEntity.noActionTime`: ticks since a mob last had a player nearby
    /// or got hurt.
    pub no_action_time: AtomicI32,
    /// Vanilla `LivingEntity.speed`: the speed the move control last set, which travel
    /// moves the entity by. Players use their movement speed attribute instead.
    pub speed: AtomicCell<f32>,
    /// The `scale` attribute the current dimensions were built with, so a change can
    /// refresh them like vanilla's `onAttributeUpdated`.
    applied_scale: AtomicCell<f32>,
    /// Stores the amount of damage the entity last received.
    pub last_damage_taken: AtomicCell<f32>,
    /// The current health level of the entity.
    pub health: AtomicCell<f32>,
    /// The current absorption (yellow hearts) on the entity.
    pub absorption: AtomicCell<f32>,
    pub item_use_time: AtomicI32,
    pub item_in_use: std::sync::Mutex<Option<ItemStack>>,
    pub active_hand: std::sync::Mutex<Option<Hand>>,
    /// Vanilla `autoSpinAttackTicks`: the riptide spin's ticks left.
    pub auto_spin_attack_ticks: AtomicI32,
    pub auto_spin_attack_dmg: AtomicCell<f32>,
    /// Vanilla `autoSpinAttackItemStack`, with the inventory slot it was used from: Pumpkin's
    /// stacks are copies, so the slot stands in for vanilla's object identity.
    pub auto_spin_attack_item: std::sync::Mutex<Option<(ItemStack, usize)>>,
    pub recent_kinetic_enemies: std::sync::Mutex<FxHashMap<i32, i32>>,
    swing_state: std::sync::Mutex<SwingState>,
    pub death_time: AtomicU8,
    /// Indicates whether the entity is dead. (`on_death` called)
    pub dead: AtomicBool,
    /// Vanilla `LivingEntity.discardFriction`: travel in air keeps the horizontal and
    /// vertical speed undamped, as a long jump needs.
    pub discard_friction: AtomicBool,
    /// Vanilla `currentImpulseContextResetGraceTime`.
    pub current_impulse_context_reset_grace_time: AtomicI32,
    /// Vanilla `currentImpulseImpactPos`: while set, falls are measured from this height.
    pub current_impulse_impact_pos: AtomicCell<Option<Vector3<f64>>>,
    pub active_effects: std::sync::Mutex<FxHashMap<&'static StatusEffect, MobEffectInstance>>,
    /// Vanilla `fallFlyTicks`.
    pub fall_fly_ticks: AtomicI32,
    /// The synced `SLEEPING_POS_ID`: the bed this entity sleeps in.
    sleeping_pos: AtomicCell<Option<BlockPos>>,
    /// The synced `DATA_ARROW_COUNT_ID` and its countdown, `removeArrowTime`.
    arrow_count: AtomicI32,
    remove_arrow_time: AtomicI32,
    /// The synced `DATA_STINGER_COUNT_ID` and its countdown, `removeStingerTime`.
    stinger_count: AtomicI32,
    remove_stinger_time: AtomicI32,
    /// Vanilla `effectsDirty`: the invisibility, glowing and particle data are refreshed
    /// before the next sync.
    effects_dirty: AtomicBool,
    /// Vanilla `AttributeMap.attributesToUpdate`: attribute ids changed since the last
    /// `refreshDirtyAttributes`.
    attributes_to_update: std::sync::Mutex<Vec<u8>>,
    /// Vanilla `locatorBarIcon`: how this entity shows on other players' locator bars.
    pub locator_bar_icon: std::sync::Mutex<crate::entity::waypoint::WaypointIconState>,
    pub entity_equipment: Arc<std::sync::Mutex<EntityEquipment>>,
    pub equipment_drop_chances: Arc<std::sync::Mutex<FxHashMap<EquipmentSlot, f32>>>,
    pub movement_input: AtomicCell<Vector3<f64>>,
    pub equipment_slots: Arc<FxHashMap<usize, EquipmentSlot>>,

    pub jumping: AtomicBool,

    pub jumping_cooldown: AtomicU8,

    /// The position where the entity was last climbing, used for death messages
    pub climbing_pos: AtomicCell<Option<BlockPos>>,

    /// The entity ID of the entity that last attacked this living entity.
    pub last_attacker_id: AtomicI32,
    /// The tick at which this entity was last attacked (entity age).
    pub last_attacked_time: AtomicI32,
    /// The UUID behind `last_attacker_id`: vanilla keeps `lastHurtByMob` as an entity
    /// reference, saved and resolved again after a reload.
    last_hurt_by_mob_uuid: AtomicCell<Option<uuid::Uuid>>,
    last_damage_type: std::sync::Mutex<Option<DamageType>>,
    last_damage_stamp: std::sync::atomic::AtomicI64,

    /// The entity ID of the entity this living entity last attacked.
    pub last_attacking_id: AtomicI32,
    /// The tick at which this entity last attacked something (entity age).
    pub last_attack_time: AtomicI32,

    /// Tracks combat entries, assisted falls, kill credit, and death messages.
    pub combat_tracker: std::sync::Mutex<CombatTracker>,

    /// Vanilla `lastHurtByPlayer`.
    pub last_hurt_by_player: AtomicCell<Option<uuid::Uuid>>,
    /// Vanilla `lastHurtByPlayerMemoryTime`: ticks left before `last_hurt_by_player` is
    /// forgotten; a kill while it runs counts as a player kill.
    pub last_hurt_by_player_memory_time: AtomicI32,

    water_movement_speed_multiplier: f32,
    livings_flags: AtomicU8,

    /// The last block position the entity occupied, used to trigger location changed effects.
    pub last_block_pos: AtomicCell<Option<BlockPos>>,

    /// The attributes of the entity
    pub attributes: RwLock<FxHashMap<u8, AttributeInstance>>,
    /// Vanilla `lastEquipmentItems`, in `EquipmentSlot::VALUES` order: the equipment as the
    /// last `detect_equipment_updates` saw it.
    last_equipment_items: std::sync::Mutex<[ItemStack; 8]>,
    /// Attributes changed outside `detect_equipment_updates` that vanilla only marks dirty;
    /// the next `detect_equipment_updates` syncs them with its own.
    pending_dirty_attributes: std::sync::Mutex<Vec<Attributes>>,
    /// Vanilla `activeLocationDependentEnchantments`: the location-changed effects active per
    /// slot, as the enchantment and the effect's index in its `location_changed` list.
    pub active_location_effects:
        std::sync::Mutex<Vec<(EquipmentSlot, &'static pumpkin_data::Enchantment, usize)>>,
}

#[derive(Clone)]
struct EffectParticle {
    particle_id: VarInt,
    /// The `entity_effect` colour; the effects with their own particle have none.
    color: Option<i32>,
}

#[derive(Clone)]
struct EffectParticles(Vec<EffectParticle>);

impl MetadataSerializer for EffectParticles {
    fn write_metadata(
        &self,
        writer: &mut impl std::io::Write,
        _version: &pumpkin_util::version::JavaMinecraftVersion,
    ) -> Result<(), WritingError> {
        let count = i32::try_from(self.0.len())
            .map_err(|_| WritingError::Message("Too many effect particles".into()))?;
        writer.write_var_int(&VarInt(count))?;
        for particle in &self.0 {
            writer.write_var_int(&particle.particle_id)?;
            if let Some(color) = particle.color {
                writer.write_i32(color)?;
            }
        }
        Ok(())
    }
}

impl EffectParticle {
    /// Vanilla `MobEffectInstance.getParticleOptions`.
    fn from_effect(effect: &Effect) -> Self {
        crate::entity::effect::effect_particle(effect.effect_type).map_or_else(
            || Self {
                particle_id: VarInt(Particle::EntityEffect as i32),
                color: Some(
                    (((if effect.ambient { 38 } else { 255 }) as u32) << 24
                        | effect.effect_type.color as u32) as i32,
                ),
            },
            |particle| Self {
                particle_id: VarInt(particle as i32),
                color: None,
            },
        )
    }
}

/// Vanilla `LivingEntity.canBeSeenAsEnemy`, with `Player`'s `abilities.invulnerable` override.
#[must_use]
pub fn can_be_seen_as_enemy(target: &dyn EntityBase) -> bool {
    target.get_player().map_or_else(
        || {
            target
                .get_living_entity()
                .is_some_and(LivingEntity::can_take_damage)
                && target
                    .get_mob()
                    .is_none_or(crate::entity::mob::Mob::can_be_seen_as_enemy)
        },
        crate::entity::player::Player::can_be_seen_as_enemy,
    )
}

fn is_allowed_by_team_rules(
    own_team: Option<&crate::world::scoreboard::Team>,
    their_team: Option<&crate::world::scoreboard::Team>,
) -> bool {
    use crate::world::scoreboard::CollisionRule;

    let own_rule = own_team.map_or(CollisionRule::Always, |team| team.collision_rule);
    let their_rule = their_team.map_or(CollisionRule::Always, |team| team.collision_rule);

    if own_rule == CollisionRule::Never || their_rule == CollisionRule::Never {
        return false;
    }

    let same_team = own_team
        .zip(their_team)
        .is_some_and(|(own, their)| own.name == their.name);

    if (own_rule == CollisionRule::PushOwnTeam || their_rule == CollisionRule::PushOwnTeam)
        && same_team
    {
        return false;
    }

    (own_rule != CollisionRule::PushOtherTeams && their_rule != CollisionRule::PushOtherTeams)
        || same_team
}

/// Whether the riptide stack used from `spin_slot` is the active item in `active_slot`. Vanilla
/// compares the objects; Pumpkin's stacks are copies, so the slots must match and the slot must
/// still hold an equal stack.
pub(crate) fn spin_stack_is_active(
    spin_stack: &ItemStack,
    spin_slot: usize,
    active_stack: &ItemStack,
    active_slot: usize,
) -> bool {
    spin_slot == active_slot && spin_stack.are_equal(active_stack)
}

impl LivingEntity {
    const USING_ITEM_FLAG: u8 = 1;
    const OFF_HAND_ACTIVE_FLAG: u8 = 2;
    const RANDOM_TELEPORT_ATTEMPTS: usize = 16;
    pub(crate) const AUTO_SPIN_ATTACK_FLAG: u8 = 4;

    fn hurt_sound_for_entity(entity_type: &'static EntityType) -> Sound {
        entity_type.hurt_sound.unwrap_or(Sound::EntityGenericHurt)
    }

    fn death_sound_for_entity(entity_type: &'static EntityType) -> Sound {
        entity_type.death_sound.unwrap_or(Sound::EntityGenericDeath)
    }

    fn get_pitch(&self) -> f32 {
        let is_baby = self
            .get_mob()
            .and_then(|x| x.as_ageable())
            .is_some_and(AgeableMob::is_baby);

        let mut random = self.entity.random();
        if is_baby {
            (random.next_f32() - random.next_f32()) * 0.2 + 1.5
        } else {
            (random.next_f32() - random.next_f32()) * 0.2 + 1.0
        }
    }

    pub fn new(entity: Entity) -> Self {
        // Vanilla `LivingEntity(type, level)`: `setYRot(random.nextFloat() * (float) (Math.PI * 2))`.
        // Every spawn path sets the real rotation afterwards, so Pumpkin keeps the yaw the
        // entity was made with and only takes the draw.
        entity.random().next_f32();
        let water_movement_speed_multiplier = if entity.entity_type == &EntityType::POLAR_BEAR {
            0.98
        } else if entity.entity_type == &EntityType::SKELETON_HORSE {
            0.96
        } else {
            0.8
        };
        let mut max_health: f32 = 20.0; // Overridden by attribute base below
        let living = Self {
            // Vanilla `AttributeMap`: instances are made from the type's supplier on first use.
            attributes: {
                for (attr, base) in entity.entity_type.attributes {
                    if attr.id == Attributes::MAX_HEALTH.id {
                        max_health = *base as f32;
                    }
                }
                std::sync::RwLock::new(FxHashMap::default())
            },
            health: AtomicCell::new(max_health), // Initial health value from attributes
            entity,
            hurt_cooldown: AtomicI32::new(0),
            hurt_time: AtomicI32::new(0),
            no_action_time: AtomicI32::new(0),
            applied_scale: AtomicCell::new(1.0),
            speed: AtomicCell::new(0.0),
            last_damage_taken: AtomicCell::new(0.0),
            absorption: AtomicCell::new(0.0),
            current_impulse_context_reset_grace_time: AtomicI32::new(0),
            current_impulse_impact_pos: AtomicCell::new(None),
            death_time: AtomicU8::new(0),
            dead: AtomicBool::new(false),
            discard_friction: AtomicBool::new(false),
            item_use_time: AtomicI32::new(0),
            item_in_use: std::sync::Mutex::new(None),
            active_hand: std::sync::Mutex::new(None),
            auto_spin_attack_ticks: AtomicI32::new(0),
            auto_spin_attack_dmg: AtomicCell::new(0.0),
            auto_spin_attack_item: std::sync::Mutex::new(None),
            recent_kinetic_enemies: std::sync::Mutex::new(FxHashMap::default()),
            swing_state: std::sync::Mutex::new(SwingState::default()),
            livings_flags: AtomicU8::new(0),
            active_effects: std::sync::Mutex::new(FxHashMap::default()),
            effects_dirty: AtomicBool::new(false),
            fall_fly_ticks: AtomicI32::new(0),
            sleeping_pos: AtomicCell::new(None),
            arrow_count: AtomicI32::new(0),
            remove_arrow_time: AtomicI32::new(0),
            stinger_count: AtomicI32::new(0),
            remove_stinger_time: AtomicI32::new(0),
            attributes_to_update: std::sync::Mutex::new(Vec::new()),
            locator_bar_icon: std::sync::Mutex::new(
                crate::entity::waypoint::WaypointIconState::default(),
            ),
            entity_equipment: Arc::new(std::sync::Mutex::new(EntityEquipment::new())),
            equipment_drop_chances: Arc::new(std::sync::Mutex::new(FxHashMap::default())),
            equipment_slots: Arc::new(build_equipment_slots()),
            jumping: AtomicBool::new(false),
            jumping_cooldown: AtomicU8::new(0),
            climbing_pos: AtomicCell::new(None),
            last_attacker_id: AtomicI32::new(0),
            last_attacked_time: AtomicI32::new(0),
            last_hurt_by_mob_uuid: AtomicCell::new(None),
            last_damage_type: std::sync::Mutex::new(None),
            last_damage_stamp: std::sync::atomic::AtomicI64::new(0),
            last_attacking_id: AtomicI32::new(0),
            last_attack_time: AtomicI32::new(0),
            combat_tracker: std::sync::Mutex::new(CombatTracker::new()),
            last_hurt_by_player: AtomicCell::new(None),
            last_hurt_by_player_memory_time: AtomicI32::new(0),
            movement_input: AtomicCell::new(Vector3::default()),
            water_movement_speed_multiplier,
            last_block_pos: AtomicCell::new(None),
            last_equipment_items: std::sync::Mutex::new(std::array::from_fn(|_| {
                ItemStack::EMPTY.clone()
            })),
            pending_dirty_attributes: std::sync::Mutex::new(Vec::new()),
            active_location_effects: std::sync::Mutex::new(Vec::new()),
            breath: BreathManager::default(),
        };
        // Vanilla's constructor `setHealth(getMaxHealth())`, so spawn data carries it.
        living
            .entity
            .set_synced_data(tracked_data::living_entity::DATA_HEALTH_ID, max_health);
        living
    }

    /// Vanilla `LivingEntity.getKillCredit`: the player that hurt it recently, otherwise
    /// `lastHurtByMob`.
    pub fn get_kill_credit(&self) -> Option<Arc<dyn EntityBase>> {
        if let Some(uuid) = self.last_hurt_by_player.load() {
            return self
                .entity
                .world
                .load()
                .get_player_by_uuid(uuid)
                .map(|player| player as Arc<dyn EntityBase>);
        }
        self.get_last_hurt_by_mob()
    }

    /// Vanilla `LivingEntity.onChangedBlock`: the location-changed enchantment effects.
    pub fn on_changed_block(&self, caller: &dyn EntityBase, _pos: BlockPos) {
        // `ServerPlayer.onChangedBlock`: not for spectators.
        if !caller.is_spectator() {
            crate::enchantment::EnchantmentHelper::run_location_changed_effects(caller, self);
        }
    }

    /// Vanilla `detectEquipmentUpdates`: what changed in the equipment since the last tick
    /// takes effect and goes to the tracking players.
    pub fn detect_equipment_updates(&self, caller: &dyn EntityBase) {
        let last = self
            .last_equipment_items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut touched = std::mem::take(
            &mut *self
                .pending_dirty_attributes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        if let Some(mut changed) = self.collect_equipment_changes(caller, &last, &mut touched) {
            self.handle_hand_swap(&mut changed);
            if !changed.is_empty() {
                self.handle_equipment_changes(&changed);
            }
        }
        // Vanilla syncs the dirty attributes later in the tick (`sendDirtyEntityData`).
        if !touched.is_empty() {
            crate::entity::attributes::send_attribute_updates_for_living(self, touched);
        }
    }

    /// Leaves `attributes` dirty for the next `detect_equipment_updates` to sync.
    pub fn mark_attributes_dirty(&self, attributes: &[Attributes]) {
        let mut pending = self
            .pending_dirty_attributes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for attribute in attributes {
            push_unique_attribute(&mut pending, attribute);
        }
    }

    /// Forgets the equipment the last `detect_equipment_updates` saw, like the empty
    /// `lastEquipmentItems` of a new entity.
    pub fn clear_last_equipment_items(&self) {
        *self
            .last_equipment_items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            std::array::from_fn(|_| ItemStack::EMPTY.clone());
    }

    /// Vanilla `collectEquipmentChanges`: the slots whose stack differs from `last`, in
    /// `EquipmentSlot::VALUES` order. The previous stacks' modifiers and location effects stop
    /// and the new stacks' start; the attributes this changes are added to `touched`.
    pub fn collect_equipment_changes(
        &self,
        caller: &dyn EntityBase,
        last: &[ItemStack; 8],
        touched: &mut Vec<Attributes>,
    ) -> Option<Vec<(EquipmentSlot, ItemStack)>> {
        let mob = caller.get_mob();
        if let Some(mob) = mob {
            mob.collect_equipment_changes_override(last, touched);
        }
        let mut changed: Option<Vec<(EquipmentSlot, ItemStack)>> = None;
        for (slot, previous) in EquipmentSlot::VALUES.iter().zip(last) {
            let current = self.get_item_by_slot(caller, slot);
            let has_changed = mob.map_or_else(
                || !ItemStack::matches(previous, &current),
                |mob| mob.equipment_has_changed(previous, &current),
            );
            if has_changed {
                changed
                    .get_or_insert_with(Vec::new)
                    .push((slot.clone(), current));
                if !previous.is_empty() {
                    self.stop_location_based_effects(previous, slot, touched);
                }
            }
        }

        if let Some(changed) = &changed {
            for (slot, current) in changed {
                if current.is_empty() || current.is_broken() {
                    continue;
                }
                for_each_item_modifier(current, slot, |attribute, modifier| {
                    // `removeModifier` then `addTransientModifier`.
                    if self.modify_supplied_attribute(attribute, |instance| {
                        instance.remove_modifier(&modifier.id);
                        instance.add_or_replace_modifier(modifier);
                        true
                    }) {
                        push_unique_attribute(touched, attribute);
                    }
                });
                EnchantmentHelper::run_location_changed_effects_in_slot(
                    caller, self, current, slot,
                );
            }
        }
        changed
    }

    /// Vanilla `handleHandSwap`: hands that only traded places play the swap animation
    /// instead of sending both stacks.
    fn handle_hand_swap(&self, changed: &mut Vec<(EquipmentSlot, ItemStack)>) {
        let stack_in = |changed: &[(EquipmentSlot, ItemStack)], slot: &EquipmentSlot| {
            changed
                .iter()
                .find(|(changed_slot, _)| changed_slot == slot)
                .map(|(_, stack)| stack.clone())
        };
        let (Some(main_hand), Some(off_hand)) = (
            stack_in(changed, &EquipmentSlot::MAIN_HAND),
            stack_in(changed, &EquipmentSlot::OFF_HAND),
        ) else {
            return;
        };
        let main_index = EquipmentSlot::MAIN_HAND.discriminant() as usize;
        let off_index = EquipmentSlot::OFF_HAND.discriminant() as usize;
        let mut last = self
            .last_equipment_items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !ItemStack::matches(&main_hand, &last[off_index])
            || !ItemStack::matches(&off_hand, &last[main_index])
        {
            return;
        }
        last[main_index] = main_hand.clone();
        last[off_index] = off_hand.clone();
        drop(last);
        self.entity.world.load().send_to_tracking_players(
            &self.entity,
            &CEntityStatus::new(self.entity.entity_id, EntityStatus::SwapHands as i8),
        );
        self.send_bedrock_hand_equipment(&[
            (EquipmentSlot::MAIN_HAND, main_hand),
            (EquipmentSlot::OFF_HAND, off_hand),
        ]);
        changed.retain(|(slot, _)| {
            *slot != EquipmentSlot::MAIN_HAND && *slot != EquipmentSlot::OFF_HAND
        });
    }

    /// Vanilla `handleEquipmentChanges`: remembers the new stacks and sends them.
    fn handle_equipment_changes(&self, changed: &[(EquipmentSlot, ItemStack)]) {
        {
            let mut last = self
                .last_equipment_items
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (slot, stack) in changed {
                last[slot.discriminant() as usize] = stack.clone();
            }
        }
        self.update_players_with_new_equipment(changed);
    }

    /// Vanilla `updatePlayersWithNewEquipment`, with Bedrock's per-hand `MobEquipment`.
    fn update_players_with_new_equipment(&self, items: &[(EquipmentSlot, ItemStack)]) {
        let world = self.entity.world.load();
        let equipment_java: Vec<(i8, ItemStackSerializer)> = items
            .iter()
            .map(|(slot, stack)| {
                (
                    slot.discriminant(),
                    ItemStackSerializer::from(stack.clone()),
                )
            })
            .collect();
        world.send_to_tracking_players(
            &self.entity,
            &CSetEquipment::new(self.entity_id().into(), equipment_java),
        );
        self.send_bedrock_hand_equipment(items);
    }

    /// Bedrock's per-hand `MobEquipment` for the hands among `items`; Bedrock has no swap
    /// event, so it gets these for a hand swap too.
    fn send_bedrock_hand_equipment(&self, items: &[(EquipmentSlot, ItemStack)]) {
        let world = self.entity.world.load();
        for (slot, stack) in items {
            let container_id = match slot {
                EquipmentSlot::MainHand(_) => 0,
                EquipmentSlot::OffHand(_) => 120,
                _ => continue,
            };
            world.send_to_tracking_players_bedrock(
                &self.entity,
                &pumpkin_protocol::bedrock::client::CMobEquipment {
                    target_runtime_id: (self.entity_id() as u64).into(),
                    item: pumpkin_protocol::bedrock::network_item::NetworkItemStackDescriptor::from(
                        stack,
                    ),
                    slot: 0,
                    selected_slot: 0,
                    container_id,
                },
            );
        }
    }

    /// Vanilla `stopLocationBasedEffects(ItemStack, EquipmentSlot, AttributeMap)`: the stack
    /// leaving `slot` takes its modifiers and location effects with it.
    fn stop_location_based_effects(
        &self,
        previous: &ItemStack,
        slot: &EquipmentSlot,
        touched: &mut Vec<Attributes>,
    ) {
        for_each_item_modifier(previous, slot, |attribute, modifier| {
            if self.modify_supplied_attribute(attribute, |instance| {
                let present = instance.get_modifier(&modifier.id).is_some();
                instance.remove_modifier(&modifier.id);
                present
            }) {
                push_unique_attribute(touched, attribute);
            }
        });
        EnchantmentHelper::stop_location_based_effects_in_slot(previous, self, slot);
    }

    /// Vanilla `AttributeMap.getInstance` followed by a change to the instance: nothing when
    /// the entity type has no such attribute. `f` says whether it changed anything (vanilla
    /// `setDirty`); the result is whether it did.
    pub fn modify_supplied_attribute(
        &self,
        attribute: &Attributes,
        f: impl FnOnce(&mut AttributeInstance) -> bool,
    ) -> bool {
        let mut map = self
            .attributes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let instance = match map.entry(attribute.id) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let Some(base) = self.supplier_base_value(attribute) else {
                    return false;
                };
                entry.insert(AttributeInstance::new(attribute, base))
            }
        };
        if !f(instance) {
            return false;
        }
        instance.dirty.store(true, Ordering::Relaxed);
        drop(map);
        self.mark_attribute_updated(attribute);
        true
    }

    /// Pumpkin's `EntityPickupItemEvent` for picking `amount` of `item` (an item entity, arrow
    /// or experience orb) up; false when a plugin cancelled it. Fired before anything changes.
    pub fn fire_pickup_event(&self, item: &Entity, amount: u32) -> bool {
        let mut pickup_event =
            crate::plugin::api::events::entity::entity_pickup_item::EntityPickupItemEvent::new(
                self.entity.entity_id,
                item.entity_type.id.to_string(),
                amount as u8,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut pickup_event);
            if pickup_event.cancelled {
                return false;
            }
        }
        true
    }

    /// Vanilla `LivingEntity.take`: to the players tracking the picked-up entity, while it's
    /// still there.
    pub fn take(&self, item: &Entity, amount: u32) {
        if item.removed.load(Relaxed) {
            return;
        }
        self.entity.world.load().send_to_tracking_players_editioned(
            item,
            &CTakeItemEntity::new(
                item.entity_id.into(),
                self.entity.entity_id.into(),
                VarInt(amount as i32),
            ),
            &CTakeItemActor {
                item_runtime_id: VarULong(item.entity_id as u64),
                actor_runtime_id: VarULong(self.entity.entity_id as u64),
            },
        );
    }

    /// Vanilla `LivingEntity.onItemPickup`, with `ServerPlayer`'s override: the
    /// thrown-item-picked-up advancement triggers.
    pub fn on_item_pickup(&self, item: &super::item::ItemEntity) {
        let Some(owner) = item.get_owner() else {
            return;
        };
        let stack = item
            .get_item_stack()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let world = self.entity.world.load();
        if let Some(thrower) = owner.get_player()
            && let Some(picker) = world.get_entity_by_id(self.entity.entity_id)
        {
            thrower.trigger_advancement(AdvancementTrigger::ThrownItemPickedUpByEntity {
                item: stack.clone(),
                entity: picker,
            });
        }
        if let Some(player) = world.get_player_by_id(self.entity.entity_id) {
            player.trigger_advancement(AdvancementTrigger::ThrownItemPickedUpByPlayer {
                item: stack,
                thrower: owner,
            });
        }
    }

    /// Vanilla `ItemStack.causeUseVibration`: the use game event, for items whose
    /// `use_effects` make vibrations.
    fn cause_use_vibration(&self, stack: &ItemStack, event: GameEvent) {
        if stack
            .get_data_component::<pumpkin_data::data_component_impl::UseEffectsImpl>()
            .is_some_and(|use_effects| use_effects.interact_vibrations)
        {
            let world = self.entity.world.load();
            let dyn_self = world.get_entity_by_id(self.entity.entity_id);
            world.game_event_from(dyn_self.as_deref(), event, self.entity.pos.load());
        }
    }

    /// Vanilla `startUsingItem`.
    pub fn set_active_hand(&self, hand: Hand, stack: ItemStack, duration: i32) {
        self.item_use_time.store(duration, Ordering::Relaxed);
        *self
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hand);
        // `updatePlayersWithNewEquipment`: the hand's item, to the tracking players.
        let slot = match hand {
            Hand::Right => EquipmentSlot::MAIN_HAND,
            Hand::Left => EquipmentSlot::OFF_HAND,
        };
        self.entity.world.load().send_to_tracking_players(
            &self.entity,
            &CSetEquipment::new(
                self.entity_id().into(),
                vec![(
                    slot.discriminant(),
                    ItemStackSerializer::from(stack.clone()),
                )],
            ),
        );
        self.set_living_flag(Self::USING_ITEM_FLAG, true);
        self.set_living_flag(Self::OFF_HAND_ACTIVE_FLAG, hand == Hand::Left);
        self.cause_use_vibration(&stack, GameEvent::ItemInteractStart);
        self.recent_kinetic_enemies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        *self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(stack);
    }

    pub(crate) fn set_living_flag(&self, flag: u8, value: bool) {
        let index = flag;
        let mut b = self.livings_flags.load(Ordering::Relaxed);
        if value {
            b |= index;
        } else {
            b &= !index;
        }
        self.livings_flags.store(b, Ordering::Relaxed);

        let bedrock_meta = (flag == Self::USING_ITEM_FLAG).then(|| {
            let index =
                pumpkin_protocol::bedrock::client::set_actor_data::entity_data_flag::USING_ITEM;
            let mask = 1i64 << index;
            if value {
                self.entity.bedrock_flags.fetch_or(mask, Ordering::Relaxed);
            } else {
                self.entity
                    .bedrock_flags
                    .fetch_and(!mask, Ordering::Relaxed);
            }

            let mut meta =
                pumpkin_protocol::bedrock::client::set_actor_data::SyncedActorDataList::new();
            meta.set(
                pumpkin_protocol::bedrock::client::set_actor_data::entity_data_key::FLAGS,
                pumpkin_protocol::bedrock::client::set_actor_data::MetadataValue::Int64(
                    self.entity.bedrock_flags.load(Ordering::Relaxed),
                ),
            );
            meta
        });

        self.entity
            .set_synced_data(tracked_data::living_entity::DATA_LIVING_ENTITY_FLAGS, b);
        if let Some(bedrock_meta) = &bedrock_meta {
            self.entity.send_bedrock_actor_data(bedrock_meta);
        }
    }

    /// Vanilla `stopUsingItem`.
    pub fn clear_active_hand(&self) {
        let was_using = self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(stack) = &was_using {
            self.cause_use_vibration(stack, GameEvent::ItemInteractFinish);
        }
        *self
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.recent_kinetic_enemies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.item_use_time.store(0, Ordering::Relaxed);

        self.set_living_flag(Self::USING_ITEM_FLAG, false);
    }

    pub fn was_recently_stabbed(&self, target_id: i32, now: i32, allowed_ticks: i32) -> bool {
        self.recent_kinetic_enemies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&target_id)
            .is_some_and(|stabbed_at| now - stabbed_at < allowed_ticks)
    }

    pub fn remember_stabbed_entity(&self, target_id: i32, now: i32) {
        self.recent_kinetic_enemies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(target_id, now);
    }

    /// Vanilla `LivingEntity.heal`.
    pub fn heal(&self, additional_health: f32) {
        let mut event =
            crate::plugin::api::events::entity::entity_regain_health::EntityRegainHealthEvent::new(
                self.entity.entity_id,
                additional_health,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
        }
        // Vanilla `heal` does nothing to a dead entity.
        let health = self.health.load();
        if health > 0.0 {
            self.set_health(health + additional_health);
        }
    }

    pub fn set_health(&self, health: f32) {
        // Clamp to [0, max_health]
        let max_health = self.get_max_health();
        let clamped = health.max(0.0).min(max_health);
        self.health.store(clamped);
        // tell everyone entities health changed
        self.entity
            .set_synced_data(tracked_data::living_entity::DATA_HEALTH_ID, clamped);
    }

    /// Vanilla `LivingEntity.getComfortableFallDistance`.
    #[must_use]
    pub fn get_comfortable_fall_distance(allowed_damage: f32) -> i32 {
        (allowed_damage + 3.0).floor() as i32
    }

    /// Returns the current maximum health for this entity
    pub fn get_max_health(&self) -> f32 {
        self.get_attribute_value(&Attributes::MAX_HEALTH) as f32
    }

    /// Sets the maximum health for this entity
    pub fn set_max_health(&self, max_health: f32) {
        // Update base attribute
        self.set_attribute_base(&Attributes::MAX_HEALTH, max_health as f64);

        // Broadcast the attribute change
        crate::entity::attributes::send_attribute_updates_for_living(
            self,
            vec![Attributes::MAX_HEALTH],
        );

        // Clamp current health to new max if needed and send metadata update
        let current_health = self.health.load();
        if current_health > max_health {
            self.set_health(max_health);
        }
    }

    /// Returns the current absorption amount for this entity (yellow hearts)
    pub fn get_absorption(&self) -> f32 {
        self.absorption.load()
    }

    /// Vanilla `getMaxAbsorption`.
    pub fn get_max_absorption(&self) -> f32 {
        self.get_attribute_value(&Attributes::MAX_ABSORPTION) as f32
    }

    /// Vanilla `setAbsorptionAmount`: clamped to `[0, max_absorption]`.
    pub fn set_absorption(&self, new_abs: f32) {
        self.internal_set_absorption(new_abs.clamp(0.0, self.get_max_absorption()));
    }

    /// Vanilla `internalSetAbsorptionAmount`, which `Player` also syncs (yellow hearts).
    pub fn internal_set_absorption(&self, new_abs: f32) {
        self.absorption.store(new_abs);
        if self.entity.entity_type == &EntityType::PLAYER {
            self.entity
                .set_synced_data(tracked_data::player::DATA_PLAYER_ABSORPTION_ID, new_abs);
        }
    }

    /// Convenience helper to mutate an attribute instance. Automatically inserts
    /// a new instance populated from the registry base if needed.
    pub fn update_attribute<F: FnOnce(&mut AttributeInstance)>(
        &self,
        attribute: &Attributes,
        f: F,
    ) {
        let mut map = self
            .attributes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let inst = map.entry(attribute.id).or_insert_with(|| {
            let base = self.supplier_base_value(attribute).unwrap_or_else(|| {
                tracing::debug!(
                    "Entity type {} has no base value for attribute {}; falling back to default {}",
                    self.entity.entity_type.resource_name,
                    attribute.name,
                    attribute.default_value,
                );
                attribute.default_value
            });
            AttributeInstance::new(attribute, base)
        });

        f(inst);
        inst.dirty.store(true, Ordering::Relaxed);
        drop(map);
        self.mark_attribute_updated(attribute);
    }

    /// Vanilla `AttributeMap.getValue`: the instance's value, or the type's base value for an
    /// attribute nothing has touched yet (falling back to `attribute.default_value`). Doesn't
    /// make an instance.
    pub fn get_attribute_value(&self, attribute: &Attributes) -> f64 {
        let map = self
            .attributes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(&attribute.id).map_or_else(
            || {
                attribute.sanitize_value(
                    self.supplier_base_value(attribute)
                        .unwrap_or(attribute.default_value),
                )
            },
            AttributeInstance::value,
        )
    }

    /// Vanilla `AttributeSupplier.getBaseValue`: the entity type's base for `attribute`, if the
    /// type has it.
    fn supplier_base_value(&self, attribute: &Attributes) -> Option<f64> {
        self.entity
            .entity_type
            .attributes
            .iter()
            .find(|a| a.0.id == attribute.id)
            .map(|a| a.1)
    }

    /// Vanilla `LivingEntity.removeFrost` then `tryAddFrost`, from `aiStep`: the powder snow
    /// slowdown follows how frozen the entity is.
    fn update_frost(&self) {
        // `removeFrost`: `getAttribute(MOVEMENT_SPEED)` makes the instance every tick.
        if !self.get_attribute_instance(&Attributes::MOVEMENT_SPEED) {
            return;
        }
        let mut changed = false;
        {
            let mut map = self
                .attributes
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(speed) = map.get_mut(&Attributes::MOVEMENT_SPEED.id)
                && speed.get_modifier(SPEED_MODIFIER_POWDER_SNOW_ID).is_some()
            {
                speed.remove_modifier(SPEED_MODIFIER_POWDER_SNOW_ID);
                speed.dirty.store(true, Ordering::Relaxed);
                changed = true;
            }
        }
        // `tryAddFrost`.
        let ticks_frozen = self.entity.frozen_ticks.load(Ordering::Relaxed);
        if !self.entity.get_block_state_on_legacy().is_air() && ticks_frozen > 0 {
            // `getPercentFrozen`.
            let percent_frozen =
                ticks_frozen.min(Entity::MAX_FROZEN_TICKS) as f32 / Entity::MAX_FROZEN_TICKS as f32;
            let slow_amount = -0.05f32 * percent_frozen;
            self.update_attribute(&Attributes::MOVEMENT_SPEED, |speed| {
                speed.add_or_replace_modifier(Modifier {
                    id: SPEED_MODIFIER_POWDER_SNOW_ID.to_string(),
                    amount: f64::from(slow_amount),
                    operation: ModifierOperation::Add,
                });
            });
            changed = true;
        }
        if changed {
            crate::entity::attributes::send_attribute_updates_for_living(
                self,
                vec![Attributes::MOVEMENT_SPEED],
            );
        }
    }

    /// Vanilla `AttributeMap.hasAttribute`: an instance, or a base value from the type.
    pub fn has_attribute(&self, attribute: &Attributes) -> bool {
        self.attributes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&attribute.id)
            || self.supplier_base_value(attribute).is_some()
    }

    /// Vanilla `AttributeMap.resetBaseValue`: back to the type's base value, without making an
    /// instance. False when the type has no such attribute.
    pub fn reset_attribute_base(&self, attribute: &Attributes) -> bool {
        let Some(base) = self.supplier_base_value(attribute) else {
            return false;
        };
        if let Some(instance) = self
            .attributes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(&attribute.id)
        {
            instance.base_value = base;
            instance.dirty.store(true, Ordering::Relaxed);
            self.mark_attribute_updated(attribute);
        }
        true
    }

    /// Vanilla `AttributeMap.getInstance`: makes the instance from the type's supplier on first
    /// use, which also puts it in the saved `attributes`. False when the type has no such
    /// attribute.
    pub fn get_attribute_instance(&self, attribute: &Attributes) -> bool {
        let mut map = self
            .attributes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if map.contains_key(&attribute.id) {
            return true;
        }
        let Some(base) = self.supplier_base_value(attribute) else {
            return false;
        };
        map.insert(attribute.id, AttributeInstance::new(attribute, base));
        true
    }

    /// Vanilla `LivingEntity.getSpeed`, which `Player.getSpeed` replaces with the
    /// movement speed attribute.
    #[must_use]
    pub fn get_speed(&self, caller: &dyn EntityBase) -> f64 {
        if caller.get_player().is_some() {
            self.get_attribute_value(&Attributes::MOVEMENT_SPEED)
        } else {
            f64::from(self.speed.load())
        }
    }

    /// The swing state of a freshly constructed vanilla `LivingEntity`: no swing in progress.
    pub fn clear_swing_state(&self) {
        *self
            .swing_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = SwingState::default();
    }

    /// The riptide spin of a freshly constructed vanilla `ServerPlayer`: `autoSpinAttackTicks`,
    /// `autoSpinAttackDmg` and `autoSpinAttackItemStack` start empty and flag 4 unset.
    pub fn clear_auto_spin_attack_for_respawn(&self) {
        self.auto_spin_attack_ticks.store(0, Relaxed);
        self.auto_spin_attack_dmg.store(0.0);
        *self
            .auto_spin_attack_item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.set_living_flag(Self::AUTO_SPIN_ATTACK_FLAG, false);
    }

    /// Vanilla `LivingEntity.getScale`: the `scale` attribute.
    #[must_use]
    pub fn get_scale(&self) -> f32 {
        self.get_attribute_value(&Attributes::SCALE) as f32
    }

    /// Returns the base attribute value for `attribute` for this entity's type.
    pub fn get_attribute_base(&self, attribute: &Attributes) -> f64 {
        // Check the local base value first (could be modified)
        let map = self
            .attributes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(instance) = map.get(&attribute.id) {
            return instance.base_value;
        }

        // Fall back to registry base value if no local instance exists
        self.supplier_base_value(attribute)
            .unwrap_or(attribute.default_value)
    }

    /// Update or insert the base value for an attribute on this entity.
    /// If the attribute doesn't exist locally yet, it will be inserted.
    pub fn set_attribute_base(&self, attribute: &Attributes, new_base: f64) {
        let mut map = self
            .attributes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(inst) = map.get_mut(&attribute.id) {
            inst.base_value = new_base;
            inst.dirty.store(true, Ordering::Relaxed);
        } else {
            let ai = AttributeInstance::new(attribute, new_base);
            map.insert(attribute.id, ai);
        }
        drop(map);
        self.mark_attribute_updated(attribute);
    }

    pub fn reset_effects_and_attributes(&self) {
        // Clear active effects and reset modified attributes
        let effects_to_remove: Vec<_> = {
            let lock = self
                .active_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            lock.keys().copied().collect()
        };

        for effect_type in effects_to_remove {
            self.remove_effect(effect_type);
        }
    }

    pub const fn entity_id(&self) -> i32 {
        self.entity.entity_id
    }

    /// Vanilla `LivingEntity.canBeAffected`, with the `Spider`, `WitherSkeleton`, `Parched`,
    /// `WitherBoss` and `AbstractNautilus` overrides.
    #[must_use]
    pub fn can_be_affected(&self, effect: &StatusEffect) -> bool {
        let entity_type = self.entity.entity_type;
        let immune_to = if entity_type == &EntityType::SPIDER
            || entity_type == &EntityType::CAVE_SPIDER
            || entity_type == &EntityType::NAUTILUS
            || entity_type == &EntityType::ZOMBIE_NAUTILUS
        {
            Some(&StatusEffect::POISON)
        } else if entity_type == &EntityType::WITHER_SKELETON || entity_type == &EntityType::WITHER
        {
            Some(&StatusEffect::WITHER)
        } else if entity_type == &EntityType::PARCHED {
            Some(&StatusEffect::WEAKNESS)
        } else {
            None
        };
        if immune_to == Some(effect) {
            return false;
        }
        if entity_type.has_tag(&tag::EntityType::MINECRAFT_IMMUNE_TO_INFESTED) {
            effect != &StatusEffect::INFESTED
        } else if entity_type.has_tag(&tag::EntityType::MINECRAFT_IMMUNE_TO_OOZING) {
            effect != &StatusEffect::OOZING
        } else {
            !entity_type.has_tag(&tag::EntityType::MINECRAFT_IGNORES_POISON_AND_REGEN)
                || effect != &StatusEffect::REGENERATION && effect != &StatusEffect::POISON
        }
    }

    /// Vanilla `LivingEntity.addEffect`: merges into the active effect of the same type
    /// (`MobEffectInstance.update`), then always runs `onEffectStarted`. Returns whether the
    /// active effects changed.
    pub fn add_effect(&self, effect: Effect) -> bool {
        if !self.can_be_affected(effect.effect_type) {
            return false;
        }
        let mut effect_event =
            crate::plugin::api::events::entity::entity_potion_effect::EntityPotionEffectEvent::new(
                self.entity.entity_id,
                effect.effect_type.translation_key.to_string(),
                effect.duration,
                effect.amplifier,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut effect_event);
        }
        if effect_event.cancelled {
            return false;
        }

        let new_effect = MobEffectInstance::from(effect);
        let effect_type = new_effect.effect_type;
        let amplifier = new_effect.amplifier;
        let mut effects = self
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = if let Some(existing) = effects.get_mut(&effect_type) {
            if existing.update(&new_effect) {
                let updated = existing.effect.clone();
                drop(effects);
                self.on_effect_updated(&updated, true);
                true
            } else {
                drop(effects);
                false
            }
        } else {
            effects.insert(effect_type, new_effect.clone());
            drop(effects);
            self.on_effect_added(&new_effect.effect);
            // `MobEffectInstance.onEffectAdded`: the effect's sound, if it has one.
            if let Some(sound) = crate::entity::effect::sound_on_added(effect_type) {
                self.entity.world.load().play_sound_raw(
                    sound as u16,
                    self.sound_source(),
                    &self.entity.pos.load(),
                    1.0,
                    1.0,
                );
            }
            true
        };
        if let Some(mob_effect) = crate::entity::effect::get_mob_effect(effect_type) {
            mob_effect.on_effect_started(self, amplifier);
        }
        changed
    }

    /// Vanilla `getSoundSource`, with `Player`'s override.
    fn sound_source(&self) -> SoundCategory {
        if self.is_player() {
            SoundCategory::Players
        } else {
            crate::entity::mob::sounds::sound_source(self.entity.entity_type)
        }
    }

    /// Vanilla `LivingEntity.isInvertedHealAndHarm`.
    #[must_use]
    pub fn is_inverted_heal_and_harm(&self) -> bool {
        self.entity
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_INVERTED_HEALING_AND_HARM)
    }

    /// Vanilla `MobEffect.addAttributeModifiers`: permanent modifiers, so they are saved.
    fn add_effect_attribute_modifiers(&self, effect_type: &'static StatusEffect, amplifier: u8) {
        for m in effect_type.attribute_modifiers {
            if !self.get_attribute_instance(m.attribute) {
                continue;
            }
            let operation = ModifierOperation::from_operation(m.operation);
            // `AttributeTemplate.create`: `amount * (amplifier + 1)`.
            let modifier = Modifier {
                id: m.id.to_string(),
                amount: m.base_value * f64::from(i32::from(amplifier) + 1),
                operation,
            };
            self.update_attribute(m.attribute, |inst| {
                inst.add_or_replace_permanent_modifier(modifier);
            });
        }
        self.send_effect_attribute_updates(effect_type);
    }

    /// Vanilla `MobEffect.removeAttributeModifiers`.
    fn remove_effect_attribute_modifiers(&self, effect_type: &'static StatusEffect) {
        for m in effect_type.attribute_modifiers {
            let mut map = self
                .attributes
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(inst) = map.get_mut(&m.attribute.id) {
                inst.remove_modifier(m.id);
                drop(map);
                self.mark_attribute_updated(m.attribute);
            }
        }
        self.send_effect_attribute_updates(effect_type);
    }

    fn send_effect_attribute_updates(&self, effect_type: &'static StatusEffect) {
        let mut touched: Vec<Attributes> = Vec::new();
        for m in effect_type.attribute_modifiers {
            push_unique_attribute(&mut touched, m.attribute);
        }
        if !touched.is_empty() {
            crate::entity::attributes::send_attribute_updates_for_living(self, touched);
        }
    }

    /// Vanilla `onEffectAdded`, with `ServerPlayer`'s override sending the effect to the player.
    fn on_effect_added(&self, effect: &Effect) {
        self.effects_dirty.store(true, Relaxed);
        self.add_effect_attribute_modifiers(effect.effect_type, effect.amplifier);
        self.send_effect_to_passengers(effect);
        if let Some(player) = self.self_player() {
            player.send_effect_with_blend(effect, true);
        }
        self.send_bedrock_effect(effect);
    }

    /// Vanilla `onEffectUpdated`.
    fn on_effect_updated(&self, effect: &Effect, do_refresh_attributes: bool) {
        self.effects_dirty.store(true, Relaxed);
        if do_refresh_attributes {
            self.remove_effect_attribute_modifiers(effect.effect_type);
            self.add_effect_attribute_modifiers(effect.effect_type, effect.amplifier);
            self.refresh_dirty_attributes();
        }
        self.send_effect_to_passengers(effect);
        if let Some(player) = self.self_player() {
            player.send_effect_with_blend(effect, false);
        }
        self.send_bedrock_effect(effect);
    }

    /// Vanilla `onEffectsRemoved`.
    fn on_effects_removed(&self, effects: &[&'static StatusEffect]) {
        self.effects_dirty.store(true, Relaxed);
        for &effect_type in effects {
            self.remove_effect_attribute_modifiers(effect_type);
            let packet = CRemoveMobEffect::new(
                self.entity.entity_id.into(),
                VarInt(i32::from(effect_type.id)),
            );
            for passenger in self.passenger_players() {
                if let Some(player) = passenger.get_player() {
                    player.try_send_client_packet(&packet);
                }
            }
        }
        self.refresh_dirty_attributes();
        if let Some(player) = self.self_player() {
            for &effect_type in effects {
                player.try_send_client_packet(&CRemoveMobEffect::new(
                    self.entity.entity_id.into(),
                    VarInt(i32::from(effect_type.id)),
                ));
            }
        }
        let world = self.entity.world.load();
        for &effect_type in effects {
            world.send_to_tracking_players_bedrock(
                &self.entity,
                &pumpkin_protocol::bedrock::client::CMobEffect {
                    target_runtime_id: VarULong(self.entity.entity_id as u64),
                    event_id: pumpkin_protocol::bedrock::client::CMobEffect::EVENT_REMOVE,
                    effect_id: VarInt(effect_type.to_bedrock_id()),
                    effect_amplifier: VarInt(0),
                    show_particles: false,
                    effect_duration_ticks: VarInt(0),
                    tick: VarULong(0),
                    ambient: false,
                },
            );
        }
    }

    /// Bedrock clients draw other entities' effects from `MobEffect` packets.
    fn send_bedrock_effect(&self, effect: &Effect) {
        self.entity.world.load().send_to_tracking_players_bedrock(
            &self.entity,
            &pumpkin_protocol::bedrock::client::CMobEffect {
                target_runtime_id: VarULong(self.entity.entity_id as u64),
                event_id: pumpkin_protocol::bedrock::client::CMobEffect::EVENT_ADD,
                effect_id: VarInt(effect.effect_type.to_bedrock_id()),
                effect_amplifier: VarInt(i32::from(effect.amplifier)),
                show_particles: effect.show_particles,
                effect_duration_ticks: VarInt(effect.duration),
                tick: VarULong(0),
                ambient: effect.ambient,
            },
        );
    }

    /// This entity as a player, for the `ServerPlayer` overrides.
    pub(crate) fn self_player(&self) -> Option<Arc<crate::entity::player::Player>> {
        if !self.is_player() {
            return None;
        }
        self.entity
            .world
            .load()
            .get_player_by_uuid(self.entity.entity_uuid)
    }

    /// The players riding this entity.
    fn passenger_players(&self) -> Vec<Arc<dyn EntityBase>> {
        self.entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|passenger| passenger.get_player().is_some())
            .cloned()
            .collect()
    }

    /// Vanilla `sendEffectToPassengers`.
    fn send_effect_to_passengers(&self, effect: &Effect) {
        for passenger in self.passenger_players() {
            if let Some(player) = passenger.get_player() {
                player.send_effect_for(self.entity.entity_id, effect, false);
            }
        }
    }

    /// Vanilla `updateDirtyEffects`, run before the entity's data is synced.
    pub fn update_dirty_effects(&self) {
        if self.effects_dirty.swap(false, Relaxed) {
            self.update_invisibility_status();
            // `updateGlowingStatus`: the glowing tag or, on the server, the effect.
            self.entity.set_glowing(
                self.entity.has_glowing_tag.load(Relaxed)
                    || self.has_effect(&StatusEffect::GLOWING),
            );
        }
    }

    /// Vanilla `updateInvisibilityStatus`, with `updateSynchronizedMobEffectParticles`.
    fn update_invisibility_status(&self) {
        let effects = self
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if effects.is_empty() {
            drop(effects);
            // `removeEffectParticles`.
            self.entity.set_synced_data(
                tracked_data::living_entity::EFFECT_PARTICLES,
                EffectParticles(Vec::new()),
            );
            self.entity.set_invisible(false);
            return;
        }
        let invisible = effects.contains_key(&StatusEffect::INVISIBILITY);
        let particles = EffectParticles(
            effects
                .values()
                .filter(|effect| effect.show_particles)
                .map(|effect| EffectParticle::from_effect(effect))
                .collect(),
        );
        // `areAllEffectsAmbient`.
        let ambient = effects
            .values()
            .all(|effect| !effect.show_particles || effect.ambient);
        drop(effects);
        self.entity.set_invisible(invisible);
        self.entity
            .set_synced_data(tracked_data::living_entity::EFFECT_PARTICLES, particles);
        self.entity
            .set_synced_data(tracked_data::living_entity::EFFECT_AMBIENCE_ID, ambient);
    }

    /// Vanilla `removeEffect`.
    pub fn remove_effect(&self, effect_type: &'static StatusEffect) -> bool {
        let removed = self
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&effect_type)
            .is_some();
        if removed {
            self.on_effects_removed(&[effect_type]);
        }
        removed
    }

    /// Vanilla `removeAllEffects`.
    pub fn remove_all_effects(&self) -> bool {
        let removed: Vec<&'static StatusEffect> = self
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain()
            .map(|(effect_type, _)| effect_type)
            .collect();
        if removed.is_empty() {
            return false;
        }
        self.on_effects_removed(&removed);
        true
    }

    pub fn has_effect(&self, effect: &'static StatusEffect) -> bool {
        self.active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&effect)
    }

    pub fn get_effect(&self, effect: &'static StatusEffect) -> Option<Effect> {
        self.active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&effect)
            .map(|instance| instance.effect.clone())
    }

    /// Vanilla `refreshDirtyAttributes`: `onAttributeUpdated` for each attribute changed since.
    pub fn refresh_dirty_attributes(&self) {
        let updated: Vec<u8> = std::mem::take(
            &mut *self
                .attributes_to_update
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for id in updated {
            self.on_attribute_updated(id);
        }
    }

    /// Records an attribute change for the next `refresh_dirty_attributes`, as vanilla's
    /// `AttributeMap.onAttributeModified` does.
    fn mark_attribute_updated(&self, attribute: &Attributes) {
        let mut updated = self
            .attributes_to_update
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !updated.contains(&attribute.id) {
            updated.push(attribute.id);
        }
    }

    /// Vanilla `onAttributeUpdated`, with `ServerPlayer`'s override first.
    fn on_attribute_updated(&self, attribute_id: u8) {
        let world = self.entity.world.load();
        // `ServerPlayer.onAttributeUpdated`: the receive range adds or removes the player from
        // the waypoint manager, before the living handling.
        if attribute_id == Attributes::WAYPOINT_RECEIVE_RANGE.id
            && self.entity.entity_type == &EntityType::PLAYER
            && let Some(player) = world.get_player_by_id(self.entity.entity_id)
        {
            if self.get_attribute_value(&Attributes::WAYPOINT_RECEIVE_RANGE) > 0.0 {
                world.waypoint_manager.add_player(&player);
            } else {
                world.waypoint_manager.remove_player(&player);
            }
        }
        if attribute_id == Attributes::MAX_HEALTH.id {
            let max_health = self.get_max_health();
            if self.health.load() > max_health {
                self.set_health(max_health);
            }
        } else if attribute_id == Attributes::MAX_ABSORPTION.id {
            let max_absorption = self.get_max_absorption();
            if self.get_absorption() > max_absorption {
                self.set_absorption(max_absorption);
            }
        }
        // `SCALE` refreshes the dimensions, which needs the full entity: the tick picks the
        // change up from `applied_scale`.
        else if attribute_id == Attributes::WAYPOINT_TRANSMIT_RANGE.id {
            if self.get_attribute_value(&Attributes::WAYPOINT_TRANSMIT_RANGE) > 0.0 {
                if let Some(entity) = world.get_entity_by_id(self.entity.entity_id) {
                    world.waypoint_manager.track_waypoint(&entity);
                }
            } else {
                world
                    .waypoint_manager
                    .untrack_waypoint(self.entity.entity_id);
            }
        }
    }

    // Check if the entity is in water
    pub fn is_in_water(&self) -> bool {
        self.entity.touching_water.load(Ordering::Relaxed)
    }

    // Check if the entity is in powder snow
    pub fn is_in_powder_snow(&self) -> bool {
        self.entity.is_in_powder_snow.load(Ordering::Relaxed)
    }

    /// Vanilla `LivingEntity.knockback`, or the mob's override of it. `attacker` is the
    /// damage source's responsible entity.
    #[expect(clippy::too_many_arguments)]
    pub fn knockback(
        &self,
        caller: &dyn EntityBase,
        power: f64,
        xd: f64,
        zd: f64,
        attacker: Option<&dyn EntityBase>,
        damage: f32,
        comes_from_effect: bool,
    ) {
        if caller.get_mob().is_some_and(|mob| {
            mob.custom_knockback(
                power,
                xd,
                zd,
                attacker.map(EntityBase::get_entity),
                damage,
                comes_from_effect,
            )
        }) {
            return;
        }
        let resistance = self.get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE);
        self.entity
            .apply_knockback(knockback_after_resistance(power, resistance), xd, zd);
    }

    /// Vanilla `LivingEntity.getKnockback`: the `attack_knockback` attribute through the
    /// weapon's enchantments, halved.
    pub fn get_knockback(
        &self,
        caller: &dyn EntityBase,
        world: &crate::world::World,
        target: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
    ) -> f32 {
        let knockback = self.get_attribute_value(&Attributes::ATTACK_KNOCKBACK) as f32;
        let weapon = caller
            .get_weapon_item()
            .unwrap_or_else(|| ItemStack::EMPTY.clone());
        EnchantmentHelper::modify_knockback(world, &weapon, target, damage_source, knockback) / 2.0
    }

    /// Vanilla `LivingEntity.postPiercingAttack`.
    pub fn post_piercing_attack(&self, caller: &dyn EntityBase) {
        let world = self.entity.world.load_full();
        EnchantmentHelper::do_post_piercing_attack_effects(&world, caller, self);
    }

    /// The `xd`/`zd` of vanilla `dealDefaultKnockback`: a projectile pushes along its own
    /// motion (fireworks and thrown potions from where they are), anything else from the
    /// damage source's position.
    pub(crate) fn default_knockback_direction(
        &self,
        direct: Option<&dyn EntityBase>,
        position: Option<Vector3<f64>>,
    ) -> (f64, f64) {
        let here = self.entity.pos.load();
        if let Some(direct) = direct {
            let entity = direct.get_entity();
            if PROJECTILE_TYPES.contains(&entity.entity_type) {
                if [
                    &EntityType::FIREWORK_ROCKET,
                    &EntityType::SPLASH_POTION,
                    &EntityType::LINGERING_POTION,
                ]
                .contains(&entity.entity_type)
                {
                    let from = entity.pos.load();
                    return (-(here.x - from.x), -(here.z - from.z));
                }
                let motion = entity.velocity.load();
                return (-motion.x, -motion.z);
            }
        }
        // `DamageSource.getSourcePosition`: an explicit position, else the direct entity's.
        position
            .or_else(|| direct.map(|direct| direct.get_entity().pos.load()))
            .map_or((0.0, 0.0), |from| (from.x - here.x, from.z - here.z))
    }

    /// Vanilla `LivingEntity.wasHurtRecently`.
    pub fn was_hurt_recently(&self) -> bool {
        self.hurt_time.load(Relaxed) > 0
    }

    pub(crate) fn get_effective_gravity(&self, caller: &dyn EntityBase) -> f64 {
        let final_gravity = caller.current_gravity();

        if self.entity.velocity.load().y <= 0.0 && self.has_effect(&StatusEffect::SLOW_FALLING) {
            final_gravity.min(0.01)
        } else {
            final_gravity
        }
    }

    /// Swings the main hand with the default animation, like vanilla `Mob.swing(MAIN_HAND)`.
    pub fn swing_hand(&self) {
        self.swing(Hand::Right, SwingAnimationImpl::DEFAULT, false);
    }

    /// Vanilla `Mob.swingForAttack`: the swing animated as the attack of the item in `hand`.
    pub fn swing_for_attack(&self, caller: &dyn EntityBase, hand: Hand) {
        let animation = self.get_stack_in_hand(caller, hand).get_attack_animation();
        self.swing(hand, animation, false);
    }

    /// Vanilla `MobEffectUtil.hasDigSpeed`.
    #[must_use]
    pub fn has_dig_speed(&self) -> bool {
        self.has_effect(&StatusEffect::HASTE) || self.has_effect(&StatusEffect::CONDUIT_POWER)
    }

    /// Vanilla `MobEffectUtil.getDigSpeedAmplification`.
    #[must_use]
    pub fn get_dig_speed_amplification(&self) -> i32 {
        let haste = self
            .get_effect(&StatusEffect::HASTE)
            .map_or(0, |effect| effect.amplifier);
        let conduit_power = self
            .get_effect(&StatusEffect::CONDUIT_POWER)
            .map_or(0, |effect| effect.amplifier);
        i32::from(haste.max(conduit_power))
    }

    /// Vanilla `LivingEntity.getModifiedSwingDuration`.
    fn get_modified_swing_duration(&self, animation: SwingAnimationImpl) -> i32 {
        let swing_duration = animation.duration;
        if self.has_dig_speed() {
            swing_duration - (1 + self.get_dig_speed_amplification())
        } else if let Some(fatigue) = self.get_effect(&StatusEffect::MINING_FATIGUE) {
            swing_duration + (1 + i32::from(fatigue.amplifier)) * 2
        } else {
            swing_duration
        }
    }

    /// Vanilla `LivingEntity.swing(hand, animation, sendToSwingingEntity)`: starts the swing
    /// unless one is still in its first half, and returns whether it started.
    pub fn swing(
        &self,
        hand: Hand,
        animation: SwingAnimationImpl,
        send_to_swinging_entity: bool,
    ) -> bool {
        let duration = self.get_modified_swing_duration(animation);
        if !self
            .swing_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .start_if_able(duration)
        {
            return false;
        }
        let world = self.entity.world.load();
        let entity_id = self.entity_id();
        let je_packet = pumpkin_protocol::java::client::play::CSwingArm::with_animation(
            entity_id.into(),
            hand == Hand::Left,
            animation,
        );
        let be_packet = pumpkin_protocol::bedrock::server::animate::SAnimate {
            action: pumpkin_protocol::bedrock::server::animate::AnimateAction::SwingArm,
            target_actor_runtime_id: pumpkin_protocol::codec::var_ulong::VarULong(entity_id as u64),
            data: 0.0,
            swing_source: None,
        };
        // The players tracking the entity, and its own player only when asked.
        if send_to_swinging_entity {
            world.send_to_tracking_players_and_self_editioned(&self.entity, &je_packet, &be_packet);
        } else {
            world.send_to_tracking_players_editioned(&self.entity, &je_packet, &be_packet);
        }
        true
    }

    #[expect(clippy::too_many_lines, reason = "mirrors vanilla LivingEntity.aiStep")]
    fn tick_movement(&self, caller: &dyn EntityBase) {
        if let Some(mob) = caller.get_mob() {
            if mob.custom_ai_step(caller) {
                return;
            }
            mob.ai_step(caller);
        }
        if self.jumping_cooldown.load(Relaxed) != 0 {
            self.jumping_cooldown.fetch_sub(1, Relaxed);
        }

        // Vanilla `aiStep`: a vehicle its rider's client moves only loses velocity here.
        if !caller.can_simulate_movement() {
            self.entity
                .velocity
                .store(self.entity.velocity.load() * 0.98);
        }

        let should_swim_in_fluids = caller.get_player().is_none_or(|player| !player.is_flying());

        self.entity.check_zero_velo();

        self.apply_input();
        // Vanilla `isImmobile`, then `serverAiStep` for mobs with AI. It runs here, after
        // `baseTick` updated the fluid state, so goals see this tick's water height.
        if self.is_dead_or_dying()
            || caller
                .get_mob()
                .is_some_and(super::mob::Mob::mob_is_immobile)
        {
            self.jumping.store(false, SeqCst);
            let input = self.movement_input.load();
            self.movement_input.store(Vector3::new(0.0, input.y, 0.0));
        } else if let Some(mob) = caller.get_mob()
            && mob.is_effective_ai()
        {
            mob.get_mob_entity().server_ai_step(mob, caller);
        }

        if self.jumping.load(SeqCst) && should_swim_in_fluids {
            let in_lava = self.entity.touching_lava.load(SeqCst);
            let fluid_height = if in_lava {
                self.entity.lava_height.load()
            } else {
                self.entity.water_height.load()
            };
            let in_water_and_has_fluid_height =
                self.entity.touching_water.load(SeqCst) && fluid_height > 0.0;
            let fluid_jump_threshold = self.fluid_jump_threshold(caller);
            let on_ground = self.entity.on_ground.load(SeqCst);
            if in_water_and_has_fluid_height && !(on_ground && fluid_height <= fluid_jump_threshold)
            {
                self.jump_in_liquid(caller, false);
            } else if in_lava
                // `isInShallowFluid(LAVA)`.
                && !(on_ground && self.entity.lava_height.load() <= fluid_jump_threshold)
            {
                self.jump_in_liquid(caller, true);
            } else if (on_ground
                || in_water_and_has_fluid_height && fluid_height <= fluid_jump_threshold)
                && self.jumping_cooldown.load(SeqCst) == 0
            {
                if let Some(mob) = caller.get_mob() {
                    mob.jump_from_ground();
                } else {
                    self.jump();
                }
                self.jumping_cooldown.store(10, SeqCst);
            }
        } else {
            self.jumping_cooldown.store(0, SeqCst);
        }

        if self.entity.is_fall_flying() {
            self.update_fall_flying(caller);
        }

        let before_travel_box = self.entity.bounding_box.load();
        if self.has_effect(&StatusEffect::SLOW_FALLING)
            || self.has_effect(&StatusEffect::LEVITATION)
        {
            self.entity.reset_fall_distance();
        }

        let touching_water = self.entity.touching_water.load(SeqCst);

        // Vanilla `Mob.isEffectiveAi`: NoAI mobs don't travel, so they neither move nor fall.
        let effective_ai = caller
            .get_mob()
            .is_none_or(super::mob::Mob::is_effective_ai);

        // Strider is the only entity that has canWalkOnFluid = false

        if let Some(mob) = caller.get_mob()
            && let Some(controller) = caller.controlling_passenger()
            && let Some(controller) = controller.get_player()
            && self.is_alive()
        {
            // Vanilla `travelRidden`. The server never simulates a vehicle a player steers
            // (`canSimulateMovement`), so of it only `tickRidden` runs; the steering client
            // moves the vehicle with `getRiddenInput` and `getRiddenSpeed`.
            mob.tick_ridden(controller);
            self.entity.velocity.store(Vector3::default());
        } else if !effective_ai || !caller.can_simulate_movement() {
            // No travel.
        } else if caller
            .get_mob()
            .is_some_and(|mob| mob.custom_travel(caller))
        {
            // The mob moved itself.
        } else if self.should_travel_in_fluid(caller) {
            self.travel_in_fluid(caller, touching_water);
            if let Some(mob) = caller.get_mob() {
                mob.travel_in_fluid_end(touching_water);
            }
        } else if self.entity.is_fall_flying() {
            self.travel_fall_flying(caller);
        } else {
            self.travel_in_air(caller);
        }

        // `applyEffectsFromBlocks`. The wall damage is `baseTick`'s.
        let _ = self.entity.tick_block_collisions(caller);

        // The freezing part of `aiStep`, before the push.
        self.entity.tick_frozen(caller);
        self.update_frost();

        self.tick_auto_spin_attack(caller, before_travel_box);
        if caller
            .get_mob()
            .is_none_or(super::mob::Mob::pushes_entities)
        {
            self.push_entities(caller);
        }

        if caller
            .get_mob()
            .is_some_and(super::mob::Mob::is_sensitive_to_water)
            && self.entity.is_in_water_or_rain()
        {
            caller.damage(caller, 1.0, DamageType::DROWN);
        }

        // Vanilla `Mob.aiStep` burns undead right after `super.aiStep()`, then loots.
        if let Some(mob) = caller.get_mob() {
            super::mob::sun_burn::tick(mob);
            super::mob::ai_step_looting(mob);
            mob.ai_step_end(caller);
        }
    }

    /// Vanilla `LivingEntity.applyInput`: the last tick's movement input decays. Vanilla
    /// runs it just before `serverAiStep`, so input the AI sets this tick is used at
    /// full strength.
    pub fn apply_input(&self) {
        let mut movement_input = self.movement_input.load();
        movement_input.x *= f64::from(INPUT_FRICTION);
        movement_input.z *= f64::from(INPUT_FRICTION);
        self.movement_input.store(movement_input);
    }

    /// Vanilla `LivingEntity.isAutoSpinAttack`.
    pub fn is_auto_spin_attack(&self) -> bool {
        self.livings_flags.load(Relaxed) & Self::AUTO_SPIN_ATTACK_FLAG != 0
    }

    /// Vanilla `aiStep`'s spin step, right before `pushEntities`.
    fn tick_auto_spin_attack(&self, caller: &dyn EntityBase, before_travel_box: BoundingBox) {
        if self.auto_spin_attack_ticks.load(Relaxed) > 0 {
            self.auto_spin_attack_ticks.fetch_sub(1, Relaxed);
            self.check_auto_spin_attack(caller, before_travel_box, self.entity.bounding_box.load());
        }
    }

    /// Vanilla `LivingEntity.checkAutoSpinAttack`: the spin hits the first living entity in
    /// the box it swept, and ends on that hit or against a wall.
    fn check_auto_spin_attack(
        &self,
        caller: &dyn EntityBase,
        old: BoundingBox,
        current: BoundingBox,
    ) {
        // `getEntities(this, minmax)`: `NO_SPECTATORS`, then the dragon parts, which aren't
        // living but still keep the spin off the wall check.
        let world = self.entity.world.load();
        let area = old.minmax(&current);
        let entities: Vec<Arc<dyn EntityBase>> = world
            .get_all_at_box(&area)
            .into_iter()
            .chain(super::boss::ender_dragon::parts_at_box(&world, &area))
            .filter(|entity| {
                entity.get_entity().entity_id != self.entity.entity_id && !entity.is_spectator()
            })
            .collect();
        if !entities.is_empty() {
            if let Some(target) = entities
                .iter()
                .find(|entity| entity.get_living_entity().is_some())
            {
                Self::do_auto_attack_on_touch(caller, target);
                self.auto_spin_attack_ticks.store(0, Relaxed);
                self.entity
                    .velocity
                    .store(self.entity.velocity.load() * -0.2);
            }
        } else if self.entity.horizontal_collision.load(Relaxed) {
            self.auto_spin_attack_ticks.store(0, Relaxed);
        }

        if self.auto_spin_attack_ticks.load(Relaxed) <= 0 {
            self.set_living_flag(Self::AUTO_SPIN_ATTACK_FLAG, false);
            self.auto_spin_attack_dmg.store(0.0);
            *self
                .auto_spin_attack_item
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        }
    }

    /// Vanilla `doAutoAttackOnTouch`: only `Player` overrides it, with `attack`.
    fn do_auto_attack_on_touch(caller: &dyn EntityBase, target: &Arc<dyn EntityBase>) {
        if let Some(player) = caller.get_player() {
            player.attack(target);
        }
    }

    /// The riptide stack while auto-spinning, for vanilla `Player.getWeaponItem`.
    pub fn auto_spin_attack_item(&self) -> Option<(ItemStack, usize)> {
        if !self.is_auto_spin_attack() {
            return None;
        }
        self.auto_spin_attack_item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn push_entities(&self, dyn_self: &dyn EntityBase) {
        if dyn_self.push_entities_override(dyn_self) {
            return;
        }
        let world = self.entity.world.load();
        let entity_bb = self.entity.bounding_box.load();
        let own_team = dyn_self.get_team();

        let pushable: Vec<Arc<dyn EntityBase>> = world
            .get_all_at_box(&entity_bb)
            .into_iter()
            .filter(|entity| {
                let entity_ref = entity.get_entity();
                entity_ref.entity_id != self.entity.entity_id
                    && !entity.is_spectator()
                    && entity.is_pushable()
                    && is_allowed_by_team_rules(own_team.as_ref(), entity.get_team().as_ref())
            })
            .collect();

        if pushable.is_empty() {
            return;
        }

        // Entity cramming check
        let max_cramming = match world.get_game_rule(&GameRule::MaxEntityCramming) {
            GameRuleValue::Int(value) => value,
            GameRuleValue::Bool(_) => 0,
        };
        if max_cramming > 0
            && pushable.len() as i64 > max_cramming - 1
            && self.entity.random().next_bounded_i32(4) == 0
        {
            let count = pushable
                .iter()
                .filter(|entity| !entity.is_passenger())
                .count();
            if count as i64 > max_cramming - 1 {
                dyn_self.damage(dyn_self, 6.0, DamageType::CRAMMING);
            }
        }

        for entity in pushable {
            entity.push(dyn_self);
            if let Some(mob) = dyn_self.get_mob() {
                mob.do_push(entity.as_ref());
            }
        }
    }

    /// Vanilla `LivingEntity.travelFlying(input, speed)`: water and lava both move at 0.02.
    pub fn travel_flying(&self, caller: &dyn EntityBase, speed: f32) {
        self.travel_flying_with_speeds(caller, 0.02, 0.02, speed);
    }

    /// Vanilla `LivingEntity.travelFlying`: moves without gravity, with a speed and drag
    /// for water, lava and air.
    pub fn travel_flying_with_speeds(
        &self,
        caller: &dyn EntityBase,
        water_speed: f32,
        lava_speed: f32,
        air_speed: f32,
    ) {
        let (speed, drag) = if self.entity.is_in_water() {
            (water_speed, f64::from(0.8f32))
        } else if self.entity.touching_lava.load(SeqCst) {
            (lava_speed, 0.5)
        } else {
            (air_speed, f64::from(0.91f32))
        };
        self.entity
            .update_velocity_from_input(self.movement_input.load(), f64::from(speed));
        self.make_move(caller);
        self.entity
            .velocity
            .store(self.entity.velocity.load() * drag);
    }

    /// Vanilla `LivingEntity.computeModifiedFriction`.
    fn compute_modified_friction(friction: f32, modifier: f32) -> f32 {
        (1.0 - (1.0 - friction) * modifier).clamp(0.0, 1.0)
    }

    /// Vanilla `LivingEntity.getAirDrag`.
    pub(crate) fn get_air_drag(&self, caller: &dyn EntityBase) -> f32 {
        Self::compute_modified_friction(
            if caller.is_flutterer() { 0.91 } else { 0.98 },
            self.get_attribute_value(&Attributes::AIR_DRAG_MODIFIER) as f32,
        )
    }

    /// Vanilla `LivingEntity.travelInAir`, in vanilla's float maths.
    pub(crate) fn travel_in_air(&self, caller: &dyn EntityBase) {
        let on_ground = self.entity.on_ground.load(Relaxed);
        let block_friction = if on_ground {
            Self::compute_modified_friction(
                self.entity
                    .get_block_with_y_offset(super::DELTA_AFFECTED_BY_BLOCKS_BELOW_0_5)
                    .1
                    .slipperiness,
                self.get_attribute_value(&Attributes::FRICTION_MODIFIER) as f32,
            )
        } else {
            1.0
        };

        // `getFrictionInfluencedSpeed`; Java compares the float friction with the double 0.6.
        let speed = if on_ground {
            let speed = self.get_speed(caller) as f32;
            if f64::from(block_friction) > 0.6 {
                speed * (0.216_000_02 / (block_friction * block_friction * block_friction))
            } else {
                speed
            }
        } else {
            caller
                .get_player()
                .map_or(0.02, |player| player.get_off_ground_speed() as f32)
        };

        self.entity
            .update_velocity_from_input(self.movement_input.load(), f64::from(speed));

        self.handle_on_climbable(caller);

        self.make_move(caller);

        let mut velo = self.entity.velocity.load();

        let can_powder_snow_climb = if self.entity.was_in_powder_snow.load(Relaxed) {
            crate::block::blocks::powder_snow::can_entity_walk_on_powder_snow(caller)
        } else {
            false
        };

        if (self.entity.horizontal_collision.load(SeqCst) || self.jumping.load(SeqCst))
            && (self.on_climbable(caller) || can_powder_snow_climb)
        {
            velo.y = 0.2;
        }

        let levitation = self.get_effect(&StatusEffect::LEVITATION);

        if let Some(lev) = levitation {
            velo.y += (0.05 * f64::from(lev.amplifier + 1) - velo.y) * 0.2;
        } else {
            velo.y -= self.get_effective_gravity(caller);

            // TODO: If world is not loaded: replace effective gravity with:

            // if below world's bottom y then -0.1, else 0.0
        }

        if self.discard_friction.load(Relaxed) {
            self.entity.velocity.store(velo);
            return;
        }

        let air_drag_modifier = self.get_attribute_value(&Attributes::AIR_DRAG_MODIFIER) as f32;
        let air_drag = Self::compute_modified_friction(0.91, air_drag_modifier);
        let friction = block_friction * air_drag;
        // `omnidirectionalAirMover` flyers drag vertically like horizontally.
        let vertical_friction = if caller.is_flutterer() {
            air_drag
        } else {
            Self::compute_modified_friction(0.98, air_drag_modifier)
        };
        velo.x *= f64::from(friction);
        velo.z *= f64::from(friction);
        velo.y *= caller
            .get_y_velocity_drag()
            .unwrap_or_else(|| f64::from(vertical_friction));

        self.entity.velocity.store(velo);
    }

    /// Vanilla `LivingEntity.travel`, used for players. Mobs travel from `tick_movement`,
    /// which adds their own hooks.
    pub(crate) fn travel(&self, caller: &dyn EntityBase) {
        let touching_water = self.entity.touching_water.load(SeqCst);
        if self.should_travel_in_fluid(caller) {
            self.travel_in_fluid(caller, touching_water);
        } else if self.entity.is_fall_flying() {
            self.travel_fall_flying(caller);
        } else {
            self.travel_in_air(caller);
        }
    }

    /// Vanilla `shouldTravelInFluid` for the fluid at the block position: a flying player
    /// isn't affected by fluids and a strider stands on lava (`canStandOnFluid`).
    fn should_travel_in_fluid(&self, caller: &dyn EntityBase) -> bool {
        if !(self.entity.touching_water.load(SeqCst) || self.entity.touching_lava.load(SeqCst))
            || caller
                .get_player()
                .is_some_and(super::player::Player::is_flying)
        {
            return false;
        }
        if self.entity.entity_type == &EntityType::STRIDER {
            let (fluid, _) = self
                .entity
                .world
                .load()
                .get_fluid_and_fluid_state(&self.entity.block_pos.load());
            if fluid.id == Fluid::LAVA.id || fluid.id == Fluid::FLOWING_LAVA.id {
                return false;
            }
        }
        true
    }

    /// Vanilla `LivingEntity.travelFallFlying`.
    pub(crate) fn travel_fall_flying(&self, caller: &dyn EntityBase) {
        // A climbable block catches the glider.
        if self.on_climbable(caller) {
            self.travel_in_air(caller);
            // `stopFallFlying`.
            if self.entity.is_fall_flying() {
                self.entity.set_fall_flying(false);
            }
            return;
        }
        let last_movement = self.entity.velocity.load();
        let last_speed = last_movement.horizontal_length();
        let movement = self.update_fall_flying_movement(caller, last_movement);
        self.entity.velocity.store(movement);
        self.make_move(caller);
        let new_speed = self.entity.velocity.load().horizontal_length();
        self.handle_fall_flying_collisions(caller, last_speed, new_speed);
    }

    /// Vanilla `LivingEntity.handleFallFlyingCollisions`: hitting a wall while gliding hurts.
    fn handle_fall_flying_collisions(
        &self,
        caller: &dyn EntityBase,
        move_hor_length: f64,
        new_move_hor_length: f64,
    ) {
        if self.entity.horizontal_collision.load(SeqCst) {
            let diff = move_hor_length - new_move_hor_length;
            let dmg = (diff * 10.0 - 3.0) as f32;
            if dmg > 0.0 {
                caller.play_sound_at_self(self.get_fall_damage_sound(dmg as i32), 1.0, 1.0);
                caller.damage(caller, dmg, DamageType::FLY_INTO_WALL);
            }
        }
    }

    /// Vanilla `LivingEntity.updateFallFlyingMovement`.
    fn update_fall_flying_movement(
        &self,
        caller: &dyn EntityBase,
        mut movement: Vector3<f64>,
    ) -> Vector3<f64> {
        let x_rot = self.entity.pitch.load();
        let look = Entity::calculate_view_vector(x_rot, self.entity.yaw.load());
        let lean_angle = x_rot * (std::f64::consts::PI / 180.0) as f32;
        let look_hor_length = look.horizontal_length();
        let move_hor_length = movement.horizontal_length();
        let gravity = self.get_effective_gravity(caller);
        let lean_cos = f64::from(lean_angle).cos();
        let lift_force = lean_cos * lean_cos;
        movement.y += gravity * (-1.0 + lift_force * 0.75);
        if movement.y < 0.0 && look_hor_length > 0.0 {
            let convert = movement.y * -0.1 * lift_force;
            movement = movement.add_raw(
                look.x * convert / look_hor_length,
                convert,
                look.z * convert / look_hor_length,
            );
        }
        if lean_angle < 0.0 && look_hor_length > 0.0 {
            let convert = move_hor_length * f64::from(-pumpkin_util::math::sin(lean_angle)) * 0.04;
            movement = movement.add_raw(
                -look.x * convert / look_hor_length,
                convert * 3.2,
                -look.z * convert / look_hor_length,
            );
        }
        if look_hor_length > 0.0 {
            movement = movement.add_raw(
                (look.x / look_hor_length * move_hor_length - movement.x) * 0.1,
                0.0,
                (look.z / look_hor_length * move_hor_length - movement.z) * 0.1,
            );
        }
        movement.multiply(f64::from(0.99f32), f64::from(0.98f32), f64::from(0.99f32))
    }

    pub(crate) fn travel_in_fluid(&self, caller: &dyn EntityBase, water: bool) {
        let movement_input = self.movement_input.load();

        let falling = self.entity.velocity.load().y <= 0.0;
        let old_y = self.entity.pos.load().y;
        let gravity = self.get_effective_gravity(caller);
        let effective_speed = self.get_speed(caller);

        if water {
            // Vanilla `travelInWater` works in floats.
            let mut friction: f32 = if self.entity.sprinting.load(Relaxed) {
                0.9
            } else {
                self.water_movement_speed_multiplier
            };

            let mut speed: f32 = 0.02;

            let mut water_movement_efficiency =
                self.get_attribute_value(&Attributes::WATER_MOVEMENT_EFFICIENCY) as f32;
            if !self.entity.on_ground.load(SeqCst) {
                water_movement_efficiency *= 0.5;
            }

            if water_movement_efficiency > 0.0 {
                friction += (0.546_000_06 - friction) * water_movement_efficiency;
                speed += (effective_speed as f32 - speed) * water_movement_efficiency;
            }

            if self.has_effect(&StatusEffect::DOLPHINS_GRACE) {
                friction = 0.96;
            }
            let friction = f64::from(friction);

            self.entity
                .update_velocity_from_input(movement_input, f64::from(speed));

            self.make_move(caller);

            let mut velo = self.entity.velocity.load();
            if self.entity.horizontal_collision.load(SeqCst) && self.on_climbable(caller) {
                velo.y = 0.2;
            }

            velo = velo.multiply(friction, f64::from(0.8f32), friction);

            self.apply_fluid_moving_speed(&mut velo.y, gravity, falling);
            self.entity.velocity.store(velo);
        } else {
            self.entity
                .update_velocity_from_input(movement_input, f64::from(0.02f32));

            self.make_move(caller);

            let mut velo = self.entity.velocity.load();

            if self.entity.lava_height.load() <= self.fluid_jump_threshold(caller) {
                velo.x *= 0.5;
                velo.z *= 0.5;
                velo.y *= f64::from(0.8f32);

                self.apply_fluid_moving_speed(&mut velo.y, gravity, falling);
            } else {
                velo = velo * 0.5;
            }

            if gravity != 0.0 {
                velo.y -= gravity / 4.0; // Negative gravity = buoyancy
            }

            self.entity.velocity.store(velo);
        }

        // `jumpOutOfFluid`: climb out when the box raised to 0.6 above the start is free.
        let mut velo = self.entity.velocity.load();
        if self.entity.horizontal_collision.load(SeqCst) {
            let offset = Vector3::new(
                velo.x,
                velo.y + f64::from(0.6f32) - self.entity.pos.load().y + old_y,
                velo.z,
            );
            let moved = self.entity.bounding_box.load().shift(offset);
            let world = self.entity.world.load();
            if world.is_space_empty(moved) && !world.check_fluid_collision(moved) {
                velo.y = f64::from(0.3f32);
                self.entity.velocity.store(velo);
            }
        }
    }

    fn apply_fluid_moving_speed(&self, dy: &mut f64, gravity: f64, falling: bool) {
        if gravity != 0.0 && !self.entity.sprinting.load(Relaxed) {
            if falling && (*dy - 0.005).abs() >= 0.003 && (*dy - gravity / 16.0).abs() < 0.003 {
                *dy = -0.003;
            } else {
                *dy -= gravity / 16.0;
            }
        }
    }

    pub(crate) fn make_move(&self, caller: &dyn EntityBase) {
        self.entity.move_entity(caller, self.entity.velocity.load());
    }

    /// Vanilla `LivingEntity.onClimbable`: in a climbable block, or in an open trapdoor over a
    /// ladder facing the same way. Remembers where, for the death message.
    pub fn on_climbable(&self, caller: &dyn EntityBase) -> bool {
        if caller.is_spectator() {
            return false;
        }
        if let Some(climbing) = caller.get_mob().and_then(super::mob::Mob::mob_on_climbable) {
            return climbing;
        }
        let pos = self.entity.block_pos.load();
        let world = self.entity.world.load();
        let (block, state) = world.get_block_and_state(&pos);
        if self.entity.is_fall_flying() && block.has_tag(&tag::Block::MINECRAFT_CAN_GLIDE_THROUGH) {
            return false;
        }
        let climbable = block.has_tag(&tag::Block::MINECRAFT_CLIMBABLE)
            || (block.has_tag(&tag::Block::MINECRAFT_TRAPDOORS)
                && Self::trapdoor_usable_as_ladder(&world, &pos, state.id));
        if climbable {
            self.climbing_pos.store(Some(pos));
        }
        climbable
    }

    /// Vanilla `LivingEntity.trapdoorUsableAsLadder`.
    fn trapdoor_usable_as_ladder(
        world: &crate::world::World,
        pos: &BlockPos,
        state_id: pumpkin_data::BlockStateId,
    ) -> bool {
        use pumpkin_data::block_properties::{LadderLikeProperties, OakTrapdoorLikeProperties};
        let trapdoor = OakTrapdoorLikeProperties::from_state_id(state_id);
        if !trapdoor.open {
            return false;
        }
        let (below_block, below_state) = world.get_block_and_state(&pos.down());
        below_block == &Block::LADDER
            && LadderLikeProperties::from_state_id(below_state.id).facing == trapdoor.facing
    }

    /// Vanilla `LivingEntity.handleOnClimbable`: climbing ends a fall and caps the speed, and a
    /// sneaking player holds on.
    fn handle_on_climbable(&self, caller: &dyn EntityBase) {
        if !self.on_climbable(caller) {
            return;
        }
        self.entity.reset_fall_distance();
        let max = f64::from(0.15f32);
        let velo = self.entity.velocity.load();
        let mut yd = velo.y.max(-max);
        if yd < 0.0
            && caller.get_player().is_some()
            && self.entity.is_sneaking()
            && self
                .entity
                .world
                .load()
                .get_block(&self.entity.block_pos.load())
                != &Block::SCAFFOLDING
        {
            yd = 0.0;
        }
        self.entity.velocity.store(Vector3::new(
            velo.x.clamp(-max, max),
            yd,
            velo.z.clamp(-max, max),
        ));
    }

    /// Vanilla `getFluidJumpThreshold`, with the mob's override.
    fn fluid_jump_threshold(&self, caller: &dyn EntityBase) -> f64 {
        caller
            .get_mob()
            .and_then(super::mob::Mob::fluid_jump_threshold)
            .unwrap_or_else(|| self.get_swim_height())
    }

    pub fn get_swim_height(&self) -> f64 {
        let eye_height = self.entity.get_eye_height();

        if self.entity.entity_type == &EntityType::BREEZE {
            eye_height
        } else if eye_height < 0.4 {
            0.0
        } else {
            0.4
        }
    }

    /// Vanilla `jumpInLiquid`, which `Mob` and some mobs override.
    fn jump_in_liquid(&self, caller: &dyn EntityBase, lava: bool) {
        if let Some(mob) = caller.get_mob() {
            mob.jump_in_liquid(lava);
        } else {
            self.jump_in_liquid_base();
        }
    }

    /// Vanilla `LivingEntity.jumpInLiquid`.
    pub fn jump_in_liquid_base(&self) {
        let mut velocity = self.entity.velocity.load();
        velocity.y += f64::from(0.04f32);
        self.entity.velocity.store(velocity);
    }

    /// Vanilla `LivingEntity.jumpFromGround`, in vanilla's float maths.
    pub fn jump(&self) {
        self.jump_with_power(self.get_jump_velocity(1.0));
    }

    /// Vanilla `LivingEntity.jumpFromGround` with a mob's own `getJumpPower()` override.
    pub fn jump_with_power(&self, jump: f32) {
        if jump <= 1.0E-5 {
            return;
        }

        let mut velo = self.entity.velocity.load();
        velo.y = f64::from(jump).max(velo.y);
        if self.entity.sprinting.load(Relaxed) {
            let angle = self.entity.yaw.load() * (std::f64::consts::PI / 180.0) as f32;
            velo.x += f64::from(-pumpkin_util::math::sin(angle)) * 0.2;
            velo.z += f64::from(pumpkin_util::math::cos(angle)) * 0.2;
        }
        self.entity.velocity.store(velo);
        self.entity.velocity_dirty.store(true, SeqCst);
    }

    /// Vanilla `getJumpPower(multiplier)`: jump strength times the block jump factor, plus
    /// jump boost, all floats.
    pub fn get_jump_velocity(&self, multiplier: f32) -> f32 {
        self.get_attribute_value(&Attributes::JUMP_STRENGTH) as f32
            * multiplier
            * self.entity.get_jump_velocity_multiplier()
            + self.get_jump_boost_power()
    }

    /// Vanilla `LivingEntity.getJumpBoostPower`.
    pub fn get_jump_boost_power(&self) -> f32 {
        self.get_effect(&StatusEffect::JUMP_BOOST)
            .map_or(0.0, |effect| 0.1 * (effect.amplifier as f32 + 1.0))
    }

    /// Vanilla `LivingEntity.checkFallDamage`: the landing also refreshes the fluid state,
    /// runs the location effects and throws up particles of the block landed on.
    pub fn check_fall_damage(
        &self,
        caller: &dyn EntityBase,
        ya: f64,
        on_ground: bool,
        state: &'static BlockState,
        pos: BlockPos,
    ) {
        if !self.entity.touching_water.load(SeqCst) {
            self.entity.update_fluid_interaction(caller);
        }
        let fall_distance = self.entity.fall_distance.load();
        if on_ground && fall_distance > 0.0 {
            self.on_changed_block(caller, pos);
            let power = f64::from(self.calculate_fall_power(fall_distance).floor().max(0.0) as i32);
            if power > 0.0 && !state.is_air() {
                let entity_pos = self.entity.pos.load();
                let (mut x, y, mut z) = (entity_pos.x, entity_pos.y, entity_pos.z);
                let entity_block_pos = self.entity.block_pos.load();
                if pos.0.x != entity_block_pos.0.x || pos.0.z != entity_block_pos.0.z {
                    let x_diff = x - f64::from(pos.0.x) - 0.5;
                    let z_diff = z - f64::from(pos.0.z) - 0.5;
                    let max_diff = x_diff.abs().max(z_diff.abs());
                    x = f64::from(pos.0.x) + 0.5 + x_diff / max_diff * 0.5;
                    z = f64::from(pos.0.z) + 0.5 + z_diff / max_diff * 0.5;
                }
                let scale = (f64::from(0.2f32) + power / 15.0).min(2.5);
                let particles = (150.0 * scale) as i32;
                self.entity.world.load().spawn_block_particle(
                    Particle::Block,
                    state.id,
                    Vector3::new(x, y, z),
                    Vector3::new(0.0, 0.0, 0.0),
                    0.15,
                    particles,
                );
            }
        }
        self.entity
            .check_fall_damage_default(caller, ya, on_ground, state, pos);
        if on_ground {
            self.climbing_pos.store(None);
        }
    }

    /// Vanilla `LivingEntity.causeFallDamage`, with `Player`'s: a player that may fly takes
    /// no fall damage, and a fall of two blocks or more counts towards its statistic.
    pub fn cause_fall_damage(
        &self,
        caller: &dyn EntityBase,
        fall_distance: f64,
        damage_modifier: f32,
        damage_type: DamageType,
    ) -> bool {
        if let Some(player) = caller.get_player() {
            if player
                .abilities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .allow_flying
            {
                return false;
            }
            if fall_distance >= 2.0 {
                player.increment_stat(
                    StatisticCategory::Custom,
                    CustomStatistic::FallOneCm as i32,
                    (fall_distance * 100.0).round() as i32,
                );
            }
        }

        // After an ignored impulse only the drop below where it struck counts.
        let effective_fall_distance =
            self.current_impulse_impact_pos
                .load()
                .map_or(fall_distance, |impact| {
                    let effective = fall_distance.min(impact.y - self.entity.pos.load().y);
                    if effective <= 0.0 {
                        self.reset_current_impulse_context();
                    } else {
                        self.try_reset_current_impulse_context();
                    }
                    effective
                });

        let damaged = self.entity.cause_fall_damage_default(
            effective_fall_distance,
            damage_modifier,
            damage_type,
        );
        let damage = caller.get_mob().map_or_else(
            || self.calculate_fall_damage(effective_fall_distance, damage_modifier),
            |mob| mob.calculate_fall_damage(effective_fall_distance, damage_modifier),
        );
        if damage > 0 {
            self.reset_current_impulse_context();
            caller.play_sound_at_self(self.get_fall_damage_sound(damage), 1.0, 1.0);
            self.play_block_fall_sound(caller);
            caller.damage(caller, damage as f32, damage_type);
            true
        } else {
            damaged
        }
    }

    /// Vanilla `LivingEntity.calculateFallDamage`.
    #[must_use]
    pub fn calculate_fall_damage(&self, fall_distance: f64, damage_modifier: f32) -> i32 {
        if self
            .entity
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_FALL_DAMAGE_IMMUNE)
        {
            return 0;
        }
        let base_damage = self.calculate_fall_power(fall_distance);
        (base_damage
            * f64::from(damage_modifier)
            * self.get_attribute_value(&Attributes::FALL_DAMAGE_MULTIPLIER))
        .floor() as i32
    }

    /// Vanilla `LivingEntity.calculateFallPower`.
    fn calculate_fall_power(&self, fall_distance: f64) -> f64 {
        fall_distance + 1.0E-6 - self.get_attribute_value(&Attributes::SAFE_FALL_DISTANCE)
    }

    /// Vanilla `LivingEntity.playBlockFallSound`: the fall sound of the block under the feet.
    pub fn play_block_fall_sound(&self, caller: &dyn EntityBase) {
        let pos = self.entity.pos.load();
        let block_pos = BlockPos::new(
            pos.x.floor() as i32,
            (pos.y - f64::from(0.2f32)).floor() as i32,
            pos.z.floor() as i32,
        );
        let state = self.entity.world.load().get_block_state(&block_pos);
        if !state.is_air() {
            let sound_type = SoundType::from_state_id(state.id.as_u16());
            caller.play_sound_at_self(
                sound_type.fall_sound,
                sound_type.volume * 0.5,
                sound_type.pitch * 0.75,
            );
        }
    }

    /// Vanilla `LivingEntity.getFallSounds` and its overrides: the small and big fall sounds.
    #[must_use]
    pub fn get_fall_sounds(&self) -> (Sound, Sound) {
        let entity_type = self.entity.entity_type;
        if entity_type == &EntityType::PLAYER {
            (Sound::EntityPlayerSmallFall, Sound::EntityPlayerBigFall)
        } else if entity_type == &EntityType::ARMOR_STAND {
            (Sound::EntityArmorStandFall, Sound::EntityArmorStandFall)
        } else if super::mob::is_monster(entity_type) {
            (Sound::EntityHostileSmallFall, Sound::EntityHostileBigFall)
        } else {
            (Sound::EntityGenericSmallFall, Sound::EntityGenericBigFall)
        }
    }

    /// Vanilla `LivingEntity.getFallDamageSound`.
    #[must_use]
    pub fn get_fall_damage_sound(&self, damage: i32) -> Sound {
        let (small, big) = self.get_fall_sounds();
        if damage > 4 { big } else { small }
    }

    /// Vanilla `LivingEntity.setIgnoreFallDamageFromCurrentImpulse`.
    pub fn set_ignore_fall_damage_from_current_impulse(
        &self,
        ignore_fall_damage: bool,
        new_impulse_impact_pos: Vector3<f64>,
    ) {
        if ignore_fall_damage {
            self.apply_post_impulse_grace_time(40);
            self.current_impulse_impact_pos
                .store(Some(new_impulse_impact_pos));
        } else {
            self.current_impulse_context_reset_grace_time
                .store(0, Relaxed);
        }
    }

    /// Vanilla `LivingEntity.applyPostImpulseGraceTime`.
    pub fn apply_post_impulse_grace_time(&self, ticks: i32) {
        self.current_impulse_context_reset_grace_time
            .fetch_max(ticks, Relaxed);
    }

    /// Vanilla `LivingEntity.isIgnoringFallDamageFromCurrentImpulse`.
    pub fn is_ignoring_fall_damage_from_current_impulse(&self) -> bool {
        self.current_impulse_impact_pos.load().is_some()
    }

    /// Vanilla `LivingEntity.tryResetCurrentImpulseContext`.
    pub fn try_reset_current_impulse_context(&self) {
        if self.current_impulse_context_reset_grace_time.load(Relaxed) == 0 {
            self.reset_current_impulse_context();
        }
    }

    /// Vanilla `LivingEntity.resetCurrentImpulseContext`.
    pub fn reset_current_impulse_context(&self) {
        self.current_impulse_context_reset_grace_time
            .store(0, Relaxed);
        self.current_impulse_impact_pos.store(None);
    }

    #[allow(clippy::redundant_closure_for_method_calls)]
    /// Builds the chat death message for this entity: picks the
    /// `death.attack.<id>` (or the `.player` variant when a killer entity is
    /// known, or the kill-credit name when the killer is offline/removed)
    /// translation and fills in the victim and killer display names.
    pub fn get_death_message(
        dyn_self: &dyn EntityBase,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> TextComponent {
        let kill_credit = dyn_self.get_living_entity().and_then(Self::get_kill_credit);
        let kill_credit_name = kill_credit.as_ref().map(|c| c.get_display_name());

        if let Some(living) = dyn_self.get_living_entity() {
            let tracker = living
                .combat_tracker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            return tracker.get_death_message(dyn_self.get_display_name(), kill_credit_name);
        }

        if let Some(cause) = cause
            && source.is_some()
        {
            TextComponent::translate_cross(
                format!("death.attack.{}", damage_type.message_id),
                format!("death.attack.{}", damage_type.message_id),
                [dyn_self.get_display_name(), cause.get_display_name()],
            )
        } else if let Some(killer) = cause
            .or(source)
            .map(|c| c.get_display_name())
            .or(kill_credit_name)
        {
            TextComponent::translate_cross(
                format!("death.attack.{}.player", damage_type.message_id),
                format!("death.attack.{}.player", damage_type.message_id),
                [dyn_self.get_display_name(), killer],
            )
        } else {
            TextComponent::translate_cross(
                format!("death.attack.{}", damage_type.message_id),
                format!("death.attack.{}", damage_type.message_id),
                [dyn_self.get_display_name()],
            )
        }
    }

    /// Vanilla `LivingEntity.die`.
    pub fn die(
        &self,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) {
        let world = self.entity.world.load();
        let Some(dyn_self) = world.get_entity_by_id(self.entity.entity_id) else {
            return;
        };
        if self.entity.removed.load(Relaxed)
            || self
                .dead
                .compare_exchange(false, true, Relaxed, Relaxed)
                .is_err()
        {
            return;
        }
        if let Some(mob) = dyn_self.get_mob() {
            mob.before_die();
        }
        self.movement_input.store(Vector3::default());
        self.jumping.store(false, Relaxed);

        // `killer.awardKillScore(this, source)`, with the victim's own death stats.
        let kill_credit = self.get_kill_credit();
        self.update_death_stats(&*dyn_self, kill_credit.as_deref());

        if Self::is_sleeping(&*dyn_self) {
            self.stop_sleeping(&*dyn_self);
        }
        self.clear_active_hand();

        // Vanilla `handleKillingBlow`, which by default is what marks the entity dead.
        if dyn_self
            .get_mob()
            .is_some_and(super::mob::Mob::handle_killing_blow)
        {
            self.dead.store(false, Relaxed);
        }

        // The attacker's `killedEntity` can claim the body, leaving no loot.
        let perished = cause
            .and_then(EntityBase::get_mob)
            .is_none_or(|mob| mob.killed_entity(&*dyn_self));
        if perished {
            world.game_event_from(
                Some(&*dyn_self),
                pumpkin_data::game_event::GameEvent::EntityDie,
                self.entity.pos.load(),
            );
            self.drop_all_death_loot(&*dyn_self, damage_type, source, cause);
            self.create_wither_rose(kill_credit.as_deref());
        }
        world.send_entity_status(&self.entity, EntityStatus::Death, Some(ActorEventID::Death));
        // `setPose(Pose.DYING)`: synced, and the dimensions follow.
        dyn_self.set_pose(EntityPose::Dying);

        // Broadcast death message if it's a player and the gamerule is enabled
        self.broadcast_death_message(&*dyn_self, damage_type, source, cause);
    }

    /// Vanilla `dropAllDeathLoot`.
    fn drop_all_death_loot(
        &self,
        dyn_self: &dyn EntityBase,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) {
        let world = self.entity.world.load();
        if let Some(mob) = dyn_self.get_mob() {
            mob.before_drop_all_death_loot();
        }
        let player_killed = self.last_hurt_by_player_memory_time.load(Relaxed) > 0;
        // `shouldDropLoot`.
        if !self.entity.is_baby() && world.level_info.load().game_rules.mob_drops {
            let tool = cause
                .and_then(|cause| cause.get_player())
                .map(|player| player.inventory().get_stack_in_hand(Hand::Right))
                .filter(|hand_stack| !hand_stack.is_empty());
            let params = LootContextParameters {
                killed_by_player: Some(player_killed),
                this_entity: Some(as_entity_base(&self.entity.entity_type)),
                // `ATTACKING_ENTITY`: `source.getEntity()`.
                killer_entity: cause.map(|c| c.get_entity().entity_type),
                attacking_entity_looting_level: cause
                    .and_then(|cause| {
                        cause.get_living_entity().map(|living| {
                            EnchantmentHelper::get_enchantment_level(
                                &Enchantment::LOOTING,
                                cause,
                                living,
                            )
                        })
                    })
                    .unwrap_or(0),
                direct_killer_entity: source.map(|s| s.get_entity().entity_type),
                position: Some(self.entity.pos.load()),
                world_time: world.get_time_of_day() as u64,
                damage_type: Some(damage_type),
                tool,
                is_raining: Some(world.is_raining()),
                is_thundering: Some(world.is_thundering()),
                is_on_fire: Some(self.entity.fire_ticks.load(Relaxed) > 0),
                direct_attacker_weapon: source.and_then(|direct| {
                    direct
                        .get_living_entity()
                        .map(|living| living.held_item(direct))
                }),
                damage_source_entity: cause.map(|cause| cause.get_entity().entity_type),
                damage_source_frog_variant: cause
                    .and_then(crate::entity::passive::frog::frog_variant_of)
                    .map(|variant| variant.asset_id()),
                this_cube_mob_size: dyn_self
                    .cast_any()
                    .downcast_ref::<crate::entity::mob::cube_mob::CubeMobEntity>()
                    .map(crate::entity::mob::cube_mob::CubeMobEntity::get_size),
                ..Default::default()
            };
            self.drop_loot(dyn_self.get_mob(), &params);
            if let Some(mob) = dyn_self.get_mob() {
                self.drop_mob_equipment(
                    mob,
                    &DamageSource {
                        damage_type,
                        direct: source,
                        causing: cause,
                    },
                    player_killed,
                );
                mob.drop_custom_death_loot();
            }
        }
        if let Some(mob) = dyn_self.get_mob() {
            mob.drop_equipment();
        }
        self.drop_experience(dyn_self, cause);
    }

    /// Vanilla `dropExperience`: a player always drops its experience, a mob only after a
    /// player hurt it recently.
    fn drop_experience(&self, dyn_self: &dyn EntityBase, killer: Option<&dyn EntityBase>) {
        let world = self.entity.world.load_full();
        // `isAlwaysExperienceDropper` is `Player`'s; `shouldDropExperience` is `!isBaby()`.
        let drops = self.is_player()
            || self.last_hurt_by_player_memory_time.load(Relaxed) > 0
                && !self.entity.is_baby()
                && world.level_info.load().game_rules.mob_drops;
        // TODO: `wasExperienceConsumed`, set by a sculk catalyst; the catalyst isn't ported.
        if drops {
            // `getExperienceReward`; no vanilla enchantment has a `mob_experience` effect.
            let amount = dyn_self.get_experience_reward(killer);
            if amount > 0 {
                ExperienceOrbEntity::spawn(&world, self.entity.pos.load(), amount);
            }
        }
    }

    /// Vanilla `createWitherRose`: a wither's kill leaves a wither rose.
    fn create_wither_rose(&self, killer: Option<&dyn EntityBase>) {
        if !killer.is_some_and(|killer| killer.get_entity().entity_type == &EntityType::WITHER) {
            return;
        }
        let world = self.entity.world.load_full();
        let mut placed = false;
        if world.level_info.load().game_rules.mob_griefing {
            let pos = self.entity.block_pos.load();
            let state = Block::WITHER_ROSE.default_state;
            if world.get_block_state(&pos).is_air()
                && world.block_registry.can_place_at(
                    None,
                    Some(&world),
                    world.as_ref(),
                    None,
                    &Block::WITHER_ROSE,
                    state,
                    &pos,
                    None,
                    None,
                )
            {
                world.set_block_state(&pos, state.id, crate::world::BlockFlags::NOTIFY_ALL);
                placed = true;
            }
        }
        if !placed {
            // `new ItemEntity(level, x, y, z, stack)`: no pickup delay.
            let entity = Entity::new(world.clone(), self.entity.pos.load(), &EntityType::ITEM);
            let item = super::item::ItemEntity::new(entity, ItemStack::new(1, &Item::WITHER_ROSE));
            item.set_pickup_delay(0);
            world.spawn_entity(Arc::new(item));
        }
    }

    /// Vanilla `triggerOnDeathMobEffects` for a kill: each active effect's `onMobRemoved`.
    /// Clearing the effects is left to the caller.
    pub fn trigger_on_death_mob_effects(&self) {
        let active_effects: Vec<_> = self
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .map(|e| (e.effect_type, e.amplifier))
            .collect();
        for (effect_type, amplifier) in active_effects {
            if let Some(mob_effect) = crate::entity::effect::get_mob_effect(effect_type) {
                mob_effect.on_mob_death(self, amplifier);
            }
        }
    }

    /// Vanilla `Mob.dropCustomDeathLoot`: each slot's equipment drops by its chance (Looting
    /// raising it), but only for a kill a player had a hand in unless the chance marks the
    /// item as preserved. A dropped item that isn't preserved comes out worn.
    fn drop_mob_equipment(
        &self,
        mob: &dyn super::mob::Mob,
        killing_blow: &DamageSource<'_>,
        killed_by_player: bool,
    ) {
        let world = self.entity.world.load();
        for slot in &EquipmentSlot::VALUES {
            let mut chance = self
                .equipment_drop_chances
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(slot)
                .copied()
                .unwrap_or(DEFAULT_EQUIPMENT_DROP_CHANCE);
            if chance == 0.0 {
                continue;
            }
            // `DropChances.isPreserved`.
            let preserved = chance > 1.0;
            if let Some(killer) = killing_blow.causing
                && let Some(killer_living) = killer.get_living_entity()
            {
                chance = EnchantmentHelper::process_equipment_drop_chance(
                    &world,
                    killer,
                    killer_living,
                    killing_blow,
                    chance,
                );
            }
            let mut item = self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(slot);
            if item.is_empty()
                || crate::entity::passive::allay::prevents_equipment_drop(&item)
                || !(killed_by_player || preserved)
                || self.entity.random().next_f32() >= chance
            {
                continue;
            }
            if !preserved && let Some(max_damage) = item.get_max_damage() {
                // `nextInt(1 + nextInt(Math.max(maxDamage - 3, 1)))`.
                let outer = {
                    let mut random = self.entity.random();
                    let inner = random.next_bounded_i32((max_damage - 3).max(1));
                    random.next_bounded_i32(1 + inner)
                };
                item.set_damage(max_damage - outer);
            }
            self.entity.spawn_at_location(item);
            // `setItemSlot`: the clients see the slot empty, with an `unequip` game event.
            mob.set_item_slot_and_notify(slot, ItemStack::EMPTY.clone());
        }
    }

    fn broadcast_death_message(
        &self,
        dyn_self: &dyn EntityBase,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) {
        let world = self.entity.world.load();
        let show_death_messages = { world.level_info.load().game_rules.show_death_messages };
        if self.entity.entity_type == &EntityType::PLAYER {
            let death_message = Self::get_death_message(dyn_self, damage_type, source, cause);
            let mut final_death_message = death_message;
            if let Some(player) = dyn_self.get_player() {
                if let Some(player_arc) = world.get_player_by_uuid(player.gameprofile.id)
                    && let Some(server) = world.server.upgrade()
                {
                    let mut event =
                        crate::plugin::api::events::entity::entity_death::PlayerDeathEvent::new(
                            player_arc,
                            final_death_message.clone(),
                            0,
                        );
                    server.plugin_manager.fire_blocking(&server, &mut event);
                    if event.cancelled {
                        return;
                    }
                    final_death_message = event.death_message;
                }

                player.handle_killed(&final_death_message);
            }

            if show_death_messages && let Some(server) = world.server.upgrade() {
                server.broadcast_system_message(&final_death_message, false);
            }
        } else if self.entity.custom_name.load().is_some() {
            let death_message = Self::get_death_message(dyn_self, damage_type, source, cause);
            tracing::info!(
                "Named entity {} died: {}",
                dyn_self.get_display_name().to_pretty_console(),
                death_message.to_pretty_console()
            );
        }
    }

    fn update_death_stats(&self, dyn_self: &dyn EntityBase, cause: Option<&dyn EntityBase>) {
        if let Some(victim_player) = dyn_self.get_player() {
            victim_player.increment_custom_stat(CustomStatistic::Deaths, 1);
            victim_player.set_stat(
                StatisticCategory::Custom,
                CustomStatistic::TimeSinceDeath as i32,
                0,
            );
            victim_player.set_stat(
                StatisticCategory::Custom,
                CustomStatistic::TimeSinceRest as i32,
                0,
            );
            if let Some(killer_entity) = cause.map(EntityBase::get_entity) {
                victim_player.increment_stat(
                    StatisticCategory::KilledBy,
                    killer_entity.entity_type.id as i32,
                    1,
                );
            }
        }

        if let Some(killer_player) = cause.and_then(|c| c.get_player()) {
            killer_player.increment_stat(
                StatisticCategory::Killed,
                self.entity.entity_type.id as i32,
                1,
            );
            if dyn_self.get_player().is_some() {
                killer_player.increment_stat(
                    StatisticCategory::Custom,
                    CustomStatistic::PlayerKills as i32,
                    1,
                );
            } else {
                killer_player.increment_stat(
                    StatisticCategory::Custom,
                    CustomStatistic::MobKills as i32,
                    1,
                );

                let resource_name = self.entity.entity_type.resource_name;
                let criterion_key = format!("minecraft:{resource_name}");
                killer_player.trigger_advancement(
                    crate::entity::player::advancement::trigger::AdvancementTrigger::PlayerKilledEntity {
                        entity_type_resource: criterion_key,
                    },
                );

                if resource_name == "skeleton" {
                    let distance_sq = killer_player
                        .position()
                        .squared_distance_to_vec(&self.entity.pos.load());
                    if distance_sq >= 2500.0 {
                        killer_player.trigger_advancement(crate::entity::player::advancement::trigger::AdvancementTrigger::SniperDuel);
                    }
                }

                if resource_name == "phantom" {
                    killer_player.trigger_advancement(crate::entity::player::advancement::trigger::AdvancementTrigger::TwoBirdsOneArrow);
                }

                let held_item = killer_player.inventory().held_item();
                let is_crossbow = held_item.item.registry_key == "crossbow";
                if is_crossbow {
                    killer_player.trigger_advancement(
                        crate::entity::player::advancement::trigger::AdvancementTrigger::Arbalistic,
                    );
                }
            }
        }
    }

    fn drop_loot(&self, mob: Option<&dyn super::mob::Mob>, params: &LootContextParameters) {
        // Vanilla `Mob.getLootTable`/`getLootTableSeed`: a `DeathLootTable` wins.
        let (death_loot_table, death_loot_table_seed) =
            mob.map_or((None, 0), |mob| mob.get_mob_entity().death_loot_table());
        let key = death_loot_table.unwrap_or_else(|| {
            let resource_name = self.get_entity().entity_type.resource_name;
            format!("minecraft:entities/{resource_name}")
        });
        let world = self.entity.world.load();
        if let Some(loot_table) = world.get_loot_table(&key) {
            for stack in crate::world::loot::generate_loot_from_handle(
                &loot_table,
                &world,
                death_loot_table_seed,
                params,
            ) {
                self.entity.spawn_at_location(stack);
            }
        }
        // Vanilla `Mob.dropFromLootTable`: the death loot table is used once.
        if let Some(mob) = mob {
            mob.get_mob_entity().clear_death_loot_table();
        }
    }

    /// Vanilla `LivingEntity.dropFromLootTable`: hands what `key` rolls to `consumer` and
    /// says whether anything dropped.
    pub fn drop_from_loot_table(
        &self,
        key: &str,
        params: &LootContextParameters,
        consumer: impl FnMut(ItemStack),
    ) -> bool {
        let world = self.entity.world.load();
        let Some(loot_table) = world.get_loot_table(key) else {
            return false;
        };
        let drops = crate::world::loot::generate_loot_from_handle(&loot_table, &world, 0, params);
        let dropped = !drops.is_empty();
        drops.into_iter().for_each(consumer);
        dropped
    }

    /// Vanilla `LivingEntity.dropFromGiftLootTable`.
    pub fn drop_from_gift_loot_table(&self, key: &str) -> bool {
        self.drop_from_gift_loot_table_with(key, |stack| self.entity.spawn_at_location(stack))
    }

    /// Vanilla `LivingEntity.dropFromGiftLootTable` with its own consumer.
    pub fn drop_from_gift_loot_table_with(
        &self,
        key: &str,
        consumer: impl FnMut(ItemStack),
    ) -> bool {
        self.drop_from_loot_table(
            key,
            &LootContextParameters {
                this_entity: Some(as_entity_base(&self.entity.entity_type)),
                position: Some(self.entity.pos.load()),
                ..Default::default()
            },
            consumer,
        )
    }

    /// Vanilla `LivingEntity.dropFromShearingLootTable`.
    pub fn drop_from_shearing_loot_table(
        &self,
        key: &str,
        tool: &ItemStack,
        consumer: impl FnMut(ItemStack),
    ) -> bool {
        self.drop_from_loot_table(
            key,
            &LootContextParameters {
                this_entity: Some(as_entity_base(&self.entity.entity_type)),
                position: Some(self.entity.pos.load()),
                tool: Some(tool.clone()),
                ..Default::default()
            },
            consumer,
        )
    }

    /// [`Self::drop_from_shearing_loot_table`] for a shearing table (`outer`) that only
    /// forwards to the table for this mob's variant (`inner`).
    pub fn drop_from_nested_shearing_loot_table(
        &self,
        outer: &str,
        inner: &str,
        tool: &ItemStack,
        consumer: impl FnMut(ItemStack),
    ) {
        let world = self.entity.world.load();
        let (Some(outer), Some(inner)) = (world.get_loot_table(outer), world.get_loot_table(inner))
        else {
            return;
        };
        let params = LootContextParameters {
            this_entity: Some(as_entity_base(&self.entity.entity_type)),
            position: Some(self.entity.pos.load()),
            tool: Some(tool.clone()),
            ..Default::default()
        };
        crate::world::loot::generate_nested_loot(&outer, &inner, &world, &params)
            .into_iter()
            .for_each(consumer);
    }

    /// Vanilla `LivingEntity.dropFromEntityInteractLootTable`.
    pub fn drop_from_entity_interact_loot_table(&self, key: &str, tool: &ItemStack) -> bool {
        self.drop_from_loot_table(
            key,
            &LootContextParameters {
                this_entity: Some(as_entity_base(&self.entity.entity_type)),
                tool: Some(tool.clone()),
                ..Default::default()
            },
            |stack| self.entity.spawn_at_location(stack),
        )
    }

    /// Vanilla `tickEffects` on the server, each effect through `MobEffectInstance.tickServer`.
    fn tick_effects(&self) {
        let effect_types: Vec<&'static StatusEffect> = self
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .copied()
            .collect();
        for effect_type in effect_types {
            if !self.tick_effect(effect_type) {
                break;
            }
        }
    }

    /// The effect types active now, to spot an `applyEffectTick` adding or removing one.
    fn active_effect_types(&self) -> Vec<*const StatusEffect> {
        let mut types: Vec<*const StatusEffect> = self
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .map(|effect_type| std::ptr::from_ref(*effect_type))
            .collect();
        types.sort_unstable();
        types
    }

    /// One step of vanilla `tickEffects`' loop over the `activeEffects` `HashMap`. Whether the
    /// loop goes on: when `applyEffectTick` adds or removes an effect (a bad omen giving the raid
    /// omen), vanilla's iterator throws a `ConcurrentModificationException` on its next
    /// `remove` or `next`, which `tickEffects` swallows, so the effect that did it isn't removed
    /// this tick and the rest of the effects aren't ticked. The set is compared before and
    /// after, so removing and re-adding the same effect inside one `applyEffectTick` (which
    /// vanilla's `modCount` would catch) goes unnoticed; no effect does that.
    fn tick_effect(&self, effect_type: &'static StatusEffect) -> bool {
        let Some((has_remaining, tick_count, amplifier)) = self
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&effect_type)
            .map(|effect| {
                let tick_count = if effect.is_infinite_duration() {
                    self.entity.tick_count.load(Relaxed)
                } else {
                    effect.duration
                };
                (
                    effect.has_remaining_duration(),
                    tick_count,
                    effect.amplifier,
                )
            })
        else {
            return true;
        };
        let mut keep = has_remaining;
        let mut modified = false;
        if keep
            && let Some(mob_effect) = crate::entity::effect::get_mob_effect(effect_type)
            && mob_effect.should_apply_effect_tick(tick_count, amplifier)
        {
            let before = self.active_effect_types();
            keep = mob_effect.apply_effect_tick(self, amplifier);
            modified = self.active_effect_types() != before;
            if !keep && modified {
                // `iterator.remove()` throws: the effect stays, unticked.
                return false;
            }
        }
        let snapshot = if keep {
            let mut effects = self
                .active_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(instance) = effects.get_mut(&effect_type) else {
                return !modified;
            };
            instance.tick_down_duration();
            let downgraded = instance.downgrade_to_hidden_effect();
            keep = instance.has_remaining_duration();
            let effect = instance.effect.clone();
            drop(effects);
            if downgraded {
                self.on_effect_updated(&effect, true);
            }
            Some(effect)
        } else {
            None
        };
        match snapshot {
            Some(effect) if keep => {
                // Every 600 ticks the effect is resent so the client's timer doesn't drift.
                if effect.duration % 600 == 0 {
                    self.on_effect_updated(&effect, false);
                }
            }
            _ => {
                // An effect that changed the set and then ran out: `iterator.remove()` throws
                // too, so it stays for another tick.
                if modified {
                    return false;
                }
                if self
                    .active_effects
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&effect_type)
                    .is_some()
                {
                    self.on_effects_removed(&[effect_type]);
                }
            }
        }
        // The next `iterator.next()` throws after a modification.
        !modified
    }

    /// Vanilla `LivingEntity.hurtArmor`, empty for most entities: players wear all four armour
    /// pieces and `Mob::hurt_armor` covers the mobs that override it.
    fn hurt_armor(&self, caller: &dyn EntityBase, damage_type: DamageType, damage: f32) {
        if caller.get_player().is_some() {
            // `Player.hurtArmor`.
            self.do_hurt_equipment(
                caller,
                damage_type,
                damage,
                &[
                    EquipmentSlot::FEET,
                    EquipmentSlot::LEGS,
                    EquipmentSlot::CHEST,
                    EquipmentSlot::HEAD,
                ],
            );
        } else if let Some(mob) = caller.get_mob() {
            mob.hurt_armor(damage_type, damage);
        }
    }

    /// Vanilla `LivingEntity.hurtHelmet`, which only `Player` overrides.
    fn hurt_helmet(&self, caller: &dyn EntityBase, damage_type: DamageType, damage: f32) {
        if caller.get_player().is_some() {
            self.do_hurt_equipment(caller, damage_type, damage, &[EquipmentSlot::HEAD]);
        }
    }

    /// Vanilla `LivingEntity.doHurtEquipment`.
    pub fn do_hurt_equipment(
        &self,
        caller: &dyn EntityBase,
        damage_type: DamageType,
        damage: f32,
        slots: &[EquipmentSlot],
    ) {
        if damage <= 0.0 {
            return;
        }
        let durability_damage = (damage / 4.0).max(1.0) as i32;
        for slot in slots {
            let stack = self.get_item_by_slot(caller, slot);
            if stack
                .get_data_component::<EquippableImpl>()
                .is_some_and(|equippable| equippable.damage_on_hurt)
                && stack.is_damageable()
                && !stack.is_unbreakable()
                && stack.can_be_hurt_by(&damage_type)
            {
                self.hurt_and_break_in_slot(caller, slot, durability_damage);
            }
        }
    }

    /// Vanilla `ItemStack.hurtAndBreak(int, LivingEntity, EquipmentSlot)` on the stack worn in
    /// `slot`.
    pub fn hurt_and_break_in_slot(
        &self,
        caller: &dyn EntityBase,
        slot: &EquipmentSlot,
        amount: i32,
    ) {
        if let Some(player) = caller.get_player() {
            player.damage_item_in_slot(slot, amount);
            return;
        }
        let (result, broken) = {
            let mut equipment = self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut stack = equipment.get(slot);
            let broken = stack.clone();
            let result = stack.hurt_and_break(&self.entity.world.load(), amount);
            if result != DamageResult::Untouched {
                equipment.put(slot, stack);
            }
            (result, broken)
        };
        if result == DamageResult::Broken {
            self.on_equipped_item_broken(&broken, slot);
        }
    }

    /// Vanilla `LivingEntity.onEquippedItemBroken`.
    pub fn on_equipped_item_broken(&self, broken: &ItemStack, slot: &EquipmentSlot) {
        self.entity.world.load().send_entity_status(
            &self.entity,
            super::equipment_break_status(slot),
            None,
        );
        let mut touched = Vec::new();
        self.stop_location_based_effects(broken, slot, &mut touched);
        // Synced with the other dirty attributes (`sendDirtyEntityData`).
        self.mark_attributes_dirty(&touched);
    }

    /// Vanilla `LivingEntity.getItemBySlot`; a player's main hand is its selected hotbar slot.
    pub fn get_item_by_slot(&self, caller: &dyn EntityBase, slot: &EquipmentSlot) -> ItemStack {
        match slot {
            EquipmentSlot::MainHand(_) => self.held_item(caller),
            EquipmentSlot::OffHand(_) => self.off_hand_item(caller),
            _ => self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(slot),
        }
    }

    /// The equipment `ServerEntity.sendPairingData` sends: every non-empty slot, in
    /// `EquipmentSlot::VALUES` order.
    pub fn pairing_equipment(
        &self,
        caller: &dyn EntityBase,
    ) -> Vec<(i8, ItemStackSerializer<'static>)> {
        EquipmentSlot::VALUES
            .iter()
            .filter_map(|slot| {
                let stack = self.get_item_by_slot(caller, slot);
                (!stack.is_empty()).then(|| (slot.discriminant(), ItemStackSerializer::from(stack)))
            })
            .collect()
    }

    pub fn held_item(&self, caller: &dyn EntityBase) -> ItemStack {
        if let Some(player) = caller.get_player() {
            return player.inventory.held_item();
        }
        let equipment = self
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        equipment
            .equipment
            .get(&EquipmentSlot::MAIN_HAND)
            .cloned()
            .unwrap_or_else(|| ItemStack::EMPTY.clone())
    }

    pub fn get_stack_in_hand(&self, caller: &dyn EntityBase, hand: Hand) -> ItemStack {
        match hand {
            Hand::Left => self.off_hand_item(caller),
            Hand::Right => self.held_item(caller),
        }
    }

    /// getOffHandStack in source
    pub fn off_hand_item(&self, caller: &dyn EntityBase) -> ItemStack {
        if let Some(player) = caller.get_player() {
            return player.inventory.off_hand_item();
        }
        let Some(slot) = self.equipment_slots.get(&PlayerInventory::OFF_HAND_SLOT) else {
            return ItemStack::EMPTY.clone();
        };
        let equipment = self
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        equipment
            .equipment
            .get(slot)
            .cloned()
            .unwrap_or_else(|| ItemStack::EMPTY.clone())
    }

    /// Forgotten after 40 ticks.
    pub fn get_last_damage_type(&self) -> Option<DamageType> {
        let stamp = self.last_damage_stamp.load(Ordering::Relaxed);
        let mut last = self
            .last_damage_type
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.entity.world.load().get_world_age() - stamp > 40 {
            *last = None;
        }
        *last
    }

    pub fn can_take_damage(&self) -> bool {
        !self.entity.is_invulnerable() && self.is_part_of_game()
    }

    /// Vanilla `LivingEntity.canBeSeenByAnyone`, whose `isAlive` is health aware, unlike
    /// `Entity::is_alive` which only means "not removed".
    pub fn is_part_of_game(&self) -> bool {
        !self.is_spectator() && self.entity.is_alive() && self.health.load() > 0.0
    }

    /// Vanilla `LivingEntity.isAlive`: not removed and with health left.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        !self.entity.is_removed() && self.health.load() > 0.0
    }

    /// Vanilla `LivingEntity.getVisibilityPercent`: how far `targeting_entity` can notice this entity.
    pub fn get_visibility_percent(&self, targeting_entity: Option<&Entity>) -> f64 {
        let mut visibility_percent = 1.0;
        if self.entity.is_sneaking() {
            visibility_percent *= 0.8;
        }

        if self.entity.invisible.load(Relaxed) {
            let cover_percentage = self.get_armor_cover_percentage().max(0.1);
            visibility_percent *= 0.7 * f64::from(cover_percentage);
        }

        if let Some(targeting_entity) = targeting_entity {
            let targeting_type = targeting_entity.entity_type;
            let equipment = self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for slot in [
                EquipmentSlot::MAIN_HAND,
                EquipmentSlot::OFF_HAND,
                EquipmentSlot::FEET,
                EquipmentSlot::LEGS,
                EquipmentSlot::CHEST,
                EquipmentSlot::HEAD,
                EquipmentSlot::BODY,
                EquipmentSlot::SADDLE,
            ] {
                let Some(item) = equipment.equipment.get(&slot) else {
                    continue;
                };
                let Some(mob_visibility) = item.get_data_component::<MobVisibilityImpl>() else {
                    continue;
                };
                let applies =
                    item.get_data_component::<EquippableImpl>()
                        .is_some_and(|equippable| {
                            *equippable.slot == slot
                                && match &mob_visibility.targeting_entity_types {
                                    IDSet::Tag(tag) => {
                                        targeting_type.is_tagged_with(tag).unwrap_or(false)
                                    }
                                    IDSet::IDs(types) => {
                                        types.iter().any(|t| t.id == targeting_type.id)
                                    }
                                }
                        });
                if applies {
                    visibility_percent *= f64::from(mob_visibility.visibility);
                }
            }
        }

        visibility_percent.clamp(0.0, 10.0)
    }

    /// Vanilla `LivingEntity.getArmorCoverPercentage`.
    pub fn get_armor_cover_percentage(&self) -> f32 {
        let equipment = self
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let slots = [
            EquipmentSlot::FEET,
            EquipmentSlot::LEGS,
            EquipmentSlot::CHEST,
            EquipmentSlot::HEAD,
        ];
        let count = slots
            .iter()
            .filter(|slot| equipment.equipment.get(slot).is_some_and(|s| !s.is_empty()))
            .count();
        count as f32 / slots.len() as f32
    }

    pub fn can_attack(&self, target: &dyn EntityBase) -> bool {
        if target.get_player().is_some()
            && self.entity.world.load().level_info.load().difficulty == Difficulty::Peaceful
        {
            return false;
        }
        can_be_seen_as_enemy(target)
    }

    pub fn reset_state(&self) {
        self.reset_fresh_entity_state();
        self.death_time.store(0, Relaxed);
        self.dead.store(false, Relaxed);

        // Restore to maximum health for this entity type
        let max_health = self.get_max_health();
        self.set_health(max_health);
        // Clear any absorption
        self.absorption.store(0.0);
        // Send health metadata
        self.entity
            .set_synced_data(tracked_data::living_entity::DATA_HEALTH_ID, max_health);

        self.reset_effects_and_attributes();

        // Give a short grace period of invulnerability after respawn
        self.hurt_cooldown.store(20, Relaxed);
        self.last_damage_taken.store(0f32);

        *self
            .entity
            .portal_manager
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;

        // If this LivingEntity corresponds to a Player, reset their hunger manager
        let world = self.entity.world.load();
        if let Some(player) = world.get_player_by_id(self.entity.entity_id) {
            player.hunger_manager.restart();
        }
    }

    /// What a respawned player (vanilla's fresh `ServerPlayer`) starts with whatever
    /// `restoreFrom` copies: standing, not fall-flying, burning or freezing, full air, no
    /// fall, speed, input, item in use, hurt time or portal cooldown. The dead state is the
    /// death path's (`reset_state`), after the respawn's awaits.
    pub fn reset_fresh_entity_state(&self) {
        self.entity.reset_state();
        // A new entity's `firstTick`: until its first tick, the respawned player's waypoint
        // makes no connection (`makeWaypointConnectionWith`), so `addRespawnedPlayer`'s
        // `onTrackingStart` doesn't show it to anyone; its first move after that tick does.
        self.entity.first_tick.store(true, Relaxed);
        self.entity.portal_cooldown.store(0, Relaxed);
        self.entity.fall_distance.store(0.0);
        self.entity.fire_ticks.store(0, Relaxed);
        self.entity.set_frozen_ticks(0);
        self.breath
            .air_supply
            .store(crate::entity::breath::MAX_AIR, Relaxed);
        self.breath.send_air_supply(&self.entity);
        self.entity.velocity.store(Vector3::default());
        self.entity.velocity_dirty.store(true, SeqCst);
        self.movement_input.store(Vector3::default());
        self.jumping.store(false, Relaxed);
        self.hurt_time.store(0, Relaxed);
        self.entity.invulnerable_time.store(0, Relaxed);
        // `stopUsingItem` state, without its game event: a fresh player was never using one.
        self.item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        *self
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.item_use_time.store(0, Relaxed);
        self.set_living_flag(Self::USING_ITEM_FLAG, false);
    }

    pub fn is_player(&self) -> bool {
        let world = self.entity.world.load();
        world.get_player_by_id(self.entity.entity_id).is_some()
    }

    pub fn get_movement(&self) -> Vector3<f64> {
        self.entity.movement.load()
    }

    fn death_sound(&self, entity: &dyn EntityBase) -> Sound {
        if let Some(sound_source) = entity.get_mob().and_then(|x| x.as_custom_sound())
            && let Some(audio) = sound_source.death_sound()
        {
            return audio;
        }

        Self::death_sound_for_entity(self.entity.entity_type)
    }

    /// Vanilla `LivingEntity.makeSound`. Mobs use their own volume, pitch and sound
    /// source; other living entities play at full volume.
    fn make_sound(&self, entity: &dyn EntityBase, sound: Sound) {
        if let Some(mob) = entity.get_mob() {
            mob.make_sound(sound);
            return;
        }
        // `getVoicePitch()` is an argument to `playSound`, so it draws before the silent check.
        let pitch = self.get_pitch();
        if self.entity.is_silent() {
            return;
        }
        let world = self.entity.world.load();
        let pos = self.entity.pos.load();
        // Vanilla `Player.playSound`: everyone but the player, whose client plays it itself.
        if let Some(player) = entity.get_player() {
            world.play_sound_raw_expect(
                player,
                sound as u16,
                SoundCategory::Players,
                &pos,
                1.0,
                pitch,
            );
        } else {
            world.play_sound_fine(sound, SoundCategory::Neutral, &pos, 1.0, pitch);
        }
    }

    fn hurt_sound(&self, entity: &dyn EntityBase) -> Sound {
        if let Some(sound_source) = entity.get_mob().and_then(|x| x.as_custom_sound())
            && let Some(audio) = sound_source.hurt_sound()
        {
            return audio;
        }

        Self::hurt_sound_for_entity(self.entity.entity_type)
    }
}

impl LivingEntity {
    /// Writes living-entity tags.
    pub fn write_living_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put("Health", NbtTag::Float(self.health.load()));
        if let Some(player) = self.last_hurt_by_player.load() {
            nbt.put_uuid("last_hurt_by_player", player);
            nbt.put_int(
                "last_hurt_by_player_memory_time",
                self.last_hurt_by_player_memory_time.load(Relaxed),
            );
        }
        if let Some(hurt_by) = self.last_hurt_by_mob_uuid.load() {
            nbt.put_uuid("last_hurt_by_mob", hurt_by);
            nbt.put_int(
                "ticks_since_last_hurt_by_mob",
                self.entity.tick_count.load(Relaxed) - self.last_attacked_time.load(Relaxed),
            );
        }
        self.write_attributes_nbt(nbt);
        // Persist current absorption amount
        nbt.put("AbsorptionAmount", NbtTag::Float(self.absorption.load()));
        nbt.put_int(
            "current_impulse_context_reset_grace_time",
            self.current_impulse_context_reset_grace_time.load(Relaxed),
        );
        if let Some(pos) = self.current_impulse_impact_pos.load() {
            nbt.put(
                "current_explosion_impact_pos",
                NbtTag::List(vec![
                    NbtTag::Double(pos.x),
                    NbtTag::Double(pos.y),
                    NbtTag::Double(pos.z),
                ]),
            );
        }
        nbt.put_short("HurtTime", self.hurt_time.load(Relaxed) as i16);
        nbt.put_short("DeathTime", i16::from(self.death_time.load(Relaxed)));
        nbt.put_bool("FallFlying", self.entity.is_fall_flying());
        if let Some(sleeping_pos) = self.get_sleeping_pos() {
            nbt.put(
                "sleeping_pos",
                NbtTag::IntArray(vec![sleeping_pos.0.x, sleeping_pos.0.y, sleeping_pos.0.z]),
            );
        }
        self.breath.write_nbt(nbt);
        {
            let effects = self
                .active_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !effects.is_empty() {
                let effects_list = effects
                    .values()
                    .map(|effect| {
                        let mut effect_nbt = NbtCompound::new();
                        effect.write_nbt(&mut effect_nbt);
                        NbtTag::Compound(effect_nbt)
                    })
                    .collect();
                nbt.put("active_effects", NbtTag::List(effects_list));
            }
        }
        let equipment = {
            let guard = self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut compound = NbtCompound::new();
            for (slot, stack) in &guard.equipment {
                if !stack.is_empty() {
                    let mut item_nbt = NbtCompound::new();
                    stack.write_item_stack(&mut item_nbt);
                    compound.put(slot.to_name(), NbtTag::Compound(item_nbt));
                }
            }
            compound
        };
        if !equipment.child_tags.is_empty() {
            nbt.put("equipment", NbtTag::Compound(equipment));
        }
        let icon = self
            .locator_bar_icon
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if icon.has_data() {
            nbt.put("locator_bar_icon", NbtTag::Compound(icon.to_nbt()));
        }
    }

    /// Vanilla `AttributeMap.pack` under `attributes`: each instance's id, base value and
    /// permanent modifiers.
    fn write_attributes_nbt(&self, nbt: &mut NbtCompound) {
        let map = self
            .attributes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut ids: Vec<u8> = map.keys().copied().collect();
        ids.sort_unstable();
        let list = ids
            .into_iter()
            .filter_map(|id| {
                let attribute = attributes_by_id(id)?;
                let instance = map.get(&id)?;
                let mut packed = NbtCompound::new();
                packed.put_string("id", attribute.name.to_string());
                packed.put("base", NbtTag::Double(instance.base_value));
                let modifiers: Vec<NbtTag> = instance
                    .permanent_modifiers()
                    .map(|modifier| {
                        let mut packed_modifier = NbtCompound::new();
                        packed_modifier.put_string("id", modifier.id.clone());
                        packed_modifier.put("amount", NbtTag::Double(modifier.amount));
                        packed_modifier.put_string(
                            "operation",
                            modifier.operation.serialized_name().to_string(),
                        );
                        NbtTag::Compound(packed_modifier)
                    })
                    .collect();
                if !modifiers.is_empty() {
                    packed.put("modifiers", NbtTag::List(modifiers));
                }
                Some(NbtTag::Compound(packed))
            })
            .collect();
        nbt.put("attributes", NbtTag::List(list));
    }

    /// Vanilla `AttributeMap.apply` from `attributes`; unknown ids are skipped.
    fn read_attributes_nbt(&self, nbt: &NbtCompound) {
        let Some(list) = nbt.get_list("attributes") else {
            return;
        };
        for packed in list.iter().filter_map(NbtTag::extract_compound) {
            let Some(id) = packed.get_string("id") else {
                continue;
            };
            let name = if id.contains(':') {
                id.to_string()
            } else {
                format!("minecraft:{id}")
            };
            let Some(attribute) = Attributes::ALL.iter().find(|attr| attr.name == name) else {
                continue;
            };
            if let Some(base) = packed.get_double("base") {
                self.set_attribute_base(attribute, base);
            }
            let Some(modifiers) = packed.get_list("modifiers") else {
                continue;
            };
            let modifiers: Vec<Modifier> = modifiers
                .iter()
                .filter_map(NbtTag::extract_compound)
                .filter_map(|modifier| {
                    Some(Modifier {
                        id: modifier.get_string("id")?.to_string(),
                        amount: modifier.get_double("amount")?,
                        operation: ModifierOperation::from_serialized_name(
                            modifier.get_string("operation")?,
                        )?,
                    })
                })
                .collect();
            self.update_attribute(attribute, |instance| {
                for modifier in modifiers {
                    instance.add_or_replace_permanent_modifier(modifier);
                }
            });
        }
    }

    pub fn read_living_nbt_non_mut(&self, nbt: &NbtCompound) {
        // `lastHurtByMob` as a reference, with its timestamp relative to the tick count.
        self.last_attacker_id.store(0, Relaxed);
        self.last_hurt_by_mob_uuid
            .store(nbt.get_uuid("last_hurt_by_mob"));
        self.last_attacked_time.store(
            self.entity.tick_count.load(Relaxed)
                - nbt.get_int("ticks_since_last_hurt_by_mob").unwrap_or(0),
            Relaxed,
        );
        // `internalSetAbsorptionAmount`: unclamped, before the attributes are read.
        self.internal_set_absorption(nbt.get_float("AbsorptionAmount").unwrap_or(0.0));
        self.set_last_hurt_by_player(
            nbt.get_uuid("last_hurt_by_player"),
            nbt.get_int("last_hurt_by_player_memory_time").unwrap_or(0),
        );
        // Vanilla reads `attributes` before `Health`, so a raised max health holds.
        self.read_attributes_nbt(nbt);
        {
            let effects: Vec<MobEffectInstance> = nbt
                .get_list("active_effects")
                .map(|list| {
                    list.iter()
                        .filter_map(NbtTag::extract_compound)
                        .filter_map(|effect| {
                            MobEffectInstance::create_from_nbt(&mut effect.clone())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut active_effects = self
                .active_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            active_effects.clear();
            for effect in effects {
                active_effects.insert(effect.effect_type, effect);
                self.effects_dirty.store(true, Relaxed);
            }
        }
        // Vanilla LivingEntity.readAdditionalSaveData defaults to the mob's own max health,
        // not a flat 20; a hoglin (40 max) or iron golem (100 max) with no saved Health would
        // otherwise be silently reset to 20 here.
        self.set_health(
            nbt.get_float("Health")
                .unwrap_or_else(|| self.get_max_health()),
        );

        if let Some(equipment) = nbt.get_compound("equipment") {
            let mut guard = self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (name, tag) in &equipment.child_tags {
                if let Some(slot) = EquipmentSlot::get_from_name(name)
                    && let Some(compound) = tag.extract_compound()
                    && let Some(stack) = ItemStack::read_item_stack(compound)
                {
                    guard.put(slot, stack);
                }
            }
        }

        self.current_impulse_context_reset_grace_time.store(
            nbt.get_int("current_impulse_context_reset_grace_time")
                .unwrap_or(0),
            Relaxed,
        );
        self.current_impulse_impact_pos.store(
            nbt.get_list("current_explosion_impact_pos")
                .and_then(|list| match list {
                    [NbtTag::Double(x), NbtTag::Double(y), NbtTag::Double(z)] => {
                        Some(Vector3::new(*x, *y, *z))
                    }
                    _ => None,
                }),
        );
        self.hurt_time
            .store(i32::from(nbt.get_short("HurtTime").unwrap_or(0)), Relaxed);
        if let Some(death_time) = nbt.get_short("DeathTime") {
            self.death_time.store(death_time as u8, Relaxed);
        }
        self.entity
            .fall_flying
            .store(nbt.get_bool("FallFlying").unwrap_or(false), Relaxed);
        // `sleeping_pos`: lie down at once, and move to the bed now unless the first tick
        // will (`baseTick`).
        if let Some(NbtTag::IntArray(bed)) = nbt.get("sleeping_pos")
            && let &[x, y, z] = bed.as_slice()
        {
            let bed = BlockPos::new(x, y, z);
            self.set_sleeping_pos(bed);
            // `entityData.set(DATA_POSE, SLEEPING)`, which refreshes the dimensions.
            self.entity.set_pose_with_dimensions(
                EntityPose::Sleeping,
                self.entity.get_dimensions(EntityPose::Sleeping),
            );
            if !self.entity.first_tick.load(Relaxed)
                && !self.set_pos_to_bed(bed)
                && let Some(caller) = self
                    .entity
                    .world
                    .load()
                    .get_entity_by_id(self.entity.entity_id)
            {
                self.stop_sleeping(caller.as_ref());
            }
        } else {
            self.clear_sleeping_pos();
        }
        self.read_locator_bar_icon(nbt);
        // todo more...
    }

    /// Vanilla `locatorBarIcon = input.read("locator_bar_icon", Icon.CODEC)
    /// .orElseGet(Icon::new)`.
    fn read_locator_bar_icon(&self, nbt: &NbtCompound) {
        *self
            .locator_bar_icon
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = nbt
            .get_compound("locator_bar_icon")
            .and_then(crate::entity::waypoint::WaypointIconState::from_nbt)
            .unwrap_or_default();
    }

    /// Vanilla `LivingEntity.getDamageAfterArmorAbsorb`.
    pub fn get_damage_after_armor_absorb(
        &self,
        caller: &dyn EntityBase,
        damage: f32,
        damage_source: &DamageSource<'_>,
    ) -> f32 {
        if damage_source
            .damage_type
            .has_tag(&tag::DamageType::MINECRAFT_BYPASSES_ARMOR)
        {
            return damage;
        }
        self.hurt_armor(caller, damage_source.damage_type, damage);

        // Vanilla `getArmorValue` (the floored ARMOR attribute, worn armor included through
        // its attribute modifiers) and the ARMOR_TOUGHNESS attribute.
        let armor = self.get_attribute_value(&Attributes::ARMOR).floor() as f32;
        let toughness = self.get_attribute_value(&Attributes::ARMOR_TOUGHNESS) as f32;
        CombatRules::get_damage_after_absorb(caller, damage, damage_source, armor, toughness)
    }

    /// Calculates damage after magic/resistance/enchantment reduction, mirroring vanilla `LivingEntity.getDamageAfterMagicAbsorb`.
    pub fn get_damage_after_magic_absorb(
        &self,
        mut damage: f32,
        caller: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
    ) -> f32 {
        let damage_type = &damage_source.damage_type;
        let cause = damage_source.causing;
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_EFFECTS) {
            return damage;
        }

        // 1. Resistance Effect (evaluated before enchantments)
        if !damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_RESISTANCE)
            && let Some(effect) = self.get_effect(&StatusEffect::RESISTANCE)
        {
            let absorb_value = (effect.amplifier + 1) * 5;
            let absorb = 25 - absorb_value;
            let v = damage * absorb as f32;
            let old_damage = damage;
            damage = (v / 25.0).max(0.0);
            let damage_resisted = old_damage - damage;
            if damage_resisted > 0.0 {
                if let Some(victim_player) = caller.get_player() {
                    victim_player.increment_stat(
                        StatisticCategory::Custom,
                        CustomStatistic::DamageResisted as i32,
                        (damage_resisted * 10.0).round() as i32,
                    );
                } else if let Some(attacker_player) = cause.and_then(|c| c.get_player()) {
                    attacker_player.increment_stat(
                        StatisticCategory::Custom,
                        CustomStatistic::DamageDealtResisted as i32,
                        (damage_resisted * 10.0).round() as i32,
                    );
                }
            }
        }

        if damage <= 0.0 {
            return 0.0;
        }

        // 2. Enchantment Protection
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_ENCHANTMENTS) {
            return damage;
        }

        let epf = EnchantmentHelper::get_damage_protection(
            &self.entity.world.load(),
            caller,
            self,
            damage_source,
        );

        if epf > 0.0 {
            damage = CombatRules::get_damage_after_magic_absorb(damage, epf);
        }

        damage
    }

    /// Vanilla `LivingEntity.hurtServer`.
    #[expect(clippy::too_many_lines)]
    pub fn damage_with_context(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        let mut amount = amount;
        let damage_source = DamageSource {
            damage_type,
            direct: source,
            causing: cause,
        };

        if self.is_invulnerable_to(caller, &damage_source) {
            return false;
        }

        if self.is_dead_or_dying() {
            return false;
        }

        let mut damage_event =
            crate::plugin::api::events::entity::entity_damage::EntityDamageEvent::new(
                self.entity.entity_id,
                damage_type,
                amount,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut damage_event);
        }
        if damage_event.cancelled {
            return false;
        }
        amount = damage_event.damage;

        if let Some(damager) = source.or(cause) {
            let mut by_entity_event =
                crate::plugin::api::events::entity::entity_damage_by_entity::EntityDamageByEntityEvent {
                    entity_id: self.entity.entity_id,
                    damager_id: damager.get_entity().entity_id,
                    damage: amount,
                    cause: format!("{damage_type:?}"),
                    cancelled: false,
                };
            if let Some(server) = self.entity.world.load().server.upgrade() {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut by_entity_event);
            }
            if by_entity_event.cancelled {
                return false;
            }
            amount = by_entity_event.damage;
        } else if position.is_some()
            || matches!(
                damage_type,
                DamageType::CACTUS
                    | DamageType::SWEET_BERRY_BUSH
                    | DamageType::CAMPFIRE
                    | DamageType::HOT_FLOOR
                    | DamageType::STALAGMITE
            )
        {
            let damager_pos = position.map(|p| {
                BlockPos(Vector3::new(
                    p.x.floor() as i32,
                    p.y.floor() as i32,
                    p.z.floor() as i32,
                ))
            });
            let mut by_block_event =
                crate::plugin::api::events::entity::entity_damage_by_block::EntityDamageByBlockEvent {
                    entity_id: self.entity.entity_id,
                    damager_pos,
                    damage: amount,
                    cause: format!("{damage_type:?}"),
                    cancelled: false,
                };
            if let Some(server) = self.entity.world.load().server.upgrade() {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut by_block_event);
            }
            if by_block_event.cancelled {
                return false;
            }
            amount = by_block_event.damage;
        }

        let world = self.entity.world.load();
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FIRE)
            && self.has_effect(&StatusEffect::FIRE_RESISTANCE)
        {
            return false;
        }

        if Self::is_sleeping(caller) {
            self.stop_sleeping(caller);
        }

        self.no_action_time.store(0, Relaxed);
        if amount < 0.0 {
            amount = 0.0;
        }

        let item_in_use = self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let damage_blocked = self.apply_item_blocking(caller, &damage_source, amount, position);
        amount -= damage_blocked;
        let blocked = damage_blocked > 0.0;

        if damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FREEZING)
            && self
                .entity
                .entity_type
                .has_tag(&tag::EntityType::MINECRAFT_FREEZE_HURTS_EXTRA_TYPES)
        {
            amount *= 5.0;
        }

        if damage_type.has_tag(&tag::DamageType::MINECRAFT_DAMAGES_HELMET)
            && !self
                .get_item_by_slot(caller, &EquipmentSlot::HEAD)
                .is_empty()
        {
            self.hurt_helmet(caller, damage_type, amount);
            amount *= 0.75;
        }

        if amount.is_nan() || amount.is_infinite() {
            amount = f32::MAX;
        }

        // `damageCooldownTime > 10`: only the part above the last hit lands.
        let took_full_damage = if self.hurt_cooldown.load(Relaxed) > 10
            && !damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_COOLDOWN)
        {
            let last_hurt = self.last_damage_taken.load();
            if amount <= last_hurt {
                return false;
            }
            self.actually_hurt(caller, &damage_source, amount - last_hurt);
            self.last_damage_taken.store(amount);
            false
        } else {
            self.last_damage_taken.store(amount);
            self.hurt_cooldown.store(20, Relaxed);
            self.actually_hurt(caller, &damage_source, amount);
            self.hurt_time.store(HURT_DURATION, Relaxed);
            true
        };

        self.resolve_mob_responsible_for_damage(damage_type, cause.or(source));
        self.resolve_player_responsible_for_damage(cause.or(source));

        let Some(server) = world.server.upgrade() else {
            return false;
        };
        let config = &server.advanced_config.pvp;

        if took_full_damage {
            let blocks_attacks = item_in_use
                .as_ref()
                .and_then(|item| item.get_data_component::<BlocksAttacksImpl>().cloned());
            if blocked && let Some(blocks_attacks) = blocks_attacks {
                // `BlocksAttacks.onBlocked`: the pitch is drawn from the level random.
                if let Some(sound) = &blocks_attacks.block_sound {
                    let pitch = 0.8
                        + world
                            .random
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .next_f32()
                            * 0.4;
                    world.play_sound_event_fine(
                        sound,
                        self.sound_source(),
                        &self.entity.pos.load(),
                        1.0,
                        pitch,
                    );
                }
            } else {
                world.broadcast_damage_event(
                    &self.entity,
                    i32::from(damage_type.id),
                    source.map(|e| e.get_entity().entity_id),
                    cause.map(|e| e.get_entity().entity_id),
                    position,
                );
                // Bedrock clients animate the hurt from the actor event instead.
                if config.hurt_animation {
                    let hurt_event = SActorEvent {
                        target_runtime_id: VarULong(self.entity.entity_id as u64),
                        event_id: ActorEventID::Hurt,
                        data: VarInt(0),
                        fire_at_position: None,
                    };
                    world.send_to_tracking_players_bedrock(&self.entity, &hurt_event);
                    if let Some(player) = caller.get_player()
                        && let crate::net::ClientPlatform::Bedrock(client) = player.client.as_ref()
                        && let Ok(data) = client.serialize_packet(&hurt_event)
                    {
                        client.try_enqueue_packet(data);
                    }
                }
            }

            // `markHurt` (the velocity also goes to the hurt entity's client).
            if !damage_type.has_tag(&tag::DamageType::MINECRAFT_NO_IMPACT)
                && (!blocked || amount > 0.0)
            {
                self.entity.sync_velocity.store(true, SeqCst);
            }

            let fully_blocked = blocked && amount <= 0.0;
            if !damage_type.has_tag(&tag::DamageType::MINECRAFT_NO_KNOCKBACK) && !fully_blocked {
                // `dealDefaultKnockback`.
                let (dx, dz) = self.default_knockback_direction(source, position);
                if caller
                    .get_mob()
                    .is_none_or(super::mob::Mob::takes_knockback)
                {
                    // `knockback(0.4F, ...)`: the float strength, widened.
                    self.knockback(caller, f64::from(0.4f32), dx, dz, cause, amount, false);
                }
                // `ServerPlayer.indicateDamage`, only for a hit that wasn't blocked.
                if !blocked
                    && config.hurt_animation
                    && let Some(player) = caller.get_player()
                {
                    let hurt_dir =
                        (dz.atan2(dx) * f64::from(180.0f32) / f64::from(std::f32::consts::PI)
                            - f64::from(self.entity.yaw.load())) as f32;
                    player.try_send_client_packet(&CHurtAnimation::new(
                        self.entity.entity_id.into(),
                        hurt_dir,
                    ));
                }
            }
        }

        if self.is_dead_or_dying() {
            if !self.check_totem_death_protection(caller, damage_type) {
                if took_full_damage {
                    self.make_sound(caller, self.death_sound(caller));
                    self.play_secondary_hurt_sound(damage_type);
                }
                let mut death_event =
                    crate::plugin::api::events::entity::entity_death::EntityDeathEvent::new(
                        self.entity.entity_id,
                        0,
                    );
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut death_event);
                self.die(damage_type, source, cause);
            }
        } else if took_full_damage {
            // `playHurtSound`; `Mob.playHurtSound` also resets the ambient sound timer.
            if let Some(mob) = caller.get_mob() {
                mob.reset_ambient_sound_time();
            }
            self.make_sound(caller, self.hurt_sound(caller));
            self.play_secondary_hurt_sound(damage_type);
        }

        let success = !blocked || amount > 0.0;
        if success {
            *self
                .last_damage_type
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(damage_type);
            self.last_damage_stamp.store(world.get_world_age(), Relaxed);

            let effects: Vec<(&'static StatusEffect, u8)> = self
                .active_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
                .map(|effect| (effect.effect_type, effect.amplifier))
                .collect();
            for (effect_type, amplifier) in effects {
                if let Some(mob_effect) = crate::entity::effect::get_mob_effect(effect_type) {
                    mob_effect.on_mob_hurt(self, amplifier, &damage_type, amount);
                }
            }
        }

        if let Some(player) = caller.get_player()
            && damage_blocked > 0.0
            && damage_blocked < 3.402_823_5E37
        {
            player.increment_stat(
                StatisticCategory::Custom,
                CustomStatistic::DamageBlockedByShield as i32,
                (damage_blocked * 10.0).round() as i32,
            );
        }

        success
    }

    /// Vanilla `LivingEntity.isInvulnerableTo`: the entity check, then enchantment immunities.
    fn is_invulnerable_to(
        &self,
        caller: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
    ) -> bool {
        let damage_type = damage_source.damage_type;
        if self
            .entity
            .is_invulnerable_to(&damage_type, damage_source.causing)
            || EnchantmentHelper::is_immune_to_damage(
                &self.entity.world.load(),
                caller,
                self,
                damage_source,
            )
        {
            return true;
        }
        // `Player.isInvulnerableTo`: the damage game rules.
        if self.is_player() {
            let rules = &self.entity.world.load().level_info.load().game_rules;
            if damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_DROWNING)
                && !rules.drowning_damage
                || damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FALL) && !rules.fall_damage
                || damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FIRE) && !rules.fire_damage
                || damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FREEZING)
                    && !rules.freeze_damage
            {
                return true;
            }
        }
        false
    }

    /// Vanilla `isDeadOrDying`.
    #[must_use]
    pub fn is_dead_or_dying(&self) -> bool {
        self.health.load() <= 0.0 || self.dead.load(Relaxed)
    }

    /// Vanilla `LivingEntity.canUsePortal`: `Entity`'s, with this class's `isAlive`, and
    /// not while sleeping.
    pub fn living_can_use_portal(&self, caller: &dyn EntityBase, ignore_passenger: bool) -> bool {
        (ignore_passenger || !self.entity.has_vehicle())
            && self.is_alive()
            && !Self::is_sleeping(caller)
    }

    /// Vanilla `LivingEntity.isAffectedByPotions`: not dead or dying; never an armor stand
    /// (`ArmorStand`'s override). `caller` is this entity.
    #[must_use]
    pub fn is_affected_by_potions(&self, caller: &dyn EntityBase) -> bool {
        caller.get_entity().entity_type != &pumpkin_data::entity::EntityType::ARMOR_STAND
            && !self.is_dead_or_dying()
    }

    /// Vanilla `LivingEntity.isSleeping`: a sleeping position, or a mob's own override
    /// (`Fox.isSleeping`, its sleeping flag). `caller` is the entity.
    #[must_use]
    pub fn is_sleeping(caller: &dyn EntityBase) -> bool {
        if let Some(mob) = caller.get_mob() {
            return mob.is_sleeping();
        }
        if let Some(player) = caller.get_player() {
            return player.is_sleeping();
        }
        caller
            .get_living_entity()
            .is_some_and(|living| living.get_sleeping_pos().is_some())
    }

    /// Vanilla `LivingEntity.getSleepingPos`.
    #[must_use]
    pub fn get_sleeping_pos(&self) -> Option<BlockPos> {
        self.sleeping_pos.load()
    }

    /// Vanilla `LivingEntity.setSleepingPos`.
    pub fn set_sleeping_pos(&self, pos: BlockPos) {
        self.sleeping_pos.store(Some(pos));
        self.entity
            .set_synced_data(tracked_data::living_entity::SLEEPING_POS_ID, Some(pos));
    }

    /// Vanilla `LivingEntity.clearSleepingPos`.
    pub fn clear_sleeping_pos(&self) {
        self.sleeping_pos.store(None);
        self.entity.set_synced_data(
            tracked_data::living_entity::SLEEPING_POS_ID,
            None::<BlockPos>,
        );
    }

    /// Vanilla `LivingEntity.startSleeping`: lies down in the bed at `bed` (its head).
    /// `caller` is this entity.
    pub fn start_sleeping(&self, caller: &dyn EntityBase, bed: BlockPos) -> bool {
        let world = self.entity.world.load_full();
        let state = world.get_block_state(&bed);
        let block = Block::from_state_id(state.id);
        if !crate::block::blocks::bed::is_abstract_bed(block) {
            return false;
        }
        let Some(sleep_height) = crate::block::blocks::bed::get_sleep_height(&world, bed, state)
        else {
            return false;
        };
        // `if (isPassenger()) stopRiding()`, which nothing can cancel.
        if let Some(vehicle) = self.entity.get_vehicle() {
            vehicle
                .get_entity()
                .remove_passenger_on_removal(self.entity.entity_id);
        }
        self.set_pos_to_bed_at(sleep_height, bed);
        // `setBlockAndUpdate(bed, OCCUPIED=true)`; the foot follows through `updateShape`.
        crate::block::blocks::bed::BedBlock::set_occupied(true, &world, block, &bed, state.id);
        caller.set_pose(EntityPose::Sleeping);
        self.set_sleeping_pos(bed);
        self.entity.velocity.store(Vector3::default());
        self.entity.velocity_dirty.store(true, SeqCst);
        true
    }

    /// Vanilla `LivingEntity.setPosToBed(BlockPos)`.
    fn set_pos_to_bed(&self, bed: BlockPos) -> bool {
        let world = self.entity.world.load();
        let state = world.get_block_state(&bed);
        if !crate::block::blocks::bed::is_abstract_bed(Block::from_state_id(state.id)) {
            return false;
        }
        let Some(sleep_height) = crate::block::blocks::bed::get_sleep_height(&world, bed, state)
        else {
            return false;
        };
        self.set_pos_to_bed_at(sleep_height, bed);
        true
    }

    /// Vanilla `LivingEntity.setPosToBed(double, BlockPos)`.
    fn set_pos_to_bed_at(&self, sleep_height: f64, bed: BlockPos) {
        self.entity.set_pos(Vector3::new(
            f64::from(bed.0.x) + 0.5,
            f64::from(bed.0.y) + sleep_height + 0.125,
            f64::from(bed.0.z) + 0.5,
        ));
    }

    /// Vanilla `LivingEntity.checkBedExists`.
    fn check_bed_exists(&self) -> bool {
        self.get_sleeping_pos().is_some_and(|bed| {
            let world = self.entity.world.load();
            let state = world.get_block_state(&bed);
            crate::block::blocks::bed::is_abstract_bed(Block::from_state_id(state.id))
                && crate::block::blocks::bed::get_sleep_height(&world, bed, state).is_some()
        })
    }

    /// Vanilla `stopSleeping` with its overrides: a player through
    /// `Player.stopSleepInBed(true, true)`. `caller` is this entity.
    pub fn stop_sleeping(&self, caller: &dyn EntityBase) {
        if let Some(player) = caller.get_player() {
            player.stop_sleep_in_bed(true, true);
            return;
        }
        self.living_stop_sleeping(caller);
        if let Some(mob) = caller.get_mob() {
            mob.after_stop_sleeping();
        }
    }

    /// Vanilla `LivingEntity.stopSleeping`: frees the bed and stands up beside it.
    pub(crate) fn living_stop_sleeping(&self, caller: &dyn EntityBase) {
        let world = self.entity.world.load_full();
        if let Some(bed) = self.get_sleeping_pos()
            && world.is_loaded(&bed)
        {
            let state = world.get_block_state(&bed);
            let block = Block::from_state_id(state.id);
            if crate::block::blocks::bed::is_abstract_bed(block) {
                let facing =
                    pumpkin_data::block_properties::WhiteBedLikeProperties::from_state_id(state.id)
                        .facing;
                crate::block::blocks::bed::BedBlock::set_occupied(
                    false, &world, block, &bed, state.id,
                );
                crate::block::blocks::bed::on_stop_sleeping(&world, bed, block);
                let stand_up = crate::block::blocks::bed::find_stand_up_position(
                    self.entity.entity_type,
                    &world,
                    bed,
                    facing,
                    self.entity.yaw.load(),
                )
                .unwrap_or_else(|| {
                    Vector3::new(
                        f64::from(bed.0.x) + 0.5,
                        f64::from(bed.0.y + 1) + 0.1,
                        f64::from(bed.0.z) + 0.5,
                    )
                });
                // `Vec3.atBottomCenterOf(bed).subtract(standUp).normalize()`.
                let look = Vector3::new(
                    f64::from(bed.0.x) + 0.5 - stand_up.x,
                    f64::from(bed.0.y) - stand_up.y,
                    f64::from(bed.0.z) + 0.5 - stand_up.z,
                );
                let length = (look.x * look.x + look.y * look.y + look.z * look.z).sqrt();
                let look = if length < f64::from(1.0E-5f32) {
                    Vector3::default()
                } else {
                    Vector3::new(look.x / length, look.y / length, look.z / length)
                };
                // `(float)Mth.wrapDegrees(Mth.atan2(z, x) * 180.0F / (float)Math.PI - 90.0)`.
                let yaw = pumpkin_util::math::wrap_degrees_f64(
                    pumpkin_util::math::atan2(look.z, look.x) * 180.0
                        / f64::from(std::f32::consts::PI)
                        - 90.0,
                ) as f32;
                self.entity.set_pos(stand_up);
                self.entity.yaw.store(yaw);
                self.entity.set_pitch(0.0);
            }
        }
        let pos = self.entity.pos.load();
        caller.set_pose(EntityPose::Standing);
        self.entity.set_pos(pos);
        self.clear_sleeping_pos();
    }

    /// Vanilla `LivingEntity.actuallyHurt`, with `Player`'s override.
    fn actually_hurt(&self, caller: &dyn EntityBase, damage_source: &DamageSource<'_>, dmg: f32) {
        if self.is_invulnerable_to(caller, damage_source) {
            return;
        }
        let damage_type = damage_source.damage_type;
        // A mob's `actuallyHurt` override may take the whole hit, as wolf armour does.
        if !caller
            .get_mob()
            .is_some_and(|mob| mob.absorb_hurt(caller, damage_type, dmg))
        {
            let dmg = self.get_damage_after_armor_absorb(caller, dmg, damage_source);
            let dmg = self.get_damage_after_magic_absorb(dmg, caller, damage_source);
            let original_damage = dmg;
            let dmg = (dmg - self.get_absorption()).max(0.0);
            self.set_absorption(self.get_absorption() - (original_damage - dmg));
            let absorbed_damage = original_damage - dmg;
            let player = caller.get_player();
            if absorbed_damage > 0.0 && absorbed_damage < 3.402_823_5E37 {
                if let Some(player) = player {
                    player.increment_stat(
                        StatisticCategory::Custom,
                        CustomStatistic::DamageAbsorbed as i32,
                        (absorbed_damage * 10.0).round() as i32,
                    );
                } else if let Some(attacker) = damage_source.causing.and_then(|c| c.get_player()) {
                    attacker.increment_stat(
                        StatisticCategory::Custom,
                        CustomStatistic::DamageDealtAbsorbed as i32,
                        (absorbed_damage * 10.0).round() as i32,
                    );
                }
            }

            if dmg != 0.0 {
                if let Some(player) = player {
                    player.add_exhaustion(damage_type.exhaustion);
                }
                self.record_combat_damage(damage_source, dmg);
                self.set_health(self.health.load() - dmg);
                if let Some(player) = player {
                    if dmg < 3.402_823_5E37 {
                        player.increment_stat(
                            StatisticCategory::Custom,
                            CustomStatistic::DamageTaken as i32,
                            (dmg * 10.0).round() as i32,
                        );
                    }
                } else {
                    self.set_absorption(self.get_absorption() - dmg);
                }
                self.entity.world.load().game_event_from(
                    Some(caller),
                    pumpkin_data::game_event::GameEvent::EntityDamage,
                    self.entity.pos.load(),
                );
            }
        }

        if let Some(mob) = caller.get_mob() {
            mob.actually_hurt(damage_type, damage_source.direct, damage_source.causing);
        }
    }

    /// Vanilla `CombatTracker.recordDamage`.
    fn record_combat_damage(&self, damage_source: &DamageSource<'_>, damage: f32) {
        let world = self.entity.world.load();
        let fall_location = FallLocation::get_current_fall_location(self, &world);
        let fall_distance = self.entity.fall_distance.load();
        // `recordDamage` starts with `recheckStatus`, which may leave combat.
        self.recheck_combat_status();
        let entered_combat = self
            .combat_tracker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record_damage(
                i64::from(self.entity.tick_count.load(Relaxed)),
                self.is_alive(),
                fall_distance,
                fall_location,
                damage_source.damage_type,
                damage,
                damage_source.direct,
                damage_source.causing,
            );
        // `ServerPlayer.onEnterCombat`.
        if entered_combat && let Some(player) = self.self_player() {
            player.try_send_client_packet(&pumpkin_protocol::java::client::play::CCombatEnter);
        }
    }

    /// Vanilla `CombatTracker.recheckStatus`, with `ServerPlayer.onLeaveCombat`.
    fn recheck_combat_status(&self) {
        let tick_count = i64::from(self.entity.tick_count.load(Relaxed));
        let mut tracker = self
            .combat_tracker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if tracker.recheck_status(tick_count, self.is_alive()) {
            let duration = tracker.get_combat_duration(tick_count);
            drop(tracker);
            if let Some(player) = self.self_player() {
                player.try_send_client_packet(
                    &pumpkin_protocol::java::client::play::CCombatEnd::new(VarInt(duration as i32)),
                );
            }
        }
    }

    /// Vanilla `playSecondaryHurtSound`: thorns sting.
    fn play_secondary_hurt_sound(&self, damage_type: DamageType) {
        if damage_type == DamageType::THORNS {
            let category = if self.is_player() {
                SoundCategory::Players
            } else {
                SoundCategory::Hostile
            };
            self.entity.world.load().play_sound(
                Sound::EnchantThornsHit,
                category,
                &self.entity.pos.load(),
            );
        }
    }

    /// Vanilla `getItemBlockingWith`: the used item once its block delay has passed.
    pub fn get_item_blocking_with(&self) -> Option<ItemStack> {
        let item_in_use = self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()?;
        let blocks_attacks = item_in_use.get_data_component::<BlocksAttacksImpl>()?;
        // Bedrock clients raise the shield without the delay.
        let block_delay_ticks = if self.self_player().is_some_and(|player| {
            matches!(
                player.client.as_ref(),
                crate::net::ClientPlatform::Bedrock(_)
            )
        }) {
            0
        } else {
            // `blockDelayTicks`.
            (blocks_attacks.block_delay_seconds * 20.0).round() as i32
        };
        let elapsed_ticks = item_in_use.get_max_use_time() - self.item_use_time.load(Relaxed);
        (elapsed_ticks >= block_delay_ticks).then_some(item_in_use)
    }

    /// Vanilla `isBlocking`.
    pub fn is_blocking(&self) -> bool {
        self.get_item_blocking_with().is_some()
    }

    /// Vanilla `applyItemBlocking`: how much of `damage` the raised item takes.
    fn apply_item_blocking(
        &self,
        caller: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
        damage: f32,
        position: Option<Vector3<f64>>,
    ) -> f32 {
        if damage <= 0.0 {
            return 0.0;
        }
        let Some(blocking_with) = self.get_item_blocking_with() else {
            return 0.0;
        };
        let Some(blocks_attacks) = blocking_with
            .get_data_component::<BlocksAttacksImpl>()
            .cloned()
        else {
            return 0.0;
        };
        let damage_type = damage_source.damage_type;
        if blocks_attacks
            .bypassed_by
            .as_ref()
            .is_some_and(|set| crate::entity::combat::damage_type_set_contains(set, &damage_type))
        {
            return 0.0;
        }
        // A piercing arrow goes through.
        if damage_source.direct.is_some_and(|direct| {
            direct
                .cast_any()
                .downcast_ref::<crate::entity::projectile::arrow::ArrowEntity>()
                .is_some_and(|arrow| arrow.pierce_level.load(Relaxed) > 0)
        }) {
            return 0.0;
        }
        // `DamageSource.getSourcePosition`.
        let source_position = position.or_else(|| {
            damage_source
                .direct
                .map(|direct| direct.get_entity().pos.load())
        });
        let angle = source_position.map_or(f64::from(std::f32::consts::PI), |source_position| {
            let view = Vector3::rotation_vector(0.0, f64::from(self.entity.head_yaw.load()));
            let mut to = source_position - self.entity.pos.load();
            to.y = 0.0;
            to.normalize().dot(&view).acos()
        });
        let damage_blocked = crate::entity::combat::resolve_blocked_damage(
            &blocks_attacks,
            &damage_type,
            damage,
            angle,
        );
        // `BlocksAttacks.hurtBlockingItem`: only a player's item wears.
        if let Some(player) = caller.get_player() {
            player.increment_stat(StatisticCategory::Used, blocking_with.item.id as i32, 1);
            let item_damage = crate::entity::combat::blocking_item_damage(
                &blocks_attacks.item_damage,
                damage_blocked,
            );
            if item_damage > 0 {
                let hand = self.used_item_hand();
                let slot = match hand {
                    Hand::Right => EquipmentSlot::MAIN_HAND,
                    Hand::Left => EquipmentSlot::OFF_HAND,
                };
                self.hurt_and_break_in_slot(caller, &slot, item_damage);
            }
        }
        if damage_blocked > 0.0
            && !damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_PROJECTILE)
            && let Some(attacker) = damage_source.direct
            && attacker.get_living_entity().is_some()
        {
            let fully_blocked = damage_blocked >= damage;
            if let Some(player) = caller.get_player() {
                player.block_using_item(attacker, damage_type, damage, fully_blocked);
            } else {
                Self::block_using_item(caller, attacker, damage_type, damage, fully_blocked);
            }
        }
        damage_blocked
    }

    /// Vanilla `LivingEntity.blockUsingItem`: the attacker's `blockedByItem`, with the mob
    /// overrides.
    pub(crate) fn block_using_item(
        caller: &dyn EntityBase,
        attacker: &dyn EntityBase,
        damage_type: DamageType,
        damage: f32,
        fully_blocked: bool,
    ) {
        if let Some(mob) = attacker.get_mob() {
            mob.blocked_by_item(caller, damage_type, damage, fully_blocked);
        } else if let Some(attacker) = attacker.get_living_entity() {
            attacker.blocked_by_item(caller, damage, fully_blocked);
        }
    }

    /// Vanilla `LivingEntity.blockedByItem`: `defender` blocked this entity's attack with an
    /// item, and a partial block knocks it back.
    pub fn blocked_by_item(&self, defender: &dyn EntityBase, damage: f32, fully_blocked: bool) {
        if fully_blocked {
            return;
        }
        let Some(defender_living) = defender.get_living_entity() else {
            return;
        };
        let pos = self.entity.pos.load();
        let defender_pos = defender.get_entity().pos.load();
        defender_living.knockback(
            defender,
            0.5,
            defender_pos.x - pos.x,
            defender_pos.z - pos.z,
            Some(self),
            damage,
            false,
        );
    }

    /// Vanilla `getSecondsToDisableBlocking`, with `Warden`'s override.
    pub fn get_seconds_to_disable_blocking(&self, caller: &dyn EntityBase) -> f32 {
        if let Some(seconds) = caller
            .get_mob()
            .and_then(super::mob::Mob::seconds_to_disable_blocking_override)
        {
            return seconds;
        }
        // `weaponItem == getActiveItem()`: the active item is the used hand's stack, else the
        // main hand's; a spectator's is empty.
        if caller.is_spectator() {
            return 0.0;
        }
        let active_hand = self.used_item_hand();
        // `Player.getWeaponItem`: the riptide stack while spinning.
        let weapon = if let Some(player) = caller.get_player()
            && let Some((spin_stack, spin_slot)) = self.auto_spin_attack_item()
        {
            let active_slot = player.inventory.slot_for_hand(active_hand);
            let active_stack = player.inventory.get_slot(active_slot);
            if !spin_stack_is_active(&spin_stack, spin_slot, &active_stack, active_slot) {
                return 0.0;
            }
            spin_stack
        } else {
            if active_hand != Hand::Right {
                return 0.0;
            }
            self.held_item(caller)
        };
        weapon
            .get_data_component::<WeaponImpl>()
            .map_or(0.0, |weapon| weapon.disable_blocking_for_seconds)
    }

    /// Vanilla `BlocksAttacks.disable`: `caller` stops blocking with `blocking_with` for the
    /// attacker's seconds scaled by the item's `disable_cooldown_scale`.
    pub(crate) fn disable_blocking(
        &self,
        caller: &dyn EntityBase,
        blocks_attacks: &BlocksAttacksImpl,
        base_seconds: f32,
        blocking_with: &ItemStack,
    ) {
        // `disableBlockingForTicks`.
        let seconds = base_seconds * blocks_attacks.disable_cooldown_scale;
        let cooldown_ticks = if seconds > 0.0 {
            (seconds * 20.0).round() as i32
        } else {
            0
        };
        if cooldown_ticks <= 0 {
            return;
        }
        if let Some(player) = caller.get_player() {
            player.start_cooldown(cooldown_group(blocking_with), cooldown_ticks);
        }
        self.clear_active_hand();
        if let Some(sound) = &blocks_attacks.disabled_sound {
            let world = self.entity.world.load();
            let pitch = 0.8
                + world
                    .random
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .next_f32()
                    * 0.4;
            world.play_sound_event_fine(
                sound,
                self.sound_source(),
                &self.entity.pos.load(),
                0.8,
                pitch,
            );
        }
    }

    /// Vanilla `checkTotemDeathProtection`: a death protection item in either hand cancels
    /// the death.
    fn check_totem_death_protection(
        &self,
        caller: &dyn EntityBase,
        damage_type: DamageType,
    ) -> bool {
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_INVULNERABILITY) {
            return false;
        }
        // `InteractionHand.values()`: the main hand first.
        let mut found = None;
        for hand in [Hand::Right, Hand::Left] {
            let stack = self.get_stack_in_hand(caller, hand);
            if let Some(protection) = stack.get_data_component::<DeathProtectionImpl>().cloned() {
                found = Some((hand, stack, protection));
                break;
            }
        }
        let Some((hand, stack, protection)) = found else {
            return false;
        };

        let mut resurrect_event =
            crate::plugin::api::events::entity::entity_resurrect::EntityResurrectEvent::new(
                self.entity.entity_id,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut resurrect_event);
        }
        if resurrect_event.cancelled {
            return false;
        }

        let protection_item = stack.clone();
        let mut shrunk = stack;
        shrunk.decrement(1);
        self.set_stack_in_hand(caller, hand, shrunk);

        if let Some(player) = caller.get_player() {
            player.increment_stat(StatisticCategory::Used, protection_item.item.id as i32, 1);
            player.trigger_advancement(AdvancementTrigger::UsedTotem {
                item: protection_item.item,
            });
            // `causeUseVibration(this, ITEM_INTERACT_FINISH)`.
            if protection_item
                .get_data_component::<pumpkin_data::data_component_impl::UseEffectsImpl>()
                .is_some_and(|use_effects| use_effects.interact_vibrations)
            {
                self.entity.world.load().game_event_from(
                    Some(caller),
                    GameEvent::ItemInteractFinish,
                    self.entity.pos.load(),
                );
            }
        }

        self.set_health(1.0);
        // `DeathProtection.applyEffects`.
        self.apply_consume_effects(caller, &protection.death_effects);
        self.entity.world.load().send_entity_status(
            &self.entity,
            EntityStatus::ProtectedFromDeath,
            Some(ActorEventID::InstantDeath),
        );
        true
    }

    /// Vanilla `setItemInHand`.
    fn set_stack_in_hand(&self, caller: &dyn EntityBase, hand: Hand, stack: ItemStack) {
        if let Some(player) = caller.get_player() {
            player.inventory.set_stack_in_hand(hand, stack);
        } else {
            let slot = match hand {
                Hand::Right => EquipmentSlot::MAIN_HAND,
                Hand::Left => EquipmentSlot::OFF_HAND,
            };
            self.entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .put(&slot, stack);
        }
    }

    pub fn damage(&self, caller: &dyn EntityBase, amount: f32, damage_type: DamageType) -> bool {
        self.damage_with_context(caller, amount, damage_type, None, None, None)
    }

    /// Vanilla `LivingEntity.getLastHurtByMob`, which is gone once its entity is.
    pub fn get_last_hurt_by_mob(&self) -> Option<Arc<dyn EntityBase>> {
        let id = self.last_attacker_id.load(Relaxed);
        let world = self.entity.world.load();
        if id != 0
            && let Some(hurt_by) = world.get_entity_by_id(id)
        {
            return Some(hurt_by);
        }
        // Like an `EntityReference`, it resolves by UUID once its entity is loaded (again).
        let hurt_by = world.get_entity_by_uuid(self.last_hurt_by_mob_uuid.load()?)?;
        self.last_attacker_id
            .store(hurt_by.get_entity().entity_id, Relaxed);
        Some(hurt_by)
    }

    /// Vanilla `LivingEntity.setLastHurtByMob`, kept in `last_attacker_id` (0 for none).
    pub fn set_last_hurt_by_mob(&self, hurt_by_id: i32) {
        self.last_attacker_id.store(hurt_by_id, Relaxed);
        self.last_hurt_by_mob_uuid.store(if hurt_by_id == 0 {
            None
        } else {
            self.entity
                .world
                .load()
                .get_entity_by_id(hurt_by_id)
                .map(|hurt_by| hurt_by.get_entity().entity_uuid)
        });
        self.last_attacked_time
            .store(self.entity.tick_count.load(Relaxed), Relaxed);
    }

    /// Vanilla `LivingEntity.resolvePlayerResponsibleForDamage`: a player, or a tame wolf's
    /// owner, is remembered for 100 ticks.
    pub fn resolve_player_responsible_for_damage(&self, attacker: Option<&dyn EntityBase>) {
        let Some(attacker) = attacker else {
            return;
        };
        if let Some(player) = attacker.get_player() {
            self.set_last_hurt_by_player(Some(player.gameprofile.id), 100);
        } else if attacker.get_entity().entity_type == &EntityType::WOLF
            && let Some(tamable) = attacker.get_mob().and_then(|mob| mob.as_tamable())
            && tamable.is_tame()
        {
            match tamable.get_owner() {
                Some(owner) => self.set_last_hurt_by_player(Some(owner), 100),
                None => self.set_last_hurt_by_player(None, 0),
            }
        }
    }

    /// Vanilla `LivingEntity.setLastHurtByPlayer`.
    pub fn set_last_hurt_by_player(&self, player: Option<uuid::Uuid>, time_to_remember: i32) {
        self.last_hurt_by_player.store(player);
        self.last_hurt_by_player_memory_time
            .store(time_to_remember, Relaxed);
    }

    /// Vanilla `LivingEntity.isLookingAtMe`: whether `target` looks at one of this entity's
    /// `gaze_heights` and can see it there. Pumpkin keeps a player's `yHeadRot` as its yaw.
    #[must_use]
    pub fn is_looking_at_me(
        &self,
        target: &Entity,
        cone_size: f64,
        adjust_for_distance: bool,
        see_through_transparent_blocks: bool,
        gaze_heights: &[f64],
    ) -> bool {
        let look =
            Entity::calculate_view_vector(target.pitch.load(), target.yaw.load()).normalize();
        let pos = self.entity.pos.load();
        let target_pos = target.pos.load();
        let target_eye_y = target.get_eye_y();
        let world = self.entity.world.load_full();
        for &gaze_height in gaze_heights {
            let dir = Vector3::new(
                pos.x - target_pos.x,
                gaze_height - target_eye_y,
                pos.z - target_pos.z,
            );
            let dist = dir.length();
            let dot = look.dot(&dir.normalize());
            let threshold = if adjust_for_distance {
                1.0 - cone_size / dist
            } else {
                1.0 - cone_size
            };
            if dot <= threshold {
                continue;
            }
            // `target.hasLineOfSight(this, block, Fluid.NONE, gazeHeight)`.
            if !Arc::ptr_eq(&world, &target.world.load()) {
                continue;
            }
            let from = Vector3::new(target_pos.x, target_eye_y, target_pos.z);
            let to = Vector3::new(pos.x, gaze_height, pos.z);
            if to.squared_distance_to_vec(&from).sqrt() > 128.0 {
                continue;
            }
            let block = if see_through_transparent_blocks {
                pumpkin_world::clip::ClipBlock::Visual
            } else {
                pumpkin_world::clip::ClipBlock::Collider
            };
            if world
                .clip(&pumpkin_world::clip::ClipContext::new(
                    from,
                    to,
                    block,
                    pumpkin_world::clip::ClipFluid::None,
                ))
                .is_miss
            {
                return true;
            }
        }
        false
    }

    /// Vanilla `LivingEntity.resolveMobResponsibleForDamage`.
    pub fn resolve_mob_responsible_for_damage(
        &self,
        damage_type: DamageType,
        attacker: Option<&dyn EntityBase>,
    ) {
        if let Some(attacker) = attacker
            && attacker.get_living_entity().is_some()
            && !damage_type.has_tag(&tag::DamageType::MINECRAFT_NO_ANGER)
            && (damage_type != DamageType::WIND_CHARGE
                || !self
                    .entity
                    .entity_type
                    .has_tag(&tag::EntityType::MINECRAFT_NO_ANGER_FROM_WIND_CHARGE))
        {
            self.set_last_hurt_by_mob(attacker.get_entity().entity_id);
        }
    }

    /// The `lastHurtByMob` part of vanilla `LivingEntity.baseTick`: forgotten once the
    /// attacker dies or 100 ticks pass.
    fn tick_last_hurt_by_mob(&self) {
        let Some(hurt_by) = self.get_last_hurt_by_mob() else {
            return;
        };
        let Some(hurt_by) = hurt_by.get_living_entity() else {
            return;
        };
        if !hurt_by.is_alive()
            || self.entity.tick_count.load(Relaxed) - self.last_attacked_time.load(Relaxed) > 100
        {
            self.set_last_hurt_by_mob(0);
        }
    }
}

impl EntityBase for LivingEntity {
    /// Vanilla `LivingEntity.getWeaponItem`.
    fn get_weapon_item(&self) -> Option<ItemStack> {
        Some(self.held_item(self))
    }

    fn can_use_portal(&self, ignore_passenger: bool) -> bool {
        self.living_can_use_portal(self, ignore_passenger)
    }

    fn damage_with_context(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        self.damage_with_context(caller, amount, damage_type, position, source, cause)
    }

    fn tick_in_void(&self, dyn_self: &dyn EntityBase) {
        dyn_self.damage(dyn_self, 4.0, DamageType::OUT_OF_WORLD);
    }

    fn get_gravity(&self) -> f64 {
        self.get_attribute_value(&Attributes::GRAVITY)
    }

    /// Vanilla `LivingEntity.tick`: `baseTick` (wall and border damage, air, effects), the
    /// item in use, then `aiStep` while not removed (a corpse still moves so knockback
    /// lands).
    #[allow(clippy::too_many_lines)]
    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        // Vanilla `LivingEntity.onAttributeUpdated` refreshes the dimensions when the
        // `scale` attribute changes. Attributes change from places that cannot reach the
        // owning entity here, so the change is picked up before the entity next moves.
        let is_player = self.entity.entity_type == &EntityType::PLAYER;
        // `ServerPlayer.tick` lowers `damageCooldownTime` before `super.tick()`.
        if is_player && self.hurt_cooldown.load(Relaxed) > 0 {
            self.hurt_cooldown.fetch_sub(1, Relaxed);
        }
        let scale = self.get_scale();
        if self.applied_scale.swap(scale) != scale {
            caller.refresh_dimensions();
        }
        // `WaterAnimal.baseTick` and friends read the air before `super.baseTick()`.
        let pre_tick_air = self.breath.air();
        // The start of vanilla `LivingEntity.baseTick`.
        self.swing_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tick();
        if self.entity.first_tick.load(Relaxed)
            && let Some(bed) = self.get_sleeping_pos()
            && !self.set_pos_to_bed(bed)
        {
            self.stop_sleeping(caller);
        }
        crate::enchantment::EnchantmentHelper::tick_effects(caller, self);
        self.entity.tick(caller, server);

        let is_alive = !self.entity.removed.load(Relaxed) && self.health.load() > 0.0;
        if is_alive {
            if self.is_in_wall(caller) {
                caller.damage(caller, 1.0, DamageType::IN_WALL);
            } else if is_player {
                self.hurt_outside_world_border(caller);
            }
            // The air part, which also dismounts riders of `#dismounts_underwater` vehicles.
            self.breath.tick(self, caller);
            let current_block_pos = self.entity.block_pos.load();
            if self.last_block_pos.load() != Some(current_block_pos) {
                self.last_block_pos.store(Some(current_block_pos));
                self.on_changed_block(caller, current_block_pos);
            }
        }
        if self.hurt_time.load(Relaxed) > 0 {
            self.hurt_time.fetch_sub(1, Relaxed);
        }
        if !is_player && self.hurt_cooldown.load(Relaxed) > 0 {
            self.hurt_cooldown.fetch_sub(1, Relaxed);
        }
        let own_tick_death = caller.get_mob().is_some_and(|mob| mob.tick_death(caller));
        // Vanilla `baseTick` runs `tickDeath`, so a corpse removed on its 20th tick never
        // reaches `aiStep` (`if (!this.isRemoved()) this.aiStep()`).
        if self.is_dead_or_dying() && !own_tick_death && !is_player {
            self.tick_death(caller);
        }
        if self.last_hurt_by_player_memory_time.load(Relaxed) > 0 {
            self.last_hurt_by_player_memory_time.fetch_sub(1, Relaxed);
        } else {
            self.last_hurt_by_player.store(None);
        }
        // `lastHurtMob` is forgotten once it dies.
        let last_hurt_mob = self.last_attacking_id.load(Relaxed);
        if last_hurt_mob != 0
            && self
                .entity
                .world
                .load()
                .get_entity_by_id(last_hurt_mob)
                .is_none_or(|mob| !mob.get_entity().is_alive())
        {
            self.last_attacking_id.store(0, Relaxed);
        }
        self.tick_last_hurt_by_mob();
        self.tick_effects();
        // `Mob.baseTick` follows `super.baseTick()` with its ambient-sound roll.
        if let Some(mob) = caller.get_mob() {
            mob.mob_base_tick();
            mob.handle_air_supply(pre_tick_air);
        }

        // Vanilla `LivingEntity.tick`.
        self.updating_using_item(caller, server);
        self.tick_arrows_and_stingers();
        self.detect_equipment_updates(caller);
        if self.entity.tick_count.load(Relaxed) % 20 == 0 {
            self.recheck_combat_status();
        }
        if Self::is_sleeping(caller)
            && (!super::leashable::can_interact_with_level(caller) || !self.check_bed_exists())
        {
            self.stop_sleeping(caller);
        }

        if !self.entity.removed.load(Relaxed) {
            if is_player {
                // A player travels in `Player::tick_server_travel`, after the other entities;
                // the server's part of its `aiStep` is here.
                if is_alive {
                    if self.entity.is_fall_flying() {
                        self.update_fall_flying(caller);
                    }
                    let _ = self.entity.tick_block_collisions(caller);
                    self.entity.tick_frozen(caller);
                    self.update_frost();
                    // The server's move is undone, so the box before and after travel is
                    // the one the client moved the player to.
                    self.tick_auto_spin_attack(caller, self.entity.bounding_box.load());
                    self.push_entities(caller);
                }
            } else {
                self.tick_movement(caller);
                if let Some(mob) = caller.get_mob() {
                    mob.get_mob_entity()
                        .body_rotation_control
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .client_tick(mob);
                }
            }
        }

        if self.entity.is_fall_flying() {
            self.fall_fly_ticks.fetch_add(1, Relaxed);
        } else {
            self.fall_fly_ticks.store(0, Relaxed);
        }
        if Self::is_sleeping(caller) {
            self.entity.set_pitch(0.0);
        }
        self.refresh_dirty_attributes();

        // The end of vanilla `LivingEntity.tick`.
        if self.current_impulse_context_reset_grace_time.load(Relaxed) > 0 {
            self.current_impulse_context_reset_grace_time
                .fetch_sub(1, Relaxed);
        }

        if self.health.load() <= 0.0 && !own_tick_death && is_player {
            let time = self
                .death_time
                .fetch_update(Relaxed, Relaxed, |time| Some(time.saturating_add(1)))
                .unwrap_or_else(|time| time)
                .saturating_add(1);
            // Bedrock keeps a dead remote player actor in its death pose.
            // Remove Java players after the animation so respawn can
            // recreate a live, interactable actor with the same identity.
            if time == 10 {
                self.entity
                    .world
                    .load()
                    .despawn_dead_java_player_for_bedrock(&self.entity);
            }
            // Vanilla `tickDeath` removes the player here (`remove(KILLED)`): it leaves the
            // level (its players, tracking, chunks, the waypoint manager and the sleeping
            // list) and waits in no level until its client asks to respawn.
            // `deathTime >= 20 && !isRemoved()`: a player that rejoined dead past tick 20
            // (its `DeathTime` saved) leaves on its first tick.
            if time >= 20
                && !self.entity.is_removed()
                && let Some(player) = caller.get_player()
                && let Some(player) = self
                    .entity
                    .world
                    .load()
                    .get_player_by_uuid(player.gameprofile.id)
            {
                player.remove_killed();
            }
        }
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        Some(self)
    }

    fn is_pushable(&self) -> bool {
        self.is_pushable_for(self)
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Vanilla `LivingEntity.INPUT_FRICTION`.
const INPUT_FRICTION: f32 = 0.98;
/// The `hurtDuration` vanilla sets on every full-damage hit.
const HURT_DURATION: i32 = 10;

pub const SPEED_MODIFIER_SPRINTING_ID: &str = "minecraft:sprinting";
pub const SPEED_MODIFIER_SPRINTING_AMOUNT: f64 = 0.300_000_011_920_928_96;

impl LivingEntity {
    /// Vanilla `LivingEntity.isPushable`: alive, not a spectator and not on a ladder.
    /// Pumpkin doesn't skip entities in chunks that aren't entity-ticking.
    #[must_use]
    pub fn is_pushable_for(&self, caller: &dyn EntityBase) -> bool {
        self.is_alive() && !caller.is_spectator() && !self.on_climbable(caller)
    }

    /// Vanilla `LivingEntity.isInWall`: a sleeping entity is never in a wall.
    fn is_in_wall(&self, caller: &dyn EntityBase) -> bool {
        !Self::is_sleeping(caller) && self.entity.is_in_wall()
    }

    /// The world border part of vanilla `baseTick`, for players.
    fn hurt_outside_world_border(&self, caller: &dyn EntityBase) {
        let world = self.entity.world.load();
        let damage = {
            let border = world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if border.contains_box(&self.entity.bounding_box.load()) {
                return;
            }
            let pos = self.entity.pos.load();
            let dist = border.distance_to_border(pos.x, pos.z) + border.buffer;
            if dist >= 0.0 || border.damage_per_block <= 0.0 {
                return;
            }
            1.max((-dist * border.damage_per_block).floor() as i32)
        };
        caller.damage(caller, damage as f32, DamageType::OUTSIDE_BORDER);
    }

    /// Vanilla `getArrowCount`.
    #[must_use]
    pub fn get_arrow_count(&self) -> i32 {
        self.arrow_count.load(Relaxed)
    }

    /// Vanilla `setArrowCount`, synced.
    pub fn set_arrow_count(&self, count: i32) {
        self.arrow_count.store(count, Relaxed);
        self.entity.set_synced_data(
            tracked_data::living_entity::DATA_ARROW_COUNT_ID,
            VarInt(count),
        );
    }

    /// Vanilla `getStingerCount`.
    #[must_use]
    pub fn get_stinger_count(&self) -> i32 {
        self.stinger_count.load(Relaxed)
    }

    /// Vanilla `setStingerCount`, synced.
    pub fn set_stinger_count(&self, count: i32) {
        self.stinger_count.store(count, Relaxed);
        self.entity.set_synced_data(
            tracked_data::living_entity::DATA_STINGER_COUNT_ID,
            VarInt(count),
        );
    }

    /// The stuck arrows and stingers falling out, from vanilla `LivingEntity.tick`.
    fn tick_arrows_and_stingers(&self) {
        let arrow_count = self.get_arrow_count();
        if arrow_count > 0 {
            if self.remove_arrow_time.load(Relaxed) <= 0 {
                self.remove_arrow_time
                    .store(20 * (30 - arrow_count), Relaxed);
            }
            if self.remove_arrow_time.fetch_sub(1, Relaxed) - 1 <= 0 {
                self.set_arrow_count(arrow_count - 1);
            }
        }
        let stinger_count = self.get_stinger_count();
        if stinger_count > 0 {
            if self.remove_stinger_time.load(Relaxed) <= 0 {
                self.remove_stinger_time
                    .store(20 * (30 - stinger_count), Relaxed);
            }
            if self.remove_stinger_time.fetch_sub(1, Relaxed) - 1 <= 0 {
                self.set_stinger_count(stinger_count - 1);
            }
        }
    }

    /// Vanilla `canGlideUsing`: a glider worn in its own slot that won't break on the next
    /// damage.
    fn can_glide_using(stack: &ItemStack, slot: &EquipmentSlot) -> bool {
        if stack
            .get_data_component::<pumpkin_data::data_component_impl::GliderImpl>()
            .is_none()
        {
            return false;
        }
        // `nextDamageWillBreak`.
        let next_damage_will_break = stack.get_max_damage().is_some_and(|max_damage| {
            !stack.is_unbreakable() && stack.get_damage() >= max_damage - 1
        });
        stack
            .get_data_component::<EquippableImpl>()
            .is_some_and(|equippable| equippable.slot == slot)
            && !next_damage_will_break
    }

    /// Vanilla `canGlide`.
    #[must_use]
    pub fn can_glide(&self, caller: &dyn EntityBase) -> bool {
        !self.entity.on_ground.load(Relaxed)
            && !self.entity.has_vehicle()
            && !self.has_effect(&StatusEffect::LEVITATION)
            && EquipmentSlot::VALUES
                .iter()
                .any(|slot| Self::can_glide_using(&self.get_item_by_slot(caller, slot), slot))
    }

    /// Vanilla `updateFallFlying`: the glider wears every 20 ticks and the glide is a
    /// vibration every 10.
    fn update_fall_flying(&self, caller: &dyn EntityBase) {
        self.entity.check_fall_distance_accumulation();
        if !self.can_glide(caller) {
            if self.entity.is_fall_flying() {
                self.entity.set_fall_flying(false);
            }
            return;
        }
        let check_fall_fly_ticks = self.fall_fly_ticks.load(Relaxed) + 1;
        if check_fall_fly_ticks % 10 == 0 {
            let free_fall_interval = check_fall_fly_ticks / 10;
            if free_fall_interval % 2 == 0 {
                let slots_with_gliders: Vec<&EquipmentSlot> = EquipmentSlot::VALUES
                    .iter()
                    .filter(|slot| {
                        Self::can_glide_using(&self.get_item_by_slot(caller, slot), slot)
                    })
                    .collect();
                // `Util.getRandom(slotsWithGliders, random)`.
                let index = self
                    .entity
                    .random()
                    .next_bounded_i32(slots_with_gliders.len() as i32);
                if let Some(slot) = slots_with_gliders.get(index as usize) {
                    self.hurt_and_break_in_slot(caller, slot, 1);
                }
            }
            self.entity.world.load().game_event_from(
                Some(caller),
                GameEvent::ElytraGlide,
                self.entity.pos.load(),
            );
        }
    }

    /// Vanilla `LivingEntity.releaseUsingItem`: while the used hand still holds the same item
    /// (`ItemStack.isSameItem`), that stack is released (`releaseUsing`) and, for an item used
    /// on release (the crossbow), updated once more (`updatingUsingItem`); then the use stops.
    pub fn release_using_item(&self, caller: &dyn EntityBase, server: &Server) {
        let use_item = self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(use_item) = use_item
            && !use_item.is_empty()
        {
            let hand_item = self.get_stack_in_hand(caller, self.used_item_hand());
            if ItemStack::is_same_item(&hand_item, &use_item) {
                *self
                    .item_in_use
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hand_item.clone());
                if let Some(player) = caller.get_player() {
                    server.item_registry.on_stopped_using(&hand_item, player);
                }
                // `useOnRelease`: only the crossbow.
                if hand_item.item.id == Item::CROSSBOW.id {
                    self.updating_using_item(caller, server);
                }
            }
        }
        self.clear_active_hand();
    }

    /// Vanilla `updatingUsingItem`: the item in use follows the hand while it is the same
    /// item, ticks (`updateUsingItem`) and completes once the time runs out.
    fn updating_using_item(&self, caller: &dyn EntityBase, server: &Server) {
        let Some(use_item) = self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        else {
            return;
        };
        let hand = self.used_item_hand();
        let hand_item = self.get_stack_in_hand(caller, hand);
        // `ItemStack.isSameItem`: the item type only.
        if hand_item.item.id != use_item.item.id {
            self.clear_active_hand();
            return;
        }
        *self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hand_item.clone());

        // `updateUsingItem`.
        let remaining_use_ticks = self.item_use_time.load(Relaxed);
        if let Some(player) = caller.get_player() {
            server
                .item_registry
                .on_use_tick(&hand_item, player, remaining_use_ticks);
        } else if let Some(mob) = caller.get_mob() {
            crate::item::items::spear::SpearItem::mob_use_tick(mob);
        }
        // `useOnRelease`: only a crossbow waits for its release.
        if self.item_use_time.fetch_sub(1, Relaxed).wrapping_sub(1) == 0
            && hand_item.item.id != Item::CROSSBOW.id
        {
            self.complete_using_item(caller, hand);
        }
    }

    /// Vanilla `getUsedItemHand`: the hand of the item in use, the main hand when none is.
    ///
    /// Vanilla reads the off-hand bit (2) of the living flags, which only `startUsingItem`
    /// writes and `stopUsingItem` leaves as it was, so after a use ends it still answers that
    /// use's hand. Pumpkin's `active_hand` is cleared with the use, so outside a use (as for
    /// `BreakDoorGoal`'s swing) this answers the main hand even after an off-hand use.
    pub fn used_item_hand(&self) -> Hand {
        self.active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .unwrap_or(Hand::Right)
    }

    /// Vanilla `completeUsingItem`.
    fn complete_using_item(&self, caller: &dyn EntityBase, hand: Hand) {
        let Some(use_item) = self
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        else {
            return;
        };
        if !use_item.is_empty() {
            // Vanilla mutates the hand's own stack; this copy goes back into the hand.
            let result = self.finish_using_item(caller, use_item);
            self.set_stack_in_hand(caller, hand, result);
            self.clear_active_hand();
        }
    }

    /// Vanilla `ItemStack.finishUsingItem`: `Item.finishUsingItem` (a consumable's
    /// `onConsume`), then the `use_remainder` and `use_cooldown` side effects.
    pub fn finish_using_item(&self, caller: &dyn EntityBase, stack: ItemStack) -> ItemStack {
        let stack_before_use = stack.clone();
        let result = match stack.get_data_component::<ConsumableImpl>().cloned() {
            Some(consumable) => self.consume(caller, stack, &consumable),
            None => stack,
        };
        Self::apply_after_use_component_side_effects(caller, result, &stack_before_use)
    }

    /// Vanilla `ItemStack.applyAfterUseComponentSideEffects`.
    fn apply_after_use_component_side_effects(
        caller: &dyn EntityBase,
        result: ItemStack,
        stack_before_using: &ItemStack,
    ) -> ItemStack {
        let player = caller.get_player();
        let mut result = result;
        if let Some(use_remainder) = stack_before_using
            .get_data_component::<pumpkin_data::data_component_impl::UseRemainderImpl>(
        ) && let Some(remainder) = use_remainder
            .remainder
            .as_deref()
            .and_then(|key| Item::from_registry_key(key.strip_prefix("minecraft:").unwrap_or(key)))
        {
            // `UseRemainder.convertIntoRemainder`.
            let has_infinite_materials =
                player.is_some_and(|player| player.gamemode.load() == GameMode::Creative);
            if !has_infinite_materials && result.item_count < stack_before_using.item_count {
                let remainder_stack = ItemStack::new(1, remainder);
                if result.is_empty() {
                    result = remainder_stack;
                } else if let Some(player) = player {
                    // `ServerPlayer.handleExtraItemsCreatedOnUse`.
                    let mut remainder_stack = remainder_stack;
                    if !player.inventory.insert_stack_anywhere(&mut remainder_stack) {
                        player.drop_from_hand(remainder_stack, false, false);
                    }
                }
            }
        }
        if let Some(player) = player
            && let Some(cooldown) = stack_before_using.get_use_cooldown()
        {
            // `UseCooldown.apply`.
            player.start_cooldown(
                cooldown_group(stack_before_using),
                (cooldown.seconds * 20.0) as i32,
            );
        }
        result
    }

    /// Vanilla `Consumable.onConsume`, with the `ConsumableListener` components.
    #[expect(
        clippy::too_many_lines,
        reason = "mirrors Consumable.onConsume and its listeners"
    )]
    fn consume(
        &self,
        caller: &dyn EntityBase,
        mut stack: ItemStack,
        consumable: &ConsumableImpl,
    ) -> ItemStack {
        let world = self.entity.world.load();
        let pos = self.entity.pos.load();
        let player = caller.get_player();
        let play = |volume: f32,
                    pitch: f32,
                    category: SoundCategory,
                    except: Option<&crate::entity::player::Player>| {
            match (&consumable.sound_event, except) {
                (IdOr::Id(sound), Some(except)) => {
                    world.play_sound_raw_expect(
                        except,
                        *sound as u16,
                        category,
                        &pos,
                        volume,
                        pitch,
                    );
                }
                (IdOr::Id(sound), None) => {
                    world.play_sound_fine(*sound, category, &pos, volume, pitch);
                }
                (IdOr::Value(sound), _) => {
                    world.play_custom_sound(&sound.sound_name, category, &pos, volume, pitch);
                }
            }
        };
        // `emitParticlesAndSounds(user.getRandom(), user, stack, 16)`, through
        // `Entity.playSound` (which a player's own client plays itself).
        let (eat_volume, eat_pitch, drink_pitch) = {
            let mut random = self.entity.random();
            let eat_volume = if random.next_bool() { 0.5 } else { 1.0 };
            let eat_pitch = 1.0 + 0.2 * (random.next_f32() - random.next_f32());
            // `Mth.randomBetween(random, 0.9F, 1.0F)`.
            let drink_pitch = random.next_f32() * (1.0f32 - 0.9) + 0.9;
            // `LivingEntity.spawnItemParticles(stack, 16)`: four floats a particle, drawn
            // on the server too.
            if consumable.consume_particles && !stack.is_empty() {
                for _ in 0..16 * 4 {
                    random.next_f32();
                }
            }
            (eat_volume, eat_pitch, drink_pitch)
        };
        let drinks = consumable.animation == ConsumeAnimation::Drink;
        if !self.entity.is_silent() {
            if drinks {
                play(0.5, drink_pitch, caller.entity_sound_source(), player);
            } else {
                play(eat_volume, eat_pitch, caller.entity_sound_source(), player);
            }
        }
        if let Some(player) = player {
            player.increment_stat(StatisticCategory::Used, stack.item.id as i32, 1);
            player.trigger_advancement(
                crate::entity::player::advancement::trigger::AdvancementTrigger::ConsumeItem {
                    item_id: format!("minecraft:{}", stack.item.registry_key),
                },
            );
        }

        // The `ConsumableListener` components.
        if let Some(food) = stack.get_data_component::<FoodImpl>().cloned() {
            // `FoodProperties.onConsume`.
            let pitch = {
                let mut random = self.entity.random();
                1.0 + 0.4 * (random.next_f32() - random.next_f32())
            };
            play(1.0, pitch, SoundCategory::Neutral, None);
            if let Some(player) = player {
                player
                    .hunger_manager
                    .eat(player, food.nutrition as u8, food.saturation);
                let burp_pitch = self.entity.random().next_f32() * (1.0f32 - 0.9) + 0.9;
                world.play_sound_fine(
                    Sound::EntityPlayerBurp,
                    SoundCategory::Players,
                    &pos,
                    0.5,
                    burp_pitch,
                );
                world.play_bedrock_level_sound("burp", &pos, -1);
            }
        }
        if stack
            .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
            .is_some()
        {
            // `PotionContents.onConsume`: `applyToLivingEntity`.
            let duration_scale = stack
                .get_data_component::<pumpkin_data::data_component_impl::PotionDurationScaleImpl>()
                .map_or(1.0, |component| component.scale);
            crate::item::potion::PotionContents::apply_effects_to(
                self,
                crate::item::potion::PotionContents::read_potion_effects(&stack),
                duration_scale,
                crate::item::potion::PotionApplicationSource::Drink,
                None,
                None,
            );
        }
        if let Some(stew) = stack
            .get_data_component::<pumpkin_data::data_component_impl::SuspiciousStewEffectsImpl>()
            .cloned()
        {
            for effect in stew.effects.iter() {
                if let Some(effect_type) = StatusEffect::from_minecraft_name(&effect.effect) {
                    self.add_effect(Effect {
                        effect_type,
                        duration: effect.duration,
                        amplifier: 0,
                        ambient: false,
                        show_particles: true,
                        show_icon: true,
                        blend: false,
                    });
                }
            }
        }
        if let Some(ominous) = stack
            .get_data_component::<pumpkin_data::data_component_impl::OminousBottleAmplifierImpl>()
        {
            self.add_effect(Effect {
                effect_type: &StatusEffect::BAD_OMEN,
                duration: 120_000,
                amplifier: ominous.amplifier.clamp(0, 255) as u8,
                ambient: false,
                show_particles: false,
                show_icon: true,
                blend: false,
            });
        }

        self.apply_consume_effects(caller, &consumable.effects);
        let event = if drinks {
            GameEvent::Drink
        } else {
            GameEvent::Eat
        };
        world.game_event_from(Some(caller), event, pos);
        // `stack.consume(1, user)`: nothing is used up in creative.
        if !player.is_some_and(|player| player.gamemode.load() == GameMode::Creative) {
            stack.decrement(1);
        }
        stack
    }

    /// Vanilla `LivingEntity.tickDeath` for anything but a player: the death animation runs
    /// 20 ticks, then the corpse goes with the POOF (60) particle event. The death event (3)
    /// was already broadcast in `die`; sending it again would restart the client-side
    /// animation.
    fn tick_death(&self, caller: &dyn EntityBase) {
        let time = self
            .death_time
            .fetch_update(Relaxed, Relaxed, |time| Some(time.saturating_add(1)))
            .unwrap_or_else(|time| time)
            .saturating_add(1);
        if time >= 20 && !self.entity.removed.swap(true, Ordering::Relaxed) {
            if let Some(mob) = caller.get_mob() {
                mob.remove_killed();
            }
            // `LivingEntity.remove(KILLED)`.
            self.trigger_on_death_mob_effects();
            self.active_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
            self.entity
                .world
                .load()
                .send_entity_status(&self.entity, EntityStatus::Poof, None);
            self.entity.remove();
        }
    }

    pub fn set_sprinting(&self, is_sprinting: bool) {
        self.entity.set_sprinting(is_sprinting);
        self.update_attribute(&Attributes::MOVEMENT_SPEED, |speed| {
            speed.remove_modifier(SPEED_MODIFIER_SPRINTING_ID);
            if is_sprinting {
                speed.add_or_replace_modifier(Modifier {
                    id: SPEED_MODIFIER_SPRINTING_ID.to_string(),
                    amount: SPEED_MODIFIER_SPRINTING_AMOUNT,
                    operation: ModifierOperation::MultiplyTotal,
                });
            }
        });
        crate::entity::attributes::send_attribute_updates_for_living(
            self,
            vec![Attributes::MOVEMENT_SPEED],
        );
    }

    #[must_use]
    pub fn get_block_speed_factor(&self) -> f32 {
        let efficiency = self.get_attribute_value(&Attributes::MOVEMENT_EFFICIENCY) as f32;
        let super_factor = self.entity.get_block_speed_factor();
        super_factor + efficiency * (1.0 - super_factor)
    }

    /// Each vanilla `ConsumeEffect.apply`, in order.
    fn apply_consume_effects(&self, caller: &dyn EntityBase, effects: &[ConsumeEffect]) {
        for consume_effect in effects {
            match consume_effect {
                ConsumeEffect::ApplyEffects((effects, probability)) => {
                    if !consume_effect_probability_applies(
                        *probability,
                        self.entity.random().next_f32(),
                    ) {
                        continue;
                    }

                    for effect in effects.iter() {
                        let Some(effect_type) =
                            StatusEffect::from_minecraft_name(&effect.effect_id)
                        else {
                            continue;
                        };
                        let Ok(amplifier) = u8::try_from(effect.amplifier) else {
                            continue;
                        };

                        self.add_effect(Effect {
                            effect_type,
                            duration: effect.duration,
                            amplifier,
                            ambient: effect.ambient,
                            show_particles: effect.show_particles,
                            show_icon: effect.show_icon,
                            blend: false,
                        });
                    }
                }
                ConsumeEffect::ClearAllEffects => {
                    self.remove_all_effects();
                }
                ConsumeEffect::RemoveEffects(idset) => {
                    if let pumpkin_data::data_component_impl::IDSet::IDs(ids) = idset {
                        for effect_type in ids.iter() {
                            self.remove_effect(effect_type);
                        }
                    }
                }
                ConsumeEffect::TeleportRandomly { diameter, .. } => {
                    // `TeleportRandomlyConsumeEffect`: a passenger stops riding first (vanilla
                    // `stopRiding`, the full dismount); a plugin that cancels it keeps it seated.
                    if let Some(vehicle) = caller.get_entity().get_vehicle() {
                        vehicle
                            .get_entity()
                            .remove_passenger(caller.get_entity().entity_id);
                        if caller.get_entity().has_vehicle() {
                            continue;
                        }
                    }

                    let center = self.entity.pos.load();
                    let Some(pos) = self.find_random_teleport_target(*diameter) else {
                        continue;
                    };
                    let world = self.entity.world.load_full();
                    // `randomTeleport`'s `teleportTo(x, y, z)`.
                    crate::entity::teleport_to(caller, pos);

                    let destination = self.entity.pos.load();
                    if destination != center {
                        self.entity.reset_fall_distance();
                        self.reset_current_impulse_context();
                        // Vanilla broadcasts entity event 46 (teleport particles) on success.
                        world.send_entity_status(&self.entity, EntityStatus::Teleport, None);
                        world.game_event(
                            pumpkin_data::game_event::GameEvent::Teleport,
                            center,
                            crate::world::game_event::GameEventContext::of_entity(caller),
                        );
                        world.play_sound(
                            Sound::ItemChorusFruitTeleport,
                            SoundCategory::Players,
                            &destination,
                        );
                    }
                }
                ConsumeEffect::PlaySound(sound) => {
                    // `level.playSound(null, user.blockPosition(), sound, user.getSoundSource())`.
                    let pos = self.entity.block_pos.load().to_centered_f64();
                    self.entity.world.load().play_sound_event_fine(
                        sound,
                        self.sound_source(),
                        &pos,
                        1.0,
                        1.0,
                    );
                }
            }
        }
    }

    /// Vanilla `LivingEntity.randomTeleport` with an avoidance tag: drops onto the first
    /// block below `(x, y, z)` that entities can teleport to, then teleports there if the
    /// spot is free, dry and clear of `avoidance_tag` blocks.
    pub fn random_teleport(
        &self,
        caller: &dyn EntityBase,
        x: f64,
        y: f64,
        z: f64,
        show_particles: bool,
        avoidance_tag: &'static pumpkin_data::tag::Tag,
    ) -> bool {
        let world = self.entity.world.load_full();
        let clamped = world
            .worldborder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clamp_vec3_to_bound(x, y, z);
        let mut pos = BlockPos::floored_v(clamped);
        if world.get_block_state_if_loaded(&pos).is_none() {
            return false;
        }
        let mut y = y;
        while pos.0.y > world.get_bottom_y() {
            pos = pos.down();
            let block = world.get_block(&pos);
            if block.has_tag(&tag::Block::MINECRAFT_ENTITIES_CAN_TELEPORT_TO) {
                return self.check_position_and_teleport(
                    caller,
                    Vector3::new(clamped.x, y, clamped.z),
                    show_particles,
                    avoidance_tag,
                    block,
                );
            }
            y -= 1.0;
        }
        false
    }

    /// Vanilla `LivingEntity.checkPositionAndTeleport`.
    fn check_position_and_teleport(
        &self,
        caller: &dyn EntityBase,
        target: Vector3<f64>,
        show_particles: bool,
        avoidance_tag: &'static pumpkin_data::tag::Tag,
        block: &Block,
    ) -> bool {
        if block.has_tag(avoidance_tag) {
            return false;
        }
        let world = self.entity.world.load_full();
        let bounding_box = BoundingBox::new_from_pos(
            target.x,
            target.y,
            target.z,
            &self.entity.entity_dimension.load(),
        );
        if !world.is_space_empty(bounding_box) || world.contains_any_liquid(bounding_box) {
            return false;
        }
        // Vanilla `findBlocksIn(aabb)`: from the block holding the min corner to the one
        // holding the max corner.
        if BlockPos::iterate(
            BlockPos::floored_v(bounding_box.min),
            BlockPos::floored_v(bounding_box.max),
        )
        .any(|pos| world.get_block(&pos).has_tag(avoidance_tag))
        {
            return false;
        }
        caller.get_entity().teleport(target, None, None, &world);
        if show_particles {
            world.send_entity_status(&self.entity, EntityStatus::Teleport, None);
        }
        if let Some(mob) = caller.get_mob() {
            // `pathfinderMob.getNavigation().stop()` (`LivingEntity.java:3721`).
            mob.get_mob_entity()
                .with_navigation(|navigation, _| navigation.stop());
        }
        true
    }

    fn find_random_teleport_target(&self, diameter: f32) -> Option<Vector3<f64>> {
        let center = self.entity.pos.load();
        let world = self.entity.world.load();
        let bottom_y = world.get_bottom_y();
        let top_y = world.get_top_y();
        let dimensions = self.entity.entity_dimension.load();

        'attempts: for _ in 0..Self::RANDOM_TELEPORT_ATTEMPTS {
            // `TeleportRandomlyConsumeEffect.apply`: x, then y, then z from `user.getRandom()`.
            let (target_x, sampled_y, target_z) = {
                let mut random = self.entity.random();
                let x = random_teleport_coordinate(center.x, diameter, random.next_f64());
                let y = random_teleport_coordinate(center.y, diameter, random.next_f64());
                let z = random_teleport_coordinate(center.z, diameter, random.next_f64());
                (x, y, z)
            };
            let sampled_y = sampled_y.clamp(f64::from(bottom_y + 1), f64::from(top_y));
            let mut block_y = sampled_y.floor() as i32;
            let block_x = target_x.floor() as i32;
            let block_z = target_z.floor() as i32;

            loop {
                if block_y <= bottom_y {
                    continue 'attempts;
                }

                let below = BlockPos::new(block_x, block_y - 1, block_z);
                let Some(below_state) = world.get_block_state_if_loaded(&below) else {
                    continue 'attempts;
                };
                if below_state.is_solid() {
                    break;
                }
                block_y -= 1;
            }

            let target = Vector3::new(target_x, f64::from(block_y), target_z);
            let bounding_box = BoundingBox::new_from_pos(target.x, target.y, target.z, &dimensions);

            for block_pos in
                BlockPos::iterate(bounding_box.min_block_pos(), bounding_box.max_block_pos())
            {
                if world.get_block_state_if_loaded(&block_pos).is_none()
                    || world.get_fluid(&block_pos).id != Fluid::EMPTY.id
                {
                    continue 'attempts;
                }
            }

            if world.is_space_empty(bounding_box) {
                return Some(target);
            }
        }

        None
    }
}

fn random_teleport_coordinate(center: f64, diameter: f32, random: f64) -> f64 {
    center + (random - 0.5) * f64::from(diameter)
}

fn attributes_by_id(id: u8) -> Option<&'static Attributes> {
    Attributes::ALL.iter().find(|attr| attr.id == id)
}

fn push_unique_attribute(touched: &mut Vec<Attributes>, attr: &Attributes) {
    if !touched.iter().any(|existing| existing.id == attr.id) {
        touched.push(attr.clone());
    }
}

/// Vanilla `ItemStack.forEachModifier(EquipmentSlot, BiConsumer)`: the component modifiers
/// for `slot`, then the enchantment ones.
pub fn for_each_item_modifier<F>(stack: &ItemStack, slot: &EquipmentSlot, mut f: F)
where
    F: FnMut(&'static Attributes, Modifier),
{
    if let Some(modifiers) = stack.get_data_component::<AttributeModifiersImpl>() {
        for item_mod in modifiers.attribute_modifiers.iter() {
            if attribute_modifier_slot_matches(&item_mod.slot, slot) {
                f(
                    item_mod.r#type,
                    Modifier {
                        id: item_mod.id.to_string(),
                        amount: item_mod.amount,
                        operation: ModifierOperation::from_operation(item_mod.operation),
                    },
                );
            }
        }
    }
    crate::enchantment::EnchantmentHelper::for_each_modifier(stack, slot, f);
}

const fn attribute_modifier_slot_matches(
    modifier_slot: &AttributeModifierSlot,
    equipment_slot: &EquipmentSlot,
) -> bool {
    match modifier_slot {
        AttributeModifierSlot::Any => true,
        AttributeModifierSlot::MainHand => matches!(equipment_slot, EquipmentSlot::MainHand(_)),
        AttributeModifierSlot::OffHand => matches!(equipment_slot, EquipmentSlot::OffHand(_)),
        AttributeModifierSlot::Hand => {
            matches!(
                equipment_slot,
                EquipmentSlot::MainHand(_) | EquipmentSlot::OffHand(_)
            )
        }
        AttributeModifierSlot::Feet => matches!(equipment_slot, EquipmentSlot::Feet(_)),
        AttributeModifierSlot::Legs => matches!(equipment_slot, EquipmentSlot::Legs(_)),
        AttributeModifierSlot::Chest => matches!(equipment_slot, EquipmentSlot::Chest(_)),
        AttributeModifierSlot::Head => matches!(equipment_slot, EquipmentSlot::Head(_)),
        AttributeModifierSlot::Armor => matches!(
            equipment_slot,
            EquipmentSlot::Feet(_)
                | EquipmentSlot::Legs(_)
                | EquipmentSlot::Chest(_)
                | EquipmentSlot::Head(_)
        ),
        AttributeModifierSlot::Body => matches!(equipment_slot, EquipmentSlot::Body(_)),
        AttributeModifierSlot::Saddle => matches!(equipment_slot, EquipmentSlot::Saddle(_)),
    }
}

/// Mirrors vanilla's strict `random < probability` consume-effect gate.
const fn consume_effect_probability_applies(probability: f32, random: f32) -> bool {
    random < probability
}

#[cfg(test)]
mod consumable_effect_tests {
    use super::{consume_effect_probability_applies, random_teleport_coordinate};

    #[test]
    fn consumable_effect_probability_matches_vanilla_strict_threshold() {
        assert!(!consume_effect_probability_applies(0.0, 0.0));
        assert!(consume_effect_probability_applies(1.0, 0.999));
        assert!(consume_effect_probability_applies(0.5, 0.499));
        assert!(!consume_effect_probability_applies(0.5, 0.5));
    }

    #[test]
    fn random_teleport_coordinate_uses_full_diameter() {
        assert_eq!(random_teleport_coordinate(10.0, 16.0, 0.0), 2.0);
        assert_eq!(random_teleport_coordinate(10.0, 16.0, 0.5), 10.0);
        assert_eq!(random_teleport_coordinate(10.0, 16.0, 1.0), 18.0);
    }
}
/// Returns `true` if `damage_type` is in `#minecraft:bypasses_armor` (1.21.11).
/// These sources bypass armor entirely (fall, drown, freeze, etc.).
pub(crate) const fn bypasses_armor_durability(damage_type: &DamageType) -> bool {
    // Bitmask lookup: O(1) with two instructions (shift + AND), no array scan.
    // DamageType IDs can exceed 31; use u64 for sufficient range.
    // TODO: Make data-driven once the data pack system can handle it without performance regressions.
    // Compile-time assertions: ensure all bypassing types fit in u64 bitmask.
    const _: () = assert!(
        DamageType::FALL.id < 64
            && DamageType::FLY_INTO_WALL.id < 64
            && DamageType::ON_FIRE.id < 64
            && DamageType::IN_WALL.id < 64
            && DamageType::CRAMMING.id < 64
            && DamageType::DROWN.id < 64
            && DamageType::GENERIC.id < 64
            && DamageType::WITHER.id < 64
            && DamageType::DRAGON_BREATH.id < 64
            && DamageType::STARVE.id < 64
            && DamageType::ENDER_PEARL.id < 64
            && DamageType::FREEZE.id < 64
            && DamageType::STALAGMITE.id < 64
            && DamageType::MAGIC.id < 64
            && DamageType::INDIRECT_MAGIC.id < 64
            && DamageType::OUT_OF_WORLD.id < 64
            && DamageType::GENERIC_KILL.id < 64
            && DamageType::SONIC_BOOM.id < 64
            && DamageType::OUTSIDE_BORDER.id < 64,
        "One or more bypass DamageType IDs exceed u64 bitmask width (>= 64)"
    );
    const BYPASS_MASK: u64 = (1u64 << DamageType::FALL.id)
        | (1u64 << DamageType::FLY_INTO_WALL.id)
        | (1u64 << DamageType::ON_FIRE.id)
        | (1u64 << DamageType::IN_WALL.id)
        | (1u64 << DamageType::CRAMMING.id)
        | (1u64 << DamageType::DROWN.id)
        | (1u64 << DamageType::GENERIC.id)
        | (1u64 << DamageType::WITHER.id)
        | (1u64 << DamageType::DRAGON_BREATH.id)
        | (1u64 << DamageType::STARVE.id)
        | (1u64 << DamageType::ENDER_PEARL.id)
        | (1u64 << DamageType::FREEZE.id)
        | (1u64 << DamageType::STALAGMITE.id)
        | (1u64 << DamageType::MAGIC.id)
        | (1u64 << DamageType::INDIRECT_MAGIC.id)
        | (1u64 << DamageType::OUT_OF_WORLD.id)
        | (1u64 << DamageType::GENERIC_KILL.id)
        | (1u64 << DamageType::SONIC_BOOM.id)
        | (1u64 << DamageType::OUTSIDE_BORDER.id);
    (damage_type.id < 64) && ((BYPASS_MASK >> damage_type.id) & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spin_stack_is_active_only_while_its_slot_still_holds_it() {
        let mut spin = ItemStack::new(1, &Item::TRIDENT);
        spin.set_damage(3);
        // The slot it was used from, still holding it.
        assert!(spin_stack_is_active(&spin, 2, &spin.clone(), 2));
        // Another slot is active (the player switched slots or uses the off hand).
        assert!(!spin_stack_is_active(&spin, 2, &spin.clone(), 40));
        // The slot holds something else now, or the same trident worn further.
        let mut worn = spin.clone();
        worn.set_damage(4);
        assert!(!spin_stack_is_active(&spin, 2, &worn, 2));
        assert!(!spin_stack_is_active(
            &spin,
            2,
            &ItemStack::EMPTY.clone(),
            2
        ));
    }

    /// A fresh `ServerPlayer` (respawn) starts with no riptide spin: `restoreFrom` copies none
    /// of `autoSpinAttackTicks`, `autoSpinAttackDmg`, `autoSpinAttackItemStack` or flag 4.
    #[tokio::test]
    async fn clearing_the_spin_for_respawn_resets_every_part_of_it() {
        let (_folder, world) = crate::world::test_support::test_world();
        let zombie = crate::entity::r#type::from_type(
            &EntityType::ZOMBIE,
            Vector3::new(0.0, 100.0, 0.0),
            &world,
            uuid::Uuid::new_v4(),
        );
        let living = zombie.get_living_entity().unwrap();
        living.auto_spin_attack_ticks.store(20, Relaxed);
        living.auto_spin_attack_dmg.store(8.0);
        *living.auto_spin_attack_item.lock().unwrap() =
            Some((ItemStack::new(1, &Item::TRIDENT), 0));
        living.set_living_flag(LivingEntity::AUTO_SPIN_ATTACK_FLAG, true);

        living.clear_auto_spin_attack_for_respawn();

        assert_eq!(living.auto_spin_attack_ticks.load(Relaxed), 0);
        assert_eq!(living.auto_spin_attack_dmg.load(), 0.0);
        assert!(living.auto_spin_attack_item.lock().unwrap().is_none());
        assert!(!living.is_auto_spin_attack());
    }

    // ── equipment detection ───────────────────────────────────────────

    use crate::world::test_support::test_world;

    fn spawn(
        world: &Arc<crate::world::World>,
        entity_type: &'static EntityType,
    ) -> Arc<dyn EntityBase> {
        crate::entity::r#type::from_type(
            entity_type,
            Vector3::new(0.0, 100.0, 0.0),
            world,
            uuid::Uuid::new_v4(),
        )
    }

    fn put(entity: &dyn EntityBase, slot: &EquipmentSlot, stack: ItemStack) {
        entity
            .get_living_entity()
            .unwrap()
            .entity_equipment
            .lock()
            .unwrap()
            .put(slot, stack);
    }

    fn last(living: &LivingEntity) -> [ItemStack; 8] {
        living.last_equipment_items.lock().unwrap().clone()
    }

    fn changes(entity: &dyn EntityBase) -> Option<Vec<(EquipmentSlot, ItemStack)>> {
        let living = entity.get_living_entity().unwrap();
        living.collect_equipment_changes(entity, &last(living), &mut Vec::new())
    }

    #[tokio::test]
    async fn equipment_takes_effect_at_the_next_detection_only() {
        let (_folder, world) = test_world();
        let zombie = spawn(&world, &EntityType::ZOMBIE);
        let living = zombie.get_living_entity().unwrap();
        let head = EquipmentSlot::HEAD.discriminant() as usize;
        let armor = living.get_attribute_value(&Attributes::ARMOR);

        put(
            zombie.as_ref(),
            &EquipmentSlot::HEAD,
            ItemStack::new(1, &Item::IRON_HELMET),
        );
        assert!(last(living)[head].is_empty());
        assert_eq!(living.get_attribute_value(&Attributes::ARMOR), armor);

        living.detect_equipment_updates(zombie.as_ref());
        assert_eq!(last(living)[head].item, &Item::IRON_HELMET);
        assert_eq!(living.get_attribute_value(&Attributes::ARMOR), armor + 2.0);
        assert!(changes(zombie.as_ref()).is_none());

        put(
            zombie.as_ref(),
            &EquipmentSlot::HEAD,
            ItemStack::EMPTY.clone(),
        );
        living.detect_equipment_updates(zombie.as_ref());
        assert_eq!(living.get_attribute_value(&Attributes::ARMOR), armor);
    }

    /// `onEquippedItemBroken` only dirties the attributes; they sync with the next
    /// `sendDirtyEntityData`, not at once.
    #[tokio::test]
    async fn broken_item_leaves_its_attributes_for_the_next_sync() {
        let (_folder, world) = test_world();
        let zombie = spawn(&world, &EntityType::ZOMBIE);
        let living = zombie.get_living_entity().unwrap();
        let helmet = ItemStack::new(1, &Item::IRON_HELMET);
        put(zombie.as_ref(), &EquipmentSlot::HEAD, helmet.clone());
        living.detect_equipment_updates(zombie.as_ref());
        assert!(living.pending_dirty_attributes.lock().unwrap().is_empty());

        living.on_equipped_item_broken(&helmet, &EquipmentSlot::HEAD);
        assert!(
            living
                .pending_dirty_attributes
                .lock()
                .unwrap()
                .contains(&Attributes::ARMOR)
        );
    }

    #[tokio::test]
    async fn swapped_hands_leave_no_change_to_send() {
        let (_folder, world) = test_world();
        let zombie = spawn(&world, &EntityType::ZOMBIE);
        let living = zombie.get_living_entity().unwrap();
        let sword = ItemStack::new(1, &Item::IRON_SWORD);
        let shield = ItemStack::new(1, &Item::SHIELD);
        put(zombie.as_ref(), &EquipmentSlot::MAIN_HAND, sword.clone());
        put(zombie.as_ref(), &EquipmentSlot::OFF_HAND, shield.clone());
        living.detect_equipment_updates(zombie.as_ref());

        put(zombie.as_ref(), &EquipmentSlot::MAIN_HAND, shield.clone());
        put(zombie.as_ref(), &EquipmentSlot::OFF_HAND, sword.clone());
        let mut changed = changes(zombie.as_ref()).unwrap();
        assert_eq!(changed.len(), 2);
        living.handle_hand_swap(&mut changed);
        assert!(changed.is_empty());
        let last = last(living);
        assert!(ItemStack::matches(
            &last[EquipmentSlot::MAIN_HAND.discriminant() as usize],
            &shield
        ));
        assert!(ItemStack::matches(
            &last[EquipmentSlot::OFF_HAND.discriminant() as usize],
            &sword
        ));
    }

    #[tokio::test]
    async fn allay_ignores_a_count_change() {
        let (_folder, world) = test_world();
        let allay = spawn(&world, &EntityType::ALLAY);
        let living = allay.get_living_entity().unwrap();
        put(
            allay.as_ref(),
            &EquipmentSlot::MAIN_HAND,
            ItemStack::new(1, &Item::DIAMOND),
        );
        living.detect_equipment_updates(allay.as_ref());

        put(
            allay.as_ref(),
            &EquipmentSlot::MAIN_HAND,
            ItemStack::new(2, &Item::DIAMOND),
        );
        assert!(changes(allay.as_ref()).is_none());
        let zombie = spawn(&world, &EntityType::ZOMBIE);
        put(
            zombie.as_ref(),
            &EquipmentSlot::MAIN_HAND,
            ItemStack::new(1, &Item::DIAMOND),
        );
        zombie
            .get_living_entity()
            .unwrap()
            .detect_equipment_updates(zombie.as_ref());
        put(
            zombie.as_ref(),
            &EquipmentSlot::MAIN_HAND,
            ItemStack::new(2, &Item::DIAMOND),
        );
        assert!(changes(zombie.as_ref()).is_some());
    }

    // ── bypasses_armor_durability ─────────────────────────────────────

    /// Every member of `minecraft:bypasses_armor` (1.21.11) must return `true`.
    #[test]
    fn bypasses_armor_durability_returns_true_for_tag_members() {
        // Exact contents of the minecraft:bypasses_armor tag in 1.21.11.
        let bypassing: &[DamageType] = &[
            DamageType::ON_FIRE,
            DamageType::IN_WALL,
            DamageType::CRAMMING,
            DamageType::DROWN,
            DamageType::FLY_INTO_WALL,
            DamageType::GENERIC,
            DamageType::WITHER,
            DamageType::DRAGON_BREATH,
            DamageType::STARVE,
            DamageType::FALL,
            DamageType::ENDER_PEARL,
            DamageType::FREEZE,
            DamageType::STALAGMITE,
            DamageType::MAGIC,
            DamageType::INDIRECT_MAGIC,
            DamageType::OUT_OF_WORLD,
            DamageType::GENERIC_KILL,
            DamageType::SONIC_BOOM,
            DamageType::OUTSIDE_BORDER,
        ];
        for dt in bypassing {
            assert!(
                bypasses_armor_durability(dt),
                "{} should bypass armor durability",
                dt.message_id
            );
        }
    }

    /// Physical/combat damage types must NOT bypass armor durability.
    #[test]
    fn bypasses_armor_durability_returns_false_for_physical_sources() {
        let physical: &[DamageType] = &[
            DamageType::MOB_ATTACK,
            DamageType::PLAYER_ATTACK,
            DamageType::ARROW,
            DamageType::CACTUS,
            DamageType::SWEET_BERRY_BUSH,
            DamageType::LAVA,
            DamageType::EXPLOSION,
            DamageType::PLAYER_EXPLOSION,
            DamageType::LIGHTNING_BOLT,
            DamageType::FIREBALL,
            DamageType::THORNS,
            DamageType::TRIDENT,
        ];
        for dt in physical {
            assert!(
                !bypasses_armor_durability(dt),
                "{} should NOT bypass armor durability",
                dt.message_id
            );
        }
    }

    #[test]
    fn hurt_sound_for_entity_uses_zombie_family_sounds() {
        let cases = [
            (&EntityType::ZOMBIE, Sound::EntityZombieHurt),
            (&EntityType::DROWNED, Sound::EntityDrownedHurt),
            (&EntityType::HUSK, Sound::EntityHuskHurt),
            (
                &EntityType::ZOMBIE_VILLAGER,
                Sound::EntityZombieVillagerHurt,
            ),
        ];

        for (entity_type, expected) in cases {
            assert_eq!(LivingEntity::hurt_sound_for_entity(entity_type), expected);
        }
    }

    #[test]
    fn hurt_sound_for_entity_uses_enderman_hurt_sound() {
        assert_eq!(
            LivingEntity::hurt_sound_for_entity(&EntityType::ENDERMAN),
            Sound::EntityEndermanHurt
        );
    }

    #[test]
    fn hurt_sound_for_entity_uses_skeleton_family_sounds() {
        let cases = [
            (&EntityType::SKELETON, Sound::EntitySkeletonHurt),
            (&EntityType::BOGGED, Sound::EntityBoggedHurt),
            (&EntityType::PARCHED, Sound::EntityParchedHurt),
            (
                &EntityType::WITHER_SKELETON,
                Sound::EntityWitherSkeletonHurt,
            ),
            (&EntityType::STRAY, Sound::EntityStrayHurt),
        ];

        for (entity_type, expected) in cases {
            assert_eq!(LivingEntity::hurt_sound_for_entity(entity_type), expected);
        }
    }

    #[test]
    fn hurt_sound_for_entity_defaults_to_generic_hurt() {
        assert_eq!(
            LivingEntity::hurt_sound_for_entity(&EntityType::ITEM),
            Sound::EntityGenericHurt
        );
    }

    #[test]
    fn regeneration_particle_metadata_uses_vanilla_argb_color() {
        let effect = Effect {
            effect_type: &StatusEffect::REGENERATION,
            duration: 200,
            amplifier: 0,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        };
        let metadata = pumpkin_protocol::java::client::play::Metadata::new(
            tracked_data::living_entity::EFFECT_PARTICLES,
            EffectParticles(vec![EffectParticle::from_effect(&effect)]),
        );
        let mut bytes = Vec::new();

        metadata
            .write(
                &mut bytes,
                &pumpkin_util::version::JavaMinecraftVersion::V_26_3,
            )
            .unwrap();

        assert_eq!(bytes, [10, 17, 1, 28, 0xff, 0xcd, 0x5c, 0xab]);
    }

    /// `getSecondsToDisableBlocking` counts the main-hand weapon only while it is the
    /// active item; a warden disables regardless of what it holds.
    #[tokio::test]
    async fn only_the_active_main_hand_weapon_disables_blocking() {
        let (_folder, world) = test_world();
        let vindicator = spawn(&world, &EntityType::VINDICATOR);
        let living = vindicator.get_living_entity().unwrap();
        put(
            vindicator.as_ref(),
            &EquipmentSlot::MAIN_HAND,
            ItemStack::new(1, &Item::IRON_AXE),
        );
        assert_eq!(
            living.get_seconds_to_disable_blocking(vindicator.as_ref()),
            5.0
        );

        living.set_active_hand(Hand::Left, ItemStack::new(1, &Item::SHIELD), 72_000);
        assert_eq!(
            living.get_seconds_to_disable_blocking(vindicator.as_ref()),
            0.0
        );

        living.clear_active_hand();
        living.set_active_hand(Hand::Right, ItemStack::new(1, &Item::IRON_AXE), 72_000);
        assert_eq!(
            living.get_seconds_to_disable_blocking(vindicator.as_ref()),
            5.0
        );

        let warden = spawn(&world, &EntityType::WARDEN);
        let warden_living = warden.get_living_entity().unwrap();
        assert_eq!(
            warden_living.get_seconds_to_disable_blocking(warden.as_ref()),
            5.0
        );
    }
}

/// Vanilla `LivingEntity.SwingState`, without the animation progress only the client and
/// `tickHeadTurn` read.
#[derive(Default)]
struct SwingState {
    /// The `durationTicks` of vanilla's `currentSwing`.
    current_swing_duration: Option<i32>,
    ticks: i32,
}

impl SwingState {
    const fn start_if_able(&mut self, duration: i32) -> bool {
        if let Some(current_duration) = self.current_swing_duration
            && self.ticks <= current_duration / 2
            && self.ticks > 0
        {
            return false;
        }
        self.current_swing_duration = Some(duration);
        self.ticks = 0;
        true
    }

    const fn tick(&mut self) {
        if let Some(duration) = self.current_swing_duration {
            let ticks = self.ticks;
            self.ticks += 1;
            if ticks > duration {
                self.current_swing_duration = None;
            }
        }
    }
}
