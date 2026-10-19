use std::sync::{
    Arc, PoisonError,
    atomic::{AtomicI32, Ordering},
};

use pumpkin_data::entity::{EntityPose, EntityStatus};
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::brain::{
        Brain, BrainTick,
        behavior::utils::{global_pos_in, is_alive},
        memory::{GlobalPos, PackedMemories, types},
    },
    ai::control::move_control::MoveControl,
    ai::goal::breed::BreedGoal,
    ai::pathfinder::{node::PathType, node_evaluator::MobData},
    ai::util::land_random_pos,
    mob::{Mob, MobEntity},
    passive::animal::{Animal, animal_can_mate, set_animal_pathfinding_malus},
    player::Player,
};

use super::sniffer_ai;

/// Vanilla `Sniffer.SNIFFER_BABY_START_AGE`.
const SNIFFER_BABY_START_AGE: i32 = -48000;
/// Vanilla `Sniffer.DIGGING_DROP_SEED_OFFSET_TICKS`.
const DIGGING_DROP_SEED_OFFSET_TICKS: i32 = 120;
/// Vanilla `Sniffer.getMaxHeadYRot`.
const MAX_HEAD_Y_ROT: f32 = 50.0;
/// Vanilla `BuiltInLootTables.SNIFFER_DIGGING`.
const SNIFFER_DIGGING: &str = "minecraft:gameplay/sniffer_digging";
/// How many explored positions `storeExploredPosition` keeps before adding the new one.
const EXPLORED_POSITIONS_KEPT: usize = 20;
/// Vanilla `Sniffer.calculateDigPosition`'s number of tries.
const DIG_POSITION_TRIES: i32 = 5;
/// Vanilla `Sniffer.getHeadPosition`'s reach in front of the body.
const HEAD_DISTANCE: f64 = 2.25;
/// Ticks between the digging game events in vanilla `Sniffer.emitDiggingParticles`.
const DIGGING_EVENT_INTERVAL: i32 = 10;

/// Vanilla `Sniffer.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum SnifferState {
    #[default]
    Idling = 0,
    FeelingHappy = 1,
    Scenting = 2,
    Sniffing = 3,
    Searching = 4,
    Digging = 5,
    Rising = 6,
}

impl SnifferState {
    /// Vanilla `BY_ID`, which reads an unknown id as `IDLING`.
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::FeelingHappy,
            2 => Self::Scenting,
            3 => Self::Sniffing,
            4 => Self::Searching,
            5 => Self::Digging,
            6 => Self::Rising,
            _ => Self::Idling,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }

    /// The states vanilla `Sniffer.canMate` accepts on both partners.
    const fn can_mate(self) -> bool {
        matches!(self, Self::Idling | Self::Scenting | Self::FeelingHappy)
    }
}

/// Represents a Sniffer, a large passive mob that digs up ancient seeds.
///
/// Wiki: <https://minecraft.wiki/w/Sniffer>
pub struct SnifferEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    state: AtomicI32,
    drop_seed_at_tick: AtomicI32,
}

impl SnifferEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_can_float(true);
        mob_entity.set_pathfinding_malus(PathType::Water, -1.0);
        mob_entity.set_pathfinding_malus(PathType::DangerPowderSnow, -1.0);
        mob_entity.set_pathfinding_malus(PathType::DamageCautious, -1.0);
        let sniffer = Arc::new(Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            state: AtomicI32::new(SnifferState::Idling.id()),
            drop_seed_at_tick: AtomicI32::new(0),
        });
        sniffer.mob_entity.init_brain(sniffer.as_ref());
        sniffer
    }

    #[must_use]
    pub fn get_state(&self) -> SnifferState {
        SnifferState::from_id(self.state.load(Ordering::Relaxed))
    }

    /// Vanilla `Sniffer.setState`, with `onSyncedDataUpdated`: only a real change resizes.
    fn set_state(&self, state: SnifferState) -> &Self {
        let previous = self.state.swap(state.id(), Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::sniffer::DATA_STATE,
            VarInt(state.id()),
        );
        if previous != state.id() {
            self.refresh_dimensions();
        }
        self
    }

    /// Vanilla `Sniffer.isTempted`.
    #[must_use]
    pub fn is_tempted(brain: &Brain) -> bool {
        brain.has_memory_value(types::TEMPTING_PLAYER.id())
    }

    /// Vanilla `Sniffer.canSniff`.
    #[must_use]
    pub fn can_sniff(&self, brain: &Brain) -> bool {
        let entity = self.get_entity();
        !Self::is_tempted(brain)
            && !brain.has_memory_value(types::IS_PANICKING.id())
            && !entity.is_in_water()
            && !self.mob_entity.is_in_love()
            && entity.on_ground.load(Ordering::Relaxed)
            && !entity.has_vehicle()
            && !entity.is_leashed()
    }

    /// Vanilla `Sniffer.getHeadBlock`.
    fn get_head_block(&self) -> BlockPos {
        let position = self.get_head_position();
        BlockPos::floored(
            position.x,
            self.get_entity().pos.load().y + f64::from(0.2f32),
            position.z,
        )
    }

    /// Vanilla `Sniffer.getHeadPosition`.
    fn get_head_position(&self) -> Vector3<f64> {
        let entity = self.get_entity();
        entity.pos.load() + entity.get_forward() * HEAD_DISTANCE
    }

    /// Vanilla `Sniffer.transitionTo`.
    pub fn transition_to(&self, state: SnifferState) -> &Self {
        match state {
            SnifferState::Idling => {
                self.set_state(SnifferState::Idling);
            }
            SnifferState::FeelingHappy => {
                self.play_sound_at_self(Sound::EntitySnifferHappy, 1.0, 1.0);
                self.set_state(SnifferState::FeelingHappy);
            }
            SnifferState::Scenting => {
                self.set_state(SnifferState::Scenting).on_scenting_start();
            }
            SnifferState::Sniffing => {
                self.play_sound_at_self(Sound::EntitySnifferSniffing, 1.0, 1.0);
                self.set_state(SnifferState::Sniffing);
            }
            SnifferState::Searching => {
                self.set_state(SnifferState::Searching);
            }
            SnifferState::Digging => {
                self.set_state(SnifferState::Digging).on_digging_start();
            }
            SnifferState::Rising => {
                self.play_sound_at_self(Sound::EntitySnifferDiggingStop, 1.0, 1.0);
                self.set_state(SnifferState::Rising);
            }
        }
        self
    }

    /// Vanilla `Sniffer.onScentingStart`.
    fn on_scenting_start(&self) {
        let pitch = if self.is_baby() { 1.3 } else { 1.0 };
        self.play_sound_at_self(Sound::EntitySnifferScenting, 1.0, pitch);
    }

    /// Vanilla `Sniffer.onDiggingStart`.
    fn on_digging_start(&self) {
        let entity = self.get_entity();
        let drop_seed_at_tick =
            entity.tick_count.load(Ordering::Relaxed) + DIGGING_DROP_SEED_OFFSET_TICKS;
        self.drop_seed_at_tick
            .store(drop_seed_at_tick, Ordering::Relaxed);
        entity.set_synced_data(
            pumpkin_data::tracked_data::sniffer::DATA_DROP_SEED_AT_TICK,
            VarInt(drop_seed_at_tick),
        );
        entity
            .world
            .load()
            .send_entity_status(entity, EntityStatus::SnifferDiggingSound, None);
    }

    /// Vanilla `Sniffer.onDiggingComplete`.
    pub fn on_digging_complete(&self, brain: &mut Brain, success: bool) {
        if success {
            self.store_explored_position(brain, self.get_entity().get_on_pos());
        }
    }

    /// Vanilla `Sniffer.calculateDigPosition`.
    #[must_use]
    pub fn calculate_dig_position(&self, brain: &Brain) -> Option<BlockPos> {
        let world = self.get_entity().world.load();
        (0..DIG_POSITION_TRIES)
            .filter_map(|idx| land_random_pos::get_pos(self, 10 + 2 * idx, 3))
            .map(BlockPos::floored_v)
            .filter(|position| {
                world
                    .worldborder
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .contains(f64::from(position.0.x), f64::from(position.0.z))
            })
            .map(|position| position.down())
            .find(|position| self.can_dig_at(brain, *position))
    }

    /// Vanilla `Sniffer.canDig()`.
    #[must_use]
    pub fn can_dig(&self, brain: &Brain) -> bool {
        let entity = self.get_entity();
        !brain.has_memory_value(types::IS_PANICKING.id())
            && !Self::is_tempted(brain)
            && !self.is_baby()
            && !entity.is_in_water()
            && entity.on_ground.load(Ordering::Relaxed)
            && !entity.has_vehicle()
            && self.can_dig_at(brain, self.get_head_block().down())
    }

    /// Vanilla `Sniffer.canDig(BlockPos)`.
    fn can_dig_at(&self, brain: &Brain, position: BlockPos) -> bool {
        let entity = self.get_entity();
        let world = entity.world.load();
        if !world
            .get_block(&position)
            .has_tag(&tag::Block::MINECRAFT_SNIFFER_DIGGABLE_BLOCK)
        {
            return false;
        }
        let global_position = global_pos_in(&world, position);
        if Self::get_explored_positions(brain).any(|explored| Some(*explored) == global_position) {
            return false;
        }
        self.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .create_path(&self.mob_entity.living_entity, position.to_f64(), 1)
            .is_some_and(|path| path.can_reach())
    }

    /// Vanilla `Sniffer.dropSeed`.
    fn drop_seed(&self) {
        let entity = self.get_entity();
        if self.drop_seed_at_tick.load(Ordering::Relaxed)
            != entity.tick_count.load(Ordering::Relaxed)
        {
            return;
        }
        let world = entity.world.load_full();
        let head = self.get_head_block().to_f64();
        self.mob_entity
            .living_entity
            .drop_from_gift_loot_table_with(SNIFFER_DIGGING, |stack| {
                world.spawn_item_at(head, stack);
            });
        self.play_sound_at_self(Sound::EntitySnifferDropSeed, 1.0, 1.0);
    }

    /// Vanilla `Sniffer.emitDiggingParticles`. The particles and their hit sounds are
    /// client-side; the server only raises the game event.
    fn emit_digging_particles(&self) -> &Self {
        let entity = self.get_entity();
        if entity.tick_count.load(Ordering::Relaxed) % DIGGING_EVENT_INTERVAL == 0 {
            entity.world.load().game_event(
                GameEvent::EntityAction,
                self.get_head_block().to_centered_f64(),
                crate::world::game_event::GameEventContext::of_entity(self),
            );
        }
        self
    }

    /// Vanilla `Sniffer.storeExploredPosition`.
    fn store_explored_position(&self, brain: &mut Brain, position: BlockPos) {
        let Some(global_position) = global_pos_in(&self.get_entity().world.load(), position) else {
            return;
        };
        let mut updated: Vec<GlobalPos> = Self::get_explored_positions(brain)
            .take(EXPLORED_POSITIONS_KEPT)
            .copied()
            .collect();
        updated.insert(0, global_position);
        brain.set(types::SNIFFER_EXPLORED_POSITIONS, updated);
    }

    /// Vanilla `Sniffer.getExploredPositions`.
    fn get_explored_positions(brain: &Brain) -> impl Iterator<Item = &GlobalPos> {
        brain
            .get(types::SNIFFER_EXPLORED_POSITIONS)
            .into_iter()
            .flatten()
    }

    /// The speed vanilla `Sniffer.jumpFromGround` reads off the move control.
    fn move_speed_modifier(&self) -> f64 {
        let guard = self
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let control: &dyn std::any::Any = &**guard;
        control
            .downcast_ref::<MoveControl>()
            .map_or(0.0, MoveControl::get_speed_modifier)
    }
}

impl AgeableMob for SnifferEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }

    fn get_baby_start_age(&self) -> i32 {
        SNIFFER_BABY_START_AGE
    }
}

impl Animal for SnifferEntity {
    /// Vanilla `Sniffer.isFood`.
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_SNIFFER_FOOD)
    }

    /// Vanilla `Sniffer.canMate`.
    fn can_mate(&self, partner: &dyn Mob) -> bool {
        partner
            .cast_any()
            .downcast_ref::<Self>()
            .is_some_and(|partner_sniffer| {
                self.get_state().can_mate()
                    && partner_sniffer.get_state().can_mate()
                    && animal_can_mate(&self.mob_entity, partner)
            })
    }

    /// Vanilla `Sniffer.spawnChildFromBreeding`: the sniffer lays an egg item instead.
    fn spawn_child_from_breeding(&self, tick: &mut BrainTick<'_>, partner: &dyn EntityBase) {
        let position = self.get_entity().pos.load();
        BreedGoal::finalize_spawn_child_from_breeding(tick.mob, partner);
        let mut random = self.get_random();
        let pitch = (random.random::<f32>() - random.random::<f32>()) * 0.2 + 0.5;
        self.play_sound_at_self(Sound::BlockSnifferEggPlop, 1.0, pitch);
        tick.world
            .spawn_item_at(position, ItemStack::new(1, &Item::SNIFFER_EGG));
    }

    /// Vanilla `Sniffer.playEatingSound`.
    fn play_eating_sound(&self, _sound: Sound) {
        let entity = self.get_entity();
        // Vanilla `Mth.randomBetween(random, 0.8F, 1.2F)`.
        let pitch = self.get_random().random::<f32>() * (1.2 - 0.8) + 0.8;
        entity.world.load().play_sound_fine(
            Sound::EntitySnifferEat,
            SoundCategory::Neutral,
            &entity.pos.load(),
            1.0,
            pitch,
        );
    }
}

/// Vanilla `Sniffer.DIGGING_DIMENSIONS`.
const DIGGING_DIMENSIONS: EntityDimensions =
    pumpkin_data::entity_dimensions::SNIFFER_DIGGING_DIMENSIONS;

impl Mob for SnifferEntity {
    /// Vanilla `Sniffer.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        (!matches!(
            self.get_state(),
            SnifferState::Digging | SnifferState::Searching
        ))
        .then_some(Sound::EntitySnifferIdle)
    }

    /// Vanilla `Sniffer.getDefaultDimensions`: lower while digging.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        if self.get_state() == SnifferState::Digging {
            DIGGING_DIMENSIONS.scale(self.age_scale())
        } else {
            self.get_entity()
                .get_dimensions(pose)
                .scale(self.age_scale())
        }
    }

    /// Vanilla `Sniffer.getMaxHeadYRot`.
    fn get_max_head_rotation(&self) -> f32 {
        MAX_HEAD_Y_ROT
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn make_brain(&self, packed: &PackedMemories) -> Brain {
        sniffer_ai::SNIFFER_PROVIDER.make_brain(self, packed)
    }

    /// Vanilla `Sniffer.customServerAiStep`'s `SnifferAi.updateActivity`.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        sniffer_ai::update_activity(tick);
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
    }

    /// Vanilla `AgeableMob.aiStep`, the end of `Sniffer`'s `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        if self.get_entity().is_alive() {
            self.ageable_ai_step();
        }
    }

    /// Vanilla `Sniffer.tick`, before `super.tick()`. Searching only makes a client-side
    /// sound.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        if self.get_state() == SnifferState::Digging {
            self.emit_digging_particles().drop_seed();
        }
    }

    /// Vanilla `Sniffer.jumpFromGround`: a jump from standstill gets a push forwards.
    fn jump_from_ground(&self) {
        let living = &self.mob_entity.living_entity;
        living.jump();
        if self.move_speed_modifier() > 0.0 {
            let entity = &living.entity;
            let current = entity.velocity.load().horizontal_length_squared();
            if current < 0.01 {
                entity.update_velocity_from_input(Vector3::new(0.0, 0.0, 1.0), f64::from(0.1f32));
            }
        }
    }

    /// Vanilla `Sniffer.die`.
    fn before_die(&self) {
        self.transition_to(SnifferState::Idling);
    }

    /// Vanilla `Sniffer.onPathfindingStart`: water is only open while burning or swimming.
    fn on_pathfinding_start(&self, mob_data: &mut MobData) {
        let entity = self.get_entity();
        if entity.is_on_fire() || entity.is_in_water() {
            mob_data.set_pathfinding_malus(PathType::Water, 0.0);
        }
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        if entity.is_baby() {
            entity.set_synced_data(pumpkin_data::tracked_data::sniffer::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::sniffer::DATA_STATE,
            VarInt(self.get_state().id()),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::sniffer::DATA_DROP_SEED_AT_TICK,
            VarInt(self.drop_seed_at_tick.load(Ordering::Relaxed)),
        );
    }

    /// Vanilla `Sniffer.mobInteract`: food is eaten loudly a second time.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let is_food = self.is_food(item_stack);
        let consumed = self.animal_interact(player, item_stack, Sound::EntitySnifferEat);
        if consumed && is_food {
            self.play_eating_sound(Sound::EntitySnifferEat);
        }
        consumed
    }
}
