use crate::entity::ai::util::goal_utils;
use crate::entity::mob::{
    first_passenger_player, is_player_holding, mob_controlling_passenger, read_legacy_saddle,
};
use crate::entity::passive::animal::horse_can_parent;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        breed::BreedGoal, escape_danger::EscapeDangerGoal, follow_parent::FollowParentGoal,
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal,
        tempt::TemptGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

const TEMPT_ITEMS: &[&Item] = &[&Item::CACTUS];

pub const FLAG_TAME: u8 = 2;
pub const FLAG_BRED: u8 = 8;
pub const FLAG_EATING: u8 = 16;
pub const FLAG_STANDING: u8 = 32;

pub struct CamelEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub flags: AtomicU8,
    pub dashing: AtomicBool,
}

impl CamelEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        let camel = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            flags: AtomicU8::new(0),
            dashing: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(camel);
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

            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(1, EscapeDangerGoal::new(2.0));
            goal_selector.add_goal(2, BreedGoal::new(1.0));
            goal_selector.add_goal(3, Box::new(TemptGoal::new(1.25, TEMPT_ITEMS, false)));
            goal_selector.add_goal(4, Box::new(FollowParentGoal::new(1.0)));
            goal_selector.add_goal(
                6,
                Box::new(WanderAroundGoal::new(0.7).ignoring_no_action_time()),
            );
            goal_selector.add_goal(
                7,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));
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
            pumpkin_data::tracked_data::camel::DATA_ID_FLAGS,
            new_flags as i8,
        );
    }

    #[must_use]
    pub fn is_dashing(&self) -> bool {
        self.dashing.load(Ordering::Relaxed)
    }

    pub fn set_dashing(&self, dashing: bool) {
        self.dashing.store(dashing, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(pumpkin_data::tracked_data::camel::DASH, dashing);
    }
}

impl AgeableMob for CamelEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for CamelEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_CAMEL_FOOD)
            || item_stack.item == &Item::CACTUS
    }

    /// Vanilla `Camel.canMate`.
    fn can_mate(&self, partner: &dyn Mob) -> bool {
        partner.get_entity().entity_id != self.get_entity().entity_id
            && goal_utils::is_instance_of(&EntityType::CAMEL, partner.get_entity().entity_type)
            && self.can_parent()
            && partner.as_animal().is_some_and(Animal::can_parent)
    }

    /// Camels are always tamed (`Camel.isTamed`).
    fn can_parent(&self) -> bool {
        horse_can_parent(self, true)
    }
}

/// Vanilla `Camel.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    EntityDimensions::scalable(0.95, 1.4).with_eye_height(1.38);

impl Mob for CamelEntity {
    /// Vanilla `Camel.playStepSound`.
    fn play_step_sound(
        &self,
        caller: &dyn crate::entity::EntityBase,
        _pos: &pumpkin_util::math::position::BlockPos,
        block: &'static pumpkin_data::Block,
        _state: &'static pumpkin_data::BlockState,
    ) {
        let sound = if block.has_tag(&tag::Block::MINECRAFT_CAMEL_SAND_STEP_SOUND_BLOCKS) {
            Sound::EntityCamelStepSand
        } else {
            Sound::EntityCamelStep
        };
        caller.play_sound_at_self(sound, 1.0, 1.0);
    }

    /// Vanilla `Camel.getDefaultDimensions`.
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
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        read_legacy_saddle(&self.mob_entity, nbt);
    }

    /// Vanilla `AbstractHorse.canUseSlot`; camels are always tame.
    fn can_use_slot(&self, slot: &EquipmentSlot) -> bool {
        *slot != EquipmentSlot::SADDLE || (self.get_entity().is_alive() && !self.is_baby())
    }

    /// Vanilla `Camel.getEquipSound`.
    fn equip_sound(&self, slot: &EquipmentSlot) -> Option<Sound> {
        (*slot == EquipmentSlot::SADDLE).then_some(Sound::EntityCamelSaddle)
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
        self.ageable_ai_step();
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::camel::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::camel::DATA_ID_FLAGS,
            self.flags.load(Ordering::Relaxed) as i8,
        );
        entity.set_synced_data(pumpkin_data::tracked_data::camel::DASH, self.is_dashing());
    }

    /// Vanilla `Camel.mobInteract`.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if player.get_entity().is_sneaking() && !self.is_baby() {
            self.open_custom_inventory_screen(player);
            return true;
        }
        if self.equip_on_target(player, item_stack) {
            return true;
        }
        // TODO: vanilla `Camel.handleEating`; Pumpkin feeds camels like other animals.
        if self.is_food(item_stack) {
            return self.animal_interact(player, item_stack, Sound::EntityCamelAmbient);
        }
        let entity = self.get_entity();
        if entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
            < 2
            && !self.is_baby()
        {
            // Vanilla `AbstractHorse.doPlayerRide`.
            self.set_flag(FLAG_EATING, false);
            self.set_flag(FLAG_STANDING, false);
            player
                .get_entity()
                .set_rotation(entity.yaw.load(), entity.pitch.load());
            let world = entity.world.load();
            if let Some(vehicle) = world.get_entity_by_id(entity.entity_id)
                && let Some(passenger) = world.get_player_by_id(player.entity_id())
            {
                entity.add_passenger(vehicle, passenger as Arc<dyn EntityBase>);
            }
            return true;
        }
        self.is_baby()
            && is_player_holding(player.as_ref(), &Item::GOLDEN_DANDELION)
            && self.animal_interact(player, item_stack, Sound::EntityCamelAmbient)
    }
}
