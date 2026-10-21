use super::{
    Entity, EntityBase, ai::pathfinder::Navigator, ai::pathfinder::node::PathType,
    ai::pathfinder::node_evaluator::MobData, living::LivingEntity,
};
use crate::enchantment::EnchantmentHelper;
use crate::entity::ai::brain::memory::PackedMemories;
use crate::entity::ai::brain::{Brain, BrainTick};
use crate::entity::ai::control::MoveControlTrait;
use crate::entity::ai::control::body_rotation_control::{
    BodyRotationControl, BodyRotationControlTrait,
};
use crate::entity::ai::control::jump_control::{JumpControl, JumpControlTrait};
use crate::entity::ai::control::look_control::LookControl;
use crate::entity::ai::control::move_control::MoveControl;
use crate::entity::ai::goal::escape_danger::EscapeDangerGoal;
use crate::entity::ai::goal::goal_selector::GoalSelector;
use crate::entity::ai::sensing::Sensing;
use crate::entity::damage_source::DamageSource;
use crate::entity::item::ItemEntity;
use crate::entity::movement_emission::MovementEmission;
use crate::entity::player::Player;
use crate::entity::predicate::EntityPredicate;
use crate::entity::projectile_deflection::ProjectileDeflectionType;
use crate::server::Server;
use crate::world::World;
use crate::world::brightness::DAYLIGHT_BRIGHTNESS;
use crossbeam::atomic::AtomicCell;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::{EquipmentSlot, EquippableImpl};
use pumpkin_data::entity::{EntityPose, EntityType};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_data::{Block, BlockState};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::Difficulty;
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_util::version::JavaMinecraftVersion;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU8, Ordering};
use uuid::Uuid;

pub mod abstract_piglin;
pub mod bat;
pub mod blaze;
pub mod breeze;
pub mod cave_spider;
pub mod conversion;
pub mod creaking;
pub mod creaking_ai;
pub mod creeper;
pub mod crossbow_attack_mob;
pub mod cube_mob;
pub mod elder_guardian;
pub mod enderman;
pub mod endermite;
pub mod equipment;
pub mod evoker;
pub mod ghast;
pub mod giant;
pub mod guardian;
pub mod hoglin;
pub mod hoglin_ai;
pub mod hoglin_base;
pub mod illusioner;
pub mod mount_inventory;
pub mod neutral;
pub mod patrol;
pub mod phantom;
pub mod piglin;
pub mod piglin_ai;
pub mod piglin_brute;
pub mod piglin_brute_ai;
pub mod pillager;
pub mod raider;
pub mod ravager;
pub mod shulker;
pub mod silverfish;
pub mod skeleton;
pub mod slime;
pub mod sounds;
pub mod spawn;
pub mod spellcaster;
pub mod spider;
pub mod sun_burn;
pub mod vex;
pub mod vindicator;
pub mod warden;
pub mod warden_spawn_tracker;
pub mod witch;
pub mod zoglin;
pub mod zombie;
pub mod zombified_piglin;

/// Work posted by another mob's tick, run against the owner's own brain on its next tick.
pub type BrainMessage = Box<dyn FnOnce(&mut BrainTick<'_>) + Send>;

pub struct MobEntity {
    pub living_entity: LivingEntity,
    pub goals_selector: std::sync::Mutex<GoalSelector>,
    pub target_selector: std::sync::Mutex<GoalSelector>,
    pub navigator: std::sync::Mutex<Navigator>,
    pub target: std::sync::Mutex<Option<Arc<dyn EntityBase>>>,
    pub look_control: std::sync::Mutex<LookControl>,
    /// How much a vanilla `getAttackBoundingBox` override shrinks the attack box sideways.
    pub attack_box_deflate: f64,
    pub sensing: std::sync::Mutex<Sensing>,
    pub move_control: std::sync::Mutex<Box<dyn MoveControlTrait>>,
    pub jump_control: std::sync::Mutex<Box<dyn JumpControlTrait>>,
    /// Vanilla `Mob.bodyRotationControl`.
    pub body_rotation_control: std::sync::Mutex<Box<dyn BodyRotationControlTrait>>,
    pub brain: std::sync::Mutex<Brain>,
    pub brain_inbox: std::sync::Mutex<Vec<BrainMessage>>,
    pub position_target: AtomicCell<BlockPos>,
    pub position_target_range: AtomicI32,
    pub love_ticks: AtomicI32,
    pub breeding_cooldown: AtomicI32,
    /// Vanilla `Mob.ambientSoundTime`.
    pub ambient_sound_time: AtomicI32,
    pub breeder: AtomicCell<Option<Uuid>>,
    pub persistence_required: AtomicBool,
    /// Vanilla `Mob.canPickUpLoot`, a plain field, not one of the synced mob flags.
    can_pick_up_loot: AtomicBool,
    /// Vanilla `Mob.lootTable`: a `DeathLootTable` that replaces the type's death loot.
    death_loot_table: std::sync::Mutex<Option<String>>,
    /// Vanilla `Mob.lootTableSeed`.
    death_loot_table_seed: AtomicI64,
    /// The follow range the navigator last sized its search for (`onAttributeUpdated`).
    applied_follow_range: AtomicCell<f64>,
    pending_riders: std::sync::Mutex<Vec<Arc<dyn EntityBase>>>,
    pending_vehicle: std::sync::Mutex<Option<Arc<dyn EntityBase>>>,
    mob_flags: AtomicU8,
}
impl MobEntity {
    const AI_DISABLED_FLAG: u8 = 1;
    const LEFT_HANDED_FLAG: u8 = 2;
    const ATTACKING_FLAG: u8 = 4;

    pub const MAX_WEARING_ARMOR_CHANCE: f32 = 0.15;
    pub const WEARING_ARMOR_UPGRADE_MATERIAL_CHANCE: f32 = 0.1087;
    pub const WEARING_ARMOR_UPGRADE_MATERIAL_ATTEMPTS: f32 = 3.0;
    pub const MAX_PICKUP_LOOT_CHANCE: f32 = 0.55;
    pub const MAX_ENCHANTED_ARMOR_CHANCE: f32 = 0.5;
    pub const MAX_ENCHANTED_WEAPON_CHANCE: f32 = 0.25;
    pub const GUARANTEED_DROP_CHANCE: f32 = 2.0;
    pub const EQUIPMENT_POPULATION_ORDER: [EquipmentSlot; 4] = [
        EquipmentSlot::HEAD,
        EquipmentSlot::CHEST,
        EquipmentSlot::LEGS,
        EquipmentSlot::FEET,
    ];

    /// Vanilla `Mob.ITEM_PICKUP_REACH`.
    pub const ITEM_PICKUP_REACH: Vector3<i32> = Vector3::new(1, 0, 1);

    /// Vanilla `Mob.setSpeed`: the speed travel uses, also set as the forward input.
    pub fn set_speed(&self, speed: f32) {
        let living = &self.living_entity;
        living.speed.store(speed);
        let input = living.movement_input.load();
        living
            .movement_input
            .store(Vector3::new(input.x, input.y, f64::from(speed)));
    }

    /// Vanilla `Mob.stopInPlace`. Pumpkin's leash has no angular momentum to reset.
    pub fn stop_in_place(&self) {
        self.navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
        let living = &self.living_entity;
        living.movement_input.store(Vector3::new(0.0, 0.0, 0.0));
        self.set_speed(0.0);
        living.entity.velocity.store(Vector3::new(0.0, 0.0, 0.0));
    }

    /// Vanilla `Mob.setPathfindingMalus`: what this mob pays to path through
    /// `path_type`, or `-1` to never enter it.
    pub fn set_pathfinding_malus(&self, path_type: PathType, malus: f32) {
        self.navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_pathfinding_malus(path_type, malus);
    }

    /// Vanilla `getNavigation().setRequiredPathLength`: how far this mob's paths
    /// may reach whatever its follow range.
    pub fn set_required_path_length(&self, length: f32) {
        self.navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_required_path_length(length, &self.living_entity);
    }

    #[must_use]
    pub const fn get_equipment_for_slot(
        slot: &EquipmentSlot,
        armor_type: i32,
    ) -> Option<&'static Item> {
        match slot {
            EquipmentSlot::Head(_) => match armor_type {
                0 => Some(&Item::LEATHER_HELMET),
                1 => Some(&Item::COPPER_HELMET),
                2 => Some(&Item::GOLDEN_HELMET),
                3 => Some(&Item::CHAINMAIL_HELMET),
                4 => Some(&Item::IRON_HELMET),
                5 => Some(&Item::DIAMOND_HELMET),
                _ => None,
            },
            EquipmentSlot::Chest(_) => match armor_type {
                0 => Some(&Item::LEATHER_CHESTPLATE),
                1 => Some(&Item::COPPER_CHESTPLATE),
                2 => Some(&Item::GOLDEN_CHESTPLATE),
                3 => Some(&Item::CHAINMAIL_CHESTPLATE),
                4 => Some(&Item::IRON_CHESTPLATE),
                5 => Some(&Item::DIAMOND_CHESTPLATE),
                _ => None,
            },
            EquipmentSlot::Legs(_) => match armor_type {
                0 => Some(&Item::LEATHER_LEGGINGS),
                1 => Some(&Item::COPPER_LEGGINGS),
                2 => Some(&Item::GOLDEN_LEGGINGS),
                3 => Some(&Item::CHAINMAIL_LEGGINGS),
                4 => Some(&Item::IRON_LEGGINGS),
                5 => Some(&Item::DIAMOND_LEGGINGS),
                _ => None,
            },
            EquipmentSlot::Feet(_) => match armor_type {
                0 => Some(&Item::LEATHER_BOOTS),
                1 => Some(&Item::COPPER_BOOTS),
                2 => Some(&Item::GOLDEN_BOOTS),
                3 => Some(&Item::CHAINMAIL_BOOTS),
                4 => Some(&Item::IRON_BOOTS),
                5 => Some(&Item::DIAMOND_BOOTS),
                _ => None,
            },
            _ => None,
        }
    }

    #[must_use]
    pub fn new(entity: Entity) -> Self {
        Self {
            living_entity: LivingEntity::new(entity),
            goals_selector: std::sync::Mutex::new(GoalSelector::default()),
            target_selector: std::sync::Mutex::new(GoalSelector::default()),
            navigator: std::sync::Mutex::new(Navigator::default()),
            target: std::sync::Mutex::new(None),
            look_control: std::sync::Mutex::new(LookControl::default()),
            attack_box_deflate: 0.0,
            sensing: std::sync::Mutex::new(Sensing::default()),
            move_control: std::sync::Mutex::new(Box::new(MoveControl::default())),
            jump_control: std::sync::Mutex::new(Box::new(JumpControl::default())),
            body_rotation_control: std::sync::Mutex::new(Box::new(BodyRotationControl::new())),
            brain: std::sync::Mutex::new(Brain::default()),
            brain_inbox: std::sync::Mutex::new(Vec::new()),
            position_target: AtomicCell::new(BlockPos::ZERO),
            position_target_range: AtomicI32::new(-1),
            love_ticks: AtomicI32::new(0),
            breeding_cooldown: AtomicI32::new(0),
            ambient_sound_time: AtomicI32::new(0),
            breeder: AtomicCell::new(None),
            persistence_required: AtomicBool::new(false),
            can_pick_up_loot: AtomicBool::new(false),
            death_loot_table: std::sync::Mutex::new(None),
            death_loot_table_seed: AtomicI64::new(0),
            applied_follow_range: AtomicCell::new(f64::NAN),
            pending_riders: std::sync::Mutex::new(Vec::new()),
            pending_vehicle: std::sync::Mutex::new(None),
            mob_flags: AtomicU8::new(0),
        }
    }

    pub fn has_position_target(&self) -> bool {
        self.position_target_range.load(Relaxed) != -1
    }

    pub fn is_in_position_target_range(&self) -> bool {
        self.is_in_position_target_range_pos(&self.living_entity.entity.block_pos.load())
    }

    pub fn is_in_position_target_range_pos(&self, block_pos: &BlockPos) -> bool {
        let position_target_range = self.position_target_range.load(Relaxed);
        if position_target_range == -1 {
            true
        } else {
            self.position_target.load().squared_distance(block_pos)
                < position_target_range * position_target_range
        }
    }

    pub fn set_attacking(&self, attacking: bool) {
        self.set_mob_flag(Self::ATTACKING_FLAG, attacking);
    }

    pub fn is_attacking(&self) -> bool {
        (self.mob_flags.load(Relaxed) & Self::ATTACKING_FLAG) != 0
    }

    pub fn set_left_handed(&self, left_handed: bool) {
        self.set_mob_flag(Self::LEFT_HANDED_FLAG, left_handed);
    }

    pub fn can_pick_up_loot(&self) -> bool {
        self.can_pick_up_loot.load(Relaxed)
    }

    pub fn set_can_pick_up_loot(&self, value: bool) {
        self.can_pick_up_loot.store(value, Relaxed);
    }

    /// Vanilla `Mob.getLootTable` and `getLootTableSeed`: a saved `DeathLootTable` and its
    /// seed, if the mob has one.
    pub fn death_loot_table(&self) -> (Option<String>, i64) {
        (
            self.death_loot_table
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
            self.death_loot_table_seed.load(Relaxed),
        )
    }

    /// Vanilla `Mob.dropFromLootTable`: the death loot table is used once.
    pub fn clear_death_loot_table(&self) {
        *self
            .death_loot_table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    pub fn is_left_handed(&self) -> bool {
        (self.mob_flags.load(Relaxed) & Self::LEFT_HANDED_FLAG) != 0
    }

    pub fn set_no_ai(&self, no_ai: bool) {
        self.set_mob_flag(Self::AI_DISABLED_FLAG, no_ai);
    }

    pub fn is_no_ai(&self) -> bool {
        (self.mob_flags.load(Relaxed) & Self::AI_DISABLED_FLAG) != 0
    }

    /// Vanilla `Mob.serverAiStep`: sensing, goals, navigation and controls.
    pub fn server_ai_step(&self, mob: &dyn Mob, caller: &dyn EntityBase) {
        self.living_entity.no_action_time.fetch_add(1, Relaxed);
        // Vanilla `tickCount`, not Pumpkin's `age`, which babies keep negative.
        let age = self.living_entity.entity.tick_count.load(Relaxed);
        let entity_id = self.living_entity.entity.entity_id;

        self.sensing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tick();

        // Vanilla `Mob.onAttributeUpdated`: a changed follow range resizes the path search.
        // Attributes change where the mob can't be reached, so the AI step, after
        // `tickEffects`, picks it up.
        let follow_range = self
            .living_entity
            .get_attribute_value(&Attributes::FOLLOW_RANGE);
        if self.applied_follow_range.swap(follow_range).to_bits() != follow_range.to_bits() {
            self.navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .update_pathfinder_max_visited_nodes(&self.living_entity);
        }

        // 1. "Take" selectors out of the mutexes
        let mut target_selector = {
            let mut guard = self
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *guard)
        };
        let mut goals_selector = {
            let mut guard = self
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *guard)
        };

        // 2. Perform AI logic
        if (age + entity_id) % 2 != 0 && age > 1 {
            target_selector.tick_goals(mob, false);
            goals_selector.tick_goals(mob, false);
        } else {
            target_selector.tick(mob);
            goals_selector.tick(mob);
        }

        // 3. "Put back" selectors
        {
            *self
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = target_selector;
            *self
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = goals_selector;
        };

        // 4. Repeat for Navigator
        let mut navigator = {
            let mut guard = self
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *guard)
        };

        if !mob.skips_navigation_tick() {
            navigator.tick(&self.living_entity);
            if let Some((target, speed_modifier)) = navigator.take_wanted_position() {
                self.move_control
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .set_wanted_position(target.x, target.y, target.z, speed_modifier);
            }
            if navigator.take_jump() {
                self.jump();
            }
        }

        {
            *self
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = navigator;
        };

        mob.custom_server_ai_step(caller);

        // Controllers are synchronous, so we can just use normal blocks
        // Vanilla ticks move, then look, then jump.
        if !mob.controls_paused() {
            self.move_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .tick(mob);
            if !mob.skips_look_control_tick() {
                let mut look_control = self
                    .look_control
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !mob.custom_look_control_tick(&mut look_control) {
                    look_control.tick(mob);
                }
            }
        }
        self.jump_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tick(mob);
    }

    /// Vanilla `Mob.lookAt(Entity, yMax, xMax)`: turns the mob itself, not its head, towards
    /// `target` by at most `y_max` and `x_max` degrees.
    pub fn look_at(&self, target: &dyn EntityBase, y_max: f32, x_max: f32) {
        // Vanilla `Mob.rotlerp`, which unlike `MoveControl.rotlerp` does not wrap the result.
        fn rotlerp(a: f32, b: f32, max: f32) -> f32 {
            a + pumpkin_util::math::wrap_degrees(b - a).clamp(-max, max)
        }
        let entity = &self.living_entity.entity;
        let pos = entity.pos.load();
        let target_entity = target.get_entity();
        let target_pos = target_entity.pos.load();
        let xd = target_pos.x - pos.x;
        let zd = target_pos.z - pos.z;
        let yd = if target.get_living_entity().is_some() {
            target_entity.get_eye_y() - entity.get_eye_y()
        } else {
            let bounding_box = target_entity.bounding_box.load();
            f64::midpoint(bounding_box.min.y, bounding_box.max.y) - entity.get_eye_y()
        };
        #[expect(
            clippy::imprecise_flops,
            reason = "vanilla's Math.sqrt rounds like this, hypot doesn't"
        )]
        let sd = (xd * xd + zd * zd).sqrt();
        // `* 180.0F / (float) Math.PI`, left to right in doubles.
        let to_degrees =
            |radians: f64| radians * f64::from(180.0f32) / f64::from(std::f32::consts::PI);
        let y_rot_d = to_degrees(pumpkin_util::math::atan2(zd, xd)) as f32 - 90.0;
        let x_rot_d = (-to_degrees(pumpkin_util::math::atan2(yd, sd))) as f32;
        entity.set_pitch(rotlerp(entity.pitch.load(), x_rot_d, x_max));
        entity.yaw.store(rotlerp(entity.yaw.load(), y_rot_d, y_max));
    }

    /// Vanilla `Mob.jumpInLiquid`: a mob whose navigation can't float leaps out.
    pub fn jump_in_liquid(&self) {
        let can_float = self
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .can_float();
        if can_float {
            self.living_entity.jump_in_liquid_base();
        } else {
            let entity = &self.living_entity.entity;
            let mut velocity = entity.velocity.load();
            velocity.y += 0.3;
            entity.velocity.store(velocity);
        }
    }

    /// Vanilla `mob.getJumpControl().jump()`: jump at the end of this AI step.
    pub fn jump(&self) {
        self.jump_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .jump();
    }

    pub fn clear_ai_goals(&self, mob: &dyn Mob) {
        let running_goals = self
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        for mut goal in running_goals {
            goal.goal.stop(mob);
        }

        let running_target_goals = self
            .target_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        for mut goal in running_target_goals {
            goal.goal.stop(mob);
        }
    }

    #[must_use]
    pub fn drop_chance(&self, slot: &EquipmentSlot) -> f32 {
        self.living_entity
            .equipment_drop_chances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(slot)
            .copied()
            .unwrap_or(crate::entity::mob::equipment::DEFAULT_EQUIPMENT_DROP_CHANCE)
    }

    pub fn set_guaranteed_drop(&self, slot: &EquipmentSlot) {
        self.living_entity
            .equipment_drop_chances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(slot.clone(), Self::GUARANTEED_DROP_CHANCE);
    }

    /// Vanilla `LivingEntity.startUsingItem` for a mob, whose hands are its equipment slots.
    pub fn start_using_item(&self, hand: pumpkin_util::Hand) {
        let slot = if hand == pumpkin_util::Hand::Right {
            EquipmentSlot::MAIN_HAND
        } else {
            EquipmentSlot::OFF_HAND
        };
        let stack = self.get_item_slot(&slot);
        let living = &self.living_entity;
        let using_item = living
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some();
        if stack.is_empty() || using_item {
            return;
        }
        let duration = living
            .entity
            .world
            .load()
            .server
            .upgrade()
            .and_then(|server| server.item_registry.get_use_duration(stack.item.id))
            .unwrap_or(0);
        living.set_active_hand(hand, stack, duration);
    }

    /// Vanilla `getItemBySlot`.
    #[must_use]
    pub fn get_item_slot(&self, slot: &EquipmentSlot) -> ItemStack {
        self.living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(slot)
    }

    /// Equips `stack` and tells tracking clients, returning what was in the slot.
    pub fn set_item_slot(&self, slot: &EquipmentSlot, stack: ItemStack) -> ItemStack {
        let previous = self
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .put(slot, stack.clone());
        self.living_entity
            .send_equipment_changes(&[(slot.clone(), stack)]);
        previous
    }

    pub fn set_item_slot_and_drop_when_killed(&self, slot: &EquipmentSlot, stack: ItemStack) {
        self.set_item_slot(slot, stack);
        self.set_guaranteed_drop(slot);
    }

    fn write_drop_chances(&self, nbt: &mut NbtCompound) {
        let chances = self
            .living_entity
            .equipment_drop_chances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut compound = NbtCompound::new();
        for (slot, chance) in chances.iter() {
            if *chance != crate::entity::mob::equipment::DEFAULT_EQUIPMENT_DROP_CHANCE {
                compound.put_float(slot.to_name(), *chance);
            }
        }
        if !compound.child_tags.is_empty() {
            nbt.put("drop_chances", pumpkin_nbt::tag::NbtTag::Compound(compound));
        }
    }

    /// Vanilla `DropChances.CODEC` through `ValueInput.read`: the saved map replaces the old
    /// one, missing slots are the default, and entries the codec rejects (an unknown slot, a
    /// negative chance) are left out.
    fn read_drop_chances(&self, nbt: &NbtCompound) {
        let mut chances = self
            .living_entity
            .equipment_drop_chances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        chances.clear();
        let Some(compound) = nbt.get_compound("drop_chances") else {
            return;
        };
        for (name, tag) in &compound.child_tags {
            if let Some(slot) = EquipmentSlot::get_from_name(name)
                && let Some(chance) = tag.as_numeric_float()
                && chance >= 0.0
            {
                chances.insert(slot.clone(), chance);
            }
        }
    }

    /// Queues a write to this mob's brain; writing another mob's brain directly deadlocks.
    pub fn post_to_brain(&self, message: BrainMessage) {
        self.brain_inbox
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(message);
    }

    fn apply_brain_messages(tick: &mut BrainTick<'_>, messages: Vec<BrainMessage>) {
        for message in messages {
            message(tick);
        }
    }

    fn take_brain_messages(&self) -> Vec<BrainMessage> {
        std::mem::take(
            &mut *self
                .brain_inbox
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Applies queued messages without running the brain, for a mob that died this tick.
    pub fn apply_brain_inbox(&self, mob: &dyn Mob) {
        let messages = self.take_brain_messages();
        if messages.is_empty() {
            return;
        }
        let world = self.living_entity.entity.world.load_full();
        let time = world.get_world_age();
        let mut brain = self
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::apply_brain_messages(
            &mut BrainTick {
                brain: &mut brain,
                world: &world,
                mob,
                time,
            },
            messages,
        );
    }

    pub fn tick_brain(&self, mob: &dyn Mob) {
        let world = self.living_entity.entity.world.load_full();
        let time = world.get_world_age();
        let messages = self.take_brain_messages();
        let mut brain = self
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::apply_brain_messages(
            &mut BrainTick {
                brain: &mut brain,
                world: &world,
                mob,
                time,
            },
            messages,
        );
        brain.tick(&world, mob, time);
        let mut tick = BrainTick {
            brain: &mut brain,
            world: &world,
            mob,
            time,
        };
        mob.after_brain_tick(&mut tick);
    }

    /// Builds the mob's brain from its provider, as vanilla does in the constructor.
    pub fn init_brain(&self, mob: &dyn Mob) {
        *self
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            mob.make_brain(&PackedMemories::empty());
    }

    /// Reaches the brain from outside a brain tick; a tick never waits on another mob's brain.
    pub fn with_brain<R>(&self, mob: &dyn Mob, f: impl FnOnce(&mut BrainTick<'_>) -> R) -> R {
        let world = self.living_entity.entity.world.load_full();
        let time = world.get_world_age();
        let mut brain = self
            .brain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut tick = BrainTick {
            brain: &mut brain,
            world: &world,
            mob,
            time,
        };
        f(&mut tick)
    }

    /// Vanilla `Mob.addAdditionalSaveData`, after `LivingEntity`'s brain. `can_pick_up_loot` is
    /// the mob's `canPickUpLoot()`, which some mobs override.
    pub fn write_mob_nbt(&self, nbt: &mut NbtCompound, can_pick_up_loot: bool) {
        nbt.put_compound(
            "Brain",
            self.brain
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pack()
                .into_nbt(),
        );
        nbt.put_bool("CanPickUpLoot", can_pick_up_loot);
        nbt.put_bool(
            "PersistenceRequired",
            self.persistence_required.load(Relaxed),
        );
        self.write_drop_chances(nbt);
        self.living_entity.entity.write_leash_nbt(nbt);
        if self.has_position_target() {
            let home = self.position_target.load();
            nbt.put_int("home_radius", self.position_target_range.load(Relaxed));
            nbt.put(
                "home_pos",
                pumpkin_nbt::tag::NbtTag::IntArray(vec![home.0.x, home.0.y, home.0.z]),
            );
        }
        nbt.put_bool("LeftHanded", self.is_left_handed());
        let (loot_table, loot_table_seed) = self.death_loot_table();
        if let Some(loot_table) = loot_table {
            nbt.put_string("DeathLootTable", loot_table);
        }
        if loot_table_seed != 0 {
            nbt.put_long("DeathLootTableSeed", loot_table_seed);
        }
        if self.is_no_ai() {
            nbt.put_bool("NoAI", true);
        }
    }

    /// Vanilla `Mob.readAdditionalSaveData`: every tag falls back to its default when missing.
    pub fn read_mob_nbt(&self, nbt: &NbtCompound) {
        self.set_can_pick_up_loot(nbt.get_bool("CanPickUpLoot").unwrap_or(false));
        self.persistence_required.store(
            nbt.get_bool("PersistenceRequired").unwrap_or(false),
            Relaxed,
        );
        self.read_drop_chances(nbt);
        self.living_entity.entity.read_leash_nbt(nbt);
        // Vanilla `home_radius` (-1 for no home), then `home_pos` (the origin if missing).
        let radius = nbt.get_int("home_radius").unwrap_or(-1);
        self.position_target_range.store(radius, Relaxed);
        if radius >= 0 {
            let home = match nbt.get_int_array("home_pos") {
                Some(&[x, y, z]) => BlockPos::new(x, y, z),
                _ => BlockPos::ZERO,
            };
            self.position_target.store(home);
        }
        self.set_left_handed(nbt.get_bool("LeftHanded").unwrap_or(false));
        // `LootTable.KEY_CODEC`: an identifier, `minecraft` when it names no namespace.
        *self
            .death_loot_table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            nbt.get_string("DeathLootTable").map(|key| {
                if key.contains(':') {
                    key.to_string()
                } else {
                    format!("minecraft:{key}")
                }
            });
        self.death_loot_table_seed
            .store(nbt.get_long("DeathLootTableSeed").unwrap_or(0), Relaxed);
        self.set_no_ai(nbt.get_bool("NoAI").unwrap_or(false));
    }

    pub fn add_goal<G: crate::entity::ai::goal::Goal + 'static>(&self, priority: u8, goal: G) {
        self.goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .add_goal(priority, Box::new(goal));
    }

    pub fn add_target_goal<G: crate::entity::ai::goal::Goal + 'static>(
        &self,
        priority: u8,
        goal: G,
    ) {
        self.target_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .add_goal(priority, Box::new(goal));
    }

    pub fn set_target(&self, target: Option<Arc<dyn EntityBase>>) {
        let mut t = self
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *t = target;
    }

    /// Vanilla `Mob.getTargetUnchecked`: the stored target, valid or not. `Mob::get_target`
    /// is vanilla's `getTarget`.
    pub fn get_target_unchecked(&self) -> Option<Arc<dyn EntityBase>> {
        self.target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn set_mob_flag(&self, flag: u8, value: bool) {
        let old_b = self.mob_flags.load(Ordering::Relaxed);

        let new_b = if value { old_b | flag } else { old_b & !flag };

        if new_b != old_b {
            self.mob_flags.store(new_b, Ordering::Relaxed);

            self.living_entity
                .entity
                .set_synced_data(tracked_data::mob::DATA_MOB_FLAGS_ID, new_b);
        }
    }

    pub fn is_in_love(&self) -> bool {
        self.love_ticks.load(Relaxed) > 0
    }

    pub fn set_love_ticks(&self, ticks: i32, breeder: Option<Uuid>) {
        self.love_ticks.store(ticks, Relaxed);
        self.breeder.store(breeder);
    }

    pub fn reset_love_ticks(&self) {
        self.love_ticks.store(0, Relaxed);
    }

    pub fn is_breeding_ready(&self) -> bool {
        self.living_entity.entity.age.load(Relaxed) >= 0
            && self.breeding_cooldown.load(Relaxed) <= 0
    }

    /// Vanilla `Mob.isWithinMeleeAttackRange`: the active item's `attack_range` (scaled by its
    /// `mob_factor`) or the default reach, against the target's hitbox.
    pub fn is_in_attack_range(&self, target: &dyn EntityBase) -> bool {
        // Vanilla `Math.sqrt(2.04F) - 0.6F`, with both literals rounded to float first.
        const DEFAULT_ATTACK_REACH: f64 = 0.828_285_648_512_600_4;

        let (max_range, min_range) = self
            .active_item()
            .get_data_component::<pumpkin_data::data_component_impl::AttackRangeImpl>()
            .map_or((DEFAULT_ATTACK_REACH, 0.0), |range| {
                // `effectiveMaxRange`/`effectiveMinRange` for a mob.
                (
                    f64::from(range.max_reach * range.mob_factor),
                    f64::from(range.min_reach * range.mob_factor),
                )
            });

        let target_hitbox = hitbox(target);
        if !self.get_attack_box(max_range).intersects(&target_hitbox) {
            return false;
        }

        min_range <= 0.0 || !self.get_attack_box(min_range).intersects(&target_hitbox)
    }

    /// Vanilla `LivingEntity.getActiveItem`: the item in use, else the main hand.
    fn active_item(&self) -> ItemStack {
        self.living_entity
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| self.get_item_slot(&EquipmentSlot::MAIN_HAND))
    }

    /// Vanilla `Monster.isDarkEnoughToSpawn`, drawing from the random `checkSpawnRules` is given.
    pub fn is_dark_enough_to_spawn(
        world: &World,
        pos: &BlockPos,
        is_thundering: bool,
        random: &mut pumpkin_util::random::legacy_rand::LegacyRand,
    ) -> bool {
        if i32::from(world.get_sky_light_level(pos)) > random.next_bounded_i32(32) {
            return false;
        }

        let dimension = &world.dimension;
        let block_light_limit = dimension.monster_spawn_block_light_limit;
        let block_light = world.get_block_light_level(pos).unwrap_or(0);
        if block_light_limit < 15 && block_light > block_light_limit {
            return false;
        }

        // `getMaxLocalRawBrightness(pos, 10)` in a thunderstorm, else with the sky darkening.
        let brightness = if is_thundering {
            world.get_raw_brightness(pos, 10)
        } else {
            world.get_max_local_raw_brightness(pos)
        };
        i32::from(brightness) <= dimension.monster_spawn_light_level.get(random)
    }

    /// Vanilla `Mob.checkMobSpawnRules`: a spawner ignores the floor, anything else needs one
    /// that `BlockState.isValidSpawn` allows for `entity_type`.
    pub fn check_mob_spawn_rules(
        world: &World,
        entity_type: &'static EntityType,
        reason: spawn::EntitySpawnReason,
        pos: &BlockPos,
    ) -> bool {
        reason.is_spawner()
            || crate::world::natural_spawner::is_valid_spawn_floor(
                world.get_block_state(&pos.down()),
                entity_type,
            )
    }

    /// Vanilla `Monster.checkMonsterSpawnRules`: dark enough, unless a trial spawner ignores
    /// the light, then `checkMobSpawnRules`.
    pub fn check_monster_spawn_rules(
        world: &World,
        entity_type: &'static EntityType,
        reason: spawn::EntitySpawnReason,
        pos: &BlockPos,
        is_thundering: bool,
        random: &mut pumpkin_util::random::legacy_rand::LegacyRand,
    ) -> bool {
        (reason.ignores_light_requirements()
            || Self::is_dark_enough_to_spawn(world, pos, is_thundering, random))
            && Self::check_mob_spawn_rules(world, entity_type, reason, pos)
    }

    /// Vanilla `Monster.checkAnyLightMonsterSpawnRules`.
    pub fn check_any_light_monster_spawn_rules(
        world: &World,
        entity_type: &'static EntityType,
        reason: spawn::EntitySpawnReason,
        pos: &BlockPos,
    ) -> bool {
        Self::check_mob_spawn_rules(world, entity_type, reason, pos)
    }

    /// Vanilla `Monster.checkSurfaceMonstersSpawnRules`: also under open sky, unless spawned by
    /// a spawner.
    pub fn check_surface_monsters_spawn_rules(
        world: &World,
        entity_type: &'static EntityType,
        reason: spawn::EntitySpawnReason,
        pos: &BlockPos,
        is_thundering: bool,
        random: &mut pumpkin_util::random::legacy_rand::LegacyRand,
    ) -> bool {
        Self::check_monster_spawn_rules(world, entity_type, reason, pos, is_thundering, random)
            && (reason.is_spawner() || world.can_see_sky(pos))
    }

    pub fn check_animal_spawn_rules(world: &World, pos: &BlockPos) -> bool {
        let below = pos.down();
        world
            .get_block(&below)
            .has_tag(&tag::Block::MINECRAFT_ANIMALS_SPAWNABLE_ON)
            && Self::is_bright_enough_to_spawn(world, pos)
    }

    pub fn is_bright_enough_to_spawn(world: &World, pos: &BlockPos) -> bool {
        world.get_max_local_raw_brightness(pos) > 8
    }

    pub fn check_surface_water_animal_spawn_rules(world: &World, pos: &BlockPos) -> bool {
        let sea_level = world.sea_level;
        let min_spawn_level = sea_level - 13;
        pos.0.y >= min_spawn_level
            && pos.0.y <= sea_level
            && world
                .get_fluid(&pos.down())
                .has_tag(&tag::Fluid::MINECRAFT_WATER)
            && (world.get_block(&pos.up()) == &Block::WATER
                || world
                    .get_fluid(&pos.up())
                    .has_tag(&tag::Fluid::MINECRAFT_WATER))
    }

    pub fn check_surface_ageable_water_creature_spawn_rules(world: &World, pos: &BlockPos) -> bool {
        Self::check_surface_water_animal_spawn_rules(world, pos)
    }

    /// Vanilla `Mob.doHurtTarget`.
    pub fn try_attack(&self, caller: &dyn EntityBase, target: &dyn EntityBase) -> bool {
        let living = &self.living_entity;
        let world = living.entity.world.load_full();
        let mut damage = living.get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32;
        let weapon = living.held_item(caller);
        let damage_source = DamageSource {
            damage_type: weapon_damage_type(caller, &weapon),
            direct: Some(caller),
            causing: Some(caller),
        };
        damage = EnchantmentHelper::modify_damage(&world, &weapon, target, &damage_source, damage);
        // `getItem().getAttackDamageBonus`: only the mace has one.
        if weapon.item == &Item::MACE {
            damage += crate::item::items::mace::MaceItem::get_attack_damage_bonus(
                living,
                &weapon,
                target,
                &damage_source,
            );
        }
        let damaged = target.damage_with_context(
            target,
            damage,
            damage_source.damage_type,
            None,
            damage_source.direct,
            damage_source.causing,
        );

        if damaged {
            let knockback = self.get_knockback(&world, &weapon, target, &damage_source);
            self.cause_extra_knockback(caller, target, knockback, damage);
            // TODO: `weaponItem.hurtEnemy`, whose only effect for a mob is the mace's smash
            // (`MaceItem.hurtEnemy`), which Pumpkin only ports for players.
            EnchantmentHelper::do_post_attack_effects(&world, target, &damage_source);
            // `setLastHurtMob`.
            living.last_attacking_id.store(
                if target.get_living_entity().is_some() {
                    target.get_entity().entity_id
                } else {
                    0
                },
                Relaxed,
            );
            living
                .last_attack_time
                .store(living.entity.tick_count.load(Relaxed), Relaxed);
            if let Some(mob) = caller.get_mob() {
                mob.play_attack_sound();
            }
        }
        // TODO: `postPiercingAttack` runs `EnchantmentHelper.doPostPiercingAttackEffects`,
        // which Pumpkin has no port of; no vanilla enchantment applies it to a melee hit.
        damaged
    }

    /// Vanilla `LivingEntity.getKnockback`: the `attack_knockback` attribute through the
    /// weapon's enchantments, halved.
    fn get_knockback(
        &self,
        world: &World,
        weapon: &ItemStack,
        target: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
    ) -> f32 {
        let knockback = self
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_KNOCKBACK) as f32;
        EnchantmentHelper::modify_knockback(world, weapon, target, damage_source, knockback) / 2.0
    }

    /// Vanilla `LivingEntity.causeExtraKnockback`: a living target is pushed along this mob's
    /// facing, and the mob slows down.
    fn cause_extra_knockback(
        &self,
        caller: &dyn EntityBase,
        target: &dyn EntityBase,
        knockback: f32,
        damage: f32,
    ) {
        let Some(target_living) = target.get_living_entity() else {
            return;
        };
        if knockback <= 0.0 {
            return;
        }
        let entity = &self.living_entity.entity;
        let yaw = entity.yaw.load() * (std::f32::consts::PI / 180.0);
        if target.get_mob().is_none_or(Mob::takes_knockback) {
            target_living.knockback(
                target,
                f64::from(knockback),
                f64::from(pumpkin_util::math::sin(yaw)),
                f64::from(-pumpkin_util::math::cos(yaw)),
                Some(caller),
                damage,
                true,
            );
        }
        let velocity = entity.velocity.load();
        entity.velocity.store(velocity.multiply(0.6, 1.0, 0.6));
    }

    fn get_attack_box(&self, attack_range: f64) -> BoundingBox {
        let vehicle_opt = self.living_entity.entity.get_vehicle();

        let base_box = vehicle_opt.as_ref().map_or_else(
            || self.living_entity.entity.bounding_box.load(),
            |vehicle| {
                let vehicle_box = vehicle.get_entity().bounding_box.load();
                let my_box = self.living_entity.entity.bounding_box.load();

                BoundingBox {
                    min: Vector3::new(
                        my_box.min.x.min(vehicle_box.min.x),
                        my_box.min.y,
                        my_box.min.z.min(vehicle_box.min.z),
                    ),
                    max: Vector3::new(
                        my_box.max.x.max(vehicle_box.max.x),
                        my_box.max.y,
                        my_box.max.z.max(vehicle_box.max.z),
                    ),
                }
            },
        );

        base_box.expand(attack_range, 0.0, attack_range).expand(
            -self.attack_box_deflate,
            0.0,
            -self.attack_box_deflate,
        )
    }

    /// Brightness at the mob's eye (sky and block light) reaches daylight.
    pub fn is_in_daylight(&self) -> bool {
        let entity = &self.living_entity.entity;
        entity
            .world
            .load()
            .get_light_level_dependent_magic_value(&entity.get_eye_pos().to_block_pos())
            >= DAYLIGHT_BRIGHTNESS
    }

    /// Vanilla `Mob.clearHome`.
    pub fn clear_home(&self) {
        self.position_target_range.store(-1, Relaxed);
    }

    /// Vanilla `Mob.checkDespawn`: Peaceful removes the types it doesn't allow; otherwise, with
    /// no player in the world nothing despawns, and a player within the no-despawn distance
    /// resets the mob's `noActionTime`.
    pub fn check_despawn(&self, mob: &dyn Mob) {
        let entity = &self.living_entity.entity;
        let world = entity.world.load();
        if world.level_info.load().difficulty == Difficulty::Peaceful
            && !entity.entity_type.allowed_in_peaceful
        {
            entity.remove();
            return;
        }
        let is_persistent =
            self.persistence_required.load(Relaxed) || mob.requires_custom_persistence();
        let pos = entity.pos.load();
        let players = world.players.load();
        let Some(dist_sqr) = players
            .iter()
            .filter(|p| p.gamemode.load() != pumpkin_util::GameMode::Spectator)
            .map(|p| p.get_entity().pos.load().squared_distance_to_vec(&pos))
            .min_by(f64::total_cmp)
        else {
            return;
        };

        let despawn_distance = f64::from(entity.entity_type.category.despawn_distance);
        if !is_persistent
            && dist_sqr > despawn_distance * despawn_distance
            && mob.remove_when_far_away(dist_sqr)
        {
            entity.remove();
        }

        // Vanilla goes on after an instant despawn, so the `nextInt(800)` roll still draws.
        let no_despawn_distance = f64::from(NO_DESPAWN_DISTANCE);
        if !is_persistent
            && self.living_entity.no_action_time.load(Relaxed) > 600
            && entity.random().next_bounded_i32(800) == 0
            && dist_sqr > no_despawn_distance * no_despawn_distance
            && mob.remove_when_far_away(dist_sqr)
        {
            entity.remove();
        } else if dist_sqr < no_despawn_distance * no_despawn_distance {
            self.living_entity.no_action_time.store(0, Relaxed);
        }
    }
}

pub trait Mob: EntityBase + Send + Sync {
    /// Vanilla `isPickable` for a mob: `LivingEntity`'s "not removed" unless overridden.
    fn is_pickable(&self) -> bool {
        !self.get_entity().is_removed()
    }

    /// Vanilla `LivingEntity.getAgeScale`: babies are half size unless a mob says
    /// otherwise.
    fn age_scale(&self) -> f32 {
        if self.get_entity().is_baby() {
            0.5
        } else {
            1.0
        }
    }

    /// Vanilla `LivingEntity.getDefaultDimensions`: the type's size for the pose, times
    /// the age scale. Mobs with their own baby size or size states override it.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        self.get_entity()
            .get_dimensions(pose)
            .scale(self.age_scale())
    }

    /// Whether `default_dimensions` is a vanilla `EntityDimensions.fixed` in this pose,
    /// which the `scale` attribute leaves alone.
    fn has_fixed_dimensions(&self, _pose: EntityPose) -> bool {
        false
    }

    /// Vanilla `Mob.getMaxFallDistance`: how far pathing may drop, more when it
    /// has a target and health to spare.
    fn get_max_fall_distance(&self) -> i32 {
        let mob_entity = self.get_mob_entity();
        if self.get_target().is_none() {
            return LivingEntity::get_comfortable_fall_distance(0.0);
        }

        let living = &mob_entity.living_entity;
        let difficulty = living.entity.world.load().level_info.load().difficulty;
        let mut sacrifice = (living.health.load() - living.get_max_health() * 0.33) as i32;
        sacrifice -= (3 - difficulty as i32) * 4;
        if sacrifice < 0 {
            sacrifice = 0;
        }
        LivingEntity::get_comfortable_fall_distance(sacrifice as f32)
    }

    /// Vanilla `Mob.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        sounds::ambient_sound(self.get_entity().entity_type)
    }

    /// Vanilla `Mob.getAmbientSoundInterval`.
    fn ambient_sound_interval(&self) -> i32 {
        sounds::ambient_sound_interval(self.get_entity().entity_type)
    }

    /// Vanilla `Mob.playAmbientSound`.
    fn play_ambient_sound(&self) {
        if let Some(sound) = self.ambient_sound() {
            self.make_sound(sound);
        }
    }

    /// Vanilla `Mob.resetAmbientSoundTime`.
    fn reset_ambient_sound_time(&self) {
        self.get_mob_entity()
            .ambient_sound_time
            .store(-self.ambient_sound_interval(), Relaxed);
    }

    /// The ambient-sound roll of vanilla `Mob.baseTick` (Mob.java:288), after
    /// `super.baseTick()` and before `LivingEntity.tick` reaches `aiStep`. `isAlive`
    /// includes health.
    fn mob_base_tick(&self) {
        let mob_entity = self.get_mob_entity();
        if !mob_entity.living_entity.is_alive() {
            return;
        }
        let roll = self.get_entity().random().next_bounded_i32(1000);
        if roll < mob_entity.ambient_sound_time.fetch_add(1, Relaxed) {
            self.reset_ambient_sound_time();
            self.play_ambient_sound();
        }
    }

    /// Vanilla `LivingEntity.getSoundVolume`.
    fn sound_volume(&self) -> f32 {
        sounds::sound_volume(self.get_entity().entity_type)
    }

    /// Vanilla `Entity.getSoundSource`.
    fn sound_source(&self) -> SoundCategory {
        sounds::sound_source(self.get_entity().entity_type)
    }

    /// Vanilla `Entity.getMovementEmission`.
    fn movement_emission(&self) -> MovementEmission {
        sounds::movement_emission(self.get_entity().entity_type)
    }

    /// Vanilla `Entity.playStepSound`.
    fn play_step_sound(
        &self,
        caller: &dyn EntityBase,
        _pos: &BlockPos,
        _block: &'static Block,
        state: &'static BlockState,
    ) {
        match sounds::step_sound(self.get_entity().entity_type) {
            sounds::StepSound::Block => self.get_entity().play_step_sound_default(caller, state),
            sounds::StepSound::Silent => {}
            sounds::StepSound::Fixed(sound, volume) => {
                caller.play_sound_at_self(sound, volume, 1.0);
            }
        }
    }

    /// Vanilla `Entity.nextStep`.
    fn next_step(&self) -> f32 {
        let move_dist = self.get_entity().move_dist.load();
        sounds::next_step_distance(self.get_entity().entity_type)
            .map_or((move_dist as i32 + 1) as f32, |distance| {
                move_dist + distance
            })
    }

    /// Vanilla `Entity.playSwimSound`.
    fn play_swim_sound(&self, caller: &dyn EntityBase, volume: f32) {
        self.get_entity().play_swim_sound_default(caller, volume);
    }

    /// Vanilla `Entity.getSwimSound`.
    fn swim_sound(&self) -> Sound {
        sounds::swim_sound(self.get_entity().entity_type)
    }

    /// Vanilla `Entity.getSwimSplashSound`.
    fn swim_splash_sound(&self) -> Sound {
        sounds::swim_splash_sound(self.get_entity().entity_type)
    }

    /// Vanilla `Entity.getSwimHighSpeedSplashSound`.
    fn swim_high_speed_splash_sound(&self) -> Sound {
        Sound::EntityGenericSplash
    }

    /// Vanilla `Entity.isFlapping`.
    fn is_flapping(&self) -> bool {
        false
    }

    /// Vanilla `Entity.onFlap`.
    fn on_flap(&self) {}

    /// Vanilla `LivingEntity.getVoicePitch`.
    fn voice_pitch(&self) -> f32 {
        let base = if self.get_entity().is_baby() {
            1.5
        } else {
            1.0
        };
        let mut random = self.get_entity().random();
        // Vanilla rounds the product before adding, so no fused multiply-add.
        (random.next_f32() - random.next_f32()) * 0.2 + base
    }

    /// Vanilla `LivingEntity.makeSound`, through `Entity.playSound`, which a silent
    /// entity skips. `getVoicePitch()` is an argument, so its draws happen even then.
    fn make_sound(&self, sound: Sound) {
        let volume = self.sound_volume();
        let pitch = self.voice_pitch();
        let entity = self.get_entity();
        if entity.is_silent() {
            return;
        }
        entity.world.load().play_sound_fine(
            sound,
            self.sound_source(),
            &entity.pos.load(),
            volume,
            pitch,
        );
    }

    fn can_attack(&self, target: &dyn EntityBase) -> bool {
        let target_entity = target.get_entity();
        if target_entity.entity_type == &EntityType::GHAST {
            return false;
        }
        if let Some(tamable) = self.as_tamable()
            && tamable.is_owned_by(&target_entity.entity_uuid)
        {
            return false;
        }
        self.get_mob_entity().living_entity.can_attack(target)
    }

    /// Takes the navigation lock, so callers must not already hold it.
    fn is_navigator_idle(&self) -> bool {
        self.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_idle()
    }

    fn has_line_of_sight(&self, target: &crate::entity::Entity) -> bool {
        let mob_entity = self.get_mob_entity();
        mob_entity
            .sensing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .has_line_of_sight(&mob_entity.living_entity.entity, target)
    }

    /// Vanilla `Mob.requiresCustomPersistence`: riding or leashed mobs stay.
    fn requires_custom_persistence(&self) -> bool {
        let entity = self.get_entity();
        entity.has_vehicle() || entity.is_leashed()
    }

    /// Whether daylight burns this mob. Default: entity tag `burn_in_daylight`.
    fn burns_in_daylight(&self) -> bool {
        self.get_entity()
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_BURN_IN_DAYLIGHT)
    }

    /// Slot whose item shields the mob from the sun and takes the wear instead.
    fn sun_protection_slot(&self) -> EquipmentSlot {
        EquipmentSlot::HEAD
    }

    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        true
    }

    fn get_max_look_yaw_change(&self) -> f32 {
        10.0
    }

    fn get_max_look_pitch_change(&self) -> f32 {
        40.0
    }

    fn get_max_head_rotation(&self) -> f32 {
        75.0
    }

    /// Vanilla `Mob.clampHeadRotationToBody`.
    fn clamp_head_rotation_to_body(&self) {
        let entity = self.get_entity();
        let limit = self.get_max_head_rotation();
        let head_y_rot = entity.head_yaw.load();
        let body_y_rot = entity.body_yaw.load();
        let delta = pumpkin_util::math::wrap_degrees(body_y_rot - head_y_rot);
        let target_delta = delta.clamp(-limit, limit);
        entity.head_yaw.store(head_y_rot + delta - target_delta);
    }

    fn get_mob_entity(&self) -> &MobEntity;

    fn mob_bedrock_identifier(&self) -> Option<&'static str> {
        None
    }

    /// Vanilla `LivingEntity.canBreatheUnderwater`.
    fn can_breathe_underwater(&self) -> bool {
        self.get_entity()
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_CAN_BREATHE_UNDER_WATER)
    }

    /// Vanilla `Entity.getMaxAirSupply`.
    fn max_air_supply(&self) -> i32 {
        crate::entity::breath::MAX_AIR
    }

    /// Vanilla `LivingEntity.increaseAirSupply`.
    fn increase_air_supply(&self, current: i32) -> i32 {
        (current + crate::entity::breath::AIR_RECOVERY_RATE).min(self.max_air_supply())
    }

    /// The `handleAirSupply` that vanilla's water mobs run after `baseTick`,
    /// given the air from before it. Most mobs have none.
    fn handle_air_supply(&self, _pre_tick_air: i32) {}

    /// Metadata which must accompany this mob whenever it is spawned for a Java client.
    fn mob_java_spawn_metadata(&self, _version: JavaMinecraftVersion) -> Option<Box<[u8]>> {
        None
    }

    /// Metadata which must accompany this mob whenever it is spawned for a Bedrock client.
    fn mob_bedrock_spawn_metadata(
        &self,
    ) -> Option<pumpkin_protocol::bedrock::client::set_actor_data::SyncedActorDataList> {
        None
    }

    fn get_job_site(&self) -> Option<BlockPos> {
        None
    }

    fn is_job_site_pending(&self) -> bool {
        false
    }

    fn release_pending_job_site(&self, _position: BlockPos) {}

    fn get_trading_player(&self) -> Option<Arc<Player>> {
        None
    }

    fn clear_trading_player(&self) {}

    fn get_home(&self) -> Option<BlockPos> {
        None
    }

    fn get_path_aware_entity(&self) -> Option<&dyn PathAwareEntity> {
        None
    }

    fn get_item_steerable(&self) -> Option<&dyn crate::entity::item_steerable::ItemSteerable> {
        None
    }

    /// Vanilla `getControllingPassenger`. Vehicles a player steers override it and fall back
    /// to [`mob_controlling_passenger`].
    fn get_controlling_passenger(&self) -> Option<Arc<dyn EntityBase>> {
        mob_controlling_passenger(self.get_mob_entity())
    }

    /// Vanilla `Mob.isEffectiveAi`: the server runs this mob's AI unless it has none or a
    /// player steers it.
    fn is_effective_ai(&self) -> bool {
        !self.get_mob_entity().is_no_ai() && !self.is_client_authoritative()
    }

    /// Vanilla `LivingEntity.tickRidden`, which the server's `travelRidden` runs every tick
    /// `controller` steers this mob.
    fn tick_ridden(&self, _controller: &Player) {}

    /// Vanilla `PlayerRideableJumping.canJump`, for mobs that implement it.
    fn can_jump(&self) -> bool {
        false
    }

    /// Vanilla `PlayerRideableJumping.handleStartJump`: the rider started a jump of
    /// `jump_scale`.
    fn handle_start_jump(&self, _jump_scale: i32) {}

    /// Vanilla `PlayerRideableJumping.handleStopJump`.
    fn handle_stop_jump(&self) {}

    /// Vanilla `HasCustomInventoryScreen.openCustomInventoryScreen`, for mobs that have one.
    fn open_custom_inventory_screen(&self, _player: &Arc<Player>) {}

    /// Vanilla `LivingEntity.canUseSlot`.
    fn can_use_slot(&self, _slot: &EquipmentSlot) -> bool {
        true
    }

    /// Vanilla `LivingEntity.isEquippableInSlot`.
    fn is_equippable_in_slot(&self, stack: &ItemStack, slot: &EquipmentSlot) -> bool {
        equipment::is_equippable_in_slot(self.get_entity().entity_type, stack, slot)
            && self.can_use_slot(slot)
    }

    /// Vanilla `Mob.hasValidEquippableItemForSlot`.
    fn has_valid_equippable_item_for_slot(&self, slot: &EquipmentSlot) -> bool {
        let stack = self.get_mob_entity().get_item_slot(slot);
        !stack.is_empty() && self.is_equippable_in_slot(&stack, slot)
    }

    /// Vanilla `Mob.isSaddled`.
    fn is_saddled(&self) -> bool {
        self.has_valid_equippable_item_for_slot(&EquipmentSlot::SADDLE)
    }

    /// A vanilla `getEquipSound` override for `slot`; `None` plays the item's own sound.
    fn equip_sound(&self, _slot: &EquipmentSlot) -> Option<Sound> {
        None
    }

    /// Vanilla `LivingEntity.setItemSlot`, with the equip sound and game event of its
    /// `onEquipItem` that [`MobEntity::set_item_slot`] leaves out.
    fn set_item_slot_and_notify(&self, slot: &EquipmentSlot, stack: ItemStack) {
        let old = self.get_mob_entity().set_item_slot(slot, stack.clone());
        equipment::notify_equip(self, slot, &old, &stack);
        self.on_equip_item(slot);
    }

    /// Vanilla `Equippable.equipOnTarget`, which `ItemStack.interactLivingEntity` runs for
    /// items equipped by using them on a mob (saddles, harnesses, nautilus armor).
    fn equip_on_target(&self, player: &Player, item_stack: &mut ItemStack) -> bool {
        let Some(equippable) = item_stack.get_data_component::<EquippableImpl>() else {
            return false;
        };
        if !equippable.equip_on_interact {
            return false;
        }
        let slot = equippable.slot;
        if !self.is_equippable_in_slot(item_stack, slot)
            || !self.get_mob_entity().get_item_slot(slot).is_empty()
            || !self.get_entity().is_alive()
        {
            return false;
        }
        let equipped = item_stack.split_unless_creative(player.gamemode.load(), 1);
        self.set_item_slot_and_notify(slot, equipped);
        self.get_mob_entity().set_guaranteed_drop(slot);
        true
    }

    /// Vanilla `Mob.checkSpawnObstruction`.
    fn check_spawn_obstruction(&self, world: &World) -> bool {
        let entity = self.get_entity();
        let bounding_box = entity.bounding_box.load();
        !world.contains_any_liquid(bounding_box)
            && world.is_unobstructed(Some(entity), &bounding_box)
    }

    /// Per-mob tick hook called each tick before AI runs. Override for mob-specific logic.
    fn mob_tick(&self, _caller: &dyn EntityBase) {}

    /// The part of a vanilla `tick` override that runs before `super.tick()`, ahead of
    /// everything `Mob.tick` does.
    fn mob_pre_tick(&self) {}

    /// The start of vanilla `aiStep`, where subclasses run their own code before
    /// `super.aiStep()`: after `baseTick`, before the AI step and travel.
    fn ai_step(&self, _caller: &dyn EntityBase) {}

    /// The end of a vanilla `aiStep` override, after `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {}

    /// A vanilla `aiStep` override that never calls `super.aiStep()`. Returns `true` when it
    /// ran in place of the shared one.
    fn custom_ai_step(&self, _caller: &dyn EntityBase) -> bool {
        false
    }

    /// A vanilla `tickDeath` override, called every tick where `baseTick` would call it.
    /// Returns `true` when the mob runs its own (only while dead or dying, like vanilla)
    /// instead of the shared removal after the 20-tick animation.
    fn tick_death(&self, _caller: &dyn EntityBase) -> bool {
        false
    }

    /// The part of a vanilla `die` override that runs before `super.die`.
    fn before_die(&self) {}

    /// The mob's own part of vanilla `Entity.canUsePortal`, which portal blocks check before
    /// they move an entity.
    fn can_use_portal(&self) -> bool {
        true
    }

    /// Vanilla `Mob.onPathfindingStart`, applied to the snapshot the search reads; vanilla's
    /// `onPathfindingDone` then only undoes it, so it has no hook.
    fn on_pathfinding_start(&self, _mob_data: &mut MobData) {}

    /// A vanilla `handleKillingBlow` override. Returns `true` when it replaces the default,
    /// which marks the mob dead.
    fn handle_killing_blow(&self) -> bool {
        false
    }

    /// `false` while a vanilla `knockback` override ignores knockback.
    fn takes_knockback(&self) -> bool {
        true
    }

    /// Vanilla `hurtArmor`, which wears nothing unless a mob overrides it.
    fn hurt_armor(&self, _damage_type: DamageType, _damage: f32) {}

    /// Vanilla `Entity.kill`, which some mobs override.
    fn mob_kill(&self, caller: &dyn EntityBase) {
        caller.damage(caller, f32::MAX, DamageType::GENERIC_KILL);
    }

    fn post_tick(&self) {}

    /// Vanilla `LivingEntity.jumpFromGround` as `aiStep` calls it, which a mob may override.
    fn jump_from_ground(&self) {
        self.get_mob_entity().living_entity.jump();
    }

    /// Vanilla `jumpInLiquid`, which some mobs override.
    fn jump_in_liquid(&self, _lava: bool) {
        self.get_mob_entity().jump_in_liquid();
    }

    fn get_preferred_weapon_type(&self) -> Option<&'static pumpkin_data::tag::Tag> {
        None
    }

    fn can_replace_current_item(
        &self,
        new_item: &ItemStack,
        current_item: &ItemStack,
        slot: &EquipmentSlot,
    ) -> bool {
        equipment::can_replace_current_item(
            self.get_mob_entity(),
            self.get_preferred_weapon_type(),
            new_item,
            current_item,
            slot,
        )
    }

    /// Called before damage is applied. Return `false` to cancel the damage entirely.
    fn pre_damage(&self, _damage_type: DamageType, _source: Option<&dyn EntityBase>) -> bool {
        true
    }

    fn on_damage(&self, _damage_type: DamageType, _source: Option<&dyn EntityBase>) {}

    /// The end of a vanilla `actuallyHurt` override, after the health is taken: `source` is
    /// the direct entity, `cause` the one responsible.
    fn actually_hurt(
        &self,
        _damage_type: DamageType,
        _source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) {
    }

    /// Vanilla `hurtServer`, for mobs whose override reorders it; `mob_hurt_server` is the
    /// shared path (vanilla `super.hurtServer`).
    fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        mob_hurt_server(self, caller, amount, damage_type, position, source, cause)
    }

    /// The start of a vanilla `hurtServer` override, before the damage is applied:
    /// `source` is the direct entity, `cause` the one responsible.
    fn before_hurt(
        &self,
        _amount: f32,
        _damage_type: DamageType,
        _source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) {
    }

    /// The start of a vanilla `dropAllDeathLoot` override, before the loot table and the
    /// equipment drop.
    fn before_drop_all_death_loot(&self) {}

    /// What a vanilla `dropCustomDeathLoot` override adds after the equipment drops.
    fn drop_custom_death_loot(&self) {}

    /// A vanilla `dropEquipment` override: what drops on death whatever the `mob_drops` rule.
    fn drop_equipment(&self) {}

    /// Vanilla `killedEntity`, run on the attacker as `victim` dies. `false` keeps the
    /// victim's loot, equipment and experience from dropping.
    fn killed_entity(&self, _victim: &dyn EntityBase) -> bool {
        true
    }

    /// Vanilla `LivingEntity.isSleeping`, for mobs that sleep without a bed.
    fn is_sleeping(&self) -> bool {
        false
    }

    /// The mob's own part of vanilla `LivingEntity.canBeSeenAsEnemy`.
    fn can_be_seen_as_enemy(&self) -> bool {
        true
    }

    /// Vanilla `LivingEntity.isSensitiveToWater`: takes drowning damage in water or rain.
    fn is_sensitive_to_water(&self) -> bool {
        false
    }

    /// A vanilla `canFreeze` override that rules freezing out, like `Skeleton`'s.
    fn can_freeze(&self) -> bool {
        true
    }

    /// Whether the move and look controls skip their tick, as vanilla's axolotl
    /// controls do while it plays dead.
    fn controls_paused(&self) -> bool {
        false
    }

    /// Whether this tick's navigation is skipped, as vanilla `Bee`'s navigation does
    /// while it pollinates.
    fn skips_navigation_tick(&self) -> bool {
        false
    }

    /// Whether the look control skips its tick, as vanilla's `BeeLookControl` does
    /// while the bee is angry.
    fn skips_look_control_tick(&self) -> bool {
        false
    }

    /// Replaces the look control's tick, as vanilla's `HappyGhastLookControl` does.
    /// Returns true when handled.
    fn custom_look_control_tick(&self, _look_control: &mut LookControl) -> bool {
        false
    }

    /// Vanilla `LookControl.resetXRotOnTick`.
    fn look_control_resets_x_rot(&self) -> bool {
        true
    }

    /// The damage type of the mob's melee attack: vanilla `doHurtTarget`'s source.
    fn melee_damage_type(&self) -> DamageType {
        DamageType::MOB_ATTACK
    }

    fn on_attack(&self, _target: &dyn EntityBase) {}

    /// Vanilla `LivingEntity.playAttackSound`, which `Mob.doHurtTarget` plays on a hit.
    fn play_attack_sound(&self) {}

    /// Vanilla `Mob.canHoldItem`.
    fn can_hold_item(&self, _stack: &ItemStack) -> bool {
        true
    }

    /// Vanilla `Mob.getPickupReach`: how far past its box the mob picks up items.
    fn pickup_reach(&self) -> Vector3<i32> {
        MobEntity::ITEM_PICKUP_REACH
    }

    /// Vanilla `Mob.pickUpItem`, with `Raider`'s override for every raider.
    fn mob_pick_up_item(&self, item: &ItemEntity) {
        if self.as_raider().is_some() {
            raider::pick_up_item(self, item);
        } else {
            equipment::pick_up_item(self, item);
        }
    }

    /// Vanilla `doHurtTarget` as `MeleeAttackGoal` calls it.
    fn goal_do_hurt_target(&self, attacker: &dyn EntityBase, target: &dyn EntityBase) -> bool {
        self.get_mob_entity().try_attack(attacker, target)
    }

    /// Vanilla `doHurtTarget` as the brain's `MeleeAttack` calls it, with this mob's brain
    /// held in `tick`.
    fn do_hurt_target(&self, tick: &mut BrainTick<'_>, target: &Arc<dyn EntityBase>) -> bool {
        let attacker: &dyn EntityBase = tick.mob;
        self.get_mob_entity().try_attack(attacker, target.as_ref())
    }

    fn on_eating_grass(&self) {}

    fn modify_incoming_damage(&self, amount: f32, _damage_type: DamageType) -> f32 {
        amount
    }

    fn can_attack_with_owner(&self, _target: &dyn EntityBase, _owner: &dyn EntityBase) -> bool {
        true
    }

    fn get_mob_gravity(&self) -> f64 {
        self.get_mob_entity().living_entity.get_gravity()
    }

    fn get_mob_y_velocity_drag(&self) -> Option<f64> {
        None
    }

    fn as_ageable(&self) -> Option<&dyn crate::entity::ageable::AgeableMob> {
        None
    }

    /// turns the mob into a baby. False when it has no baby form.
    fn spawn_as_baby(&self) -> bool {
        self.as_ageable().is_some_and(|ageable| {
            ageable.set_baby(true);
            true
        })
    }

    fn as_custom_sound(&self) -> Option<&dyn crate::entity::custom_sound::CustomSound> {
        None
    }

    fn as_animal(&self) -> Option<&dyn crate::entity::passive::animal::Animal> {
        None
    }

    /// How much this mob likes standing on `pos`, used to rank stroll candidates: vanilla
    /// `PathfinderMob.getWalkTargetValue`, with `Animal`'s and `Monster`'s overrides.
    fn get_walk_target_value(&self, pos: &BlockPos) -> f32 {
        if let Some(animal) = self.as_animal() {
            return animal.animal_walk_target_value(pos);
        }
        if is_monster(self.get_entity().entity_type) {
            return monster_walk_target_value(&self.get_entity().world.load(), pos);
        }
        0.0
    }

    /// Vanilla `Entity.omnidirectionalAirMover`: drags vertically like horizontally in the air.
    /// Vanilla `Entity.isSteppingCarefully`, which cats and ocelots extend to crouching.
    fn mob_is_stepping_carefully(&self) -> bool {
        self.get_entity().is_sneaking()
    }

    fn omnidirectional_air_mover(&self) -> bool {
        false
    }

    /// Vanilla `Bucketable.loadFromBucketTag`: what a mob placed from a bucket reads back
    /// from the bucket's `bucket_entity_data`.
    fn load_from_bucket_tag(&self, _tag: &NbtCompound) {}

    /// A vanilla `actuallyHurt` override that takes the hit instead of the mob: whether it did.
    fn absorb_hurt(
        &self,
        _caller: &dyn EntityBase,
        _damage_type: DamageType,
        _damage: f32,
    ) -> bool {
        false
    }

    /// A vanilla `onClimbable` override: spiders climb walls.
    fn mob_on_climbable(&self) -> Option<bool> {
        None
    }

    /// A vanilla `maxUpStep` override.
    fn mob_max_up_step(&self) -> f32 {
        crate::entity::living_max_up_step(
            &self.get_mob_entity().living_entity,
            self.get_controlling_passenger(),
        )
    }

    /// A vanilla `getPassengerAttachmentPoint` override.
    fn mob_passenger_attachment_point(
        &self,
        passenger: &dyn EntityBase,
        dimensions: EntityDimensions,
        _scale: f32,
    ) -> Vector3<f64> {
        self.get_entity()
            .default_passenger_attachment_point(passenger, &dimensions.attachments)
    }

    /// A vanilla `getVehicleAttachmentPoint` override.
    fn mob_vehicle_attachment_point(&self, _vehicle: &dyn EntityBase) -> Vector3<f64> {
        let entity = self.get_entity();
        entity
            .entity_dimension
            .load()
            .attachments
            .vehicle(entity.yaw.load())
    }

    /// What a vanilla `positionRider` override does after placing the passenger.
    fn mob_after_position_rider(&self, _passenger: &dyn EntityBase) {}

    /// What a vanilla `rideTick` override does after `LivingEntity.rideTick`.
    fn mob_after_ride_tick(&self) {}

    /// A vanilla `checkFallDamage` override. Flyers such as the phantom don't add up falls.
    fn mob_check_fall_damage(
        &self,
        caller: &dyn EntityBase,
        ya: f64,
        on_ground: bool,
        state: &'static BlockState,
        pos: BlockPos,
    ) {
        self.get_mob_entity()
            .living_entity
            .check_fall_damage(caller, ya, on_ground, state, pos);
    }

    /// A vanilla `causeFallDamage` override.
    fn mob_cause_fall_damage(
        &self,
        caller: &dyn EntityBase,
        fall_distance: f64,
        damage_modifier: f32,
        damage_type: DamageType,
    ) -> bool {
        let living = &self.get_mob_entity().living_entity;
        let entity_type = living.entity.entity_type;
        if entity_type == &EntityType::LLAMA || entity_type == &EntityType::TRADER_LLAMA {
            return llama_cause_fall_damage(
                self,
                caller,
                fall_distance,
                damage_modifier,
                damage_type,
            );
        }
        if self.as_abstract_horse().is_some()
            || entity_type == &EntityType::CAMEL
            || entity_type == &EntityType::CAMEL_HUSK
        {
            return abstract_horse_cause_fall_damage(
                self,
                caller,
                fall_distance,
                damage_modifier,
                damage_type,
            );
        }
        living.cause_fall_damage(caller, fall_distance, damage_modifier, damage_type)
    }

    /// A vanilla `calculateFallDamage` override.
    fn calculate_fall_damage(&self, fall_distance: f64, damage_modifier: f32) -> i32 {
        self.get_mob_entity()
            .living_entity
            .calculate_fall_damage(fall_distance, damage_modifier)
    }

    /// A vanilla `LivingEntity.knockback` override. Returns true when the mob moved itself;
    /// `attacker` is the damage source's responsible entity.
    fn custom_knockback(
        &self,
        _power: f64,
        _xd: f64,
        _zd: f64,
        _attacker: Option<&Entity>,
        _damage: f32,
        _comes_from_effect: bool,
    ) -> bool {
        false
    }

    /// The end of a vanilla `LivingEntity.doPush` override: this mob pushed `entity`.
    fn do_push(&self, _entity: &dyn EntityBase) {}

    /// A vanilla `Entity.deflection` override.
    fn mob_deflection(&self, _projectile: &dyn EntityBase) -> ProjectileDeflectionType {
        ProjectileDeflectionType::of_entity_type(self.get_entity())
    }

    /// A vanilla `Entity.getFluidJumpThreshold` override.
    fn fluid_jump_threshold(&self) -> Option<f64> {
        None
    }

    /// The end of a vanilla `LivingEntity.travelInFluid` override.
    fn travel_in_fluid_end(&self, _water: bool) {}

    /// The horse family's shared `AbstractHorse` behaviour.
    fn as_abstract_horse(
        &self,
    ) -> Option<&dyn crate::entity::passive::abstract_horse::AbstractHorse> {
        None
    }

    fn as_tamable(&self) -> Option<&dyn crate::entity::passive::tamable::TamableAnimal> {
        None
    }

    fn as_schooling_fish(&self) -> Option<&crate::entity::passive::schooling_fish::SchoolingFish> {
        None
    }

    fn as_patrolling_monster(&self) -> Option<&dyn patrol::PatrollingMonster> {
        None
    }

    fn as_raider(&self) -> Option<&dyn raider::Raider> {
        None
    }

    /// Must return `Some(self)` for every `NeutralMob` implementor. Not compiler-enforced.
    /// Vanilla `instanceof Zombie`.
    fn as_zombie(&self) -> Option<&zombie::ZombieEntityBase> {
        None
    }

    fn as_neutral(&self) -> Option<&dyn neutral::NeutralMob> {
        None
    }

    fn as_iron_golem(&self) -> Option<&crate::entity::passive::iron_golem::IronGolemEntity> {
        None
    }

    fn as_crossbow_attack_mob(&self) -> Option<&dyn crossbow_attack_mob::CrossbowAttackMob> {
        None
    }

    /// Vanilla `Mob.finalizeSpawn`, run before the mob enters the world on spawns that
    /// vanilla finalizes. Overrides call `finalize_spawn_base` first. Riders queued with
    /// `add_pending_rider` are added by the world together with the mob.
    fn finalize_spawn(
        &self,
        _world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<spawn::SpawnGroupData>,
    ) -> Option<spawn::SpawnGroupData> {
        self.get_mob_entity().finalize_spawn_base();
        group_data
    }

    /// Vanilla `Mob.populateDefaultEquipmentSlots`, on the `level.getRandom()` that
    /// `finalizeSpawn` passes down.
    fn populate_default_equipment_slots(
        &self,
        world: &Arc<World>,
        difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
    ) {
        populate_default_armor(self, world, difficulty);
    }

    /// Vanilla `Mob.populateDefaultEquipmentEnchantments`: the weapon, then each armor slot in
    /// `EquipmentSlot.VALUES` order (feet first).
    fn populate_default_equipment_enchantments(
        &self,
        world: &Arc<World>,
        difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
    ) {
        self.enchant_spawned_weapon(world, difficulty);
        for slot in &EquipmentSlot::VALUES {
            if slot.slot_type() == pumpkin_data::data_component_impl::EquipmentType::HumanoidArmor {
                self.enchant_spawned_armor(world, slot, difficulty);
            }
        }
    }

    /// Vanilla `Mob.enchantSpawnedWeapon`.
    fn enchant_spawned_weapon(
        &self,
        world: &Arc<World>,
        difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
    ) {
        enchant_spawned_equipment(
            self,
            world,
            &EquipmentSlot::MAIN_HAND,
            MobEntity::MAX_ENCHANTED_WEAPON_CHANCE,
            difficulty,
        );
    }

    /// Vanilla `Mob.enchantSpawnedArmor`.
    fn enchant_spawned_armor(
        &self,
        world: &Arc<World>,
        slot: &EquipmentSlot,
        difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
    ) {
        enchant_spawned_equipment(
            self,
            world,
            slot,
            MobEntity::MAX_ENCHANTED_ARMOR_CHANCE,
            difficulty,
        );
    }

    /// Runs after navigation and before the movement controls, where vanilla ticks a mob's brain.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {}

    /// Vanilla `Mob.wantsToPickUp`, which a plain mob answers with `canHoldItem`. Takes the
    /// brain because the nearest item sensor asks from inside the brain tick, where locking it
    /// again would deadlock.
    fn wants_to_pick_up(&self, _brain: &Brain, stack: &ItemStack) -> bool {
        self.can_hold_item(stack)
    }

    /// Vanilla `Mob.canPickUpLoot`, which a plain mob answers from its saved flag. Takes the
    /// brain for the same reason as `wants_to_pick_up`.
    fn can_pick_up_loot(&self, _brain: &Brain) -> bool {
        self.get_mob_entity().can_pick_up_loot()
    }

    /// The part of a vanilla `considersEntityAsAlly` override before `super`'s.
    fn mob_considers_entity_as_ally(&self, _other: &dyn EntityBase) -> bool {
        false
    }

    /// Vanilla `Mob.canUseNonMeleeWeapon`, false unless the mob knows how to fire what it holds.
    fn can_use_non_melee_weapon(&self, _stack: &ItemStack) -> bool {
        false
    }

    /// Vanilla reads `ProjectileWeaponItem.getDefaultProjectileRange` off the main hand; a mob
    /// answers `None` when it is not holding a weapon it can use at range.
    fn non_melee_weapon_range(&self) -> Option<i32> {
        None
    }

    /// Runs inside the brain lock right after `Brain::tick`, where vanilla mobs run their own
    /// `updateActivity`.
    fn after_brain_tick(&self, _tick: &mut BrainTick<'_>) {}

    /// Moves the mob for this tick in place of the usual walking and swimming
    /// physics, returning whether it did; vanilla mobs that override `travel`.
    fn custom_travel(&self, _caller: &dyn EntityBase) -> bool {
        false
    }

    /// Whether currents push this mob along. Water animals hold their ground.
    /// Whether the mob runs `updatePersistentAnger` in its own `customServerAiStep`, so a
    /// `NoAI` mob's anger stands still, instead of the shared call every tick.
    fn updates_anger_in_ai_step(&self) -> bool {
        false
    }

    /// Extra conditions for vanilla `isImmobile` on top of dying: an immobile mob skips its
    /// AI step and loses its movement input.
    fn mob_is_immobile(&self) -> bool {
        false
    }

    /// Vanilla `blockedByItem`: `defender` blocked this mob's attack with an item. Pumpkin
    /// only blocks fully, where vanilla's default does nothing.
    fn blocked_by_item(
        &self,
        _defender: &dyn EntityBase,
        _damage_type: DamageType,
        _damage: f32,
        _fully_blocked: bool,
    ) {
    }

    /// Vanilla `isPushable`, which some mobs turn off.
    fn mob_is_pushable(&self) -> bool {
        true
    }

    /// Whether the mob pushes the entities it touches; vanilla bats override
    /// `pushEntities` to do nothing.
    fn pushes_entities(&self) -> bool {
        true
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        true
    }

    /// Vanilla `Entity.updateSwimming`, run right after the fluid update.
    fn update_swimming(&self) {
        self.get_entity().update_swimming_base();
    }

    /// Builds this mob's brain from its saved memories; goal mobs keep the brain-dead default.
    fn make_brain(&self, _packed: &PackedMemories) -> Brain {
        Brain::default()
    }

    fn mob_write_nbt(&self, _nbt: &mut NbtCompound) {}

    fn mob_read_nbt(&self, _nbt: &NbtCompound) {}

    /// Mob tags vanilla reads before `super.readAdditionalSaveData`.
    fn mob_read_nbt_before_super(&self, _nbt: &NbtCompound) {}

    /// Vanilla `LivingEntity.onEquipItem`: an equipment slot changed. Skeletons re-pick their
    /// weapon goal here.
    fn on_equip_item(&self, _slot: &EquipmentSlot) {}

    /// Vanilla `Mob.getTarget`: the stored target while it is still a valid one, so a player
    /// who turned creative or spectator, or a target the mob may no longer attack, reads as none.
    fn get_target(&self) -> Option<Arc<dyn EntityBase>> {
        self.as_valid_target(self.get_mob_entity().get_target_unchecked())
    }

    /// Drops a target the mob is not allowed to attack, such as a creative player.
    fn as_valid_target(&self, target: Option<Arc<dyn EntityBase>>) -> Option<Arc<dyn EntityBase>> {
        let target = target?;
        if !EntityPredicate::ExceptCreativeOrSpectator.test(target.get_entity()) {
            return None;
        }
        self.can_attack(target.as_ref()).then_some(target)
    }

    /// Set or clear the mob's target. Override to add side effects when targeting changes.
    fn set_mob_target(&self, target: Option<Arc<dyn EntityBase>>) {
        set_mob_target_base(self, target);
    }

    /// Vanilla `Mob.mobInteract`, which runs after the name tag, the spawn egg and the leash
    /// had their turn. Returns whether it consumed the interaction.
    fn mob_interact(&self, _player: &Arc<Player>, _item_stack: &mut ItemStack) -> bool {
        false
    }

    /// The start of a vanilla `interact` override before `super.interact`, which runs even on
    /// a dead mob.
    fn before_interact(&self) {}

    /// Vanilla `Mob.canShearEquipment`: not while something rides it.
    fn can_shear_equipment(&self, _player: &Player) -> bool {
        !self.get_entity().has_passengers()
    }

    /// Vanilla `Leashable.canBeLeashed`: `Mob`'s "anything but an `Enemy`", with the fixed
    /// answers of `AmbientCreature`, `WaterAnimal`, `AbstractVillager`, `Turtle`, `Panda`,
    /// `Hoglin` and `Zoglin`. Mobs whose answer changes override it.
    fn can_be_leashed(&self) -> bool {
        let entity_type = self.get_entity().entity_type;
        if [
            &EntityType::BAT,
            &EntityType::COD,
            &EntityType::SALMON,
            &EntityType::PUFFERFISH,
            &EntityType::TROPICAL_FISH,
            &EntityType::TADPOLE,
            &EntityType::VILLAGER,
            &EntityType::WANDERING_TRADER,
            &EntityType::TURTLE,
            &EntityType::PANDA,
        ]
        .contains(&entity_type)
        {
            return false;
        }
        entity_type == &EntityType::HOGLIN
            || entity_type == &EntityType::ZOGLIN
            || !entity_type.is_enemy
    }

    /// Vanilla `Mob.onLeashRemoved`: losing the leash clears the home it gave.
    fn on_leash_removed(&self) {
        if !self.get_entity().is_leashed() {
            self.get_mob_entity().clear_home();
        }
    }

    fn tame(&self, player: &Arc<Player>) {
        let mob = self.get_mob_entity();
        let mut event = crate::plugin::api::events::entity::entity_tame::EntityTameEvent::new(
            mob.living_entity.entity.entity_id,
            player.clone(),
        );
        if let Some(server) = mob.living_entity.entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }

    fn breed(&self, father_id: i32, mother_id: i32, child_id: i32) {
        let mob = self.get_mob_entity();
        let mut event = crate::plugin::api::events::entity::entity_breed::EntityBreedEvent::new(
            father_id, mother_id, child_id,
        );
        if let Some(server) = mob.living_entity.entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }

    fn dye(
        &self,
        color: crate::plugin::api::events::entity::entity_dye::DyeColor,
        player: Option<&Arc<Player>>,
    ) {
        let mob = self.get_mob_entity();
        let mut event = crate::plugin::api::events::entity::entity_dye::EntityDyeEvent::new(
            mob.living_entity.entity.entity_id,
            color,
            player.cloned(),
        );
        if let Some(server) = mob.living_entity.entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }

    fn enter_love_mode(&self, human_entity_id: Option<i32>, ticks_in_love: i32) {
        let mob = self.get_mob_entity();
        let mut event = crate::plugin::api::events::entity::entity_enter_love_mode::EntityEnterLoveModeEvent::new(
            mob.living_entity.entity.entity_id,
            human_entity_id,
            ticks_in_love,
        );
        if let Some(server) = mob.living_entity.entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }

    fn transform(&self, new_entity_id: i32, transform_reason: String) {
        let mob = self.get_mob_entity();
        let mut event =
            crate::plugin::api::events::entity::entity_transform::EntityTransformEvent::new(
                mob.living_entity.entity.entity_id,
                new_entity_id,
                transform_reason,
            );
        if let Some(server) = mob.living_entity.entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }

    fn break_door(&self, block_pos: BlockPos) {
        let mob = self.get_mob_entity();
        let mut event =
            crate::plugin::api::events::entity::entity_break_door::EntityBreakDoorEvent::new(
                mob.living_entity.entity.entity_id,
                block_pos,
            );
        if let Some(server) = mob.living_entity.entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }

    fn enter_block(&self, block_pos: BlockPos) {
        let mob = self.get_mob_entity();
        let mut event =
            crate::plugin::api::events::entity::entity_enter_block::EntityEnterBlockEvent::new(
                mob.living_entity.entity.entity_id,
                block_pos,
            );
        if let Some(server) = mob.living_entity.entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }

    fn interact(&self, block_pos: BlockPos) {
        let mob = self.get_mob_entity();
        let mut event =
            crate::plugin::api::events::entity::entity_interact::EntityInteractEvent::new(
                mob.living_entity.entity.entity_id,
                block_pos,
            );
        if let Some(server) = mob.living_entity.entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }

    fn place_block(&self, block_pos: BlockPos, block_name: String) {
        let mob = self.get_mob_entity();
        let mut event = crate::plugin::api::events::entity::entity_place::EntityPlaceEvent::new(
            mob.living_entity.entity.entity_id,
            block_pos,
            block_name,
        );
        if let Some(server) = mob.living_entity.entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
    }

    fn mob_player_collision(&self, _player: &Arc<Player>) {}

    /// The start of a vanilla `push(Entity)` override, after the mob was pushed by `entity`.
    fn mob_push(&self, _entity: &dyn EntityBase) {}

    /// A vanilla `canBeCollidedWith` override; mobs aren't solid by default.
    fn mob_can_be_collided_with(&self, _other: Option<&dyn EntityBase>) -> bool {
        false
    }

    /// A vanilla `push(x, y, z)` override.
    fn mob_push_impulse(&self, impulse: Vector3<f64>) {
        self.get_entity().push_impulse(impulse);
    }

    /// The start of a vanilla `remove` override for a mob removed once its death
    /// animation ends (`RemovalReason.KILLED`).
    fn remove_killed(&self) {}

    fn get_owner_uuid(&self) -> Option<Uuid> {
        self.as_tamable()
            .and_then(crate::entity::passive::tamable::TamableAnimal::get_owner)
    }

    fn is_sitting(&self) -> bool {
        self.as_tamable()
            .is_some_and(crate::entity::passive::tamable::TamableAnimal::is_in_sitting_pose)
    }

    fn is_tamed(&self) -> bool {
        self.as_tamable()
            .is_some_and(crate::entity::passive::tamable::TamableAnimal::is_tame)
    }

    /// Vanilla `Mob.xpReward`.
    fn xp_reward(&self) -> u32 {
        self.get_entity().entity_type.experience_reward
    }

    /// Vanilla `shouldDropExperience`: no babies, though `Monster` drops it for its babies too.
    fn should_drop_experience(&self) -> bool {
        let entity = self.get_entity();
        !entity.is_baby() || is_monster(entity.entity_type)
    }

    /// Vanilla `getBaseExperienceReward`: `Animal`'s override for animals, `Mob`'s otherwise.
    fn get_base_experience_reward(&self) -> u32 {
        self.as_animal().map_or_else(
            || mob_base_experience_reward(self),
            crate::entity::passive::animal::Animal::animal_base_experience_reward,
        )
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(std::sync::atomic::Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(tracked_data::ageable_mob::DATA_BABY_ID, true);
        }
    }

    fn mob_set_variant_name(&self, _name: &str) {}
    fn mob_set_sound_variant_name(&self, _name: &str) {}

    fn mob_on_lightning_strike(
        &self,
        caller: &dyn EntityBase,
        lightning: &crate::entity::lightning::LightningBoltEntity,
    ) {
        self.get_mob_entity()
            .living_entity
            .on_lightning_strike(caller, lightning);
    }
}
/// The looting part of vanilla `Mob.aiStep`, right after the sun burn: every item in reach
/// that the mob wants, while it picks up loot and `mobGriefing` is on.
pub fn ai_step_looting(mob: &dyn Mob) {
    let mob_entity = mob.get_mob_entity();
    let living = &mob_entity.living_entity;
    let entity = &living.entity;
    let world = entity.world.load_full();
    if !living.is_alive()
        || living.dead.load(Relaxed)
        || !world.level_info.load().game_rules.mob_griefing
        || !mob_entity.with_brain(mob, |tick| mob.can_pick_up_loot(tick.brain))
    {
        return;
    }
    let reach = mob.pickup_reach();
    let pickup_box = entity.bounding_box.load().expand(
        f64::from(reach.x),
        f64::from(reach.y),
        f64::from(reach.z),
    );
    for candidate in world.get_entities_at_box(&pickup_box) {
        let Some(item) = candidate.get_item_entity() else {
            continue;
        };
        let stack = item
            .get_item_stack()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if !item.get_entity().removed.load(Relaxed)
            && !stack.is_empty()
            && item.get_pickup_delay() == 0
            && mob_entity.with_brain(mob, |tick| mob.wants_to_pick_up(tick.brain, &stack))
        {
            mob.mob_pick_up_item(item);
        }
    }
}

/// Vanilla `Mob.interact`: the name tag and spawn egg first, then `Entity.interact`'s leash
/// handling, then `mobInteract`, with an `entity_interact` game event when either of the
/// mob's own steps takes the click.
pub fn mob_interact_entry<M: Mob + ?Sized>(
    mob: &M,
    player: &Arc<Player>,
    item_stack: &mut ItemStack,
) -> bool {
    mob.before_interact();
    if !mob.get_mob_entity().living_entity.is_alive() {
        return false;
    }
    if check_and_handle_important_interactions(mob, player, item_stack) {
        entity_interact_game_event(mob, player);
        return true;
    }
    if entity_interact(mob, player, item_stack) {
        return true;
    }
    if mob.mob_interact(player, item_stack) {
        entity_interact_game_event(mob, player);
        return true;
    }
    false
}

/// `this.gameEvent(GameEvent.ENTITY_INTERACT, player)`.
fn entity_interact_game_event<M: Mob + ?Sized>(mob: &M, player: &Arc<Player>) {
    let entity = mob.get_entity();
    entity.world.load().game_event_from(
        Some(player.as_ref()),
        pumpkin_data::game_event::GameEvent::EntityInteract,
        entity.pos.load(),
    );
}

/// Vanilla `Mob.checkAndHandleImportantInteractions`: a name tag, then a spawn egg, used on
/// the mob before anything else.
fn check_and_handle_important_interactions<M: Mob + ?Sized>(
    mob: &M,
    player: &Arc<Player>,
    item_stack: &mut ItemStack,
) -> bool {
    let is_name_tag = item_stack.item == &Item::NAME_TAG;
    let is_spawn_egg = pumpkin_data::entity::entity_from_egg(item_stack.item.id).is_some();
    if !is_name_tag && !is_spawn_egg {
        return false;
    }
    let entity = mob.get_entity();
    let world = entity.world.load();
    let (Some(server), Some(target)) = (
        world.server.upgrade(),
        world.get_entity_by_id(entity.entity_id),
    ) else {
        return false;
    };
    // `itemStack.interactLivingEntity` for the name tag, `spawnOffspringFromSpawnEgg` for the
    // egg; both pass when they did nothing.
    server
        .item_registry
        .use_on_entity(item_stack, player, target)
}

/// Vanilla `Entity.interact` for a mob: a sneaking player hands the mobs it leads over to this
/// one, shears cut the leashes on this mob or shear off its equipment, the holder takes its lead
/// back, and a lead ties up a mob that isn't held by a player.
fn entity_interact<M: Mob + ?Sized>(
    mob: &M,
    player: &Arc<Player>,
    item_stack: &mut ItemStack,
) -> bool {
    let entity = mob.get_entity();
    let world = entity.world.load_full();
    let player_base: &dyn EntityBase = player.as_ref();
    let sneaking = player.get_entity().is_sneaking();
    if sneaking && mob.can_be_leashed() && !entity.is_baby() {
        // `leashableInArea(this, l -> l.getLeashHolder() == player)`.
        let mut any_leashed = false;
        let this = world.get_entity_by_id(entity.entity_id);
        for leashed in leashed_in_area(&world, entity, player.entity_id()) {
            if let Some(this) = &this
                && can_have_a_leash_attached_to(leashed.as_ref(), entity)
            {
                let leashed_entity = leashed.get_entity();
                leashed_entity.leash_to(this.clone());
                if let Some(vehicle) = leashed_entity.get_vehicle() {
                    vehicle
                        .get_entity()
                        .remove_passenger(leashed_entity.entity_id);
                }
                any_leashed = true;
            }
        }
        if any_leashed {
            world.game_event_from(
                Some(player_base),
                pumpkin_data::game_event::GameEvent::EntityAction,
                entity.block_pos.load().to_centered_f64(),
            );
            mob.play_sound_at_self(Sound::ItemLeadTied, 1.0, 1.0);
            return true;
        }
    }

    if item_stack.item == &Item::SHEARS && shear_off_all_leash_connections(mob, player) {
        player.damage_held_item(1);
        return true;
    }
    if item_stack.item == &Item::SHEARS
        && mob.can_shear_equipment(player)
        && !sneaking
        && attempt_to_shear_equipment(mob, player)
    {
        return true;
    }

    let holder = entity
        .leashed_to
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if holder
        .as_ref()
        .is_some_and(|holder| holder.get_entity().entity_id == player.entity_id())
    {
        // `removeLeash` in creative, else `dropLeash`.
        drop_leash(
            mob,
            player.gamemode.load() != pumpkin_util::GameMode::Creative,
        );
        entity_interact_game_event(mob, player);
        mob.play_sound_at_self(Sound::ItemLeadUntied, 1.0, 1.0);
        return true;
    }
    if item_stack.item != &Item::LEAD
        || holder
            .as_ref()
            .is_some_and(|holder| holder.get_player().is_some())
    {
        return false;
    }
    let Some(this) = world.get_entity_by_id(entity.entity_id) else {
        return false;
    };
    if !can_have_a_leash_attached_to(this.as_ref(), player.get_entity()) {
        return false;
    }
    if holder.is_some() {
        drop_leash(mob, true);
    }
    entity.leash_to(player.clone());
    // `setLeashedTo`: a leashed mob gets off what it rides.
    if let Some(vehicle) = entity.get_vehicle() {
        vehicle.get_entity().remove_passenger(entity.entity_id);
    }
    mob.play_sound_at_self(Sound::ItemLeadTied, 1.0, 1.0);
    item_stack.decrement_unless_creative(player.gamemode.load(), 1);
    true
}

/// Vanilla `Leashable.leashableInArea`: entities in the 32-block box around `entity` that are
/// leashed to the entity with id `holder_id`.
fn leashed_in_area(world: &World, entity: &Entity, holder_id: i32) -> Vec<Arc<dyn EntityBase>> {
    let center = entity.bounding_box.load().get_center();
    let area = BoundingBox::new(
        Vector3::new(center.x - 16.0, center.y - 16.0, center.z - 16.0),
        Vector3::new(center.x + 16.0, center.y + 16.0, center.z + 16.0),
    );
    world
        .get_entities_at_box(&area)
        .into_iter()
        .filter(|candidate| {
            candidate
                .get_entity()
                .leashed_to
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .is_some_and(|holder| holder.get_entity().entity_id == holder_id)
        })
        .collect()
}

/// Vanilla `Leashable.canHaveALeashAttachedTo`: not itself, box centres within the snap
/// distance, and `canBeLeashed`.
fn can_have_a_leash_attached_to(leashable: &dyn EntityBase, holder: &Entity) -> bool {
    let entity = leashable.get_entity();
    if entity.entity_id == holder.entity_id {
        return false;
    }
    let leash_distance = holder
        .bounding_box
        .load()
        .get_center()
        .squared_distance_to_vec(&entity.bounding_box.load().get_center())
        .sqrt();
    leash_distance <= Entity::LEASH_SNAP_DISTANCE
        && leashable.get_mob().is_none_or(Mob::can_be_leashed)
}

/// Vanilla `Entity.shearOffAllLeashConnections` through `dropAllLeashConnections`: every leash
/// held by this mob and its own drop, with a `shear` game event and the snip sound.
fn shear_off_all_leash_connections<M: Mob + ?Sized>(mob: &M, player: &Arc<Player>) -> bool {
    let entity = mob.get_entity();
    let world = entity.world.load_full();
    let leashed = leashed_in_area(&world, entity, entity.entity_id);
    let dropped = if entity.is_leashed() {
        drop_leash(mob, true);
        true
    } else {
        !leashed.is_empty()
    };
    for leashed in &leashed {
        let leashed_entity = leashed.get_entity();
        leashed_entity.unleash();
        if let Some(leashed_mob) = leashed.get_mob() {
            leashed_mob.on_leash_removed();
        }
        leashed_entity.spawn_at_location(ItemStack::new(1, &Item::LEAD));
    }
    if !dropped {
        return false;
    }
    let player_base: &dyn EntityBase = player.as_ref();
    world.game_event_from(
        Some(player_base),
        pumpkin_data::game_event::GameEvent::Shear,
        entity.pos.load(),
    );
    // `level.playSound(null, blockPosition(), SHEARS_SNIP, player.getSoundSource())`.
    world.play_sound(
        Sound::ItemShearsSnip,
        SoundCategory::Players,
        &entity.block_pos.load().to_centered_f64(),
    );
    true
}

/// Vanilla `Mob.attemptToShearEquipment`: the first equipped item, in `EquipmentSlot.VALUES`
/// order, that can be sheared off and isn't held on by `prevent_armor_change` (creative
/// ignores it).
fn attempt_to_shear_equipment<M: Mob + ?Sized>(mob: &M, player: &Arc<Player>) -> bool {
    let mob_entity = mob.get_mob_entity();
    for slot in &EquipmentSlot::VALUES {
        let stack = mob_entity.get_item_slot(slot);
        let Some(equippable) = stack.get_data_component::<EquippableImpl>() else {
            continue;
        };
        let held_on = has_prevent_armor_change(&stack);
        if !equippable.can_be_sheared
            || (held_on && player.gamemode.load() != pumpkin_util::GameMode::Creative)
        {
            continue;
        }
        let shearing_sound = equippable.shearing_sound.clone();
        shear_item(mob, player, slot, stack.clone());
        // `this.playSound(equippable.shearingSound().value())`.
        let entity = mob.get_entity();
        if !entity.is_silent() {
            entity.world.load().play_seeded_sound_event(
                &shearing_sound,
                mob.sound_source(),
                &entity.pos.load(),
                rand::random(),
            );
        }
        return true;
    }
    false
}

/// Vanilla `Mob.shearItem`: the shears wear, the item leaves its slot and drops from the
/// passenger seat.
fn shear_item<M: Mob + ?Sized>(
    mob: &M,
    player: &Arc<Player>,
    slot: &EquipmentSlot,
    sheared: ItemStack,
) {
    player.damage_held_item(1);
    let entity = mob.get_entity();
    let offset = entity
        .entity_dimension
        .load()
        .attachments
        .passenger_average();
    mob.set_item_slot_and_notify(slot, ItemStack::EMPTY.clone());
    let player_base: &dyn EntityBase = player.as_ref();
    let world = entity.world.load_full();
    world.game_event_from(
        Some(player_base),
        pumpkin_data::game_event::GameEvent::Shear,
        entity.pos.load(),
    );
    // `spawnAtLocation(level, stack, offset)`.
    world.spawn_item_at(entity.pos.load() + offset, sheared);
}

/// Vanilla `Leashable.dropLeash` (`drop_lead`) and `removeLeash`.
fn drop_leash<M: Mob + ?Sized>(mob: &M, drop_lead: bool) {
    let entity = mob.get_entity();
    entity.unleash();
    mob.on_leash_removed();
    if drop_lead {
        entity.spawn_at_location(ItemStack::new(1, &Item::LEAD));
    }
}

/// Vanilla `ItemStack.getDamageSource(attacker)`: the weapon's `damage_type` component, the
/// mace's smash while the attacker can smash, else the attacker's own melee source.
fn weapon_damage_type(attacker: &dyn EntityBase, weapon: &ItemStack) -> DamageType {
    if let Some(damage_type) =
        weapon.get_data_component::<pumpkin_data::data_component_impl::DamageTypeImpl>()
    {
        return damage_type.damage_type;
    }
    if weapon.item == &Item::MACE
        && attacker
            .get_living_entity()
            .is_some_and(crate::item::items::mace::MaceItem::can_smash_attack)
    {
        return DamageType::MACE_SMASH;
    }
    attacker
        .get_mob()
        .map_or(DamageType::MOB_ATTACK, Mob::melee_damage_type)
}

/// Vanilla `LivingEntity.getHitbox` for a target: a rider's box starts at its seat.
fn hitbox(target: &dyn EntityBase) -> BoundingBox {
    let entity = target.get_entity();
    let mut hitbox = entity.bounding_box.load();
    if target.get_living_entity().is_some()
        && let Some(vehicle) = entity.get_vehicle()
    {
        let seat = crate::entity::passenger_riding_position(vehicle.as_ref(), target);
        hitbox.min.y = seat.y.max(hitbox.min.y);
    }
    hitbox
}

/// Vanilla `Mob.populateDefaultEquipmentSlots`, which overrides call as `super`.
///
/// Now and then a set of armor, its material rolled up from leather, often missing its lower
/// pieces, drawn from `level.getRandom()`.
pub fn populate_default_armor<M: Mob + ?Sized>(
    mob: &M,
    world: &World,
    difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
) {
    // One lock per draw: `level.getRandom()` is shared.
    let random = || {
        world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    };
    if random().next_f32() >= MobEntity::MAX_WEARING_ARMOR_CHANCE * difficulty.special_multiplier {
        return;
    }
    let mut armor_type = random().next_bounded_i32(3);
    let mut attempt = 1;
    while attempt as f32 <= MobEntity::WEARING_ARMOR_UPGRADE_MATERIAL_ATTEMPTS {
        if random().next_f32() < MobEntity::WEARING_ARMOR_UPGRADE_MATERIAL_CHANCE {
            armor_type += 1;
        }
        attempt += 1;
    }
    let partial_chance = if world.level_info.load().difficulty == Difficulty::Hard {
        0.1f32
    } else {
        0.25f32
    };
    let mob_entity = mob.get_mob_entity();
    let mut first = true;
    for slot in &MobEntity::EQUIPMENT_POPULATION_ORDER {
        let current = mob_entity.get_item_slot(slot);
        if !first && random().next_f32() < partial_chance {
            break;
        }
        first = false;
        if current.is_empty()
            && let Some(item) = MobEntity::get_equipment_for_slot(slot, armor_type)
        {
            mob_entity
                .living_entity
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .put(slot, ItemStack::new(1, item));
        }
    }
}

/// Vanilla `Mob.enchantSpawnedEquipment`: the `level.getRandom()` roll, then the
/// `mob_spawn_equipment` enchantment provider.
// TODO: the provider's own picks still come from Pumpkin's `EnchantmentProvider`, not
// vanilla's `EnchantmentHelper.enchantItemFromProvider` on the level random.
fn enchant_spawned_equipment<M: Mob + ?Sized>(
    mob: &M,
    world: &World,
    slot: &EquipmentSlot,
    chance: f32,
    difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
) {
    let mut equipment = mob
        .get_mob_entity()
        .living_entity
        .entity_equipment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(stack) = equipment.equipment.get_mut(slot)
        && !stack.is_empty()
        && world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_f32()
            < chance * difficulty.special_multiplier
    {
        crate::entity::mob::equipment::apply_vanilla_enchantments(
            stack,
            slot,
            difficulty.special_multiplier,
        );
    }
}

/// Vanilla `canDispenserEquipIntoSlot` for a mob.
///
/// `Mob` answers with `canPickUpLoot`; the saddled and armored mounts, the happy ghast, the
/// sulfur cube and the mobs that only carry an item in their mouth or hand override it.
#[must_use]
pub fn can_dispenser_equip_into_slot(mob: &dyn Mob, slot: &EquipmentSlot) -> bool {
    let entity_type = mob.get_entity().entity_type;
    let mob_entity = mob.get_mob_entity();
    let can_pick_up_loot = || mob_entity.with_brain(mob, |tick| mob.can_pick_up_loot(tick.brain));
    let is_body_or_saddle = *slot == EquipmentSlot::BODY || *slot == EquipmentSlot::SADDLE;
    if entity_type == &EntityType::SULFUR_CUBE || entity_type == &EntityType::HAPPY_GHAST {
        *slot == EquipmentSlot::BODY
    } else if entity_type == &EntityType::ALLAY {
        false
    } else if entity_type == &EntityType::DOLPHIN
        || entity_type == &EntityType::FOX
        || entity_type == &EntityType::PANDA
    {
        *slot == EquipmentSlot::MAIN_HAND && can_pick_up_loot()
    } else if entity_type == &EntityType::PIG || entity_type == &EntityType::STRIDER {
        *slot == EquipmentSlot::SADDLE || can_pick_up_loot()
    } else if entity_type == &EntityType::NAUTILUS || entity_type == &EntityType::ZOMBIE_NAUTILUS {
        is_body_or_saddle || can_pick_up_loot()
    } else if let Some(horse) = mob.as_abstract_horse() {
        // `AbstractHorse`: body armor and saddles once tamed.
        is_body_or_saddle && horse.is_horse_tamed() || can_pick_up_loot()
    } else if entity_type == &EntityType::CAMEL || entity_type == &EntityType::CAMEL_HUSK {
        // `Camel.isTamed` is always true.
        is_body_or_saddle || can_pick_up_loot()
    } else {
        can_pick_up_loot()
    }
}

/// Vanilla `Mob.dropPreservedEquipment`: every guaranteed drop leaves its slot and falls, except
/// the items `should_drop` refuses, whose slots are returned.
pub fn drop_preserved_equipment<M: Mob + ?Sized>(
    mob: &M,
    should_drop: impl Fn(&ItemStack) -> bool,
) -> Vec<EquipmentSlot> {
    let mob_entity = mob.get_mob_entity();
    let mut prevented = Vec::new();
    for slot in &EquipmentSlot::VALUES {
        let stack = mob_entity.get_item_slot(slot);
        if stack.is_empty() {
            continue;
        }
        if !should_drop(&stack) {
            prevented.push(slot.clone());
        } else if mob_entity.drop_chance(slot) > 1.0 {
            mob.set_item_slot_and_notify(slot, ItemStack::EMPTY.clone());
            mob.get_entity().spawn_at_location(stack);
        }
    }
    prevented
}

/// Vanilla `EnchantmentHelper.has(stack, PREVENT_ARMOR_CHANGE)`.
#[must_use]
pub fn has_prevent_armor_change(stack: &ItemStack) -> bool {
    stack
        .get_data_component::<pumpkin_data::data_component_impl::EnchantmentsImpl>()
        .is_some_and(|enchantments| {
            enchantments
                .enchantment
                .iter()
                .any(|(enchantment, _)| enchantment.effects.prevent_armor_change)
        })
}

/// Vanilla `TamableAnimal.isTame` for a target or avoid candidate.
#[must_use]
pub fn is_tame(target: &crate::entity::living::LivingEntity) -> bool {
    target
        .entity
        .world
        .load()
        .get_entity_by_id(target.entity.entity_id)
        .is_some_and(|entity| entity.get_mob().is_some_and(Mob::is_tamed))
}

/// Vanilla `Mob.setTarget`, for overrides that run their own part first.
pub fn set_mob_target_base<M: Mob + ?Sized>(mob: &M, target: Option<Arc<dyn EntityBase>>) {
    let mob_entity = mob.get_mob_entity();
    let target = mob.as_valid_target(target);
    let target_id = target.as_ref().map(|t| t.get_entity().entity_id);
    *mob_entity
        .target
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = target;
    let world = mob_entity.living_entity.entity.world.load_full();
    let entity_id = mob_entity.living_entity.entity.entity_id;
    if let Some(server) = world.server.upgrade() {
        let mut event = crate::plugin::api::events::entity::entity_target::EntityTargetEvent::new(
            entity_id, target_id,
        );
        server.plugin_manager.fire_blocking(&server, &mut event);
    }
}

/// Vanilla `AbstractHorse.causeFallDamage`: a landing sound, and the fall hurts the riders
/// only when it hurts the horse.
fn abstract_horse_cause_fall_damage<M: Mob + ?Sized>(
    mob: &M,
    caller: &dyn EntityBase,
    fall_distance: f64,
    damage_modifier: f32,
    damage_type: DamageType,
) -> bool {
    let living = &mob.get_mob_entity().living_entity;
    if fall_distance > 1.0 {
        let sound = if living.entity.is_baby() {
            Sound::EntityBabyHorseLand
        } else {
            Sound::EntityHorseLand
        };
        caller.play_sound_at_self(sound, 0.4, 1.0);
    }
    let damage = mob.calculate_fall_damage(fall_distance, damage_modifier);
    if damage <= 0 {
        return false;
    }
    caller.damage(caller, damage as f32, damage_type);
    living
        .entity
        .propagate_fall_to_passengers(fall_distance, damage_modifier, damage_type);
    living.play_block_fall_sound(caller);
    true
}

/// Vanilla `Llama.causeFallDamage`: only a fall of six blocks or more hurts.
fn llama_cause_fall_damage<M: Mob + ?Sized>(
    mob: &M,
    caller: &dyn EntityBase,
    fall_distance: f64,
    damage_modifier: f32,
    damage_type: DamageType,
) -> bool {
    let living = &mob.get_mob_entity().living_entity;
    let damage = mob.calculate_fall_damage(fall_distance, damage_modifier);
    if damage <= 0 {
        return false;
    }
    if fall_distance >= 6.0 {
        caller.damage(caller, damage as f32, damage_type);
        living
            .entity
            .propagate_fall_to_passengers(fall_distance, damage_modifier, damage_type);
    }
    living.play_block_fall_sound(caller);
    true
}

/// Whether vanilla's class for `entity_type` extends `Monster`: every `Enemy` except the
/// slimes, phantoms, ghasts, shulkers, hoglins and the ender dragon.
#[must_use]
pub fn is_monster(entity_type: &EntityType) -> bool {
    entity_type.is_enemy
        && ![
            &EntityType::SLIME,
            &EntityType::MAGMA_CUBE,
            &EntityType::PHANTOM,
            &EntityType::GHAST,
            &EntityType::SHULKER,
            &EntityType::HOGLIN,
            &EntityType::ENDER_DRAGON,
        ]
        .contains(&entity_type)
}

/// Vanilla `Monster.getWalkTargetValue`: `-getPathfindingCostFromLightLevels`, so the
/// darker the spot the better.
#[must_use]
pub fn monster_walk_target_value(world: &World, pos: &BlockPos) -> f32 {
    -(world.get_light_level_dependent_magic_value(pos) - 0.5)
}

/// Vanilla `Mob.getControllingPassenger`: a first passenger that is a mob able to steer,
/// while this mob has AI.
#[must_use]
pub fn mob_controlling_passenger(mob: &MobEntity) -> Option<Arc<dyn EntityBase>> {
    let passenger = mob.living_entity.entity.get_first_passenger()?;
    (!mob.is_no_ai()
        && passenger.get_mob().is_some()
        && !passenger
            .get_entity()
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_NON_CONTROLLING_RIDER))
    .then_some(passenger)
}

/// The first passenger when it is a player, the rider a saddled vehicle hands control to.
#[must_use]
pub fn first_passenger_player(mob: &MobEntity) -> Option<Arc<dyn EntityBase>> {
    mob.living_entity
        .entity
        .get_first_passenger()
        .filter(|passenger| passenger.get_player().is_some())
}

/// The rotation vanilla's `tickRidden` overrides end with: `setRot(y_rot, x_rot)`, then
/// `yRotO = yBodyRot = yHeadRot = getYRot()`.
pub fn set_ridden_rotation(entity: &Entity, y_rot: f32, x_rot: f32) {
    let y_rot = y_rot % 360.0;
    entity.yaw.store(y_rot);
    entity.set_pitch(x_rot % 360.0);
    entity.body_yaw.store(y_rot);
    entity.head_yaw.store(y_rot);
}

/// Vanilla `AbstractHorse.tickRidden` on the server, which camels share: the horse faces
/// where its rider looks. The rider's jump it runs is the steering client's.
pub fn horse_tick_ridden(entity: &Entity, controller: &Player) {
    let controller_entity = controller.get_entity();
    set_ridden_rotation(
        entity,
        controller_entity.yaw.load(),
        controller_entity.pitch.load() * 0.5,
    );
}

/// Reads the `Saddle` tag Pumpkin wrote before saddles were equipment. Like vanilla's data
/// fixer, it becomes a saddle in the saddle slot that always drops.
pub fn read_legacy_saddle(mob: &MobEntity, nbt: &NbtCompound) {
    if nbt.get_bool("Saddle").unwrap_or(false)
        && mob.get_item_slot(&EquipmentSlot::SADDLE).is_empty()
    {
        mob.set_item_slot_and_drop_when_killed(
            &EquipmentSlot::SADDLE,
            ItemStack::new(1, &Item::SADDLE),
        );
    }
}

/// Vanilla `LivingEntity.isHolding(item)` for a player rider.
#[must_use]
pub fn is_player_holding(passenger: &dyn EntityBase, item: &Item) -> bool {
    passenger.get_player().is_some_and(|player| {
        let inventory = player.inventory();
        inventory.held_item().item.id == item.id || inventory.off_hand_item().item.id == item.id
    })
}

/// Vanilla `ConversionType.convertCommon`: what a converted mob takes over from the old one.
/// Scoreboard teams are not carried over.
pub fn convert_common(from: &dyn Mob, to: &dyn Mob, params: conversion::ConversionParams) {
    let (old, new) = (from.get_mob_entity(), to.get_mob_entity());
    let (old_living, new_living) = (&old.living_entity, &new.living_entity);
    let (old_entity, new_entity) = (&old_living.entity, &new_living.entity);
    new_living.set_absorption(old_living.absorption.load());
    let effects: Vec<_> = old_living
        .active_effects
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        // `new MobEffectInstance(effect)`: the copy leaves the hidden effect behind.
        .map(|instance| instance.effect.clone())
        .collect();
    for effect in effects {
        new_living.add_effect(effect);
    }
    if old_entity.is_baby() {
        match to.as_ageable() {
            Some(new_ageable) => new_ageable.set_baby(true),
            None => {
                to.spawn_as_baby();
            }
        }
    }
    if let (Some(old_ageable), Some(new_ageable)) = (from.as_ageable(), to.as_ageable()) {
        new_ageable.set_age(old_ageable.get_age());
        let (old_data, new_data) = (
            old_ageable.get_ageable_data(),
            new_ageable.get_ageable_data(),
        );
        new_data
            .forced_age
            .store(old_data.forced_age.load(Relaxed), Relaxed);
        new_data
            .forced_age_timer
            .store(old_data.forced_age_timer.load(Relaxed), Relaxed);
    }
    if params.preserve_can_pick_up_loot {
        new.set_can_pick_up_loot(old.can_pick_up_loot());
    }
    new.set_left_handed(old.is_left_handed());
    new.set_no_ai(old.is_no_ai());
    if old.persistence_required.load(Relaxed) {
        new.persistence_required.store(true, Relaxed);
    }
    new_entity.set_custom_name_visible(old_entity.custom_name_visible.load(Relaxed));
    // `setSharedFlagOnFire(from.isOnFire())`, the server's `isOnFire`.
    new_entity.set_flag(
        crate::entity::Flag::OnFire,
        !old_entity.entity_type.fire_immune && old_entity.fire_ticks.load(Relaxed) > 0,
    );
    new_entity.set_invulnerable(old_entity.invulnerable.load(Relaxed));
    // `setInvulnerableTime(from.getInvulnerableTime())`.
    new_entity
        .invulnerable_time
        .store(old_entity.invulnerable_time.load(Relaxed), Relaxed);
    new_entity.set_has_no_gravity(old_entity.has_no_gravity.load(Relaxed));
    new_entity
        .portal_cooldown
        .store(old_entity.portal_cooldown.load(Relaxed), Relaxed);
    new_entity.set_silent(old_entity.silent.load(Relaxed));
    let tags = old_entity
        .scoreboard_tags
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    new_entity
        .scoreboard_tags
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .extend(tags);
    if let Some(name) = &**old_entity.custom_name.load() {
        new_entity.set_custom_name(name.clone());
    }
    if let (Some(old_zombie), Some(new_zombie)) = (from.as_zombie(), to.as_zombie())
        && old_zombie.can_break_doors()
    {
        new_zombie.set_can_break_doors(true, to);
    }
}

/// Vanilla `Mob.getBaseExperienceReward`, which `Mob::get_base_experience_reward` overrides call
/// as `super`: `xpReward` plus 1 to 3 for each equipped item that isn't a guaranteed drop.
pub fn mob_base_experience_reward<M: Mob + ?Sized>(mob: &M) -> u32 {
    let xp_reward = mob.xp_reward();
    if xp_reward == 0 {
        return xp_reward;
    }
    let mob_entity = mob.get_mob_entity();
    let mut result = xp_reward;
    for slot in &EquipmentSlot::VALUES {
        if slot.can_increase_experience()
            && !mob_entity.get_item_slot(slot).is_empty()
            && mob_entity.drop_chance(slot) <= 1.0
        {
            result += 1 + mob.get_entity().random().next_bounded_i32(3) as u32;
        }
    }
    result
}

/// The shared mob `hurtServer`, which `Mob::hurt_server` overrides call as `super`.
pub fn mob_hurt_server<M: Mob + ?Sized>(
    mob: &M,
    caller: &dyn EntityBase,
    amount: f32,
    damage_type: DamageType,
    position: Option<Vector3<f64>>,
    source: Option<&dyn EntityBase>,
    cause: Option<&dyn EntityBase>,
) -> bool {
    // pre_damage hook: allows mobs to cancel damage (e.g. shulker, witch).
    if !mob.pre_damage(damage_type, source) {
        return false;
    }
    mob.before_hurt(amount, damage_type, source, cause);
    // Mob-specific damage modifier (e.g. shulker armor when closed).
    let amount = mob.modify_incoming_damage(amount, damage_type);
    let damaged = mob.get_mob_entity().living_entity.damage_with_context(
        caller,
        amount,
        damage_type,
        position,
        source,
        cause,
    );
    if damaged {
        mob.on_damage(damage_type, source);
    }
    damaged
}

impl<T: Mob + Send + 'static> EntityBase for T {
    /// Vanilla `LivingEntity.getWeaponItem`; Pumpkin doesn't track the riptide spin stack.
    fn get_weapon_item(&self) -> Option<ItemStack> {
        self.get_living_entity()
            .map(|living| living.held_item(self))
    }

    fn get_mob(&self) -> Option<&dyn Mob> {
        Some(self)
    }

    fn check_fall_damage(
        &self,
        caller: &dyn EntityBase,
        ya: f64,
        on_ground: bool,
        state: &'static BlockState,
        pos: BlockPos,
    ) {
        self.mob_check_fall_damage(caller, ya, on_ground, state, pos);
    }

    fn cause_fall_damage(
        &self,
        caller: &dyn EntityBase,
        fall_distance: f64,
        damage_modifier: f32,
        damage_type: DamageType,
    ) -> bool {
        self.mob_cause_fall_damage(caller, fall_distance, damage_modifier, damage_type)
    }

    fn controlling_passenger(&self) -> Option<Arc<dyn EntityBase>> {
        self.get_controlling_passenger()
    }

    fn passenger_attachment_point(
        &self,
        passenger: &dyn EntityBase,
        dimensions: EntityDimensions,
        scale: f32,
    ) -> Vector3<f64> {
        self.mob_passenger_attachment_point(passenger, dimensions, scale)
    }

    fn vehicle_attachment_point(&self, vehicle: &dyn EntityBase) -> Vector3<f64> {
        self.mob_vehicle_attachment_point(vehicle)
    }

    fn max_up_step(&self) -> f32 {
        self.mob_max_up_step()
    }

    fn position_rider(&self, caller: &dyn EntityBase, passenger: &dyn EntityBase) {
        crate::entity::set_rider_position(caller, passenger);
        self.mob_after_position_rider(passenger);
    }

    fn is_pushed_by_fluids(&self) -> bool {
        self.mob_is_pushed_by_fluids()
    }

    fn deflection(&self, projectile: &dyn EntityBase) -> ProjectileDeflectionType {
        self.mob_deflection(projectile)
    }

    fn is_pushable(&self) -> bool {
        self.mob_is_pushable() && self.get_mob_entity().living_entity.is_pushable_for(self)
    }

    /// Vanilla `LivingEntity.push`: a sleeping entity is not pushed.
    fn push(&self, entity: &dyn EntityBase) {
        if !self.is_sleeping() {
            super::push_apart(self.get_entity(), self.is_pushable(), entity);
        }
        self.mob_push(entity);
    }

    fn push_impulse(&self, impulse: Vector3<f64>) {
        self.mob_push_impulse(impulse);
    }

    fn on_lightning_strike(
        &self,
        caller: &dyn EntityBase,
        lightning: &crate::entity::lightning::LightningBoltEntity,
    ) {
        self.mob_on_lightning_strike(caller, lightning);
    }

    fn get_item_steerable(&self) -> Option<&dyn crate::entity::item_steerable::ItemSteerable> {
        Mob::get_item_steerable(self)
    }

    fn init_data_tracker(&self) {
        self.mob_init_data_tracker();
    }

    fn set_variant_name(&self, name: &str) {
        self.mob_set_variant_name(name);
    }

    fn set_sound_variant_name(&self, name: &str) {
        self.mob_set_sound_variant_name(name);
    }

    #[allow(clippy::too_many_lines)]
    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.mob_pre_tick();
        let mob_entity = self.get_mob_entity();
        mob_entity.living_entity.entity.tick_leash();

        if mob_entity.breeding_cooldown.load(Relaxed) > 0 {
            mob_entity.breeding_cooldown.fetch_sub(1, Relaxed);
        }

        if mob_entity.love_ticks.load(Relaxed) > 0 {
            let ticks = mob_entity.love_ticks.fetch_sub(1, Relaxed);
            if ticks % 10 == 0 {
                let entity = &mob_entity.living_entity.entity;
                let pos = entity.pos.load();
                let world = entity.world.load();
                world.spawn_particle(
                    pos + Vector3::new(0.0, f64::from(entity.height()) + 0.5, 0.0),
                    Vector3::new(0.5, 0.5, 0.5),
                    1.0,
                    1,
                    pumpkin_data::particle::Particle::Heart,
                );
            }
        }

        mob_entity.check_despawn(self);

        if let Some(neutral) = self.as_neutral()
            && !self.updates_anger_in_ai_step()
        {
            neutral.update_persistent_anger();
        }

        self.mob_tick(caller);

        // Vanilla Mob.isEffectiveAi: NoAI skips the whole serverAiStep, which
        // `LivingEntity::tick_movement` runs at its vanilla place in `aiStep`. The
        // ambient-sound roll of `Mob.baseTick` runs inside it too, via `mob_base_tick`.
        mob_entity.living_entity.tick(caller, server);
        self.post_tick();
    }

    fn is_collidable(&self, entity: Option<&dyn EntityBase>) -> bool {
        self.mob_can_be_collided_with(entity)
    }

    fn kill(&self, caller: &dyn EntityBase) {
        self.mob_kill(caller);
    }

    fn can_hit(&self) -> bool {
        self.is_pickable()
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
        self.hurt_server(caller, amount, damage_type, position, source, cause)
    }

    fn interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        mob_interact_entry(self, player, item_stack)
    }

    fn on_player_collision(&self, player: &Arc<Player>) {
        self.mob_player_collision(player);
    }

    fn get_entity(&self) -> &Entity {
        &self.get_mob_entity().living_entity.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        Some(&self.get_mob_entity().living_entity)
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn is_in_love(&self) -> bool {
        self.get_mob_entity().is_in_love()
    }

    fn is_breeding_ready(&self) -> bool {
        self.get_mob_entity().is_breeding_ready()
    }

    fn reset_love(&self) {
        self.get_mob_entity().reset_love_ticks();
    }

    fn set_breeding_cooldown(&self, ticks: i32) {
        self.get_mob_entity()
            .breeding_cooldown
            .store(ticks, Relaxed);
    }

    /// Vanilla `PathfinderMob.isPanicking` for goal-driven mobs: a running `PanicGoal`.
    /// Inside the mob's own AI tick its selector is detached and this answers `false`.
    // TODO: brain mobs answer from their `is_panicking` memory, which needs the brain lock.
    fn is_panicking(&self) -> bool {
        self.get_mob_entity()
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_running::<EscapeDangerGoal>()
    }

    fn get_job_site_pos(&self) -> Option<pumpkin_util::math::position::BlockPos> {
        <T as Mob>::get_job_site(self)
    }

    fn get_home_pos(&self) -> Option<pumpkin_util::math::position::BlockPos> {
        <T as Mob>::get_home(self)
    }

    fn as_mob_entity(&self) -> Option<&MobEntity> {
        Some(<T as Mob>::get_mob_entity(self))
    }

    fn entity_sound_source(&self) -> SoundCategory {
        <T as Mob>::sound_source(self)
    }

    fn movement_emission(&self) -> MovementEmission {
        <T as Mob>::movement_emission(self)
    }

    fn play_step_sound(
        &self,
        caller: &dyn EntityBase,
        pos: &BlockPos,
        block: &'static Block,
        state: &'static BlockState,
    ) {
        <T as Mob>::play_step_sound(self, caller, pos, block, state);
    }

    fn next_step(&self) -> f32 {
        <T as Mob>::next_step(self)
    }

    fn play_swim_sound(&self, caller: &dyn EntityBase, volume: f32) {
        <T as Mob>::play_swim_sound(self, caller, volume);
    }

    fn swim_sound(&self) -> Sound {
        <T as Mob>::swim_sound(self)
    }

    fn swim_splash_sound(&self) -> Sound {
        <T as Mob>::swim_splash_sound(self)
    }

    fn swim_high_speed_splash_sound(&self) -> Sound {
        <T as Mob>::swim_high_speed_splash_sound(self)
    }

    fn is_flapping(&self) -> bool {
        <T as Mob>::is_flapping(self)
    }

    fn on_flap(&self) {
        <T as Mob>::on_flap(self);
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        let mob_entity = self.get_mob_entity();
        let can_pick_up_loot =
            mob_entity.with_brain(self, |tick| self.can_pick_up_loot(tick.brain));
        mob_entity.write_mob_nbt(nbt, can_pick_up_loot);
        if let Some(ageable) = self.as_ageable() {
            ageable.write_ageable_nbt(nbt);
        }
        if let Some(animal) = self.as_animal() {
            animal.write_animal_nbt(nbt);
        }
        if let Some(tamable) = self.as_tamable() {
            tamable.write_tamable_nbt(nbt);
        }
        if let Some(neutral) = self.as_neutral() {
            neutral.write_anger_nbt(nbt);
        }
        self.mob_write_nbt(nbt);
    }

    fn read_nbt_before_living(&self, nbt: &NbtCompound) {
        self.mob_read_nbt_before_super(nbt);
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        self.get_mob_entity().read_mob_nbt(nbt);
        if let Some(brain) = nbt.get_compound("Brain") {
            *self
                .get_mob_entity()
                .brain
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                self.make_brain(&PackedMemories::from_nbt(brain));
        }
        if let Some(ageable) = self.as_ageable() {
            ageable.read_ageable_nbt(nbt);
        }
        if let Some(animal) = self.as_animal() {
            animal.read_animal_nbt(nbt);
        }
        if let Some(tamable) = self.as_tamable() {
            tamable.read_tamable_nbt(nbt);
        }
        if let Some(neutral) = self.as_neutral() {
            neutral.read_anger_nbt(nbt);
        }
        self.mob_read_nbt(nbt);
    }

    fn get_gravity(&self) -> f64 {
        self.get_mob_gravity()
    }

    fn get_y_velocity_drag(&self) -> Option<f64> {
        self.get_mob_y_velocity_drag()
    }

    fn is_flutterer(&self) -> bool {
        self.omnidirectional_air_mover()
    }

    fn is_stepping_carefully(&self) -> bool {
        self.mob_is_stepping_carefully()
    }

    fn get_experience_reward(&self, _killer: Option<&dyn EntityBase>) -> u32 {
        if !Mob::should_drop_experience(self) {
            return 0;
        }
        // TODO: apply enchantment processing like in vanilla
        Mob::get_base_experience_reward(self)
    }

    fn get_base_experience_reward(&self) -> u32 {
        Mob::get_base_experience_reward(self)
    }
}

#[expect(dead_code)]
const DEFAULT_PATHFINDING_FAVOR: f32 = 0.0;
/// Vanilla `MobCategory.noDespawnDistance`, the same for every category.
const NO_DESPAWN_DISTANCE: i32 = 32;

pub trait PathAwareEntity: Mob + Send + Sync {
    fn get_pathfinding_favor(&self, _block_pos: BlockPos, _world: Arc<World>) -> f32 {
        0.0
    }

    // TODO: missing SpawnReason attribute
    fn can_spawn(&self, world: Arc<World>) -> bool {
        self.get_pathfinding_favor(
            self.get_mob_entity().living_entity.entity.block_pos.load(),
            world,
        ) >= 0.0
    }

    fn is_navigation(&self) -> bool {
        let navigator = self
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !navigator.is_idle()
    }

    fn should_follow_leash(&self) -> bool {
        true
    }

    fn on_short_leash_tick(&self) {
        // TODO: implement
    }

    fn before_leash_tick(&self) {
        // TODO: implement
    }

    fn get_follow_leash_speed(&self) -> f32 {
        1.0
    }
}

pub trait RangedAttackMob: Mob + Send + Sync {
    fn perform_ranged_attack(&self, target: &Arc<dyn EntityBase>, power: f32);

    /// Vanilla `RangedAttackMob.rangedAttackUncertainty`.
    fn ranged_attack_uncertainty(&self, world: &World) -> f32 {
        (14 - world.level_info.load().difficulty as i32 * 4) as f32
    }
}

/// Vanilla `instanceof PathfinderMob`: every mob but the bat, the phantom, the ghast and the
/// ender dragon.
#[must_use]
pub fn is_pathfinder_mob(entity: &dyn EntityBase) -> bool {
    use pumpkin_data::entity::EntityType;
    entity.get_mob().is_some()
        && ![
            &EntityType::BAT,
            &EntityType::PHANTOM,
            &EntityType::GHAST,
            &EntityType::ENDER_DRAGON,
        ]
        .contains(&entity.get_entity().entity_type)
}

/// The `rideTick` of vanilla `AbstractSkeleton` and `Drowned`: the body turns with a
/// pathfinding vehicle this mob steers (`getControlledVehicle`).
pub fn turn_body_with_controlled_vehicle(mob: &dyn Mob) {
    let entity = mob.get_entity();
    let vehicle = entity
        .vehicle
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(vehicle) = vehicle
        && vehicle
            .controlling_passenger()
            .is_some_and(|c| c.get_entity().entity_id == entity.entity_id)
        && is_pathfinder_mob(vehicle.as_ref())
    {
        entity.body_yaw.store(vehicle.get_entity().body_yaw.load());
    }
}
