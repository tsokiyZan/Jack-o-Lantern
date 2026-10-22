use crate::entity::ai::util::goal_utils;
use crate::entity::mob::mount_inventory::open_mount_inventory;
use crate::entity::mob::{
    first_passenger_player, horse_tick_ridden, is_player_holding, mob_controlling_passenger,
    read_legacy_saddle,
};
use crate::entity::passive::animal::horse_can_parent;
use crate::item::interaction_result::InteractionResult;
use pumpkin_inventory::inventory::SimpleInventory;
use pumpkin_util::Hand;
use pumpkin_util::random::RandomImpl;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering},
};

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
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

/// Vanilla `Camel.DASH_COOLDOWN_TICKS`.
const DASH_COOLDOWN_TICKS: i32 = 55;

pub struct CamelEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub flags: AtomicU8,
    pub dashing: AtomicBool,
    /// Vanilla `Camel.dashCooldown`.
    pub dash_cooldown: AtomicI32,
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
            dash_cooldown: AtomicI32::new(0),
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

    /// Vanilla `Camel.setDashing`, with the dash cooldown its `onSyncedDataUpdated` starts.
    pub fn set_dashing(&self, dashing: bool) {
        if self.dashing.swap(dashing, Ordering::Relaxed) == dashing {
            return;
        }
        let entity = self.get_entity();
        entity.set_synced_data(pumpkin_data::tracked_data::camel::DASH, dashing);
        // Vanilla's `firstTick` holds until the entity first ticks.
        if entity.tick_count.load(Ordering::Relaxed) > 0 {
            let _ = self.dash_cooldown.compare_exchange(
                0,
                DASH_COOLDOWN_TICKS,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }
}

impl CamelEntity {
    /// Vanilla `Camel.handleEating` for a food stack: heal, breed (camels count as tamed) and
    /// grow up, with the eating sound and the `eat` game event when any of it happened.
    fn handle_eating(&self, player: &Player) -> bool {
        let living = &self.mob_entity.living_entity;
        let could_heal = living.health.load() < living.get_max_health();
        if could_heal {
            living.heal(2.0);
        }
        let could_set_in_love = self.get_age() == 0 && self.can_fall_in_love();
        if could_set_in_love {
            self.set_in_love_by(player);
        }
        let could_age_up = self.can_age_up();
        if could_age_up {
            // `level.addParticle(HAPPY_VILLAGER, getRandomX(1.0), getRandomY() + 0.5, getRandomZ(1.0),
            // ...)` does nothing on the server but its arguments still draw from the mob's random.
            let _ = (
                self.get_entity().get_random_x(1.0),
                self.get_entity().get_random_y(),
                self.get_entity().get_random_z(1.0),
            );
            self.age_up(10, false);
        }
        if !could_heal && !could_set_in_love && !could_age_up {
            return false;
        }
        let entity = self.get_entity();
        if !entity.is_silent() {
            let sound = if entity.entity_type == &pumpkin_data::entity::EntityType::CAMEL_HUSK {
                Sound::EntityCamelHuskEat
            } else {
                Sound::EntityCamelEat
            };
            let pitch = {
                let mut random = entity.random();
                1.0 + (random.next_f32() - random.next_f32()) * 0.2
            };
            entity.world.load().play_sound_fine(
                sound,
                self.sound_source(),
                &entity.pos.load(),
                1.0,
                pitch,
            );
        }
        entity.world.load().game_event_from(
            Some(entity),
            pumpkin_data::game_event::GameEvent::Eat,
            entity.pos.load(),
        );
        true
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
    pumpkin_data::entity_dimensions::CAMEL_BABY_STANDING_DIMENSIONS;

impl Mob for CamelEntity {
    /// Vanilla `Camel.onElasticLeashPull`: `AbstractHorse`'s. Its stand-up of a sitting camel
    /// has nothing to do, as Pumpkin's camel never sits.
    fn on_elastic_leash_pull(&self, _caller: &dyn EntityBase) {
        self.get_entity().check_fall_distance_accumulation();
        if self.has_flag(FLAG_EATING) {
            self.set_flag(FLAG_EATING, false);
        }
    }

    /// Vanilla `AbstractHorse.supportQuadLeash`.
    fn support_quad_leash(&self) -> bool {
        true
    }

    /// Vanilla `Camel` (and `CamelHusk`) runs on a brain and leaves its goal selector empty, so
    /// `Mob.updateControlFlags` gates nothing; the goals standing in for that brain here must
    /// not be gated either.
    fn update_control_flags(&self) {}

    /// Vanilla `CamelHusk.interact`, before `super.interact`.
    fn before_interact(&self) {
        if self.get_entity().entity_type == &pumpkin_data::entity::EntityType::CAMEL_HUSK {
            self.get_mob_entity()
                .persistence_required
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Vanilla `CamelHusk.canBeLeashed`, with `isMobControlled`: a mob rides in front.
    fn can_be_leashed(&self) -> bool {
        self.get_entity().entity_type != &pumpkin_data::entity::EntityType::CAMEL_HUSK
            || self
                .get_entity()
                .get_first_passenger()
                .is_none_or(|passenger| passenger.get_mob().is_none())
    }

    /// Vanilla `Camel.getPassengerAttachmentPoint`. Pumpkin's camel never sits, so the body
    /// anchor is always the standing one of `getBodyAnchorAnimationYOffset`.
    fn mob_passenger_attachment_point(
        &self,
        passenger: &dyn EntityBase,
        dimensions: EntityDimensions,
        scale: f32,
    ) -> pumpkin_util::math::vector3::Vector3<f64> {
        let entity = self.get_entity();
        let passengers = entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let index = passengers
            .iter()
            .position(|p| p.get_entity().entity_id == passenger.get_entity().entity_id)
            .unwrap_or(0);
        let driver = index == 0;
        let mut offset = 0.5f32;
        let age_sit_y_offset = if entity.is_baby() { 0.093_75 } else { 0.375 };
        let height = if entity.is_removed() {
            0.01
        } else {
            (f64::from(dimensions.height) - age_sit_y_offset) as f32
        };
        if passengers.len() > 1 {
            if !driver {
                offset = -0.7;
            }
            if passenger.get_mob().is_some_and(|m| m.as_animal().is_some()) {
                offset += 0.2;
            }
        }
        pumpkin_util::math::vector3::Vector3::new(0.0, f64::from(height), f64::from(offset * scale))
            .y_rot(-entity.yaw.load() * (std::f32::consts::PI / 180.0))
    }

    /// Vanilla `AbstractHorse.positionRider`, which the camel inherits.
    fn mob_after_position_rider(&self, passenger: &dyn EntityBase) {
        if passenger.get_living_entity().is_some() {
            passenger
                .get_entity()
                .body_yaw
                .store(self.get_entity().body_yaw.load());
        }
    }

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

    fn tick_ridden(&self, controller: &Player) {
        horse_tick_ridden(self.get_entity(), controller);
    }

    /// Vanilla `Camel.openCustomInventoryScreen`: the saddle slot, as camels carry nothing.
    fn open_custom_inventory_screen(&self, player: &Arc<Player>) {
        open_mount_inventory(
            player,
            self,
            Arc::new(SimpleInventory::new(0)),
            0,
            |_, _| true,
        );
    }

    /// Vanilla `Camel.canJump`. A Pumpkin camel never sits, so it never refuses to move.
    fn can_jump(&self) -> bool {
        self.is_saddled()
    }

    /// Vanilla `Camel.handleStartJump`: the rider began a dash.
    fn handle_start_jump(&self, _jump_scale: i32) {
        self.make_sound(Sound::EntityCamelDash);
        let entity = self.get_entity();
        entity
            .world
            .load()
            .game_event_from(Some(self), GameEvent::EntityAction, entity.pos.load());
        self.set_dashing(true);
    }

    /// The part of vanilla `Camel.tick` after `super.tick()`.
    fn post_tick(&self) {
        let entity = self.get_entity();
        if self.is_dashing()
            && self.dash_cooldown.load(Ordering::Relaxed) < 50
            && (entity.on_ground.load(Ordering::Relaxed)
                || entity.touching_water.load(Ordering::Relaxed)
                || entity.touching_lava.load(Ordering::Relaxed)
                || entity.has_vehicle())
        {
            self.set_dashing(false);
        }
        let dash_cooldown = self.dash_cooldown.load(Ordering::Relaxed);
        if dash_cooldown > 0 {
            self.dash_cooldown
                .store(dash_cooldown - 1, Ordering::Relaxed);
            if dash_cooldown == 1 {
                entity.world.load().play_sound_fine(
                    Sound::EntityCamelDashReady,
                    SoundCategory::Neutral,
                    &entity.block_pos.load().to_centered_f64(),
                    1.0,
                    1.0,
                );
            }
        }
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
    fn mob_interact(
        &self,
        player: &Arc<Player>,
        _hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        if player.get_entity().is_sneaking() && !self.is_baby() {
            self.open_custom_inventory_screen(player);
            return InteractionResult::SUCCESS;
        }
        if self.equip_on_target(player, item_stack) {
            return InteractionResult::SUCCESS;
        }
        // `fedFood`: the stack used up when the camel ate (`SUCCESS_SERVER`), else `PASS`.
        if self.is_food(item_stack) {
            return if self.handle_eating(player) {
                item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                InteractionResult::SUCCESS_SERVER
            } else {
                InteractionResult::PASS
            };
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
            return InteractionResult::CONSUME;
        }
        if self.is_baby() && is_player_holding(player.as_ref(), &Item::GOLDEN_DANDELION) {
            self.animal_interact(player, item_stack, Sound::EntityCamelAmbient)
        } else {
            InteractionResult::FAIL
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::entity::ai::goal::Controls;
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::vector3::Vector3;

    /// A husk riding a camel (vanilla `Husk.finalizeSpawn`'s camel-husk jockey) steers it, but
    /// the camel's brain stand-in goals keep every control. A zombie mount loses them.
    #[tokio::test]
    async fn husk_rider_leaves_camel_goals_enabled() {
        let (_folder, world) = crate::world::test_support::test_world();
        let spawn = |entity_type| {
            crate::entity::r#type::from_type(
                entity_type,
                Vector3::new(0.0, 100.0, 0.0),
                &world,
                uuid::Uuid::new_v4(),
            )
        };
        for (mount_type, gated) in [(&EntityType::CAMEL, false), (&EntityType::ZOMBIE, true)] {
            let mount = spawn(mount_type);
            let husk = spawn(&EntityType::HUSK);
            mount
                .get_entity()
                .add_passenger_without_event(mount.clone(), husk);
            let mob = mount.get_mob().unwrap();
            assert!(mob.get_controlling_passenger().is_some());

            mob.update_control_flags();
            let goals = mob.get_mob_entity().goals_selector.lock().unwrap();
            for control in [Controls::MOVE, Controls::JUMP, Controls::LOOK] {
                assert_eq!(
                    goals.is_control_disabled(control),
                    gated,
                    "{}",
                    mount_type.resource_name
                );
            }
        }
    }
}
