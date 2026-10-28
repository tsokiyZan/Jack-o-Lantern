use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::block_properties::{
    ChestLikeProperties, ChestType, CopperGolemPose, CopperGolemStatueLikeProperties, EnumVariants,
};
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, tracked_data};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::text::TextComponent;
use pumpkin_world::world::BlockFlags;
use uuid::Uuid;

use crate::block::blocks::chests;
use crate::block::blocks::weathering_copper::WeatherState;
use crate::block::entities::copper_golem_statue::CopperGolemStatueBlockEntity;
use crate::entity::ai::brain::behavior::utils::is_alive;
use crate::entity::ai::brain::memory::{PackedMemories, types};
use crate::entity::ai::brain::{Brain, BrainTick};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::custom_sound::CustomSound;
use crate::entity::lightning::LightningBoltEntity;
use crate::entity::mob::{Mob, MobEntity};
use crate::entity::passive::copper_golem_ai;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};
use crate::world::World;

const IGNORE_WEATHERING_TICK: i64 = -2;
const UNSET_WEATHERING_TICK: i64 = -1;
const WEATHERING_TICK_FROM: i32 = 504_000;
const WEATHERING_TICK_TO: i32 = 552_000;
const TURN_TO_STATUE_CHANCE: f32 = 0.0058;
const SPAWN_COOLDOWN_MIN: i32 = 60;
const SPAWN_COOLDOWN_MAX: i32 = 100;

/// Vanilla `CopperGolemState`; the ordinal is the synced id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CopperGolemState {
    #[default]
    Idle,
    GettingItem,
    GettingNoItem,
    DroppingItem,
    DroppingNoItem,
}

/// Vanilla `CopperGolem`.
pub struct CopperGolemEntity {
    pub mob_entity: MobEntity,
    weather_state: AtomicCell<WeatherState>,
    state: AtomicCell<CopperGolemState>,
    opened_chest_pos: AtomicCell<Option<BlockPos>>,
    last_lightning_bolt_uuid: AtomicCell<Option<Uuid>>,
    next_weathering_tick: AtomicI64,
}

impl CopperGolemEntity {
    /// Vanilla `CopperGolem.EQUIPMENT_SLOT_ANTENNA`.
    pub const EQUIPMENT_SLOT_ANTENNA: EquipmentSlot = EquipmentSlot::SADDLE;
    /// Vanilla `CopperGolem.getContainerInteractionRange`.
    pub const CONTAINER_INTERACTION_RANGE: f64 = 3.0;

    pub fn new(entity: Entity) -> Arc<Self> {
        let golem = Arc::new(Self {
            mob_entity: MobEntity::new(entity),
            weather_state: AtomicCell::new(WeatherState::Unaffected),
            state: AtomicCell::new(CopperGolemState::Idle),
            opened_chest_pos: AtomicCell::new(None),
            last_lightning_bolt_uuid: AtomicCell::new(None),
            next_weathering_tick: AtomicI64::new(UNSET_WEATHERING_TICK),
        });
        golem.mob_entity.init_brain(golem.as_ref());
        let mob_entity = &golem.mob_entity;
        mob_entity.set_required_path_length(48.0);
        mob_entity.own_navigation().set_can_open_doors(true);
        mob_entity
            .persistence_required
            .store(true, Ordering::Relaxed);
        golem.set_state(CopperGolemState::Idle);
        // Vanilla's `FIRE_IN_NEIGHBOR`, `DAMAGING_IN_NEIGHBOR` and `FIRE`.
        mob_entity.set_pathfinding_malus(PathType::DangerFire, 16.0);
        mob_entity.set_pathfinding_malus(PathType::DangerOther, 16.0);
        mob_entity.set_pathfinding_malus(PathType::DamageFire, -1.0);
        let cooldown = golem
            .get_entity()
            .random()
            .next_int_range(SPAWN_COOLDOWN_MIN, SPAWN_COOLDOWN_MAX);
        mob_entity.with_brain(golem.as_ref(), |tick| {
            tick.brain
                .set(types::TRANSPORT_ITEMS_COOLDOWN_TICKS, cooldown);
        });
        golem
    }

    #[must_use]
    pub fn get_state(&self) -> CopperGolemState {
        self.state.load()
    }

    pub fn set_state(&self, state: CopperGolemState) {
        self.state.store(state);
        self.get_entity().set_synced_data(
            tracked_data::copper_golem::COPPER_GOLEM_STATE,
            VarInt(state as i32),
        );
    }

    #[must_use]
    pub fn get_weather_state(&self) -> WeatherState {
        self.weather_state.load()
    }

    pub fn set_weather_state(&self, state: WeatherState) {
        self.weather_state.store(state);
        self.get_entity().set_synced_data(
            tracked_data::copper_golem::WEATHER_STATE,
            VarInt(state.ordinal() as i32),
        );
    }

    pub fn set_opened_chest_pos(&self, opened_chest_pos: BlockPos) {
        self.opened_chest_pos.store(Some(opened_chest_pos));
    }

    pub fn clear_opened_chest_pos(&self) {
        self.opened_chest_pos.store(None);
    }

    /// Vanilla `CopperGolem.updateWeathering`, drawing from the level random.
    fn update_weathering(&self, world: &Arc<World>, game_time: i64) {
        let next_weathering_tick = self.next_weathering_tick.load(Ordering::Relaxed);
        if next_weathering_tick == IGNORE_WEATHERING_TICK {
            return;
        }
        let level_random = || {
            i64::from(
                world
                    .random
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .next_inbetween_i32(WEATHERING_TICK_FROM, WEATHERING_TICK_TO),
            )
        };
        if next_weathering_tick == UNSET_WEATHERING_TICK {
            self.next_weathering_tick
                .store(game_time + level_random(), Ordering::Relaxed);
            return;
        }
        let weather_state = self.get_weather_state();
        let is_fully_oxidized = weather_state == WeatherState::Oxidized;
        if game_time >= next_weathering_tick && !is_fully_oxidized {
            let new_state = weather_state.next().unwrap_or(WeatherState::Oxidized);
            let is_new_state_fully_oxidized = new_state == WeatherState::Oxidized;
            self.set_weather_state(new_state);
            self.next_weathering_tick.store(
                if is_new_state_fully_oxidized {
                    0
                } else {
                    next_weathering_tick + level_random()
                },
                Ordering::Relaxed,
            );
        }
        if is_fully_oxidized && self.can_turn_to_statue(world) {
            self.turn_to_statue(world);
        }
    }

    fn can_turn_to_statue(&self, world: &World) -> bool {
        world
            .get_block_state(&self.get_entity().block_pos.load())
            .is_air()
            && world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .next_f32()
                <= TURN_TO_STATUE_CHANCE
    }

    fn turn_to_statue(&self, world: &Arc<World>) {
        let entity = self.get_entity();
        let pos = entity.block_pos.load();
        let statue = &Block::OXIDIZED_COPPER_GOLEM_STATUE;
        let mut props = CopperGolemStatueLikeProperties::default(statue);
        props.copper_golem_pose = CopperGolemPose::from_index(
            entity
                .random()
                .next_int_range(0, CopperGolemPose::variant_count() as i32) as u16,
        );
        props.facing = entity.get_horizontal_facing();
        world.set_block_state(&pos, props.to_state_id(statue), BlockFlags::NOTIFY_ALL);
        let Some(block_entity) = world.get_block_entity(&pos) else {
            return;
        };
        let Some(statue_entity) = block_entity
            .as_any()
            .downcast_ref::<CopperGolemStatueBlockEntity>()
        else {
            return;
        };
        // Vanilla `CopperGolemStatueBlockEntity.createStatue`.
        *statue_entity
            .custom_name
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = (**entity.custom_name.load())
            .clone()
            .map(TextComponent::get_text);
        world.update_block_entity(&block_entity);
        self.mob_entity.drop_preserved_equipment();
        entity.remove();
        self.play_sound_at_self(Sound::EntityCopperGolemBecomeStatue, 1.0, 1.0);
        if entity.is_leashed() {
            // `dropLeash()` is (true, true), `removeLeash()` (true, false).
            let drop_lead = world.level_info.load().game_rules.entity_drops;
            crate::entity::leashable::drop_leash(self, true, drop_lead);
        }
    }

    /// Vanilla `CopperGolem.spawn`.
    pub fn spawn(&self, weather_state: WeatherState) {
        self.set_weather_state(weather_state);
        self.play_spawn_sound();
    }

    /// Vanilla `CopperGolem.playSpawnSound`.
    pub fn play_spawn_sound(&self) {
        self.play_sound_at_self(Sound::EntityCopperGolemSpawn, 1.0, 1.0);
    }

    /// Vanilla `CopperGolemOxidationLevels.getOxidationLevel(state).stepSound()`.
    const fn step_sound(weather_state: WeatherState) -> Sound {
        match weather_state {
            WeatherState::Unaffected | WeatherState::Exposed => Sound::EntityCopperGolemStep,
            WeatherState::Weathered => Sound::EntityCopperGolemWeatheredStep,
            WeatherState::Oxidized => Sound::EntityCopperGolemOxidizedStep,
        }
    }

    /// Vanilla `CopperGolem.hasContainerOpen`.
    #[must_use]
    pub fn has_container_open(&self, world: &World, block_pos: &BlockPos) -> bool {
        let Some(opened_chest_pos) = self.opened_chest_pos.load() else {
            return false;
        };
        if opened_chest_pos == *block_pos {
            return true;
        }
        let (block, state_id) = world.get_block_and_state_id(&opened_chest_pos);
        if !chests::is_chest_block(block) {
            return false;
        }
        let props = ChestLikeProperties::from_state_id(state_id);
        props.r#type != ChestType::Single
            && chests::get_connected_block_pos(&opened_chest_pos, &props) == *block_pos
    }
}

impl CustomSound for CopperGolemEntity {
    fn hurt_sound(&self) -> Option<Sound> {
        Some(match self.get_weather_state() {
            WeatherState::Unaffected | WeatherState::Exposed => Sound::EntityCopperGolemHurt,
            WeatherState::Weathered => Sound::EntityCopperGolemWeatheredHurt,
            WeatherState::Oxidized => Sound::EntityCopperGolemOxidizedHurt,
        })
    }

    fn death_sound(&self) -> Option<Sound> {
        Some(match self.get_weather_state() {
            WeatherState::Unaffected | WeatherState::Exposed => Sound::EntityCopperGolemDeath,
            WeatherState::Weathered => Sound::EntityCopperGolemWeatheredDeath,
            WeatherState::Oxidized => Sound::EntityCopperGolemOxidizedDeath,
        })
    }
}

impl CopperGolemEntity {
    /// Vanilla `CopperGolem.readyForShearing`: a poppy on the antenna (the saddle slot).
    #[must_use]
    pub fn ready_for_shearing(&self) -> bool {
        self.mob_entity
            .get_item_slot(&pumpkin_data::data_component_impl::EquipmentSlot::SADDLE)
            .item
            .has_tag(&tag::Item::MINECRAFT_SHEARABLE_FROM_COPPER_GOLEM)
    }

    /// Vanilla `CopperGolem.shear`: the snip from the golem, and the antenna item off and
    /// dropped 1.5 blocks up.
    pub fn shear(&self, source: SoundCategory) {
        use pumpkin_data::data_component_impl::EquipmentSlot;
        let entity = self.get_entity();
        let world = entity.world.load_full();
        world.play_sound_from_entity(entity, Sound::EntityCopperGolemShear, source, 1.0, 1.0);
        let antenna = self.mob_entity.get_item_slot(&EquipmentSlot::SADDLE);
        self.set_item_slot_and_notify(&EquipmentSlot::SADDLE, ItemStack::EMPTY.clone());
        if !antenna.is_empty() {
            world.spawn_item_at(entity.pos.load() + Vector3::new(0.0, 1.5, 0.0), antenna);
        }
    }
}

impl Mob for CopperGolemEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn make_brain(&self, packed: &PackedMemories) -> Brain {
        copper_golem_ai::COPPER_GOLEM_PROVIDER.make_brain(self, packed)
    }

    /// The `CopperGolemAi.updateActivity` part of vanilla `CopperGolem.customServerAiStep`.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        copper_golem_ai::update_activity(tick);
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
    }

    /// The server half of vanilla `CopperGolem.tick`, after `super.tick()`.
    fn post_tick(&self) {
        let world = self.get_entity().world.load_full();
        self.update_weathering(&world, world.get_world_age());
    }

    /// Vanilla `AbstractGolem.removeWhenFarAway`.
    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        false
    }

    /// Vanilla `CopperGolem.playStepSound`.
    fn play_step_sound(
        &self,
        caller: &dyn EntityBase,
        _pos: &BlockPos,
        _block: &'static Block,
        _state: &'static pumpkin_data::BlockState,
    ) {
        caller.play_sound_at_self(Self::step_sound(self.get_weather_state()), 1.0, 1.0);
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    /// Vanilla `CopperGolem.actuallyHurt`.
    fn actually_hurt(
        &self,
        _damage_type: DamageType,
        _source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) {
        self.set_state(CopperGolemState::Idle);
    }

    /// Vanilla `CopperGolem.dropEquipment`, which `dropAllDeathLoot` runs after the
    /// equipment's chance drops.
    fn drop_custom_death_loot(&self) {
        self.mob_entity.drop_preserved_equipment();
    }

    /// Vanilla `CopperGolem.finalizeSpawn`.
    fn finalize_spawn(
        &self,
        _world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        self.play_spawn_sound();
        self.mob_entity.finalize_spawn_base();
        group_data
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_long(
            "next_weather_age",
            self.next_weathering_tick.load(Ordering::Relaxed),
        );
        nbt.put_string(
            "weather_state",
            self.get_weather_state().as_str().to_owned(),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.next_weathering_tick.store(
            nbt.get_long("next_weather_age")
                .unwrap_or(UNSET_WEATHERING_TICK),
            Ordering::Relaxed,
        );
        self.set_weather_state(
            nbt.get_string("weather_state")
                .and_then(WeatherState::from_name)
                .unwrap_or(WeatherState::Unaffected),
        );
    }

    /// Vanilla `CopperGolem.thunderHit`.
    fn mob_on_lightning_strike(&self, caller: &dyn EntityBase, lightning: &LightningBoltEntity) {
        self.mob_entity
            .living_entity
            .on_lightning_strike(caller, lightning);
        let lightning_bolt_uuid = lightning.get_entity().entity_uuid;
        if self.last_lightning_bolt_uuid.load() == Some(lightning_bolt_uuid) {
            return;
        }
        self.last_lightning_bolt_uuid
            .store(Some(lightning_bolt_uuid));
        if let Some(previous) = self.get_weather_state().previous() {
            self.next_weathering_tick
                .store(UNSET_WEATHERING_TICK, Ordering::Relaxed);
            self.set_weather_state(previous);
        }
    }

    fn mob_init_data_tracker(&self) {
        self.set_weather_state(self.get_weather_state());
        self.set_state(self.get_state());
    }

    /// Vanilla `CopperGolem.mobInteract`.
    fn mob_interact(
        &self,
        player: &Arc<Player>,
        hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        use pumpkin_data::data_component_impl::EquipmentSlot;
        use pumpkin_data::world::WorldEvent;
        let entity = self.get_entity();
        let world = entity.world.load();

        // An empty hand takes the item the golem carries, thrown towards the player.
        if item_stack.is_empty() {
            let carried = self.mob_entity.get_item_slot(&EquipmentSlot::MAIN_HAND);
            if !carried.is_empty() {
                crate::entity::ai::brain::behavior::utils::throw_item(
                    entity,
                    carried,
                    player.get_entity().pos.load(),
                    // `BehaviorUtils.throwItem`'s `new Vec3(0.3F, 0.3F, 0.3F)`.
                    Vector3::new(f64::from(0.3f32), f64::from(0.3f32), f64::from(0.3f32)),
                    0.3,
                );
                // `setItemInHand`, through `onEquipItem`.
                self.set_item_slot_and_notify(&EquipmentSlot::MAIN_HAND, ItemStack::EMPTY.clone());
                return InteractionResult::SUCCESS;
            }
        }

        if item_stack.item == &Item::SHEARS && self.ready_for_shearing() {
            if crate::entity::mob::shear_event_cancelled(player, entity.entity_id, hand) {
                return InteractionResult::PASS;
            }
            self.shear(SoundCategory::Players);
            world.game_event_from(
                Some(player.as_ref()),
                pumpkin_data::game_event::GameEvent::Shear,
                entity.pos.load(),
            );
            player.hurt_and_break_in_hand(item_stack, hand, 1);
            return InteractionResult::SUCCESS;
        }

        let block_pos = entity.block_pos.load();
        if item_stack.item.id == Item::HONEYCOMB.id
            && self.next_weathering_tick.load(Ordering::Relaxed) != -2
        {
            world.sync_world_event(WorldEvent::ParticlesWaxOn, block_pos, 0);
            world.play_sound(
                Sound::ItemHoneycombWaxOn,
                SoundCategory::Blocks,
                &block_pos.to_centered_f64(),
            );
            self.next_weathering_tick.store(-2, Ordering::Relaxed);
            // `usePlayerItem`.
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            return InteractionResult::SUCCESS_SERVER;
        }

        if item_stack.item.has_tag(&tag::Item::MINECRAFT_AXES) {
            let event = if self.next_weathering_tick.load(Ordering::Relaxed) == -2 {
                Some(WorldEvent::ParticlesWaxOff)
            } else if self.get_weather_state() == WeatherState::Unaffected {
                None
            } else {
                Some(WorldEvent::ParticlesScrape)
            };
            if let Some(event) = event {
                world.play_sound(
                    Sound::ItemAxeScrape,
                    self.sound_source(),
                    &entity.pos.load(),
                );
                world.sync_world_event(event, block_pos, 0);
                self.next_weathering_tick.store(-1, Ordering::Relaxed);
                if event == WorldEvent::ParticlesScrape
                    && let Some(previous) = self.get_weather_state().previous()
                {
                    self.set_weather_state(previous);
                }
                player.hurt_and_break_in_hand(item_stack, hand, 1);
                return InteractionResult::SUCCESS_SERVER;
            }
        }

        InteractionResult::PASS
    }
}
