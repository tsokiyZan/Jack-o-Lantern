use crate::entity::mob::mount_inventory::MountInventory;
use crate::entity::mob::{first_passenger_player, horse_tick_ridden, mob_controlling_passenger};
use crate::entity::passive::abstract_horse::add_abstract_horse_goals;
use crate::entity::passive::abstract_horse::{AbstractHorse, abstract_horse_mob_interact};
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
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::random::RandomImpl;
use uuid::Uuid;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

pub const FLAG_TAME: u8 = 2;
pub const FLAG_EATING: u8 = 16;
pub const FLAG_STANDING: u8 = 32;
pub const FLAG_OPEN_MOUTH: u8 = 64;

pub struct SkeletonHorseEntity {
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
    /// Vanilla `isTrap`: lightning spawned it and it waits for a player.
    is_trap: AtomicBool,
    /// Vanilla `trapTime`.
    trap_time: AtomicI32,
}

impl SkeletonHorseEntity {
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
            is_trap: AtomicBool::new(false),
            trap_time: AtomicI32::new(0),
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

            // `addBehaviourGoals` is empty for skeleton horses.
            add_abstract_horse_goals(&mut goal_selector, mob_arc.as_ref(), mob_weak, true);
            // Vanilla adds `SkeletonTrapGoal` at 1 in `setTrap(true)` and removes it after; here
            // it is always present and only starts while the horse is a trap.
            goal_selector.add_goal(1, Box::new(SkeletonTrapGoal));
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
            pumpkin_data::tracked_data::skeleton_horse::DATA_ID_FLAGS,
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

/// Vanilla `SkeletonHorse.TRAP_MAX_LIFE`.
const TRAP_MAX_LIFE: i32 = 18000;

impl SkeletonHorseEntity {
    #[must_use]
    pub fn is_trap(&self) -> bool {
        self.is_trap.load(Ordering::Relaxed)
    }

    pub fn set_trap(&self, trap: bool) {
        self.is_trap.store(trap, Ordering::Relaxed);
    }

    /// Vanilla `SkeletonTrapGoal.createHorse`.
    fn create_trap_horse(&self, world: &Arc<crate::world::World>) -> Arc<dyn EntityBase> {
        let pos = self.get_entity().pos.load();
        let horse = crate::entity::r#type::from_type(
            &EntityType::SKELETON_HORSE,
            pos,
            world,
            Uuid::new_v4(),
        );
        crate::entity::mob::spawn::finalize_spawn(
            &horse,
            world,
            crate::entity::mob::spawn::EntitySpawnReason::Triggered,
            None,
        );
        let entity = horse.get_entity();
        entity.set_pos(pos);
        entity.invulnerable_time.store(60, Ordering::Relaxed);
        if let Some(mob) = horse.get_mob() {
            mob.get_mob_entity()
                .persistence_required
                .store(true, Ordering::Relaxed);
        }
        if let Some(skeleton_horse) = horse.cast_any().downcast_ref::<Self>() {
            skeleton_horse.set_tame(true);
            skeleton_horse.set_age(0);
        }
        horse
    }

    /// Vanilla `SkeletonTrapGoal.createSkeleton`: an armoured, enchanted rider.
    fn create_trap_skeleton(
        world: &Arc<crate::world::World>,
        horse: &Arc<dyn EntityBase>,
    ) -> Arc<dyn EntityBase> {
        let pos = horse.get_entity().pos.load();
        let skeleton =
            crate::entity::r#type::from_type(&EntityType::SKELETON, pos, world, Uuid::new_v4());
        crate::entity::mob::spawn::finalize_spawn(
            &skeleton,
            world,
            crate::entity::mob::spawn::EntitySpawnReason::Triggered,
            None,
        );
        skeleton.get_entity().set_pos(pos);
        skeleton
            .get_entity()
            .invulnerable_time
            .store(60, Ordering::Relaxed);
        if let Some(mob) = skeleton.get_mob() {
            let mob_entity = mob.get_mob_entity();
            mob_entity
                .persistence_required
                .store(true, Ordering::Relaxed);
            let head = pumpkin_data::data_component_impl::EquipmentSlot::HEAD;
            let main_hand = pumpkin_data::data_component_impl::EquipmentSlot::MAIN_HAND;
            if mob_entity.get_item_slot(&head).is_empty() {
                mob_entity.set_item_slot(&head, ItemStack::new(1, &Item::IRON_HELMET));
            }
            // `enchant`: cleared, then `MOB_SPAWN_EQUIPMENT` on the skeleton's own random.
            let difficulty = crate::entity::mob::equipment::RegionalDifficulty::at(world, pos);
            for slot in [main_hand, head] {
                let mut stack = mob_entity.get_item_slot(&slot);
                stack.patch.retain(|(id, _)| {
                    *id != pumpkin_data::data_component::DataComponent::Enchantments
                });
                crate::enchantment::EnchantmentHelper::enchant_item_from_provider(
                    &mut stack,
                    &pumpkin_data::enchantment_provider::EnchantmentProvider::MOB_SPAWN_EQUIPMENT,
                    &difficulty,
                    &mut *skeleton.get_entity().random(),
                );
                mob_entity.set_item_slot(&slot, stack);
            }
        }
        skeleton
    }
}

/// Vanilla `SkeletonTrapGoal`: a player within 10 blocks springs the trap.
struct SkeletonTrapGoal;

impl crate::entity::ai::goal::Goal for SkeletonTrapGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(horse) = mob.cast_any().downcast_ref::<SkeletonHorseEntity>() else {
            return false;
        };
        let entity = mob.get_entity();
        horse.is_trap()
            && entity
                .world
                .load()
                .has_nearby_alive_player(entity.pos.load(), 10.0)
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(horse) = mob.cast_any().downcast_ref::<SkeletonHorseEntity>() else {
            return;
        };
        let entity = mob.get_entity();
        let world = entity.world.load_full();
        horse.set_trap(false);
        horse.set_tame(true);
        horse.set_age(0);

        let pos = entity.pos.load();
        let bolt = crate::entity::lightning::LightningBoltEntity::new(crate::entity::Entity::new(
            world.clone(),
            pos,
            &EntityType::LIGHTNING_BOLT,
        ));
        bolt.set_visual_only(true);
        world.spawn_entity(Arc::new(bolt));

        let Some(this) = world.get_entity_by_id(entity.entity_id) else {
            return;
        };
        let skeleton = SkeletonHorseEntity::create_trap_skeleton(&world, &this);
        world.spawn_entity(skeleton.clone());
        entity.add_passenger(this.clone(), skeleton);

        for _ in 0..3 {
            let other_horse = horse.create_trap_horse(&world);
            let other_skeleton = SkeletonHorseEntity::create_trap_skeleton(&world, &other_horse);
            // `otherHorse.push(this.horse.getRandom().triangle(0.0, 1.1485), 0.0, ...)`.
            let (push_x, push_z) = {
                let mut random = entity.random();
                let x = random.next_triangular(0.0, 1.1485);
                (x, random.next_triangular(0.0, 1.1485))
            };
            let push = pumpkin_util::math::vector3::Vector3::new(push_x, 0.0, push_z);
            other_horse.get_entity().push_impulse(push);
            world.spawn_entity(other_horse.clone());
            world.spawn_entity(other_skeleton.clone());
            other_horse
                .get_entity()
                .add_passenger(other_horse.clone(), other_skeleton);
        }
    }
}

impl AgeableMob for SkeletonHorseEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for SkeletonHorseEntity {
    fn is_food(&self, _item_stack: &ItemStack) -> bool {
        false
    }
}

/// Vanilla `SkeletonHorse.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    pumpkin_data::entity_dimensions::SKELETON_HORSE_BABY_DIMENSIONS;

impl Mob for SkeletonHorseEntity {
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

    /// Vanilla `SkeletonHorse.getSwimSound`.
    fn swim_sound(&self) -> Sound {
        let entity = self.get_entity();
        if entity.on_ground.load(Ordering::Relaxed) {
            if !self.is_vehicle() {
                return Sound::EntitySkeletonHorseStepWater;
            }
            let count = self.gallop_sound_counter.fetch_add(1, Ordering::Relaxed) + 1;
            if count > 5 && count % 3 == 0 {
                return Sound::EntitySkeletonHorseGallopWater;
            }
            if count <= 5 {
                return Sound::EntitySkeletonHorseStepWater;
            }
        }
        Sound::EntitySkeletonHorseSwim
    }

    /// Vanilla `SkeletonHorse.playSwimSound`.
    fn play_swim_sound(&self, caller: &dyn EntityBase, volume: f32) {
        let volume = if self.get_entity().on_ground.load(Ordering::Relaxed) {
            0.3
        } else {
            (volume * 25.0).min(0.1)
        };
        self.get_entity().play_swim_sound_default(caller, volume);
    }

    /// Vanilla `SkeletonHorse.getAmbientSound`.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        Some(if self.get_entity().is_submerged_in_water() {
            pumpkin_data::sound::Sound::EntitySkeletonHorseAmbientWater
        } else {
            pumpkin_data::sound::Sound::EntitySkeletonHorseAmbient
        })
    }

    /// Vanilla `SkeletonHorse.getDefaultDimensions`.
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
        nbt.put_bool("Tame", self.is_tame());
        nbt.put_int("Temper", self.temper.load(Ordering::Relaxed));
        if let Some(owner) = self.owner.load() {
            nbt.put_uuid("Owner", owner);
        }
        nbt.put_bool("SkeletonTrap", self.is_trap());
        nbt.put_int("SkeletonTrapTime", self.trap_time.load(Ordering::Relaxed));
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
        self.set_trap(nbt.get_bool("SkeletonTrap").unwrap_or(false));
        self.trap_time.store(
            nbt.get_int("SkeletonTrapTime").unwrap_or(0),
            Ordering::Relaxed,
        );
    }

    /// Vanilla `SkeletonHorse.canUseSlot`.
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
        // Vanilla `SkeletonHorse.aiStep`: an untriggered trap goes after 18000 ticks.
        if !self.mob_entity.persistence_required.load(Ordering::Relaxed)
            && self.is_trap()
            && self.trap_time.fetch_add(1, Ordering::Relaxed) >= TRAP_MAX_LIFE
        {
            self.get_entity().remove();
        }
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(
                pumpkin_data::tracked_data::skeleton_horse::DATA_BABY_ID,
                true,
            );
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::skeleton_horse::DATA_ID_FLAGS,
            self.flags.load(Ordering::Relaxed) as i8,
        );
    }

    /// Vanilla `SkeletonHorse.mobInteract`.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        self.is_tame()
            && abstract_horse_mob_interact(
                self,
                player,
                item_stack,
                Sound::EntitySkeletonHorseAmbient,
            )
    }
}

impl AbstractHorse for SkeletonHorseEntity {
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

    /// Vanilla `SkeletonHorse.playJumpSound`.
    fn jump_sound(&self) -> Sound {
        if self.get_entity().is_in_water() {
            Sound::EntitySkeletonHorseJumpWater
        } else {
            Sound::EntityHorseJump
        }
    }
}
