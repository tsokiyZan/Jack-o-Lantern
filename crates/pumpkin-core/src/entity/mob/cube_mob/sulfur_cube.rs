//! Vanilla `SulfurCube`: a passive cube mob that swallows a block and takes on its
//! `SulfurCubeArchetype`.

use crate::enchantment::HurtAndBreak;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::{
    BucketEntityDataImpl, EquipmentSlot, SulfurCubeContentImpl,
};
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::sulfur_cube_archetype::{
    ContactDamage, ExplosionData, KnockbackModifiers, SoundSettings, SulfurCubeArchetype,
};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_data::translation;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::GameMode;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::{clamped_map, cos, sin};
use pumpkin_util::random::RandomImpl;
use pumpkin_util::text::TextComponent;

use super::CubeMobEntity;
use crate::block::blocks::redstone::get_best_own_or_neighbour_signal;
use crate::entity::ageable::AgeableMob;
use crate::entity::ai::goal::goal_selector::GoalSelector;
use crate::entity::ai::goal::tempt::{TemptGoal, TemptNavigation};
use crate::entity::ai::goal::{Controls, Goal};
use crate::entity::attributes::{Modifier, ModifierOperation};
use crate::entity::bucketable;
use crate::entity::item::ItemEntity;
use crate::entity::mob::Mob;
use crate::entity::player::Player;
use crate::entity::tnt::UsedPortalExplosionDamageCalculator;
use crate::entity::{Entity, EntityBase};
use crate::world::explosion::{ExplosionDamageCalculator, ExplosionEffects, ExplosionInteraction};

pub(super) const MIN_SIZE: i32 = 1;
pub(super) const MAX_SIZE: i32 = 2;
const SPLIT_COUNT: i32 = 2;
const PICKUP_TIMER_DURATION: i32 = 100;
const PUSH_DISTANCE_THRESHOLD: f32 = 1.3;
const MAX_PLAYER_PUSH_SPEED: f64 = 0.5;
const PLAYER_PUSH_SPEED_SCALE_MULTIPLIER: f32 = 0.3;
const VEHICLE_PUSH_SPEED_SCALE_MULTIPLIER: f32 = 0.16;
const VERTICAL_PUSH_MULTIPLIER: f32 = 0.3;
const HORIZONTAL_HIT_ANGLE_SCALE: f32 = 1.6;
const VERTICAL_HIT_ANGLE_SCALE: f32 = 0.5;
const VERTICAL_POSITION_ANGLE_SCALE: f32 = 0.8;
const EXTRA_KNOCKBACK_DAMPENING: f32 = 0.25;
/// How far `SulfurCubeSearchForItemsGoal` looks for items.
const ITEM_SEARCH_RANGE: f64 = 8.0;
/// Vanilla `SulfurCubeArchetype.DEFAULT_KNOCKBACK_MODIFIERS`.
const DEFAULT_KNOCKBACK_MODIFIERS: KnockbackModifiers = KnockbackModifiers {
    horizontal_power: 0.33,
    vertical_power: 0.06,
};
/// Vanilla `SulfurCubeArchetype.DEFAULT_SOUND_SETTINGS`.
const DEFAULT_SOUND_SETTINGS: SoundSettings = SoundSettings {
    hit_sound: Sound::EntitySulfurCubeRegularHit,
    push_sound: Sound::EntitySulfurCubeRegularPush,
    push_sound_impulse_threshold: 0.2,
    push_sound_cooldown: 0.5,
};
/// Vanilla `AbstractArrow` and its subclasses.
const ABSTRACT_ARROWS: [&EntityType; 3] = [
    &EntityType::ARROW,
    &EntityType::SPECTRAL_ARROW,
    &EntityType::TRIDENT,
];

/// What the swallowed block's archetypes make of the cube.
#[derive(Clone)]
struct ArchetypeState {
    floats_in_liquids: bool,
    explosion_data: Option<ExplosionData>,
    knockback_modifier: KnockbackModifiers,
    sound_settings: SoundSettings,
    contact_damages: Vec<ContactDamage>,
}

impl Default for ArchetypeState {
    fn default() -> Self {
        Self {
            floats_in_liquids: false,
            explosion_data: None,
            knockback_modifier: DEFAULT_KNOCKBACK_MODIFIERS,
            sound_settings: DEFAULT_SOUND_SETTINGS,
            contact_damages: Vec::new(),
        }
    }
}

pub(super) struct SulfurCubeData {
    pickup_timer: AtomicI32,
    push_sound_cooldown: AtomicI32,
    fuse: AtomicI32,
    /// Vanilla `FROM_BUCKET`.
    from_bucket: AtomicBool,
    archetype: Mutex<ArchetypeState>,
}

impl Default for SulfurCubeData {
    fn default() -> Self {
        Self {
            pickup_timer: AtomicI32::new(0),
            push_sound_cooldown: AtomicI32::new(0),
            fuse: AtomicI32::new(-1),
            from_bucket: AtomicBool::new(false),
            archetype: Mutex::new(ArchetypeState::default()),
        }
    }
}

impl SulfurCubeData {
    fn archetype(&self) -> MutexGuard<'_, ArchetypeState> {
        self.archetype
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn is_primed(&self) -> bool {
        self.fuse.load(Relaxed) >= 0
    }

    /// Vanilla `SulfurCube.customServerAiStep`.
    pub(super) fn custom_server_ai_step(&self) {
        if self.pickup_timer.load(Relaxed) > 0 {
            self.pickup_timer.fetch_sub(1, Relaxed);
        }
        if self.push_sound_cooldown.load(Relaxed) > 0 {
            self.push_sound_cooldown.fetch_sub(1, Relaxed);
        }
    }

    /// Vanilla `SulfurCube.getSplitCount`.
    pub(super) fn split_count(&self) -> i32 {
        if self.is_primed() { 0 } else { SPLIT_COUNT }
    }
}

/// Vanilla `SulfurCube.isSwallowableItem`.
fn is_swallowable_item(stack: &ItemStack) -> bool {
    stack
        .item
        .has_tag(&tag::Item::MINECRAFT_SULFUR_CUBE_SWALLOWABLE)
}

/// Vanilla `SulfurCube.isFood`.
fn is_food(stack: &ItemStack) -> bool {
    stack.item.has_tag(&tag::Item::MINECRAFT_SULFUR_CUBE_FOOD)
}

/// Vanilla `SulfurCube.matchingArchetypes`.
fn matching_archetypes(stack: &ItemStack) -> impl Iterator<Item = &'static SulfurCubeArchetype> {
    let empty = stack.is_empty();
    let item = stack.item;
    SulfurCubeArchetype::ALL
        .iter()
        .copied()
        .filter(move |archetype| !empty && item.has_tag(archetype.items))
}

/// Vanilla `Vec2.rotate`.
fn rotate(x: f32, y: f32, angle: f32) -> (f32, f32) {
    let cosine = cos(angle);
    let sine = sin(angle);
    (x * cosine - y * sine, y * cosine + x * sine)
}

/// Vanilla `SulfurCube.applyHorizontalHitAngleScale`.
fn apply_horizontal_hit_angle_scale(
    horizontal_angle_scale: f32,
    original_angle: (f32, f32),
    attacker_position: Vector3<f64>,
    attacker_aim_direction: Vector3<f64>,
    target_center: Vector3<f64>,
) -> (f32, f32) {
    let attacker_to_target = (target_center - attacker_position).normalize();
    let angle_diff = (attacker_aim_direction.x * attacker_to_target.z
        - attacker_aim_direction.z * attacker_to_target.x)
        .atan2(
            attacker_aim_direction.x * attacker_to_target.x
                + attacker_aim_direction.z * attacker_to_target.z,
        ) as f32;
    rotate(
        original_angle.0,
        original_angle.1,
        angle_diff * horizontal_angle_scale,
    )
}

/// Vanilla `SulfurCube.applyVerticalHitAnglePowerTransfer`.
fn apply_vertical_hit_angle_power_transfer(
    vertical_hit_angle_scale: f32,
    horizontal_power: f32,
    vertical_power: f32,
    attacker_position: Vector3<f64>,
    attacker_aim_direction: Vector3<f64>,
    target_centered_position: Vector3<f64>,
    target_height: f32,
) -> (f32, f32) {
    let target_half_height = f64::from(0.5 * target_height);
    let target_top_pos = target_centered_position.add_raw(0.0, target_half_height, 0.0);
    let target_bottom_pos = target_centered_position.add_raw(0.0, -target_half_height, 0.0);
    let attacker_to_target_top = (target_top_pos - attacker_position).normalize();
    let attacker_to_target_bottom = (target_bottom_pos - attacker_position).normalize();
    let vertical_hit_angle_factor = clamped_map(
        attacker_aim_direction.y,
        attacker_to_target_top.y,
        attacker_to_target_bottom.y,
        -1.0,
        1.0,
    ) as f32;
    let mut transferred_power_ratio = (vertical_hit_angle_factor * vertical_hit_angle_scale).abs();
    if vertical_hit_angle_factor < 0.0 {
        transferred_power_ratio = -transferred_power_ratio;
    }
    (
        horizontal_power * (1.0 - transferred_power_ratio),
        vertical_power * (1.0 + transferred_power_ratio),
    )
}

/// Vanilla `SulfurCube.applyVerticalPositionAnglePowerRotation`.
fn apply_vertical_position_angle_power_rotation(
    vertical_position_angle_scale: f32,
    horizontal_power: f32,
    vertical_power: f32,
    original_horizontal_power: f32,
    original_vertical_power: f32,
    attacker_feet_position: Vector3<f64>,
    target_feet_position: Vector3<f64>,
) -> (f32, f32) {
    let attacker_feet_to_target_feet = target_feet_position - attacker_feet_position;
    let vertical_position_angle = (-attacker_feet_to_target_feet.y)
        .atan2(attacker_feet_to_target_feet.horizontal_length())
        as f32;
    let mut rotated_power = rotate(
        horizontal_power,
        vertical_power,
        -vertical_position_angle * vertical_position_angle_scale,
    );
    let horizontal_ratio = if original_horizontal_power > 0.0 {
        rotated_power.0.abs() / original_horizontal_power
    } else {
        0.0
    };
    let vertical_ratio = if original_vertical_power > 0.0 {
        rotated_power.1.abs() / original_vertical_power
    } else {
        0.0
    };
    let max_ratio = horizontal_ratio.max(vertical_ratio);
    if max_ratio > 1.0 {
        let scale = 1.0 / max_ratio;
        rotated_power = (rotated_power.0 * scale, rotated_power.1 * scale);
    }
    rotated_power
}

impl CubeMobEntity {
    pub(super) fn body_item(&self) -> ItemStack {
        self.mob_entity.get_item_slot(&EquipmentSlot::BODY)
    }

    /// Vanilla `SulfurCube.hasBodyItem`; false for the other cube mobs.
    pub fn has_body_item(&self) -> bool {
        self.sulfur.is_some() && !self.body_item().is_empty()
    }

    pub(super) fn is_from_bucket(&self) -> bool {
        self.sulfur
            .as_ref()
            .is_some_and(|sulfur| sulfur.from_bucket.load(Relaxed))
    }

    fn set_from_bucket(&self, sulfur: &SulfurCubeData, from_bucket: bool) {
        sulfur.from_bucket.store(from_bucket, Relaxed);
        self.get_entity()
            .set_synced_data(tracked_data::sulfur_cube::FROM_BUCKET, from_bucket);
    }

    /// Vanilla `SulfurCube.isPrimed`.
    #[must_use]
    pub fn is_primed(&self) -> bool {
        self.sulfur.as_ref().is_some_and(SulfurCubeData::is_primed)
    }

    /// Vanilla `SulfurCube.canExplode`.
    #[must_use]
    pub fn can_explode(&self) -> bool {
        self.sulfur.as_ref().is_some_and(|sulfur| {
            sulfur.archetype().explosion_data.is_some()
                && self.mob_entity.living_entity.is_alive()
                && !sulfur.is_primed()
        })
    }

    /// `setFuse` and the `MAX_FUSE` it is synced through.
    fn set_fuse(&self, sulfur: &SulfurCubeData, fuse: i32) {
        sulfur.fuse.store(fuse, Relaxed);
        self.get_entity()
            .set_synced_data(tracked_data::sulfur_cube::MAX_FUSE, fuse);
    }

    /// Vanilla `SulfurCube.tickFuse`.
    pub(super) fn tick_fuse(&self) {
        let Some(sulfur) = &self.sulfur else {
            return;
        };
        if sulfur.fuse.load(Relaxed) > 0 {
            sulfur.fuse.fetch_sub(1, Relaxed);
        }
        let Some(explosion) = sulfur.archetype().explosion_data else {
            return;
        };
        if sulfur.fuse.load(Relaxed) != 0 {
            return;
        }
        self.drop_leash();
        let living = &self.mob_entity.living_entity;
        living.dead.store(true, Relaxed);
        let entity = self.get_entity();
        let world = entity.world.load_full();
        let game_rules = &world.level_info.load().game_rules;
        if game_rules.tnt_explodes {
            let interaction = if game_rules.mob_griefing {
                ExplosionInteraction::Tnt
            } else {
                ExplosionInteraction::None
            };
            let calculator: Option<Arc<dyn ExplosionDamageCalculator>> =
                (entity.portal_cooldown.load(Relaxed) > 0)
                    .then(|| Arc::new(UsedPortalExplosionDamageCalculator) as _);
            let pos = entity.pos.load();
            // `getY(0.0625)`.
            let y = pos.y + f64::from(entity.height()) * 0.0625;
            world.explode_full(
                world.get_entity_by_id(entity.entity_id),
                None,
                calculator,
                Vector3::new(pos.x, y, pos.z),
                explosion.power as f32,
                explosion.causes_fire,
                interaction,
                ExplosionEffects::DEFAULT,
            );
        }
        // `triggerOnDeathMobEffects(level, KILLED)`, which then clears the effects.
        living.trigger_on_death_mob_effects();
        living
            .active_effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        entity.remove();
    }

    /// Vanilla `SulfurCube.primeWhenOnPoweredPosition`.
    pub(super) fn prime_when_on_powered_position(&self) {
        if !self.can_explode() {
            return;
        }
        let entity = self.get_entity();
        let here = BlockPos::floored_v(entity.pos.load());
        if get_best_own_or_neighbour_signal(&entity.world.load(), &here) != 0 {
            self.prime_time(false);
        }
    }

    /// Vanilla `SulfurCube.primeTime`: lights the swallowed explosive.
    pub fn prime_time(&self, imminent: bool) -> bool {
        let Some(sulfur) = &self.sulfur else {
            return false;
        };
        let Some(explosion) = sulfur.archetype().explosion_data else {
            return false;
        };
        let entity = self.get_entity();
        if !self.mob_entity.living_entity.is_alive()
            || !entity
                .world
                .load()
                .level_info
                .load()
                .game_rules
                .tnt_explodes
            || sulfur.is_primed()
        {
            return false;
        }
        let fuse = explosion.fuse;
        let fuse_time = if imminent {
            // `PrimedTnt.getRandomShortFuse(fuse, this.getRandom())`.
            entity.random().next_bounded_i32((fuse / 4).max(1)) + fuse / 8
        } else {
            fuse
        };
        entity.set_invulnerable(true);
        self.set_fuse(sulfur, fuse_time);
        self.make_sound(Sound::EntityTntPrimed);
        entity
            .world
            .load()
            .game_event_from(Some(self), GameEvent::PrimeFuse, entity.pos.load());
        true
    }

    /// The part of vanilla `SulfurCube.collectEquipmentChanges` before
    /// `super.collectEquipmentChanges`: a changed body item swaps the archetype.
    pub(super) fn collect_body_equipment_changes(
        &self,
        last: &[ItemStack; 8],
        touched: &mut Vec<Attributes>,
    ) {
        let Some(sulfur) = &self.sulfur else {
            return;
        };
        let previous = &last[EquipmentSlot::BODY.discriminant() as usize];
        let current = self.body_item();
        if !self.equipment_has_changed(previous, &current) {
            return;
        }
        if current.is_empty() {
            self.register_goals();
        } else {
            // `removeAllGoals(g -> true)`.
            let running = self
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            for mut goal in running {
                goal.stop(self);
            }
            self.mob_entity.set_speed(0.0);
        }

        let living = &self.mob_entity.living_entity;
        let mut touch = |attribute: &Attributes| {
            if !touched.iter().any(|touched| touched.id == attribute.id) {
                touched.push(attribute.clone());
            }
        };
        for archetype in matching_archetypes(previous) {
            for entry in archetype.attribute_modifiers {
                if living.modify_supplied_attribute(entry.attribute, |instance| {
                    let present = instance.get_modifier(entry.id).is_some();
                    instance.remove_modifier(entry.id);
                    present
                }) {
                    touch(entry.attribute);
                }
            }
        }

        let mut state = ArchetypeState::default();
        for archetype in matching_archetypes(&current) {
            if archetype.buoyant {
                state.floats_in_liquids = true;
            }
            if archetype.explosion.is_some() {
                state.explosion_data = archetype.explosion;
            }
            if let Some(contact_damage) = &archetype.contact_damage {
                state.contact_damages.push(contact_damage.clone());
            }
            state.knockback_modifier = archetype.knockback_modifiers;
            state.sound_settings = archetype.sound_settings;
            for entry in archetype.attribute_modifiers {
                if living.modify_supplied_attribute(entry.attribute, |instance| {
                    instance.add_or_replace_modifier(Modifier {
                        id: entry.id.to_string(),
                        amount: entry.amount,
                        operation: ModifierOperation::from_operation(entry.operation),
                    });
                    true
                }) {
                    touch(entry.attribute);
                }
            }
        }
        *sulfur.archetype() = state;
    }

    /// Vanilla `SulfurCube.equipItem`: swallows one of `held`, spitting out what it held.
    pub fn equip_item(&self, held: &ItemStack) -> bool {
        if self.sulfur.is_none() || self.is_baby() {
            return false;
        }
        let swallowed = self.body_item();
        if !swallowed.is_empty() {
            if held.item == swallowed.item {
                return false;
            }
            self.spawn_at_passenger_attachment(swallowed.clone());
        }
        self.mob_entity
            .set_item_slot_and_drop_when_killed(&EquipmentSlot::BODY, held.copy_with_count(1));
        if !swallowed.is_empty() {
            let mut last: [ItemStack; 8] = std::array::from_fn(|_| ItemStack::EMPTY.clone());
            last[EquipmentSlot::BODY.discriminant() as usize] = swallowed;
            // Vanilla ignores the result; the attributes it touched stay dirty until synced.
            let living = &self.mob_entity.living_entity;
            let mut touched = Vec::new();
            living.collect_equipment_changes(self, &last, &mut touched);
            living.mark_attributes_dirty(&touched);
        }
        self.play_sound(Sound::EntitySulfurCubeAbsorb, 1.0, 1.0);
        true
    }

    /// Vanilla `spawnAtLocation` with `getAttachments().getAverage(PASSENGER)` as the offset:
    /// the top of the cube.
    fn spawn_at_passenger_attachment(&self, stack: ItemStack) {
        if stack.is_empty() {
            return;
        }
        let entity = self.get_entity();
        let world = entity.world.load_full();
        let pos = entity
            .pos
            .load()
            .add_raw(0.0, f64::from(entity.height()), 0.0);
        let item = ItemEntity::new(Entity::new(world.clone(), pos, &EntityType::ITEM), stack);
        world.spawn_entity(Arc::new(item));
    }

    /// Vanilla `Leashable.dropLeash`.
    fn drop_leash(&self) {
        let entity = self.get_entity();
        if !entity.is_leashed() {
            return;
        }
        entity.unleash();
        entity
            .world
            .load()
            .drop_stack(&entity.block_pos.load(), ItemStack::new(1, &Item::LEAD));
    }

    /// Vanilla `SulfurCube.applyContactDamage`.
    pub(super) fn apply_contact_damage(&self, entity: &dyn EntityBase) {
        let Some(sulfur) = &self.sulfur else {
            return;
        };
        let contact_damages = sulfur.archetype().contact_damages.clone();
        for damage in contact_damages {
            let source = (damage.attribute_to_source || entity.get_player().is_some())
                .then_some(self as &dyn EntityBase);
            let amount = damage.amount.get(&mut *self.get_entity().random());
            entity.damage_with_context(entity, amount, damage.damage_type, None, source, source);
        }
    }

    /// Vanilla `SulfurCube.shear`: spits out the swallowed block.
    pub fn shear(&self) {
        let Some(sulfur) = &self.sulfur else {
            return;
        };
        let stack = self.body_item();
        self.mob_entity
            .set_item_slot(&EquipmentSlot::BODY, ItemStack::EMPTY.clone());
        self.spawn_at_passenger_attachment(stack);
        self.play_sound(Sound::EntitySulfurCubeEject, 1.0, 1.0);
        sulfur.pickup_timer.store(PICKUP_TIMER_DURATION, Relaxed);
    }

    /// Vanilla `SulfurCube.readyForShearing`.
    #[must_use]
    pub fn ready_for_shearing(&self) -> bool {
        self.has_body_item()
    }

    /// Vanilla `SulfurCube.canHoldItem`.
    pub(super) fn sulfur_can_hold_item(&self, stack: &ItemStack) -> bool {
        self.body_item().is_empty() && is_swallowable_item(stack) && !self.is_baby()
    }

    /// Vanilla `SulfurCube.pickUpItem`, which runs while `canPickUpLoot` (an empty body)
    /// and `wantsToPickUp` (`canHoldItem`) allow it.
    pub(super) fn sulfur_pick_up_item(&self, item: &ItemEntity) {
        let Some(sulfur) = &self.sulfur else {
            return;
        };
        if self.has_body_item() {
            return;
        }
        let stack = item
            .get_item_stack()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if !self.sulfur_can_hold_item(&stack) || sulfur.pickup_timer.load(Relaxed) > 0 {
            return;
        }
        // `take` tells the clients before the item goes.
        if !self.mob_entity.living_entity.pickup(item.get_entity(), 1) {
            return;
        }
        let swallowed = item
            .get_item_stack()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .split(1);
        let emptied = item
            .get_item_stack()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty();
        if emptied {
            item.get_entity().remove();
        } else {
            item.init_data_tracker();
        }
        self.set_item_slot_and_notify(&EquipmentSlot::BODY, swallowed);
        self.play_sound(Sound::EntitySulfurCubeAbsorb, 1.0, 1.0);
        self.mob_entity.set_guaranteed_drop(&EquipmentSlot::BODY);
    }

    /// Vanilla `SulfurCube.getBaseExperienceReward`.
    pub(super) fn sulfur_base_experience_reward(&self) -> u32 {
        if self.is_baby() {
            0
        } else {
            1 + self.get_entity().random().next_bounded_i32(2) as u32
        }
    }

    /// Vanilla `SulfurCube.addAdditionalSaveData`.
    pub(super) fn sulfur_write_nbt(&self, nbt: &mut NbtCompound) {
        let Some(sulfur) = &self.sulfur else {
            return;
        };
        nbt.put_int("pickup_timer", sulfur.pickup_timer.load(Relaxed));
        nbt.put_bool("from_bucket", sulfur.from_bucket.load(Relaxed));
        nbt.put_int("fuse", sulfur.fuse.load(Relaxed));
    }

    /// Vanilla `SulfurCube.readAdditionalSaveData`, before its `super` call.
    pub(super) fn sulfur_read_nbt(&self, nbt: &NbtCompound) {
        let Some(sulfur) = &self.sulfur else {
            return;
        };
        sulfur
            .pickup_timer
            .store(nbt.get_int("pickup_timer").unwrap_or(0), Relaxed);
        self.set_from_bucket(sulfur, nbt.get_bool("from_bucket").unwrap_or(false));
        self.set_fuse(sulfur, nbt.get_int("fuse").unwrap_or(-1));
    }

    /// Vanilla `Player.touch` reaches every entity in the player's pickup area, but Pumpkin
    /// only touches the entities a player's box overlaps, so the cube looks for the players
    /// whose pickup area it is in.
    pub(super) fn touch_nearby_players(&self) {
        let entity = self.get_entity();
        let world = entity.world.load();
        let cube_box = entity.bounding_box.load();
        let players = world.players.load();
        for player in players.iter() {
            let living = &player.living_entity;
            if player.is_spectator() || living.health.load() <= 0.0 {
                continue;
            }
            let player_entity = player.get_entity();
            let player_box = player_entity.bounding_box.load();
            let pickup_area = match player_entity.get_vehicle() {
                Some(vehicle) if !vehicle.get_entity().is_removed() => {
                    // `AABB.minmax`.
                    let vehicle_box = vehicle.get_entity().bounding_box.load();
                    BoundingBox {
                        min: Vector3::new(
                            player_box.min.x.min(vehicle_box.min.x),
                            player_box.min.y.min(vehicle_box.min.y),
                            player_box.min.z.min(vehicle_box.min.z),
                        ),
                        max: Vector3::new(
                            player_box.max.x.max(vehicle_box.max.x),
                            player_box.max.y.max(vehicle_box.max.y),
                            player_box.max.z.max(vehicle_box.max.z),
                        ),
                    }
                    .expand(1.0, 0.0, 1.0)
                }
                _ => player_box.expand(1.0, 0.5, 1.0),
            };
            if pickup_area.intersects(&cube_box) {
                self.player_push(player);
            }
        }
    }

    /// Vanilla `SulfurCube.playerPush`: a player walking into a cube holding a block shoves it.
    fn player_push(&self, player: &Player) {
        let Some(sulfur) = &self.sulfur else {
            return;
        };
        if !self.has_body_item() {
            return;
        }
        let player_entity = player.get_entity();
        let is_passenger = player_entity.has_vehicle();
        let mut root_vehicle = player_entity.get_vehicle();
        while let Some(vehicle) = root_vehicle
            .as_ref()
            .and_then(|vehicle| vehicle.get_entity().get_vehicle())
        {
            root_vehicle = Some(vehicle);
        }
        let pusher = root_vehicle
            .as_ref()
            .map_or(player_entity, |vehicle| vehicle.get_entity());

        let entity = self.get_entity();
        let cube_to_pusher = entity.pos.load() - pusher.pos.load();
        let pusher_feet_position = pusher.pos.load().y;
        let sulfur_cube_bottom_position = entity.pos.load().y;
        let sulfur_cube_top_position = sulfur_cube_bottom_position + f64::from(entity.height());
        let pusher_top_position = pusher_feet_position + f64::from(pusher.height());
        if cube_to_pusher.horizontal_length() >= f64::from(PUSH_DISTANCE_THRESHOLD)
            || pusher_feet_position > sulfur_cube_top_position
            || pusher_top_position <= sulfur_cube_bottom_position
        {
            return;
        }
        let living = &self.mob_entity.living_entity;
        let knockback =
            (1.0 - living.get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE)).max(0.0);
        let push_direction =
            Vector3::new(cube_to_pusher.x, 0.0, cube_to_pusher.z).normalize() * knockback;
        let push_speed_scale = if is_passenger {
            VEHICLE_PUSH_SPEED_SCALE_MULTIPLIER
        } else {
            PLAYER_PUSH_SPEED_SCALE_MULTIPLIER
        };
        // `getKnownSpeed`: how far the player moved last tick.
        let player_speed =
            (player_entity.movement.load().length() * 2.0 * f64::from(push_speed_scale))
                .clamp(0.0, MAX_PLAYER_PUSH_SPEED);
        let vertical = if entity.on_ground.load(Relaxed) {
            knockback * f64::from(VERTICAL_PUSH_MULTIPLIER)
        } else {
            0.0
        };
        let push_velocity =
            Vector3::new(push_direction.x, vertical, push_direction.z) * player_speed;
        let sound_settings = sulfur.archetype().sound_settings;
        let threshold = sound_settings.push_sound_impulse_threshold;
        if push_velocity.length_squared() > f64::from(threshold * threshold)
            && sulfur.push_sound_cooldown.load(Relaxed) <= 0
        {
            sulfur
                .push_sound_cooldown
                .store((sound_settings.push_sound_cooldown * 20.0) as i32, Relaxed);
            self.play_sound(sound_settings.push_sound, 1.0, 1.0);
        }
        entity
            .velocity
            .store(entity.velocity.load() + push_velocity);
        entity.velocity_dirty.store(true, Relaxed);
        self.apply_contact_damage(player);
    }

    /// Vanilla `SulfurCube.knockback` while it holds a block. Returns false where vanilla
    /// falls back to `super.knockback`.
    pub(super) fn sulfur_knockback(
        &self,
        power: f64,
        xd: f64,
        zd: f64,
        attacker: Option<&Entity>,
        damage: f32,
        comes_from_effect: bool,
    ) -> bool {
        let (Some(sulfur), Some(attacker)) = (&self.sulfur, attacker) else {
            return false;
        };
        if !self.has_body_item() {
            return false;
        }
        let (knockback_modifier, hit_sound) = {
            let state = sulfur.archetype();
            (state.knockback_modifier, state.sound_settings.hit_sound)
        };
        let original_horizontal_power = knockback_modifier.horizontal_power;
        let original_vertical_power = knockback_modifier.vertical_power;
        let entity = self.get_entity();
        let center = entity.bounding_box.load().get_center();
        let eye_position = attacker.get_eye_pos();
        let look_angle = attacker.get_look_angle().normalize();
        let new_angle = apply_horizontal_hit_angle_scale(
            HORIZONTAL_HIT_ANGLE_SCALE,
            (xd as f32, zd as f32),
            eye_position,
            look_angle,
            center,
        );
        let (horizontal_power, vertical_power) = apply_vertical_hit_angle_power_transfer(
            VERTICAL_HIT_ANGLE_SCALE,
            original_horizontal_power,
            original_vertical_power,
            eye_position,
            look_angle,
            center,
            entity.height(),
        );
        let (mut horizontal_power, mut vertical_power) =
            apply_vertical_position_angle_power_rotation(
                VERTICAL_POSITION_ANGLE_SCALE,
                horizontal_power,
                vertical_power,
                original_horizontal_power,
                original_vertical_power,
                attacker.pos.load(),
                entity.pos.load(),
            );
        let xd = f64::from(new_angle.0);
        let zd = f64::from(new_angle.1);
        let power_multiplier = damage.sqrt()
            * if comes_from_effect {
                power as f32 * EXTRA_KNOCKBACK_DAMPENING
            } else {
                1.0
            };
        horizontal_power *= power_multiplier;
        vertical_power *= power_multiplier;
        let knockback_resistance = self
            .mob_entity
            .living_entity
            .get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE);
        horizontal_power *= (1.0 - knockback_resistance) as f32;
        vertical_power *= (1.0 - knockback_resistance) as f32;
        let velocity = entity.velocity.load();
        horizontal_power *= 0.4;
        let horizontal_power = horizontal_power.clamp(-128.0, 128.0);
        let vertical_power = vertical_power.clamp(-128.0, 128.0);
        let horizontal_knockback =
            Vector3::new(xd, 0.0, zd).normalize() * f64::from(horizontal_power);
        entity.velocity.store(Vector3::new(
            velocity.x - horizontal_knockback.x,
            velocity.y + f64::from(vertical_power) * 1.2,
            velocity.z - horizontal_knockback.z,
        ));
        entity.velocity_dirty.store(true, Relaxed);
        self.play_sound(hit_sound, 1.0, 1.0);
        true
    }

    /// The part of vanilla `SulfurCube.hurtServer` before `super.hurtServer`. Returns true
    /// when the swallowed block shrugs the hit off.
    pub(super) fn sulfur_hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if !self.has_body_item() {
            return false;
        }
        if self.can_explode() && !self.is_primed() {
            let burning_arrow = source.is_some_and(|source| {
                let source = source.get_entity();
                ABSTRACT_ARROWS.contains(&source.entity_type) && source.is_on_fire()
            });
            if damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FIRE) || burning_arrow {
                self.prime_time(false);
            } else if damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_EXPLOSION) {
                self.prime_time(true);
            }
        }
        if !damage_type.has_tag(&tag::DamageType::MINECRAFT_SULFUR_CUBE_WITH_BLOCK_IMMUNE_TO) {
            return false;
        }
        if !damage_type.has_tag(&tag::DamageType::MINECRAFT_NO_KNOCKBACK) {
            // `dealDefaultKnockback(source, damage, true)`.
            let living = &self.mob_entity.living_entity;
            let (xd, zd) = living.default_knockback_direction(source, position);
            living.knockback(caller, 0.4, xd, zd, cause, amount, false);
        }
        true
    }

    /// Vanilla `SulfurCube.mobInteract`, after `Entity.interact`'s leash handling.
    pub(super) fn sulfur_mob_interact(&self, player: &Arc<Player>, stack: &mut ItemStack) -> bool {
        let entity = self.get_entity();
        let game_mode = player.gamemode.load();
        if self.is_baby() {
            if is_food(stack) && self.can_age_up() {
                let age = self.get_age();
                stack.decrement_unless_creative(game_mode, 1);
                self.age_up(Self::get_speed_up_seconds_when_feeding(-age), true);
                self.make_sound(Sound::EntitySmallSulfurCubeEat);
                return true;
            }
            return false;
        }

        if self.is_primed() {
            return false;
        }

        let igniter = stack.item == &Item::FLINT_AND_STEEL || stack.item == &Item::FIRE_CHARGE;
        if self.can_explode() && igniter {
            let world = entity.world.load();
            if !world.level_info.load().game_rules.tnt_explodes {
                player.send_system_message_raw(
                    &TextComponent::translate(translation::java::BLOCK_MINECRAFT_TNT_DISABLED, []),
                    true,
                );
                return false;
            }
            self.prime_time(false);
            if game_mode != GameMode::Creative {
                if stack.item == &Item::FLINT_AND_STEEL {
                    let _ =
                        stack.hurt_and_break(&self.mob_entity.living_entity.entity.world.load(), 1);
                } else {
                    stack.decrement(1);
                }
            }
            return true;
        }

        if stack.item == &Item::SHEARS && self.ready_for_shearing() {
            self.shear();
            entity.world.load().game_event_from(
                Some(player.as_ref()),
                GameEvent::Shear,
                entity.pos.load(),
            );
            if game_mode != GameMode::Creative {
                let _ = stack.hurt_and_break(&self.mob_entity.living_entity.entity.world.load(), 1);
            }
            return true;
        }

        if is_swallowable_item(stack) {
            let worked = self.equip_item(stack);
            if worked {
                stack.decrement_unless_creative(game_mode, 1);
                entity.world.load().game_event_from(
                    Some(self),
                    GameEvent::EntityInteract,
                    entity.pos.load(),
                );
            }
            return worked;
        }

        self.bucket_mob_pickup(player, stack)
    }

    /// Vanilla `Bucketable.bucketMobPickup`, for `canBePickedUpWithBucket`'s empty bucket.
    fn bucket_mob_pickup(&self, player: &Player, stack: &mut ItemStack) -> bool {
        if stack.item != &Item::BUCKET || !self.mob_entity.living_entity.is_alive() {
            return false;
        }
        self.play_sound(Sound::ItemBucketFillSulfurCube, 1.0, 1.0);
        let mut bucket = ItemStack::new(1, &Item::SULFUR_CUBE_BUCKET);
        self.save_to_bucket_tag(&mut bucket);
        crate::item::create_filled_result(stack, player, bucket, false);
        self.drop_leash();
        self.get_entity().remove();
        true
    }

    /// Vanilla `SulfurCube.saveToBucketTag`.
    fn save_to_bucket_tag(&self, bucket: &mut ItemStack) {
        let mut tag = bucketable::save_default_data_to_bucket_tag(self, bucket);
        let body = self.body_item();
        if !body.is_empty() {
            bucket.set_data_component(SulfurCubeContentImpl {
                absorbed_block: body.copy_with_count(1),
            });
        }
        tag.put_int("age", self.get_age());
        tag.put_bool("age_locked", self.is_age_locked());
        bucket.set_data_component(BucketEntityDataImpl { nbt: Some(tag) });
    }

    /// Vanilla `MobBucketItem.spawn` for a cube poured out of `bucket`: its implicit
    /// `SULFUR_CUBE_CONTENT`, then `loadFromBucketTag` and `setFromBucket`.
    pub fn load_from_bucket(&self, bucket: &ItemStack) {
        let Some(sulfur) = &self.sulfur else {
            return;
        };
        if let Some(content) = bucket.get_data_component::<SulfurCubeContentImpl>() {
            // `setSulfurCubeContent`.
            self.mob_entity.set_item_slot_and_drop_when_killed(
                &EquipmentSlot::BODY,
                content.absorbed_block.copy_with_count(1),
            );
        }
        let tag = bucket
            .get_data_component::<BucketEntityDataImpl>()
            .map(BucketEntityDataImpl::nbt)
            .unwrap_or_default();
        bucketable::load_default_data_from_bucket_tag(self, &tag);
        self.set_age(tag.get_int("age").unwrap_or(0));
        self.set_age_locked(tag.get_bool("age_locked").unwrap_or(false));
        self.set_from_bucket(sulfur, true);
    }

    /// The end of vanilla `SulfurCube.travelInFluid`: a buoyant block floats the cube.
    pub(super) fn float_in_fluid(&self, water: bool) {
        let Some(sulfur) = &self.sulfur else {
            return;
        };
        if !self.has_body_item() || !sulfur.archetype().floats_in_liquids {
            return;
        }
        let entity = self.get_entity();
        let vibe_amount = 0.2 * sin(entity.tick_count.load(Relaxed) as f32 * 0.4);
        let fluid_height = if water {
            entity.water_height.load()
        } else {
            entity.lava_height.load()
        };
        let immersion = fluid_height - f64::from(entity.height()) * 0.2 + f64::from(vibe_amount);
        if immersion > 0.0 {
            let velocity = entity.velocity.load();
            entity.velocity.store(velocity.add_raw(
                0.0,
                immersion.min(1.0) * f64::from(0.04f32),
                0.0,
            ));
        }
    }
}

/// Vanilla `SulfurCube.addBehaviourGoals`.
pub(super) fn add_behaviour_goals(goals: &mut GoalSelector, cube: &Weak<CubeMobEntity>) {
    let stop_cube = cube.clone();
    let towards_cube = cube.clone();
    let tempt = TemptGoal::for_non_pathfinders(1.0, is_tempted_by, false, 1.0)
        .with_controls(Controls::LOOK)
        .with_navigation(TemptNavigation {
            // `SulfurCubeTemptGoal.stopNavigation`.
            stop: Box::new(move |_mob| {
                if let Some(cube) = stop_cube.upgrade() {
                    cube.set_wanted_movement(0.0);
                }
            }),
            // `SulfurCubeTemptGoal.navigateTowards`.
            towards: Box::new(move |_mob, player| {
                if let Some(cube) = towards_cube.upgrade() {
                    cube.mob_entity.look_at(player, 10.0, 10.0);
                    cube.set_direction(cube.get_entity().yaw.load(), true);
                }
            }),
        });
    goals.add_goal(2, Box::new(tempt));
    goals.add_goal(
        3,
        Box::new(SulfurCubeSearchForItemsGoal {
            cube: cube.clone(),
            target_item: None,
        }),
    );
}

/// The item test of `SulfurCube`'s tempt goal.
fn is_tempted_by(mob: &dyn Mob, stack: &ItemStack) -> bool {
    if mob.as_ageable().is_some_and(AgeableMob::is_baby) {
        is_food(stack)
    } else {
        is_swallowable_item(stack)
    }
}

/// Vanilla `SulfurCube.SulfurCubeSearchForItemsGoal`: turns towards the nearest block it
/// could swallow.
struct SulfurCubeSearchForItemsGoal {
    cube: Weak<CubeMobEntity>,
    target_item: Option<Arc<dyn EntityBase>>,
}

impl Goal for SulfurCubeSearchForItemsGoal {
    fn can_start(&mut self, _mob: &dyn Mob) -> bool {
        let Some(cube) = self.cube.upgrade() else {
            return false;
        };
        let pickup_timer = cube
            .sulfur
            .as_ref()
            .map_or(0, |sulfur| sulfur.pickup_timer.load(Relaxed));
        if cube.is_baby() || pickup_timer > 0 {
            return false;
        }
        let entity = cube.get_entity();
        let pos = entity.pos.load();
        let search_box: BoundingBox = entity.bounding_box.load().expand(
            ITEM_SEARCH_RANGE,
            ITEM_SEARCH_RANGE,
            ITEM_SEARCH_RANGE,
        );
        // `getNearestEntity`: the first of the closest.
        let mut best: Option<(f64, Arc<dyn EntityBase>)> = None;
        for candidate in entity.world.load().get_entities_at_box(&search_box) {
            let Some(item) = candidate.get_item_entity() else {
                continue;
            };
            // `ALLOWED_ITEMS`.
            let swallowable = is_swallowable_item(
                &item
                    .get_item_stack()
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
            );
            if item.get_pickup_delay() != 0 || !item.get_entity().is_alive() || !swallowable {
                continue;
            }
            let distance = candidate
                .get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&pos);
            if best.as_ref().is_none_or(|(best, _)| distance < *best) {
                best = Some((distance, candidate));
            }
        }
        self.target_item = best.map(|(_, item)| item);
        self.target_item.is_some()
    }

    fn tick(&mut self, _mob: &dyn Mob) {
        let (Some(cube), Some(target)) = (self.cube.upgrade(), &self.target_item) else {
            return;
        };
        cube.mob_entity.look_at(target.as_ref(), 10.0, 10.0);
        cube.set_direction(cube.get_entity().yaw.load(), true);
    }

    fn controls(&self) -> Controls {
        Controls::LOOK
    }
}
