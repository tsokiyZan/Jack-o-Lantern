use crate::item::interaction_result::InteractionResult;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_util::random::RandomImpl;
use uuid::Uuid;

use crate::entity::mob::mount_inventory::{MountInventory, open_mount_inventory};
use crate::entity::mob::{Mob, is_player_holding};
use crate::entity::passive::animal::Animal;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};
use pumpkin_nbt::compound::NbtCompound;

/// `AbstractHorse` data flag: tamed.
pub const FLAG_TAME: u8 = 2;
/// `AbstractHorse` data flag: bred.
pub const FLAG_BRED: u8 = 8;
/// `AbstractHorse` data flag: eating.
pub const FLAG_EATING: u8 = 16;
/// `AbstractHorse` data flag: standing (rearing).
pub const FLAG_STANDING: u8 = 32;
/// `AbstractHorse` data flag: mouth open.
pub const FLAG_OPEN_MOUTH: u8 = 64;

/// Vanilla `AbstractHorse.onElasticLeashPull`: `Leashable`'s, then the horse stops eating. Free
/// so the camel, which keeps its own flags, shares it.
pub fn on_elastic_leash_pull(entity: &Entity, is_eating: bool, stop_eating: impl FnOnce()) {
    entity.check_fall_distance_accumulation();
    if is_eating {
        stop_eating();
    }
}

/// Vanilla `AbstractHorse`'s taming by riding and rearing, shared by horses, donkeys, mules,
/// llamas and the undead horses.
pub trait AbstractHorse: Mob {
    fn horse_flag(&self, flag: u8) -> bool;
    fn set_horse_flag(&self, flag: u8, value: bool);
    fn set_horse_tamed(&self, tamed: bool);
    fn set_horse_owner(&self, owner: Uuid);
    fn horse_temper(&self) -> &AtomicI32;
    fn stand_counter(&self) -> &AtomicI32;
    /// Vanilla `mouthCounter`.
    fn mouth_counter(&self) -> &AtomicI32;
    /// Vanilla `standAnim` and `standAnimO`, in that order.
    fn stand_anim(&self) -> &crossbeam::atomic::AtomicCell<[f32; 2]>;
    /// Vanilla `AbstractHorse.inventory`.
    fn horse_inventory(&self) -> &MountInventory;

    /// Vanilla `getAngrySound`; plain `AbstractHorse` has none.
    fn angry_sound(&self) -> Option<Sound> {
        None
    }

    /// Vanilla `getMaxTemper`.
    fn max_temper(&self) -> i32 {
        100
    }

    /// Vanilla `playJumpSound`'s sound.
    fn jump_sound(&self) -> Sound {
        Sound::EntityHorseJump
    }

    /// Vanilla `AbstractHorse.handleStartJump`: the rider began a jump. The stand sliding it
    /// also allows only matters to the steering client.
    fn horse_handle_start_jump(&self) {
        self.stand_if_possible();
        self.play_sound_at_self(self.jump_sound(), 0.4, 1.0);
    }

    /// Vanilla `canPerformRearing`.
    fn can_perform_rearing(&self) -> bool {
        true
    }

    fn is_horse_tamed(&self) -> bool {
        self.horse_flag(FLAG_TAME)
    }

    fn is_standing(&self) -> bool {
        self.horse_flag(FLAG_STANDING)
    }

    /// The chest flag of a vanilla `AbstractChestedHorse` and its synced data; `None` for
    /// the kinds that never carry a chest.
    fn chest(&self) -> Option<(&AtomicBool, TrackedData)> {
        None
    }

    /// Vanilla `AbstractChestedHorse.hasChest`.
    fn has_chest(&self) -> bool {
        self.chest().is_some_and(|(flag, _)| flag.load(Relaxed))
    }

    /// Vanilla `AbstractChestedHorse.setChest`.
    fn set_chest(&self, chest: bool) {
        if let Some((flag, data)) = self.chest() {
            flag.store(chest, Relaxed);
            self.get_entity().set_synced_data(data, chest);
        }
    }

    /// Vanilla `getInventoryColumns`: none for most kinds, five with a chest
    /// (`AbstractChestedHorse`).
    fn inventory_columns(&self) -> usize {
        if self.has_chest() { 5 } else { 0 }
    }

    /// Vanilla `AbstractHorse.createInventory`.
    fn create_inventory(&self) {
        self.horse_inventory().create(self.inventory_columns());
    }

    /// Vanilla `AbstractHorse.openCustomInventoryScreen`.
    fn open_horse_inventory(&self, player: &Arc<Player>) {
        let entity = self.get_entity();
        if (!self.is_vehicle() || entity.has_passenger(player.entity_id()))
            && self.is_horse_tamed()
            && let Some(mob) = self.get_mob()
        {
            open_mount_inventory(
                player,
                mob,
                self.horse_inventory().current(),
                self.inventory_columns(),
                |mob, inventory| {
                    mob.as_abstract_horse()
                        .is_some_and(|horse| !horse.horse_inventory().has_changed(inventory))
                },
            );
        }
    }

    /// Vanilla `AbstractHorse.dropEquipment` and `AbstractChestedHorse.dropEquipment`, after
    /// the equipment itself.
    fn drop_horse_inventory(&self) {
        self.horse_inventory().drop_items(self.get_entity());
        if self.has_chest() {
            self.get_entity()
                .spawn_at_location(ItemStack::new(1, &Item::CHEST));
            self.set_chest(false);
        }
    }

    /// The chest and `Items` of vanilla `AbstractChestedHorse.addAdditionalSaveData`.
    fn write_chest_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("ChestedHorse", self.has_chest());
        if self.has_chest() {
            self.horse_inventory().write_items(nbt);
        }
    }

    /// The chest and `Items` of vanilla `AbstractChestedHorse.readAdditionalSaveData`.
    fn read_chest_nbt(&self, nbt: &NbtCompound) {
        self.set_chest(nbt.get_bool("ChestedHorse").unwrap_or(false));
        self.create_inventory();
        if self.has_chest() {
            self.horse_inventory().read_items(nbt);
        }
    }

    /// Vanilla `AbstractChestedHorse.equipChest`. Returns `false` for a kind that carries no
    /// chest.
    fn equip_chest(&self, player: &Player, item_stack: &mut ItemStack) -> bool {
        if self.chest().is_none() {
            return false;
        }
        self.set_chest(true);
        // `playChestEquipsSound`.
        let pitch = {
            let mut random = self.get_entity().random();
            (random.next_f32() - random.next_f32()) * 0.2 + 1.0
        };
        self.play_sound_at_self(Sound::EntityDonkeyChest, 1.0, pitch);
        item_stack.decrement_unless_creative(player.gamemode.load(), 1);
        self.create_inventory();
        true
    }

    /// Vanilla `AbstractHorse.canUseSlot`: only a tame adult takes a saddle.
    fn horse_can_use_slot(&self, slot: &EquipmentSlot) -> bool {
        *slot != EquipmentSlot::SADDLE
            || (self.get_entity().is_alive()
                && !self.get_entity().is_baby()
                && self.is_horse_tamed())
    }

    /// Vanilla `AbstractHorse.doPlayerRide`.
    fn do_player_ride(&self, player: &Arc<Player>) {
        self.set_horse_flag(FLAG_EATING, false);
        self.clear_standing();
        let entity = self.get_entity();
        player
            .get_entity()
            .set_rotation(entity.yaw.load(), entity.pitch.load());
        let world = entity.world.load();
        if let Some(vehicle) = world.get_entity_by_id(entity.entity_id)
            && let Some(passenger) = world.get_player_by_id(player.entity_id())
        {
            entity.add_passenger(vehicle, passenger as Arc<dyn EntityBase>);
        }
    }

    /// Vanilla `AbstractHorse.addAdditionalSaveData`'s flags, right after the ageable tags.
    fn write_horse_flags_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("EatingHaystack", self.horse_flag(FLAG_EATING));
        nbt.put_bool("Bred", self.horse_flag(FLAG_BRED));
    }

    /// Vanilla `AbstractHorse.readAdditionalSaveData`'s flags: `setEating` and `setBred`,
    /// `false` when missing.
    fn read_horse_flags_nbt(&self, nbt: &NbtCompound) {
        self.set_horse_flag(FLAG_EATING, nbt.get_bool("EatingHaystack").unwrap_or(false));
        self.set_horse_flag(FLAG_BRED, nbt.get_bool("Bred").unwrap_or(false));
    }

    /// Vanilla `AbstractHorse.onElasticLeashPull`.
    fn horse_on_elastic_leash_pull(&self) {
        on_elastic_leash_pull(self.get_entity(), self.horse_flag(FLAG_EATING), || {
            self.set_horse_flag(FLAG_EATING, false);
        });
    }

    /// Vanilla `modifyTemper`.
    fn modify_temper(&self, amount: i32) -> i32 {
        let temper = (self.horse_temper().load(Relaxed) + amount).clamp(0, self.max_temper());
        self.horse_temper().store(temper, Relaxed);
        temper
    }

    /// Vanilla `setStanding`.
    fn set_standing(&self, ticks: i32) {
        self.set_horse_flag(FLAG_EATING, false);
        self.set_horse_flag(FLAG_STANDING, true);
        self.stand_counter().store(ticks, Relaxed);
    }

    /// Vanilla `clearStanding`.
    fn clear_standing(&self) {
        self.set_horse_flag(FLAG_STANDING, false);
        self.stand_counter().store(0, Relaxed);
    }

    /// Vanilla `standIfPossible`.
    fn stand_if_possible(&self) {
        if self.can_perform_rearing() {
            self.set_standing(20);
        }
    }

    /// Vanilla `makeMad`: rears and complains, unless already standing.
    fn make_mad(&self) {
        if !self.is_standing() {
            self.stand_if_possible();
            if let Some(sound) = self.angry_sound() {
                self.make_sound(sound);
            }
        }
    }

    /// Vanilla `tameWithName`.
    fn tame_with_name(&self, owner: Uuid) {
        self.set_horse_owner(owner);
        self.set_horse_tamed(true);
        let entity = self.get_entity();
        entity
            .world
            .load()
            .send_entity_status(entity, EntityStatus::TamingSucceeded, None);
    }

    /// The standing countdown and the rearing animation from vanilla `AbstractHorse.tick`.
    fn tick_standing(&self) {
        // `tick`: the mouth closes 30 ticks after `openMouth`.
        let mouth = self.mouth_counter().load(Relaxed);
        if mouth > 0 {
            if mouth + 1 > 30 {
                self.mouth_counter().store(0, Relaxed);
                self.set_horse_flag(FLAG_OPEN_MOUTH, false);
            } else {
                self.mouth_counter().store(mouth + 1, Relaxed);
            }
        }

        let counter = self.stand_counter().load(Relaxed);
        if counter > 0 {
            self.stand_counter().store(counter - 1, Relaxed);
            if counter - 1 <= 0 {
                self.clear_standing();
            }
        }

        let [mut stand_anim, _] = self.stand_anim().load();
        let stand_anim_o = stand_anim;
        if self.is_standing() {
            stand_anim += (1.0 - stand_anim) * 0.4 + 0.05;
            if stand_anim > 1.0 {
                stand_anim = 1.0;
            }
        } else {
            stand_anim += (0.8 * stand_anim * stand_anim * stand_anim - stand_anim) * 0.6 - 0.05;
            if stand_anim < 0.0 {
                stand_anim = 0.0;
            }
        }
        self.stand_anim().store([stand_anim, stand_anim_o]);
    }

    /// Vanilla `AbstractHorse.getPassengerAttachmentPoint`: the default point, moved with the
    /// rearing animation.
    fn horse_passenger_attachment_point(
        &self,
        passenger: &dyn EntityBase,
        dimensions: pumpkin_util::math::boundingbox::EntityDimensions,
        scale: f32,
    ) -> pumpkin_util::math::vector3::Vector3<f64> {
        let entity = self.get_entity();
        let stand_anim_o = f64::from(self.stand_anim().load()[1]);
        let scale = f64::from(scale);
        entity.default_passenger_attachment_point(passenger, &dimensions.attachments)
            + pumpkin_util::math::vector3::Vector3::new(
                0.0,
                0.15 * stand_anim_o * scale,
                -0.7 * stand_anim_o * scale,
            )
            .y_rot(-entity.yaw.load() * (std::f32::consts::PI / 180.0))
    }

    /// Vanilla `AbstractHorse.positionRider` (and `Chicken`'s) after placing the passenger:
    /// a living passenger's body turns with the vehicle.
    fn turn_rider_body(&self, passenger: &dyn EntityBase) {
        if passenger.get_living_entity().is_some() {
            passenger
                .get_entity()
                .body_yaw
                .store(self.get_entity().body_yaw.load());
        }
    }
}

/// Vanilla `Horse.mobInteract`, which `AbstractChestedHorse.mobInteract` repeats with the chest:
/// food feeds an untamed or tamed horse, any other item angers an untamed one.
pub fn horse_mob_interact<H: AbstractHorse + Animal + ?Sized>(
    horse: &H,
    player: &Arc<Player>,
    item_stack: &mut ItemStack,
    ambient_sound: Sound,
) -> InteractionResult {
    let is_baby = horse.get_entity().is_baby();
    let should_open_inventory =
        !is_baby && horse.is_horse_tamed() && player.get_entity().is_sneaking();
    if !horse.is_vehicle()
        && !should_open_inventory
        && (!is_baby || !is_player_holding(player.as_ref(), &Item::GOLDEN_DANDELION))
        && !item_stack.is_empty()
    {
        if horse.is_food(item_stack) {
            return fed_food(horse, player, item_stack);
        }
        if !horse.is_horse_tamed() {
            horse.make_mad();
            return InteractionResult::SUCCESS;
        }
        if !horse.has_chest()
            && item_stack.item == &Item::CHEST
            && horse.equip_chest(player, item_stack)
        {
            return InteractionResult::SUCCESS;
        }
    }
    abstract_horse_mob_interact(horse, player, item_stack, ambient_sound)
}

/// Vanilla `AbstractHorse.mobInteract`. Of `ItemStack.interactLivingEntity` it runs the
/// equipping of saddles; other items are used on the horse after it passes.
pub fn abstract_horse_mob_interact<H: AbstractHorse + Animal + ?Sized>(
    horse: &H,
    player: &Arc<Player>,
    item_stack: &mut ItemStack,
    ambient_sound: Sound,
) -> InteractionResult {
    if horse.is_vehicle() || horse.get_entity().is_baby() {
        return horse.animal_interact(player, item_stack, ambient_sound);
    }
    if horse.is_horse_tamed() && player.get_entity().is_sneaking() {
        horse.open_custom_inventory_screen(player);
        return InteractionResult::SUCCESS;
    }
    if !item_stack.is_empty() {
        if horse.equip_on_target(player, item_stack) {
            return InteractionResult::SUCCESS;
        }
        let mob_entity = horse.get_mob_entity();
        if horse.is_equippable_in_slot(item_stack, &EquipmentSlot::BODY)
            && mob_entity.get_item_slot(&EquipmentSlot::BODY).is_empty()
        {
            // `equipBodyArmor`.
            let armor = item_stack.split_unless_creative(player.gamemode.load(), 1);
            horse.set_item_slot_and_notify(&EquipmentSlot::BODY, armor);
            mob_entity.set_guaranteed_drop(&EquipmentSlot::BODY);
            return InteractionResult::SUCCESS;
        }
    }
    horse.do_player_ride(player);
    InteractionResult::SUCCESS
}

/// Vanilla `AbstractHorse.fedFood`: `handleEating` (the llamas' own for them), the stack
/// used up when the horse ate (`SUCCESS_SERVER`), else `PASS`.
pub fn fed_food<H: AbstractHorse + Animal + ?Sized>(
    horse: &H,
    player: &Player,
    item_stack: &mut ItemStack,
) -> InteractionResult {
    let entity_type = horse.get_entity().entity_type;
    let ate = if entity_type == &pumpkin_data::entity::EntityType::LLAMA
        || entity_type == &pumpkin_data::entity::EntityType::TRADER_LLAMA
    {
        llama_handle_eating(horse, player, item_stack)
    } else {
        horse_handle_eating(horse, player, item_stack)
    };
    if ate {
        item_stack.decrement_unless_creative(player.gamemode.load(), 1);
        InteractionResult::SUCCESS_SERVER
    } else {
        InteractionResult::PASS
    }
}

/// Vanilla `getEatingSound` of the horse family; the skeleton horse has none.
fn horse_eating_sound(entity: &crate::entity::Entity) -> Option<Sound> {
    use pumpkin_data::entity::EntityType;
    let entity_type = entity.entity_type;
    if entity_type == &EntityType::HORSE {
        Some(if entity.is_baby() {
            Sound::EntityBabyHorseEat
        } else {
            Sound::EntityHorseEat
        })
    } else if entity_type == &EntityType::DONKEY {
        Some(Sound::EntityDonkeyEat)
    } else if entity_type == &EntityType::MULE {
        Some(Sound::EntityMuleEat)
    } else if entity_type == &EntityType::ZOMBIE_HORSE {
        Some(Sound::EntityZombieHorseEat)
    } else if entity_type == &EntityType::LLAMA || entity_type == &EntityType::TRADER_LLAMA {
        Some(Sound::EntityLlamaEat)
    } else {
        None
    }
}

/// The eating sound of `AbstractHorse.eating` and `Llama.handleEating`; a silent horse plays
/// none and draws no pitch.
fn play_horse_eating_sound<H: AbstractHorse + ?Sized>(horse: &H) {
    let entity = horse.get_entity();
    if entity.is_silent() {
        return;
    }
    let Some(sound) = horse_eating_sound(entity) else {
        return;
    };
    let pitch = {
        let mut random = entity.random();
        1.0 + (random.next_f32() - random.next_f32()) * 0.2
    };
    entity.world.load().play_sound_fine(
        sound,
        horse.sound_source(),
        &entity.pos.load(),
        1.0,
        pitch,
    );
}

/// The heal, growth and temper steps both `handleEating`s share.
fn eat_heal_grow_temper<H: AbstractHorse + Animal + ?Sized>(
    horse: &H,
    mut item_used: bool,
    heal: f32,
    age_up: i32,
    temper: i32,
) -> bool {
    let living = &horse.get_mob_entity().living_entity;
    if living.health.load() < living.get_max_health() && heal > 0.0 {
        living.heal(heal);
        item_used = true;
    }
    if horse.get_entity().is_baby()
        && age_up > 0
        && let Some(ageable) = horse.as_ageable()
        && !ageable.is_age_locked()
    {
        // `level.addParticle(HAPPY_VILLAGER, getRandomX(1.0), getRandomY() + 0.5, getRandomZ(1.0),
        // ...)` does nothing on the server but its arguments still draw from the mob's random.
        let _ = (
            horse.get_entity().get_random_x(1.0),
            horse.get_entity().get_random_y(),
            horse.get_entity().get_random_z(1.0),
        );
        ageable.age_up(age_up, false);
        item_used = true;
    }
    if temper > 0
        && (item_used || !horse.is_horse_tamed())
        && horse.horse_temper().load(Relaxed) < horse.max_temper()
    {
        horse.modify_temper(temper);
        item_used = true;
    }
    item_used
}

/// Vanilla `AbstractHorse.handleEating`.
fn horse_handle_eating<H: AbstractHorse + Animal + ?Sized>(
    horse: &H,
    player: &Player,
    item_stack: &ItemStack,
) -> bool {
    let item = item_stack.item;
    let mut item_used = false;
    let (heal, age_up, temper) = if item == &Item::WHEAT {
        (2.0, 20, 3)
    } else if item == &Item::SUGAR {
        (1.0, 30, 3)
    } else if item == &Item::HAY_BLOCK {
        (20.0, 180, 0)
    } else if item == &Item::APPLE || item == &Item::CARROT {
        (3.0, 60, 3)
    } else if item == &Item::RED_MUSHROOM {
        (3.0, 0, 3)
    } else if item == &Item::GOLDEN_CARROT
        || item == &Item::GOLDEN_APPLE
        || item == &Item::ENCHANTED_GOLDEN_APPLE
    {
        let mob_entity = horse.get_mob_entity();
        if horse.is_horse_tamed()
            && horse.get_entity().age.load(Relaxed) == 0
            && !mob_entity.is_in_love()
        {
            item_used = true;
            horse.set_in_love_by(player);
        }
        if item == &Item::GOLDEN_CARROT {
            (4.0, 60, 5)
        } else {
            (10.0, 240, 10)
        }
    } else {
        (0.0, 0, 0)
    };
    let item_used = eat_heal_grow_temper(horse, item_used, heal, age_up, temper);
    if item_used {
        // `eating`: `openMouth`, then the sound.
        horse.mouth_counter().store(1, Relaxed);
        horse.set_horse_flag(FLAG_OPEN_MOUTH, true);
        play_horse_eating_sound(horse);
        let entity = horse.get_entity();
        entity.world.load().game_event_from(
            Some(entity),
            pumpkin_data::game_event::GameEvent::Eat,
            entity.pos.load(),
        );
    }
    item_used
}

/// Vanilla `Llama.handleEating`.
fn llama_handle_eating<H: AbstractHorse + Animal + ?Sized>(
    horse: &H,
    player: &Player,
    item_stack: &ItemStack,
) -> bool {
    let item = item_stack.item;
    let mut item_used = false;
    let (heal, age_up, temper) = if item == &Item::WHEAT {
        (2.0, 10, 3)
    } else if item == &Item::HAY_BLOCK {
        if horse.is_horse_tamed()
            && horse.get_entity().age.load(Relaxed) == 0
            && horse.can_fall_in_love()
        {
            item_used = true;
            horse.set_in_love_by(player);
        }
        (10.0, 90, 6)
    } else {
        (0.0, 0, 0)
    };
    let item_used = eat_heal_grow_temper(horse, item_used, heal, age_up, temper);
    if item_used {
        play_horse_eating_sound(horse);
    }
    item_used
}

/// Vanilla `AbstractHorse.registerGoals`, before each kind's `addBehaviourGoals`.
pub fn add_abstract_horse_goals(
    goal_selector: &mut crate::entity::ai::goal::goal_selector::GoalSelector,
    mob: &dyn Mob,
    mob_weak: std::sync::Weak<dyn Mob>,
    can_perform_rearing: bool,
) {
    use crate::entity::ai::goal::{
        ambient_stand::AmbientStandGoal, breed::BreedGoal, follow_parent::FollowParentGoal,
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal,
        run_around_like_crazy::RunAroundLikeCrazyGoal, wander_around::WanderAroundGoal,
    };
    use crate::entity::ai::util::goal_utils;
    use pumpkin_data::entity::EntityType;

    goal_selector.add_goal(1, Box::new(RunAroundLikeCrazyGoal::new(1.2)));
    goal_selector.add_goal(
        2,
        BreedGoal::with_partner_class(1.0, goal_utils::is_abstract_horse),
    );
    goal_selector.add_goal(4, Box::new(FollowParentGoal::new(1.0)));
    goal_selector.add_goal(6, Box::new(WanderAroundGoal::avoiding_water(0.7)));
    goal_selector.add_goal(
        7,
        LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
    );
    goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));
    if can_perform_rearing {
        goal_selector.add_goal(9, Box::new(AmbientStandGoal::new(mob)));
    }
}
