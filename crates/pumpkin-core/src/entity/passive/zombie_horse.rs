use crate::entity::ai::goal::tempt::TemptGoal;
use crate::entity::mob::mount_inventory::MountInventory;
use crate::entity::mob::{first_passenger_player, horse_tick_ridden, mob_controlling_passenger};
use crate::entity::passive::abstract_horse::add_abstract_horse_goals;
use crate::entity::passive::abstract_horse::{AbstractHorse, abstract_horse_mob_interact};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicI32, AtomicU8, Ordering},
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;
use uuid::Uuid;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::swim::SwimGoal,
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

pub const FLAG_TAME: u8 = 2;
pub const FLAG_EATING: u8 = 16;
pub const FLAG_STANDING: u8 = 32;
pub const FLAG_OPEN_MOUTH: u8 = 64;

pub struct ZombieHorseEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub flags: AtomicU8,
    pub temper: AtomicI32,
    /// Vanilla `AbstractHorse.gallopSoundCounter`.
    pub gallop_sound_counter: AtomicI32,
    pub owner: AtomicCell<Option<Uuid>>,
    /// Vanilla `AbstractHorse.standCounter`.
    pub stand_counter: AtomicI32,
    /// Vanilla `AbstractHorse.standAnim` and `standAnimO`.
    pub stand_anim: crossbeam::atomic::AtomicCell<[f32; 2]>,
    pub inventory: MountInventory,
}

impl ZombieHorseEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        let horse = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            flags: AtomicU8::new(0),
            temper: AtomicI32::new(0),
            gallop_sound_counter: AtomicI32::new(0),
            owner: AtomicCell::new(None),
            stand_counter: AtomicI32::new(0),
            stand_anim: crossbeam::atomic::AtomicCell::new([0.0; 2]),
            inventory: MountInventory::default(),
        };
        let mob_arc = Arc::new(horse);
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
            goal_selector.add_goal(
                3,
                Box::new(TemptGoal::new(1.25, &[&Item::RED_MUSHROOM], false)),
            );
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
            pumpkin_data::tracked_data::zombie_horse::DATA_ID_FLAGS,
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

impl AgeableMob for ZombieHorseEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for ZombieHorseEntity {
    fn is_food(&self, _item_stack: &ItemStack) -> bool {
        false
    }
}

/// Vanilla `ZombieHorse.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    pumpkin_data::entity_dimensions::ZOMBIE_HORSE_BABY_DIMENSIONS;

impl Mob for ZombieHorseEntity {
    /// Vanilla `ZombieHorse.interact`, before `super.interact`.
    fn before_interact(&self) {
        self.get_mob_entity()
            .persistence_required
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Vanilla `ZombieHorse.canBeLeashed`, with `isMobControlled`: a mob rides in front.
    fn can_be_leashed(&self) -> bool {
        self.is_tame()
            || self
                .get_entity()
                .get_first_passenger()
                .is_none_or(|passenger| passenger.get_mob().is_none())
    }

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
        crate::entity::passive::horse::play_horse_step_sound(
            caller,
            pos,
            state,
            Some(&self.gallop_sound_counter),
            false,
        );
    }

    /// Vanilla `ZombieHorse.getDefaultDimensions`.
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

    fn sun_protection_slot(&self) -> pumpkin_data::data_component_impl::EquipmentSlot {
        pumpkin_data::data_component_impl::EquipmentSlot::BODY
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
        nbt.put_bool("Tame", self.is_tame());
        nbt.put_int("Temper", self.temper.load(Ordering::Relaxed));
        if let Some(owner) = self.owner.load() {
            nbt.put_uuid("Owner", owner);
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
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

    /// Vanilla `ZombieHorse.canUseSlot`.
    fn can_use_slot(&self, _slot: &EquipmentSlot) -> bool {
        true
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
            entity.set_synced_data(pumpkin_data::tracked_data::zombie_horse::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::zombie_horse::DATA_ID_FLAGS,
            self.flags.load(Ordering::Relaxed) as i8,
        );
    }

    /// Vanilla `ZombieHorse.mobInteract`.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let should_open_inventory =
            !self.is_baby() && self.is_tame() && player.get_entity().is_sneaking();
        if !self.is_vehicle() && !should_open_inventory && !item_stack.is_empty() {
            // TODO: vanilla `AbstractHorse.fedFood`; Pumpkin feeds horses like other animals.
            if self.is_food(item_stack) {
                return self.animal_interact(player, item_stack, Sound::EntityZombieHorseAmbient);
            }
            if !self.is_tame() {
                self.make_mad();
                return true;
            }
        }
        abstract_horse_mob_interact(self, player, item_stack, Sound::EntityZombieHorseAmbient)
    }
}

impl AbstractHorse for ZombieHorseEntity {
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

    fn angry_sound(&self) -> Option<Sound> {
        Some(Sound::EntityZombieHorseAngry)
    }
}
