use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use pumpkin_util::math::vector3::Vector3;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicI32, AtomicI64, Ordering},
};

use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;

use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal,
        wander_around::WanderAroundGoal,
    },
    custom_sound::CustomSound,
    mob::{Mob, MobEntity},
    player::Player,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum WeatherState {
    #[default]
    Unaffected = 0,
    Exposed = 1,
    Weathered = 2,
    Oxidized = 3,
}

impl WeatherState {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Exposed,
            2 => Self::Weathered,
            3 => Self::Oxidized,
            _ => Self::Unaffected,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Unaffected => Self::Exposed,
            Self::Exposed => Self::Weathered,
            Self::Weathered | Self::Oxidized => Self::Oxidized,
        }
    }

    #[must_use]
    pub const fn previous(self) -> Self {
        match self {
            Self::Oxidized => Self::Weathered,
            Self::Weathered => Self::Exposed,
            Self::Exposed | Self::Unaffected => Self::Unaffected,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum CopperGolemState {
    #[default]
    Idle = 0,
    GettingItem = 1,
    GettingNoItem = 2,
    DroppingItem = 3,
    DroppingNoItem = 4,
}

impl CopperGolemState {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::GettingItem,
            2 => Self::GettingNoItem,
            3 => Self::DroppingItem,
            4 => Self::DroppingNoItem,
            _ => Self::Idle,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }
}

/// Represents a Copper Golem, a passive creation mob made of copper blocks and lightning rod.
///
/// Wiki: <https://minecraft.wiki/w/Copper_Golem>
pub struct CopperGolemEntity {
    pub mob_entity: MobEntity,
    pub weather_state: AtomicI32,
    pub state: AtomicI32,
    pub next_weathering_tick: AtomicI64,
}

impl CopperGolemEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `CopperGolem`'s constructor.
        mob_entity.set_required_path_length(48.0);
        mob_entity.set_pathfinding_malus(PathType::DangerFire, 16.0);
        mob_entity.set_pathfinding_malus(PathType::DangerOther, 16.0);
        mob_entity.set_pathfinding_malus(PathType::DamageFire, -1.0);
        let golem = Self {
            mob_entity,
            weather_state: AtomicI32::new(WeatherState::Unaffected.id()),
            state: AtomicI32::new(CopperGolemState::Idle.id()),
            next_weathering_tick: AtomicI64::new(-1),
        };
        let mob_arc = Arc::new(golem);
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

            goal_selector.add_goal(
                1,
                Box::new(WanderAroundGoal::new(1.0).ignoring_no_action_time()),
            );
            goal_selector.add_goal(
                2,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(3, Box::new(RandomLookAroundGoal::default()));
        };

        mob_arc
    }

    #[must_use]
    pub fn get_weather_state(&self) -> WeatherState {
        WeatherState::from_id(self.weather_state.load(Ordering::Relaxed))
    }

    pub fn set_weather_state(&self, state: WeatherState) {
        self.weather_state.store(state.id(), Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::copper_golem::WEATHER_STATE,
            VarInt(state.id()),
        );
    }

    #[must_use]
    pub fn get_state(&self) -> CopperGolemState {
        CopperGolemState::from_id(self.state.load(Ordering::Relaxed))
    }

    pub fn set_state(&self, state: CopperGolemState) {
        self.state.store(state.id(), Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::copper_golem::COPPER_GOLEM_STATE,
            VarInt(state.id()),
        );
    }

    #[must_use]
    pub fn step_sound(&self) -> Sound {
        match self.get_weather_state() {
            WeatherState::Unaffected | WeatherState::Exposed => Sound::EntityCopperGolemStep,
            WeatherState::Weathered => Sound::EntityCopperGolemWeatheredStep,
            WeatherState::Oxidized => Sound::EntityCopperGolemOxidizedStep,
        }
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
    /// Vanilla `CopperGolem` runs on a brain and leaves its goal selector empty, so
    /// `Mob.updateControlFlags` gates nothing; the goals standing in for that brain here must
    /// not be gated either.
    fn update_control_flags(&self) {}

    /// Vanilla `CopperGolem.playStepSound`.
    fn play_step_sound(
        &self,
        caller: &dyn crate::entity::EntityBase,
        _pos: &pumpkin_util::math::position::BlockPos,
        _block: &'static pumpkin_data::Block,
        _state: &'static pumpkin_data::BlockState,
    ) {
        caller.play_sound_at_self(self.step_sound(), 1.0, 1.0);
    }

    fn as_custom_sound(&self) -> Option<&dyn crate::entity::custom_sound::CustomSound> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_long(
            "next_weather_age",
            self.next_weathering_tick.load(Ordering::Relaxed),
        );
        nbt.put_int("weather_state", self.get_weather_state().id());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if let Some(next) = nbt.get_long("next_weather_age") {
            self.next_weathering_tick.store(next, Ordering::Relaxed);
        }
        if let Some(state) = nbt.get_int("weather_state") {
            self.set_weather_state(WeatherState::from_id(state));
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_on_lightning_strike(
        &self,
        caller: &dyn EntityBase,
        lightning: &crate::entity::lightning::LightningBoltEntity,
    ) {
        self.set_weather_state(WeatherState::Unaffected);
        self.mob_entity
            .living_entity
            .on_lightning_strike(caller, lightning);
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::copper_golem::WEATHER_STATE,
            VarInt(self.get_weather_state().id()),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::copper_golem::COPPER_GOLEM_STATE,
            VarInt(self.get_state().id()),
        );
    }

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
                if event == WorldEvent::ParticlesScrape {
                    self.set_weather_state(self.get_weather_state().previous());
                }
                player.hurt_and_break_in_hand(item_stack, hand, 1);
                return InteractionResult::SUCCESS_SERVER;
            }
        }

        InteractionResult::PASS
    }
}
