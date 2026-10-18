use crate::entity::mob::mount_inventory::MountInventory;
use crate::entity::mob::{first_passenger_player, horse_tick_ridden, mob_controlling_passenger};
use crate::entity::passive::abstract_horse::add_abstract_horse_goals;
use crate::entity::passive::abstract_horse::{AbstractHorse, horse_mob_interact};
use crate::entity::passive::animal::horse_can_parent;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering},
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data::{self, TrackedData};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::{AttachmentPoints, EntityDimensions};
use uuid::Uuid;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{escape_danger::EscapeDangerGoal, swim::SwimGoal, tempt::TemptGoal},
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

const TEMPT_ITEMS: &[&Item] = &[
    &Item::GOLDEN_APPLE,
    &Item::ENCHANTED_GOLDEN_APPLE,
    &Item::GOLDEN_CARROT,
];

pub const FLAG_TAME: u8 = 2;
pub const FLAG_BRED: u8 = 8;
pub const FLAG_EATING: u8 = 16;
pub const FLAG_STANDING: u8 = 32;
pub const FLAG_OPEN_MOUTH: u8 = 64;

pub struct DonkeyEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub flags: AtomicU8,
    pub has_chest: AtomicBool,
    pub temper: AtomicI32,
    pub owner: AtomicCell<Option<Uuid>>,
    /// Vanilla `AbstractHorse.standCounter`.
    pub stand_counter: AtomicI32,
    /// Vanilla `AbstractHorse.standAnim` and `standAnimO`.
    pub stand_anim: crossbeam::atomic::AtomicCell<[f32; 2]>,
    pub inventory: MountInventory,
}

impl DonkeyEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        let donkey = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            flags: AtomicU8::new(0),
            has_chest: AtomicBool::new(false),
            temper: AtomicI32::new(0),
            owner: AtomicCell::new(None),
            stand_counter: AtomicI32::new(0),
            stand_anim: crossbeam::atomic::AtomicCell::new([0.0; 2]),
            inventory: MountInventory::default(),
        };
        let mob_arc = Arc::new(donkey);
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

            add_abstract_horse_goals(&mut goal_selector, mob_arc.as_ref(), mob_weak, true);
            // `addBehaviourGoals`.
            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            // MountPanicGoal
            goal_selector.add_goal(1, EscapeDangerGoal::new(1.2));
            goal_selector.add_goal(3, Box::new(TemptGoal::new(1.25, TEMPT_ITEMS, false)));
        };

        mob_arc
    }

    #[must_use]
    pub fn has_flag(&self, flag: u8) -> bool {
        (self.flags.load(Ordering::Relaxed) & flag) != 0
    }

    pub fn set_flag(&self, flag: u8, val: bool) {
        let current = self.flags.load(Ordering::Relaxed);
        let new_flags = if val { current | flag } else { current & !flag };
        self.flags.store(new_flags, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::donkey::DATA_ID_FLAGS,
            new_flags as i8,
        );
    }

    #[must_use]
    pub fn is_tame(&self) -> bool {
        self.has_flag(FLAG_TAME)
    }

    pub fn set_tame(&self, val: bool) {
        self.set_flag(FLAG_TAME, val);
    }
}

impl AgeableMob for DonkeyEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for DonkeyEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_HORSE_FOOD)
            || item_stack.item == &Item::WHEAT
            || item_stack.item == &Item::SUGAR
            || item_stack.item == &Item::HAY_BLOCK
            || item_stack.item == &Item::APPLE
            || item_stack.item == &Item::GOLDEN_CARROT
            || item_stack.item == &Item::GOLDEN_APPLE
            || item_stack.item == &Item::ENCHANTED_GOLDEN_APPLE
    }

    /// Vanilla `Donkey.canMate`: horses and donkeys breed with each other (into mules).
    fn can_mate(&self, partner: &dyn Mob) -> bool {
        let partner_entity = partner.get_entity();
        partner_entity.entity_id != self.get_entity().entity_id
            && (partner_entity.entity_type == &EntityType::DONKEY
                || partner_entity.entity_type == &EntityType::HORSE)
            && self.can_parent()
            && partner.as_animal().is_some_and(Animal::can_parent)
    }

    fn can_parent(&self) -> bool {
        horse_can_parent(self, self.is_tame())
    }
}

/// Vanilla `AbstractChestedHorse.babyDimensions`.
// Vanilla `AbstractChestedHorse`'s constructor: one passenger point over the back, then half size.
const BABY_DIMENSIONS: EntityDimensions = Entity::type_dimensions(&EntityType::DONKEY)
    .with_attachments(
        AttachmentPoints::Points(&[[
            0.0,
            (EntityType::DONKEY.dimension[1] + 0.031_25) as f64,
            -0.3125,
        ]]),
        AttachmentPoints::AtFeet,
    )
    .scale(0.5);

impl Mob for DonkeyEntity {
    fn mob_passenger_attachment_point(
        &self,
        passenger: &dyn EntityBase,
        dimensions: pumpkin_util::math::boundingbox::EntityDimensions,
        scale: f32,
    ) -> pumpkin_util::math::vector3::Vector3<f64> {
        self.horse_passenger_attachment_point(passenger, dimensions, scale)
    }

    fn mob_after_position_rider(&self, passenger: &dyn EntityBase) {
        self.turn_rider_body(passenger);
    }

    fn as_abstract_horse(&self) -> Option<&dyn AbstractHorse> {
        Some(self)
    }

    /// Vanilla `AbstractHorse.playStepSound`.
    fn play_step_sound(
        &self,
        caller: &dyn EntityBase,
        pos: &pumpkin_util::math::position::BlockPos,
        _block: &'static pumpkin_data::Block,
        state: &'static pumpkin_data::BlockState,
    ) {
        crate::entity::passive::horse::play_horse_step_sound(caller, pos, state, None, false);
    }

    /// Vanilla `AbstractChestedHorse.getDefaultDimensions`.
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

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
        self.write_chest_nbt(nbt);
        nbt.put_bool("Tame", self.is_tame());
        nbt.put_int("Temper", self.temper.load(Ordering::Relaxed));
        if let Some(owner) = self.owner.load() {
            nbt.put_uuid("Owner", owner);
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        self.read_chest_nbt(nbt);
        if let Some(tame) = nbt.get_bool("Tame") {
            self.set_tame(tame);
        }
        if let Some(temper) = nbt.get_int("Temper") {
            self.temper.store(temper, Ordering::Relaxed);
        }
        if let Some(owner) = nbt.get_uuid("Owner") {
            self.owner.store(Some(owner));
        }
    }

    /// Vanilla `AbstractHorse.canUseSlot`.
    fn can_use_slot(&self, slot: &EquipmentSlot) -> bool {
        self.horse_can_use_slot(slot)
    }

    /// Vanilla `AbstractHorse.getEquipSound`.
    fn equip_sound(&self, slot: &EquipmentSlot) -> Option<Sound> {
        (*slot == EquipmentSlot::SADDLE).then_some(Sound::EntityHorseSaddle)
    }

    fn tick_ridden(&self, controller: &Player) {
        horse_tick_ridden(self.get_entity(), controller);
    }

    fn open_custom_inventory_screen(&self, player: &Arc<Player>) {
        self.open_horse_inventory(player);
    }

    fn drop_custom_death_loot(&self) {
        self.drop_horse_inventory();
    }

    /// Vanilla `AbstractHorse.canJump`.
    fn can_jump(&self) -> bool {
        self.is_saddled()
    }

    fn handle_start_jump(&self, _jump_scale: i32) {
        self.horse_handle_start_jump();
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `AbstractHorse.getControllingPassenger`.
    fn get_controlling_passenger(&self) -> Option<Arc<dyn EntityBase>> {
        if self.is_saddled()
            && let Some(player) = first_passenger_player(&self.mob_entity)
        {
            return Some(player);
        }
        mob_controlling_passenger(&self.mob_entity)
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        AbstractHorse::tick_standing(self);
        self.ageable_ai_step();
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::donkey::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::donkey::DATA_ID_CHEST,
            self.has_chest(),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::donkey::DATA_ID_FLAGS,
            self.flags.load(Ordering::Relaxed) as i8,
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        horse_mob_interact(self, player, item_stack, Sound::EntityDonkeyAmbient)
    }
}

impl AbstractHorse for DonkeyEntity {
    fn horse_flag(&self, flag: u8) -> bool {
        (self.flags.load(Ordering::Relaxed) & flag) != 0
    }

    fn set_horse_flag(&self, flag: u8, value: bool) {
        self.set_flag(flag, value);
    }

    fn set_horse_tamed(&self, tamed: bool) {
        self.set_tame(tamed);
    }

    fn set_horse_owner(&self, owner: Uuid) {
        self.owner.store(Some(owner));
    }

    fn horse_temper(&self) -> &AtomicI32 {
        &self.temper
    }

    fn stand_counter(&self) -> &AtomicI32 {
        &self.stand_counter
    }

    fn stand_anim(&self) -> &crossbeam::atomic::AtomicCell<[f32; 2]> {
        &self.stand_anim
    }

    fn horse_inventory(&self) -> &MountInventory {
        &self.inventory
    }

    /// Vanilla `Donkey.playJumpSound`.
    fn jump_sound(&self) -> Sound {
        Sound::EntityDonkeyJump
    }

    fn chest(&self) -> Option<(&AtomicBool, TrackedData)> {
        Some((&self.has_chest, tracked_data::donkey::DATA_ID_CHEST))
    }

    fn angry_sound(&self) -> Option<Sound> {
        Some(Sound::EntityDonkeyAngry)
    }
}
