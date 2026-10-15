//! Vanilla `AbstractNautilus` and its two kinds, `Nautilus` and `ZombieNautilus`.

use std::sync::{
    Arc, PoisonError,
    atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering},
};

use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::{EquipmentSlot, EquippableImpl};
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::{EntityPose, EntityStatus, EntityType};
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Effect;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_data::zombie_nautilus_variant::ZombieNautilusVariant;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;
use uuid::Uuid;

use crate::entity::ai::brain::behavior::utils::is_alive;
use crate::entity::ai::brain::{Brain, BrainTick, memory::PackedMemories, memory::types};
use crate::entity::ai::control::look_control::LookControl;
use crate::entity::ai::control::smooth_swimming_move_control::SmoothSwimmingMoveControl;
use crate::entity::ai::pathfinder::{Navigator, WaterBoundPathNavigation, node::PathType};
use crate::entity::breath::BreathManager;
use crate::entity::mob::equipment::is_equippable_in_slot;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::{first_passenger_player, mob_controlling_passenger, mob_hurt_server};
use crate::entity::r#type::from_type;
use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    custom_sound::CustomSound,
    mob::{Mob, MobEntity},
    passive::{
        animal::{Animal, set_animal_pathfinding_malus},
        nautilus_ai,
        tamable::{SITTING_FLAG, TAME_FLAG, TamableAnimal, TamableData},
        zombie_nautilus_ai,
    },
    player::Player,
};
use crate::world::World;

/// Vanilla `AbstractNautilus.SMALL_RESTRICTION_RADIUS`.
const SMALL_RESTRICTION_RADIUS: i32 = 16;
/// Vanilla `AbstractNautilus.LARGE_RESTRICTION_RADIUS`.
const LARGE_RESTRICTION_RADIUS: i32 = 32;
/// Vanilla `AbstractNautilus.RESTRICTION_RADIUS_BUFFER`.
const RESTRICTION_RADIUS_BUFFER: i32 = 8;
/// Vanilla `AbstractNautilus.EFFECT_DURATION`.
const EFFECT_DURATION: i32 = 60;
/// Vanilla `AbstractNautilus.EFFECT_REFRESH_RATE`.
const EFFECT_REFRESH_RATE: i64 = 40;
/// Vanilla `AbstractNautilus.NAUTILUS_WATER_RESISTANCE`.
const NAUTILUS_WATER_RESISTANCE: f64 = 0.9;
/// Vanilla `AbstractNautilus.IN_WATER_SPEED_MODIFIER`.
const IN_WATER_SPEED_MODIFIER: f32 = 0.011;
/// Vanilla `AbstractNautilus.DASH_COOLDOWN_TICKS`.
const DASH_COOLDOWN_TICKS: i32 = 40;
/// The dash flag clears once the cooldown drops below this, `DASH_COOLDOWN_TICKS` less
/// vanilla's `DASH_MINIMUM_DURATION_TICKS`.
const DASH_MINIMUM_COOLDOWN: i32 = 35;
/// The `maxTurnX` and `maxTurnY` of the nautilus's `SmoothSwimmingMoveControl`.
const MAX_TURN_X: i32 = 85;
const MAX_TURN_Y: i32 = 10;
/// The `maxYRotFromCenter` of the nautilus's `SmoothSwimmingLookControl`.
const MAX_Y_ROT_FROM_CENTER: f32 = 10.0;
/// The healing factor and default heal of `AbstractNautilus.mobInteract`'s `feed`.
const FEED_HEALING_FACTOR: f32 = 2.0;
const FEED_DEFAULT_HEAL: f32 = 1.0;
/// One taming attempt in this many succeeds, as `random.nextInt(3) == 0`.
const TAMING_CHANCE_BOUND: u32 = 3;
/// Vanilla `Nautilus.BABY_DIMENSIONS`. Pumpkin has no passenger attachment points.
const BABY_DIMENSIONS: EntityDimensions = Entity::type_dimensions(&EntityType::NAUTILUS).scale(0.5);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NautilusKind {
    Nautilus,
    ZombieNautilus,
}

pub struct NautilusEntity {
    pub mob_entity: MobEntity,
    kind: NautilusKind,
    ageable_data: AgeableData,
    tamable_data: TamableData,
    dashing: AtomicBool,
    dash_cooldown: AtomicI32,
    /// Vanilla `ZombieNautilus.DATA_VARIANT_ID`; unused by a nautilus.
    variant: AtomicU8,
}

impl NautilusEntity {
    pub fn new(entity: Entity, kind: NautilusKind) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        *mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            Navigator::new(WaterBoundPathNavigation::new(false).steered_by_move_control());
        // Vanilla `Animal`'s and `AbstractNautilus`'s constructors; after the navigator,
        // which holds the maluses.
        set_animal_pathfinding_malus(&mob_entity);
        mob_entity.set_pathfinding_malus(PathType::Water, 0.0);
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(SmoothSwimmingMoveControl::new(
            MAX_TURN_X,
            MAX_TURN_Y,
            IN_WATER_SPEED_MODIFIER,
            0.0,
            true,
        ));
        *mob_entity
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            LookControl::smooth_swimming(MAX_Y_ROT_FROM_CENTER);
        let nautilus = Arc::new(Self {
            mob_entity,
            kind,
            ageable_data: AgeableData::default(),
            tamable_data: TamableData::default(),
            dashing: AtomicBool::new(false),
            dash_cooldown: AtomicI32::new(0),
            variant: AtomicU8::new(ZombieNautilusVariant::Temperate.id()),
        });
        nautilus.mob_entity.init_brain(nautilus.as_ref());
        nautilus
    }

    const fn is_zombie(&self) -> bool {
        matches!(self.kind, NautilusKind::ZombieNautilus)
    }

    #[must_use]
    pub fn is_dashing(&self) -> bool {
        self.dashing.load(Ordering::Relaxed)
    }

    /// Vanilla `AbstractNautilus.setDashing`, with the server's own
    /// `onSyncedDataUpdated(DASH)`, which starts the cooldown if none runs.
    pub fn set_dashing(&self, dashing: bool) {
        if self.dashing.swap(dashing, Ordering::Relaxed) == dashing {
            return;
        }
        let entity = self.get_entity();
        entity.set_synced_data(tracked_data::abstract_nautilus::DASH, dashing);
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

    #[must_use]
    pub fn get_variant(&self) -> ZombieNautilusVariant {
        ZombieNautilusVariant::from_id(self.variant.load(Ordering::Relaxed)).unwrap_or_default()
    }

    /// Vanilla `ZombieNautilus.setVariant`.
    pub fn set_variant(&self, variant: ZombieNautilusVariant) {
        self.variant.store(variant.id(), Ordering::Relaxed);
        self.get_entity().set_synced_data(
            tracked_data::zombie_nautilus::DATA_VARIANT_ID,
            VarInt(i32::from(variant.id())),
        );
    }

    /// Vanilla `AbstractNautilus.canUseSlot`: the saddle and armor only go on a tame adult.
    fn can_use_slot(&self, slot: &EquipmentSlot) -> bool {
        if *slot == EquipmentSlot::SADDLE || *slot == EquipmentSlot::BODY {
            self.get_entity().is_alive() && !self.is_baby() && self.is_tame()
        } else {
            true
        }
    }

    /// Vanilla `LivingEntity.isEquippableInSlot` for an item that has the `equippable`
    /// component.
    fn is_equippable_in_slot(&self, stack: &ItemStack, slot: &EquipmentSlot) -> bool {
        is_equippable_in_slot(self.get_entity().entity_type, stack, slot) && self.can_use_slot(slot)
    }

    /// Vanilla `Mob.hasValidEquippableItemForSlot`.
    fn has_valid_equippable_item_for_slot(&self, slot: &EquipmentSlot) -> bool {
        let stack = self.mob_entity.get_item_slot(slot);
        !stack.is_empty() && self.is_equippable_in_slot(&stack, slot)
    }

    /// Vanilla `Equippable.equipOnTarget`, which `ItemStack.interactLivingEntity` runs for
    /// items equipped on interaction (saddles and nautilus armor).
    fn equip_on_target(&self, player: &Player, item_stack: &mut ItemStack) -> bool {
        let Some(equippable) = item_stack.get_data_component::<EquippableImpl>() else {
            return false;
        };
        if !equippable.equip_on_interact {
            return false;
        }
        let slot = equippable.slot;
        if !self.is_equippable_in_slot(item_stack, slot)
            || !self.mob_entity.get_item_slot(slot).is_empty()
            || !self.get_entity().is_alive()
        {
            return false;
        }
        let equipped = item_stack.split_unless_creative(player.gamemode.load(), 1);
        self.mob_entity.set_item_slot(slot, equipped.clone());
        self.on_equip_item(slot, &equipped);
        self.mob_entity.set_guaranteed_drop(slot);
        true
    }

    /// Vanilla `LivingEntity.onEquipItem` for a newly filled slot, which Pumpkin's
    /// `set_item_slot` leaves out.
    fn on_equip_item(&self, slot: &EquipmentSlot, stack: &ItemStack) {
        let Some(equippable) = stack.get_data_component::<EquippableImpl>() else {
            return;
        };
        let entity = self.get_entity();
        let world = entity.world.load();
        let pos = entity.pos.load();
        if !entity.is_silent() && equippable.slot == slot {
            // Vanilla `AbstractNautilus.getEquipSound`.
            if *slot == EquipmentSlot::SADDLE {
                let sound = if entity.is_under_water() {
                    Sound::ItemNautilusSaddleUnderwaterEquip
                } else {
                    Sound::ItemNautilusSaddleEquip
                };
                world.play_sound_fine(sound, self.sound_source(), &pos, 1.0, 1.0);
            } else {
                world.play_sound_event(&equippable.equip_sound, self.sound_source(), &pos);
            }
        }
        world.emit_game_event(GameEvent::Equip.name(), pos);
    }

    /// Vanilla `AbstractNautilus.getNautilusRestrictionRadius`.
    fn get_nautilus_restriction_radius(&self) -> i32 {
        if !self.is_baby()
            && self
                .mob_entity
                .get_item_slot(&EquipmentSlot::SADDLE)
                .is_empty()
        {
            LARGE_RESTRICTION_RADIUS
        } else {
            SMALL_RESTRICTION_RADIUS
        }
    }

    /// Vanilla `AbstractNautilus.checkRestriction`: a free tame nautilus keeps its home
    /// where it is.
    fn check_restriction(&self) {
        let entity = self.get_entity();
        if entity.is_leashed() || entity.has_passengers() || !self.is_tame() {
            return;
        }
        let mob_entity = &self.mob_entity;
        let radius = self.get_nautilus_restriction_radius();
        let block_pos = entity.block_pos.load();
        let buffered = radius + RESTRICTION_RADIUS_BUFFER;
        if !mob_entity.has_position_target()
            || mob_entity
                .position_target
                .load()
                .squared_distance(&block_pos)
                >= buffered * buffered
            || radius != mob_entity.position_target_range.load(Ordering::Relaxed)
        {
            mob_entity.position_target.store(block_pos);
            mob_entity
                .position_target_range
                .store(radius, Ordering::Relaxed);
        }
    }

    /// Vanilla `AbstractNautilus.applyEffects`: the rider breathes underwater.
    fn apply_effects(&self) {
        let entity = self.get_entity();
        let Some(passenger) = entity.get_first_passenger() else {
            return;
        };
        let Some(player) = passenger.get_player() else {
            return;
        };
        let has_effect = player
            .living_entity
            .has_effect(&StatusEffect::BREATH_OF_THE_NAUTILUS);
        let should_refresh = entity.world.load().get_world_age() % EFFECT_REFRESH_RATE == 0;
        if !has_effect || should_refresh {
            player.add_effect(Effect {
                effect_type: &StatusEffect::BREATH_OF_THE_NAUTILUS,
                duration: EFFECT_DURATION,
                amplifier: 0,
                ambient: true,
                show_particles: true,
                show_icon: true,
                // Vanilla blends a newly added effect, not a refreshed one.
                blend: !has_effect,
            });
        }
    }

    /// Vanilla `AbstractNautilus.doPlayerRide`.
    fn do_player_ride(&self, player: &Arc<Player>) {
        let entity = self.get_entity();
        // Vanilla `canAddPassenger`: one rider only.
        if !entity.has_passengers()
            && let Some(vehicle) = entity.world.load().get_entity_by_id(entity.entity_id)
        {
            entity.add_passenger(vehicle, player.clone());
        }
        if !entity.has_passengers() {
            self.mob_entity
                .position_target_range
                .store(-1, Ordering::Relaxed);
        }
    }

    /// Vanilla `AbstractNautilus.tryToTame`.
    fn try_to_tame(&self, player: &Arc<Player>) {
        let entity = self.get_entity();
        let world = entity.world.load();
        if self.get_random().random_range(0..TAMING_CHANCE_BOUND) == 0 {
            TamableAnimal::tame(self, player.gameprofile.id);
            Mob::tame(self, player);
            self.mob_entity
                .navigator
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .stop();
            world.send_entity_status(
                entity,
                EntityStatus::TamingSucceeded,
                Some(ActorEventID::TamingSucceeded),
            );
        } else {
            world.send_entity_status(
                entity,
                EntityStatus::TamingFailed,
                Some(ActorEventID::TamingFailed),
            );
        }
        self.play_eating_sound(self.eat_sound());
    }

    /// Vanilla `AbstractNautilus.isAggravated`.
    fn is_aggravated(&self) -> bool {
        self.mob_entity.with_brain(self, |tick| {
            tick.brain.has_memory_value(types::ANGRY_AT.id())
                || tick.brain.has_memory_value(types::ATTACK_TARGET.id())
        })
    }

    /// Vanilla `AbstractNautilus.isMobControlled`.
    fn is_mob_controlled(&self) -> bool {
        self.get_entity()
            .get_first_passenger()
            .is_some_and(|passenger| passenger.get_mob().is_some())
    }

    /// Vanilla `Nautilus.canBeLeashed` and `ZombieNautilus.canBeLeashed`.
    fn can_be_leashed(&self) -> bool {
        if self.is_zombie() {
            !self.is_aggravated() && !self.is_mob_controlled()
        } else {
            !self.is_aggravated()
        }
    }

    /// The lead handling of vanilla `Entity.interact`, which comes before `mobInteract`:
    /// its holder takes the lead back, and a lead ties up a nautilus that may be leashed.
    fn handles_lead(&self, player: &Player, item_stack: &ItemStack) -> bool {
        let holder = self
            .get_entity()
            .leashed_to
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|holder| holder.get_entity().entity_id);
        holder == Some(player.entity_id())
            || (item_stack.get_item() == &Item::LEAD && holder.is_none() && self.can_be_leashed())
    }

    /// Vanilla `Animal.mobInteract`, the `super.mobInteract` of the nautilus's. Leads were
    /// dealt with by `handles_lead`, so anything else is left alone.
    fn animal_mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        self.is_food(item_stack) && self.animal_interact(player, item_stack, self.eat_sound())
    }

    /// Vanilla `getDashSound`.
    fn get_dash_sound(&self) -> Sound {
        match (self.is_zombie(), self.get_entity().is_under_water()) {
            (false, true) => Sound::EntityNautilusDash,
            (false, false) => Sound::EntityNautilusDashLand,
            (true, true) => Sound::EntityZombieNautilusDash,
            (true, false) => Sound::EntityZombieNautilusDashLand,
        }
    }

    /// Vanilla `getDashReadySound`.
    fn get_dash_ready_sound(&self) -> Sound {
        match (self.is_zombie(), self.get_entity().is_under_water()) {
            (false, true) => Sound::EntityNautilusDashReady,
            (false, false) => Sound::EntityNautilusDashReadyLand,
            (true, true) => Sound::EntityZombieNautilusDashReady,
            (true, false) => Sound::EntityZombieNautilusDashReadyLand,
        }
    }

    /// The sound of vanilla `playEatingSound`.
    fn eat_sound(&self) -> Sound {
        if self.is_zombie() {
            Sound::EntityZombieNautilusEat
        } else if self.is_baby() {
            Sound::EntityBabyNautilusEat
        } else {
            Sound::EntityNautilusEat
        }
    }

    /// Vanilla `ZombieNautilus.finalizeSpawn`'s `VariantUtils.selectVariantToSpawn`: the
    /// coral variant in its biomes, the temperate one anywhere else.
    fn select_variant_to_spawn(world: &World, pos: &BlockPos) -> ZombieNautilusVariant {
        let biome = world.get_biome(pos).registry_id;
        let biome = biome.strip_prefix("minecraft:").unwrap_or(biome);
        if tag::WorldgenBiome::MINECRAFT_SPAWNS_CORAL_VARIANT_ZOMBIE_NAUTILUS
            .0
            .contains(&biome)
        {
            ZombieNautilusVariant::Warm
        } else {
            ZombieNautilusVariant::Temperate
        }
    }
}

impl AgeableMob for NautilusEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }

    /// Vanilla `ZombieNautilus.canBeABaby`.
    fn can_be_a_baby(&self) -> bool {
        !self.is_zombie()
    }
}

impl Animal for NautilusEntity {
    /// Vanilla `AbstractNautilus.isFood`: a wild adult only takes taming items.
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        if !self.is_tame() && !self.is_baby() {
            item_stack
                .item
                .has_tag(&tag::Item::MINECRAFT_NAUTILUS_TAMING_ITEMS)
        } else {
            item_stack.item.has_tag(&tag::Item::MINECRAFT_NAUTILUS_FOOD)
        }
    }

    /// Vanilla `AbstractNautilus.usePlayerItem`: a bucket of fish leaves a water bucket.
    fn use_player_item(&self, player: &Player, item_stack: &mut ItemStack) {
        if item_stack
            .item
            .has_tag(&tag::Item::MINECRAFT_NAUTILUS_BUCKET_FOOD)
        {
            crate::item::create_filled_result(
                item_stack,
                player,
                ItemStack::new(1, &Item::WATER_BUCKET),
                true,
            );
        } else {
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
        }
    }

    /// Vanilla `Nautilus.getBreedOffspring`, and `ZombieNautilus.getBreedOffspring`'s none.
    fn get_breed_offspring(
        &self,
        world: &Arc<World>,
        _partner: &dyn EntityBase,
    ) -> Option<Arc<dyn EntityBase>> {
        if self.is_zombie() {
            return None;
        }
        let baby = from_type(
            &EntityType::NAUTILUS,
            self.get_entity().pos.load(),
            world,
            Uuid::new_v4(),
        );
        if self.is_tame()
            && let Some(nautilus) = baby.cast_any().downcast_ref::<Self>()
        {
            nautilus.set_owner(self.get_owner());
            nautilus.set_tame(true);
        }
        Some(baby)
    }

    /// Vanilla `playEatingSound`.
    fn play_eating_sound(&self, _sound: Sound) {
        self.make_sound(self.eat_sound());
    }
}

impl TamableAnimal for NautilusEntity {
    fn get_tamable_data(&self) -> &TamableData {
        &self.tamable_data
    }
}

impl CustomSound for NautilusEntity {
    /// Vanilla `getHurtSound`.
    fn hurt_sound(&self) -> Option<Sound> {
        let under_water = self.get_entity().is_under_water();
        Some(if self.is_zombie() {
            if under_water {
                Sound::EntityZombieNautilusHurt
            } else {
                Sound::EntityZombieNautilusHurtLand
            }
        } else {
            match (self.is_baby(), under_water) {
                (true, true) => Sound::EntityBabyNautilusHurt,
                (true, false) => Sound::EntityBabyNautilusHurtLand,
                (false, true) => Sound::EntityNautilusHurt,
                (false, false) => Sound::EntityNautilusHurtLand,
            }
        })
    }

    /// Vanilla `getDeathSound`.
    fn death_sound(&self) -> Option<Sound> {
        let under_water = self.get_entity().is_under_water();
        Some(if self.is_zombie() {
            if under_water {
                Sound::EntityZombieNautilusDeath
            } else {
                Sound::EntityZombieNautilusDeathLand
            }
        } else {
            match (self.is_baby(), under_water) {
                (true, true) => Sound::EntityBabyNautilusDeath,
                (true, false) => Sound::EntityBabyNautilusDeathLand,
                (false, true) => Sound::EntityNautilusDeath,
                (false, false) => Sound::EntityNautilusDeathLand,
            }
        })
    }
}

impl Mob for NautilusEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn as_tamable(&self) -> Option<&dyn TamableAnimal> {
        Some(self)
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    /// Vanilla `AbstractNautilus.getWalkTargetValue`.
    fn get_walk_target_value(&self, _pos: &BlockPos) -> f32 {
        0.0
    }

    /// Vanilla `getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        let under_water = self.get_entity().is_under_water();
        Some(if self.is_zombie() {
            if under_water {
                Sound::EntityZombieNautilusAmbient
            } else {
                Sound::EntityZombieNautilusAmbientLand
            }
        } else {
            match (self.is_baby(), under_water) {
                (true, true) => Sound::EntityBabyNautilusAmbient,
                (true, false) => Sound::EntityBabyNautilusAmbientLand,
                (false, true) => Sound::EntityNautilusAmbient,
                (false, false) => Sound::EntityNautilusAmbientLand,
            }
        })
    }

    /// Vanilla `getSwimSound`.
    fn swim_sound(&self) -> Sound {
        if self.is_zombie() {
            Sound::EntityZombieNautilusSwim
        } else if self.is_baby() {
            Sound::EntityBabyNautilusSwim
        } else {
            Sound::EntityNautilusSwim
        }
    }

    /// Vanilla `Nautilus.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        if self.is_baby() {
            BABY_DIMENSIONS
        } else {
            self.get_entity().get_dimensions(pose)
        }
    }

    /// Vanilla `Nautilus.handleAirSupply`, which `Nautilus.baseTick` skips for mobs
    /// without AI.
    fn handle_air_supply(&self, pre_tick_air: i32) {
        if self.is_zombie() || self.mob_entity.is_no_ai() {
            return;
        }
        BreathManager::handle_air_out_of_water(
            self,
            pre_tick_air,
            self.get_entity().is_in_water(),
            self.max_air_supply(),
            DamageType::DRY_OUT,
        );
    }

    /// Vanilla `ZombieNautilus.sunProtectionSlot`.
    fn sun_protection_slot(&self) -> EquipmentSlot {
        if self.is_zombie() {
            EquipmentSlot::BODY
        } else {
            EquipmentSlot::HEAD
        }
    }

    /// Vanilla `AbstractNautilus.isPushedByFluid`.
    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }

    /// Vanilla `AbstractNautilus.checkSpawnObstruction`: water does not obstruct it.
    fn check_spawn_obstruction(&self, world: &World) -> bool {
        world
            .get_entities_at_box(&self.get_entity().bounding_box.load())
            .is_empty()
    }

    /// Vanilla `AbstractNautilus.requiresCustomPersistence`.
    fn requires_custom_persistence(&self) -> bool {
        let entity = self.get_entity();
        entity.has_vehicle() || entity.is_leashed() || self.is_tame()
    }

    /// Vanilla `Mob.isSaddled`.
    fn is_saddled(&self) -> bool {
        self.has_valid_equippable_item_for_slot(&EquipmentSlot::SADDLE)
    }

    /// Vanilla `AbstractNautilus.getControllingPassenger`.
    // TODO: vanilla `LivingEntity.travelRidden` and `AbstractNautilus.tickRidden`; Pumpkin
    // takes a player-steered vehicle's movement from `SMoveVehicle` and keeps running its
    // AI, where vanilla stops the AI and travel while the client steers.
    fn get_controlling_passenger(&self) -> Option<Arc<dyn EntityBase>> {
        if self.is_saddled()
            && let Some(player) = first_passenger_player(&self.mob_entity)
        {
            return Some(player);
        }
        mob_controlling_passenger(&self.mob_entity)
    }

    /// Vanilla `AbstractNautilus.canJump`.
    fn can_jump(&self) -> bool {
        self.is_saddled()
    }

    /// Vanilla `AbstractNautilus.handleStartJump`.
    fn handle_start_jump(&self, _jump_scale: i32) {
        self.make_sound(self.get_dash_sound());
        let entity = self.get_entity();
        entity
            .world
            .load()
            .emit_game_event(GameEvent::EntityAction.name(), entity.pos.load());
        self.set_dashing(true);
    }

    /// Vanilla `AgeableMob.aiStep`, the end of the nautilus's `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        if self.get_entity().is_alive() {
            self.ageable_ai_step();
        }
    }

    fn make_brain(&self, packed: &PackedMemories) -> Brain {
        if self.is_zombie() {
            zombie_nautilus_ai::ZOMBIE_NAUTILUS_PROVIDER.make_brain(self, packed)
        } else {
            nautilus_ai::NAUTILUS_PROVIDER.make_brain(self, packed)
        }
    }

    /// The activity update of vanilla `Nautilus.customServerAiStep` and
    /// `ZombieNautilus.customServerAiStep`.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        nautilus_ai::update_activity(tick);
    }

    /// Vanilla `Nautilus.customServerAiStep` and `ZombieNautilus.customServerAiStep`, then
    /// `AbstractNautilus.customServerAiStep`.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
        self.check_restriction();
    }

    /// The part of vanilla `AbstractNautilus.tick` after `super.tick()`. The bubbles it
    /// spawns are client-side.
    fn post_tick(&self) {
        self.apply_effects();
        if self.is_dashing() && self.dash_cooldown.load(Ordering::Relaxed) < DASH_MINIMUM_COOLDOWN {
            self.set_dashing(false);
        }
        let dash_cooldown = self.dash_cooldown.load(Ordering::Relaxed);
        if dash_cooldown > 0 {
            self.dash_cooldown
                .store(dash_cooldown - 1, Ordering::Relaxed);
            if dash_cooldown == 1 {
                self.make_sound(self.get_dash_ready_sound());
            }
        }
    }

    /// Vanilla `AbstractNautilus.travelInWater`.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        if !entity.is_in_water() {
            return false;
        }
        entity.update_velocity_from_input(living.movement_input.load(), living.get_speed(caller));
        living.make_move(caller);
        entity
            .velocity
            .store(entity.velocity.load() * NAUTILUS_WATER_RESISTANCE);
        true
    }

    /// Vanilla `AbstractNautilus.hurtServer`. The anger reaches the brain on its next tick,
    /// since the attacker may be holding its own brain.
    fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        let was_hurt = mob_hurt_server(self, caller, amount, damage_type, position, source, cause);
        if was_hurt
            && let Some(attacker) = cause
                .and_then(|cause| {
                    self.get_entity()
                        .world
                        .load()
                        .get_entity_by_id(cause.get_entity().entity_id)
                })
                .filter(|attacker| attacker.get_living_entity().is_some())
        {
            self.mob_entity.post_to_brain(Box::new(move |tick| {
                nautilus_ai::set_anger_target(tick, attacker.as_ref());
            }));
        }
        was_hurt
    }

    /// Vanilla `ZombieNautilus.finalizeSpawn` and `AbstractNautilus.finalizeSpawn`, then
    /// `AgeableMob`'s and `Mob`'s.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        if self.is_zombie() {
            self.set_variant(Self::select_variant_to_spawn(
                world,
                &self.get_entity().block_pos.load(),
            ));
        }
        self.mob_entity.with_brain(self, nautilus_ai::init_memories);
        let group_data = self.finalize_ageable_spawn(group_data);
        self.mob_entity.finalize_spawn_base();
        Some(group_data)
    }

    /// Vanilla `AbstractNautilus.interact` and `mobInteract`.
    // TODO: vanilla `openCustomInventoryScreen`; Pumpkin opens no mount inventories yet.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        self.mob_entity
            .persistence_required
            .store(true, Ordering::Relaxed);
        if self.handles_lead(player, item_stack) {
            return self.mob_entity.mob_interact(player, item_stack);
        }
        if self.is_baby() {
            return self.animal_mob_interact(player, item_stack);
        }
        let secondary_use = player.get_entity().is_sneaking();
        if self.is_tame() && secondary_use {
            return true;
        }
        if !item_stack.is_empty() {
            if !self.is_tame() && self.is_food(item_stack) {
                self.use_player_item(player, item_stack);
                self.try_to_tame(player);
                return true;
            }
            let living = &self.mob_entity.living_entity;
            if self.is_food(item_stack) && living.health.load() < living.get_max_health() {
                self.feed(player, item_stack, FEED_HEALING_FACTOR, FEED_DEFAULT_HEAL);
                self.play_eating_sound(self.eat_sound());
                return true;
            }
            if self.equip_on_target(player, item_stack) {
                return true;
            }
        }
        if self.is_tame() && !secondary_use && !self.is_food(item_stack) {
            self.do_player_ride(player);
            return true;
        }
        self.animal_mob_interact(player, item_stack)
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        if self.is_baby() {
            entity.set_synced_data(tracked_data::nautilus::DATA_BABY_ID, true);
        }
        let mut flags = if self.is_in_sitting_pose() {
            SITTING_FLAG
        } else {
            0
        };
        if self.is_tame() {
            flags |= TAME_FLAG;
        }
        entity.set_synced_data(tracked_data::tamable_animal::DATA_FLAGS_ID, flags as i8);
        entity.set_synced_data(
            tracked_data::tamable_animal::DATA_OWNERUUID_ID,
            self.get_owner(),
        );
        entity.set_synced_data(tracked_data::abstract_nautilus::DASH, self.is_dashing());
        if self.is_zombie() {
            entity.set_synced_data(
                tracked_data::zombie_nautilus::DATA_VARIANT_ID,
                VarInt(i32::from(self.get_variant().id())),
            );
        }
    }

    /// Vanilla `ZombieNautilus.addAdditionalSaveData`.
    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        if self.is_zombie() {
            nbt.put_string(
                "variant",
                format!("minecraft:{}", self.get_variant().to_name()),
            );
        }
    }

    /// Vanilla `ZombieNautilus.readAdditionalSaveData`.
    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if self.is_zombie()
            && let Some(variant) = nbt
                .get_string("variant")
                .and_then(ZombieNautilusVariant::from_name)
        {
            self.set_variant(variant);
        }
    }

    /// Vanilla `ZombieNautilus.applyImplicitComponent` for its variant.
    fn mob_set_variant_name(&self, name: &str) {
        if self.is_zombie()
            && let Some(variant) = ZombieNautilusVariant::from_name(name)
        {
            self.set_variant(variant);
        }
    }
}
