//! Vanilla `net.minecraft.world.level.gameevent.vibrations`: `VibrationSystem` (its `Data`,
//! `Listener`, `Ticker` and `User`), `VibrationInfo` and `VibrationSelector`.

use std::sync::{Arc, Mutex, PoisonError, Weak};

use crate::world::ParticleRandomization;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::particle::Particle;
use pumpkin_data::tag::{self, Tag, Taggable};
use pumpkin_data::{Block, BlockDirection};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::{VarInt, ser::NetworkWriteExt};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::math::vector3::Vector3;
use uuid::Uuid;

use super::{GameEventContext, GameEventListener, PositionSource};
use crate::entity::EntityBase;
use crate::entity::player::advancement::trigger::AdvancementTrigger;
use crate::world::World;

/// Vanilla `VibrationSystem.RESONANCE_EVENTS`.
pub const RESONANCE_EVENTS: [GameEvent; 15] = [
    GameEvent::Resonate1,
    GameEvent::Resonate2,
    GameEvent::Resonate3,
    GameEvent::Resonate4,
    GameEvent::Resonate5,
    GameEvent::Resonate6,
    GameEvent::Resonate7,
    GameEvent::Resonate8,
    GameEvent::Resonate9,
    GameEvent::Resonate10,
    GameEvent::Resonate11,
    GameEvent::Resonate12,
    GameEvent::Resonate13,
    GameEvent::Resonate14,
    GameEvent::Resonate15,
];

/// Vanilla `VibrationSystem.NO_VIBRATION_FREQUENCY`.
pub const NO_VIBRATION_FREQUENCY: i32 = 0;

/// Vanilla `VibrationSystem.getGameEventFrequency` (`VIBRATION_FREQUENCY_FOR_EVENT`).
#[must_use]
pub const fn get_game_event_frequency(event: GameEvent) -> i32 {
    match event {
        GameEvent::Step | GameEvent::Swim | GameEvent::Flap | GameEvent::Resonate1 => 1,
        GameEvent::ProjectileLand
        | GameEvent::HitGround
        | GameEvent::Splash
        | GameEvent::Bounce
        | GameEvent::Resonate2 => 2,
        GameEvent::ItemInteractFinish
        | GameEvent::ProjectileShoot
        | GameEvent::InstrumentPlay
        | GameEvent::Resonate3 => 3,
        GameEvent::EntityAction
        | GameEvent::ElytraGlide
        | GameEvent::Unequip
        | GameEvent::Resonate4 => 4,
        GameEvent::EntityDismount | GameEvent::Equip | GameEvent::Resonate5 => 5,
        GameEvent::EntityInteract
        | GameEvent::Shear
        | GameEvent::EntityMount
        | GameEvent::Resonate6 => 6,
        GameEvent::EntityDamage | GameEvent::Resonate7 => 7,
        GameEvent::Drink | GameEvent::Eat | GameEvent::Resonate8 => 8,
        GameEvent::ContainerClose
        | GameEvent::BlockClose
        | GameEvent::BlockDeactivate
        | GameEvent::BlockDetach
        | GameEvent::Resonate9 => 9,
        GameEvent::ContainerOpen
        | GameEvent::BlockOpen
        | GameEvent::BlockActivate
        | GameEvent::BlockAttach
        | GameEvent::PrimeFuse
        | GameEvent::NoteBlockPlay
        | GameEvent::Resonate10 => 10,
        GameEvent::BlockChange | GameEvent::Resonate11 => 11,
        GameEvent::BlockDestroy | GameEvent::FluidPickup | GameEvent::Resonate12 => 12,
        GameEvent::BlockPlace | GameEvent::FluidPlace | GameEvent::Resonate13 => 13,
        GameEvent::EntityPlace
        | GameEvent::LightningStrike
        | GameEvent::Teleport
        | GameEvent::Resonate14 => 14,
        GameEvent::EntityDie | GameEvent::Explode | GameEvent::Resonate15 => 15,
        _ => NO_VIBRATION_FREQUENCY,
    }
}

/// Vanilla `VibrationSystem.getResonanceEventByFrequency`, for a frequency in `1..=15`.
#[must_use]
pub const fn get_resonance_event_by_frequency(vibration_frequency: i32) -> GameEvent {
    RESONANCE_EVENTS[(vibration_frequency - 1) as usize]
}

/// Vanilla `VibrationSystem.getRedstoneStrengthForDistance`.
#[must_use]
pub fn get_redstone_strength_for_distance(distance: f32, listener_radius: i32) -> i32 {
    let power_scale = 15.0 / f64::from(listener_radius);
    1.max(15 - (power_scale * f64::from(distance)).floor() as i32)
}

/// Vanilla `VibrationInfo`. Vanilla also keeps the source entity object; Pumpkin finds it again
/// by UUID when the vibration arrives.
#[derive(Clone, Debug)]
pub struct VibrationInfo {
    pub game_event: GameEvent,
    pub distance: f32,
    pub pos: Vector3<f64>,
    pub uuid: Option<Uuid>,
    pub projectile_owner_uuid: Option<Uuid>,
}

impl VibrationInfo {
    /// Vanilla `new VibrationInfo(gameEvent, distance, pos, entity)`.
    fn new(
        world: &World,
        game_event: GameEvent,
        distance: f32,
        pos: Vector3<f64>,
        entity: Option<&dyn EntityBase>,
    ) -> Self {
        Self {
            game_event,
            distance,
            pos,
            uuid: entity.map(|entity| entity.get_entity().entity_uuid),
            projectile_owner_uuid: entity
                .and_then(EntityBase::get_owner_id)
                .and_then(|owner_id| world.get_entity_by_id(owner_id))
                .map(|owner| owner.get_entity().entity_uuid),
        }
    }

    /// Vanilla `VibrationInfo.getEntity`.
    #[must_use]
    pub fn get_entity(&self, world: &World) -> Option<Arc<dyn EntityBase>> {
        self.uuid.and_then(|uuid| world.get_entity_by_uuid(uuid))
    }

    /// Vanilla `VibrationInfo.getProjectileOwner`.
    #[must_use]
    pub fn get_projectile_owner(&self, world: &World) -> Option<Arc<dyn EntityBase>> {
        self.get_entity(world)
            .and_then(|entity| entity.get_owner_id())
            .and_then(|owner_id| world.get_entity_by_id(owner_id))
            .or_else(|| {
                self.projectile_owner_uuid
                    .and_then(|uuid| world.get_entity_by_uuid(uuid))
            })
    }

    /// Vanilla `VibrationInfo.CODEC` encoding.
    fn write_nbt(&self) -> NbtCompound {
        let mut nbt = NbtCompound::new();
        nbt.put_string(
            "game_event",
            format!("minecraft:{}", self.game_event.name()),
        );
        nbt.put_float("distance", self.distance);
        nbt.put_list(
            "pos",
            vec![
                NbtTag::Double(self.pos.x),
                NbtTag::Double(self.pos.y),
                NbtTag::Double(self.pos.z),
            ],
        );
        if let Some(uuid) = self.uuid {
            nbt.put_uuid("source", uuid);
        }
        if let Some(uuid) = self.projectile_owner_uuid {
            nbt.put_uuid("projectile_owner", uuid);
        }
        nbt
    }

    /// Vanilla `VibrationInfo.CODEC` decoding; `None` where the codec fails.
    fn read_nbt(nbt: &NbtCompound) -> Option<Self> {
        let game_event = GameEvent::from_name(nbt.get_string("game_event")?)?;
        let distance = nbt.get_numeric_float("distance")?;
        if !(0.0..=f32::MAX).contains(&distance) {
            return None;
        }
        let [x, y, z] = nbt.get_list("pos")? else {
            return None;
        };
        Some(Self {
            game_event,
            distance,
            pos: Vector3::new(
                x.extract_double()?,
                y.extract_double()?,
                z.extract_double()?,
            ),
            uuid: nbt.get_uuid("source"),
            projectile_owner_uuid: nbt.get_uuid("projectile_owner"),
        })
    }
}

/// Vanilla `VibrationSelector`: picks the closest vibration of a tick, then the one with the
/// highest frequency.
#[derive(Clone, Debug, Default)]
pub struct VibrationSelector {
    current_vibration_data: Option<(VibrationInfo, i64)>,
}

impl VibrationSelector {
    pub fn add_candidate(&mut self, new_vibration: VibrationInfo, tick_time: i64) {
        if self.should_replace_vibration(&new_vibration, tick_time) {
            self.current_vibration_data = Some((new_vibration, tick_time));
        }
    }

    fn should_replace_vibration(&self, new_vibration: &VibrationInfo, tick_time: i64) -> bool {
        let Some((previous_vibration, previous_tick)) = &self.current_vibration_data else {
            return true;
        };
        if tick_time != *previous_tick {
            return false;
        }
        if new_vibration.distance < previous_vibration.distance {
            return true;
        }
        if new_vibration.distance > previous_vibration.distance {
            return false;
        }
        get_game_event_frequency(new_vibration.game_event)
            > get_game_event_frequency(previous_vibration.game_event)
    }

    #[must_use]
    pub fn chosen_candidate(&self, time: i64) -> Option<VibrationInfo> {
        self.current_vibration_data
            .as_ref()
            .filter(|(_, tick)| *tick < time)
            .map(|(event, _)| event.clone())
    }

    pub const fn start_over(&mut self) {
        self.current_vibration_data = None;
    }

    fn write_nbt(&self) -> NbtCompound {
        let mut nbt = NbtCompound::new();
        if let Some((event, _)) = &self.current_vibration_data {
            nbt.put_compound("event", event.write_nbt());
        }
        nbt.put_long(
            "tick",
            self.current_vibration_data
                .as_ref()
                .map_or(-1, |(_, tick)| *tick),
        );
        nbt
    }

    fn read_nbt(nbt: &NbtCompound) -> Option<Self> {
        let tick = nbt.get_long("tick")?;
        Some(Self {
            current_vibration_data: nbt
                .get_compound("event")
                .and_then(VibrationInfo::read_nbt)
                .map(|event| (event, tick)),
        })
    }
}

/// Vanilla `VibrationSystem.Data`.
#[derive(Debug, Default)]
pub struct VibrationData {
    current_vibration: Option<VibrationInfo>,
    travel_time_in_ticks: i32,
    selection_strategy: VibrationSelector,
    reload_vibration_particle: bool,
}

impl VibrationData {
    /// Vanilla `VibrationSystem.Data.NBT_TAG_KEY`.
    pub const NBT_TAG_KEY: &'static str = "listener";

    #[must_use]
    pub const fn get_current_vibration(&self) -> Option<&VibrationInfo> {
        self.current_vibration.as_ref()
    }

    #[must_use]
    pub const fn get_travel_time_in_ticks(&self) -> i32 {
        self.travel_time_in_ticks
    }

    fn decrement_travel_time(&mut self) {
        self.travel_time_in_ticks = 0.max(self.travel_time_in_ticks - 1);
    }

    /// Vanilla `VibrationSystem.Data.CODEC` encoding.
    #[must_use]
    pub fn write_nbt(&self) -> NbtCompound {
        let mut nbt = NbtCompound::new();
        if let Some(event) = &self.current_vibration {
            nbt.put_compound("event", event.write_nbt());
        }
        nbt.put_compound("selector", self.selection_strategy.write_nbt());
        nbt.put_int("event_delay", self.travel_time_in_ticks);
        nbt
    }

    /// `input.read("listener", VibrationSystem.Data.CODEC).orElseGet(VibrationSystem.Data::new)`.
    #[must_use]
    pub fn read_nbt(parent: &NbtCompound) -> Self {
        parent
            .get_compound(Self::NBT_TAG_KEY)
            .and_then(Self::decode)
            .unwrap_or_default()
    }

    fn decode(nbt: &NbtCompound) -> Option<Self> {
        let selection_strategy = VibrationSelector::read_nbt(nbt.get_compound("selector")?)?;
        let travel_time_in_ticks = match nbt.get("event_delay") {
            Some(tag) => tag.extract_int().filter(|delay| *delay >= 0)?,
            None => 0,
        };
        Some(Self {
            current_vibration: nbt.get_compound("event").and_then(VibrationInfo::read_nbt),
            travel_time_in_ticks,
            selection_strategy,
            reload_vibration_particle: true,
        })
    }
}

/// Vanilla `VibrationSystem.User`.
pub trait VibrationUser: Send + Sync {
    fn get_listener_radius(&self) -> i32;

    fn get_position_source(&self) -> &PositionSource;

    fn can_receive_vibration(
        &self,
        world: &World,
        pos: BlockPos,
        event: GameEvent,
        context: &GameEventContext<'_>,
    ) -> bool;

    fn on_receive_vibration(
        &self,
        world: &Arc<World>,
        pos: BlockPos,
        event: GameEvent,
        source_entity: Option<&dyn EntityBase>,
        projectile_owner: Option<&dyn EntityBase>,
        receiving_distance: f32,
    );

    fn get_listenable_events(&self) -> &'static Tag {
        &tag::GameEvent::MINECRAFT_VIBRATIONS
    }

    fn can_trigger_avoid_vibration(&self) -> bool {
        false
    }

    fn requires_adjacent_chunks_to_be_ticking(&self) -> bool {
        false
    }

    fn calculate_travel_time_in_ticks(&self, distance_to_destination: f32) -> i32 {
        distance_to_destination.floor() as i32
    }

    fn is_valid_vibration(&self, event: GameEvent, context: &GameEventContext<'_>) -> bool {
        if !event.has_tag(self.get_listenable_events()) {
            return false;
        }
        if let Some(source_entity) = context.source_entity {
            if source_entity.is_spectator() {
                return false;
            }
            if source_entity.is_stepping_carefully()
                && event.has_tag(&tag::GameEvent::MINECRAFT_IGNORE_VIBRATIONS_SNEAKING)
            {
                if self.can_trigger_avoid_vibration()
                    && let Some(player) = source_entity.get_player()
                {
                    player.trigger_advancement(AdvancementTrigger::AvoidVibration);
                }
                return false;
            }
            if source_entity.dampens_vibrations() {
                return false;
            }
        }
        context.affected_state.is_none_or(|state| {
            !Block::from_state_id(state.id).has_tag(&tag::Block::MINECRAFT_DAMPENS_VIBRATIONS)
        })
    }

    fn on_data_changed(&self) {}
}

/// Vanilla `VibrationSystem`: what owns a vibration listener.
pub trait VibrationSystem: Send + Sync {
    fn get_vibration_data(&self) -> &Mutex<VibrationData>;

    fn get_vibration_user(&self) -> &dyn VibrationUser;
}

/// Vanilla `VibrationSystem.Listener.handleGameEvent`.
pub fn handle_game_event(
    system: &dyn VibrationSystem,
    world: &World,
    event: GameEvent,
    context: &GameEventContext<'_>,
    source_position: Vector3<f64>,
) -> bool {
    let data = system.get_vibration_data();
    let user = system.get_vibration_user();
    if data
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .current_vibration
        .is_some()
    {
        return false;
    }
    if !user.is_valid_vibration(event, context) {
        return false;
    }
    let Some(destination) = user.get_position_source().get_position() else {
        return false;
    };
    if !user.can_receive_vibration(world, BlockPos::floored_v(source_position), event, context) {
        return false;
    }
    if is_occluded(world, source_position, destination) {
        return false;
    }
    schedule_vibration(world, data, event, context, source_position, destination);
    true
}

/// Vanilla `VibrationSystem.Listener.forceScheduleVibration`.
pub fn force_schedule_vibration(
    system: &dyn VibrationSystem,
    world: &World,
    event: GameEvent,
    context: &GameEventContext<'_>,
    origin: Vector3<f64>,
) {
    if let Some(destination) = system
        .get_vibration_user()
        .get_position_source()
        .get_position()
    {
        schedule_vibration(
            world,
            system.get_vibration_data(),
            event,
            context,
            origin,
            destination,
        );
    }
}

/// Vanilla `VibrationSystem.Listener.scheduleVibration`.
fn schedule_vibration(
    world: &World,
    data: &Mutex<VibrationData>,
    event: GameEvent,
    context: &GameEventContext<'_>,
    origin: Vector3<f64>,
    destination: Vector3<f64>,
) {
    let vibration = VibrationInfo::new(
        world,
        event,
        origin.squared_distance_to_vec(&destination).sqrt() as f32,
        origin,
        context.source_entity,
    );
    let game_time = world.get_world_age();
    data.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .selection_strategy
        .add_candidate(vibration, game_time);
}

/// Vanilla `VibrationSystem.Listener.distanceBetweenInBlocks`.
#[must_use]
pub fn distance_between_in_blocks(origin: BlockPos, destination: BlockPos) -> f32 {
    let dx = f64::from(origin.0.x - destination.0.x);
    let dy = f64::from(origin.0.y - destination.0.y);
    let dz = f64::from(origin.0.z - destination.0.z);
    (dx * dx + dy * dy + dz * dz).sqrt() as f32
}

/// Vanilla `VibrationSystem.Listener.isOccluded`: blocked when every line from a nudge of the
/// origin block's center to the destination block's center crosses an
/// `#occludes_vibration_signals` block.
fn is_occluded(world: &World, origin: Vector3<f64>, destination: Vector3<f64>) -> bool {
    let from = BlockPos::floored_v(origin).to_centered_f64();
    let to = BlockPos::floored_v(destination).to_centered_f64();
    let nudge = f64::from(1.0e-5f32);
    BlockDirection::all().into_iter().all(|direction| {
        let offset = direction.to_offset();
        let nudged_source = from.add_raw(
            f64::from(offset.x) * nudge,
            f64::from(offset.y) * nudge,
            f64::from(offset.z) * nudge,
        );
        pumpkin_world::clip::is_block_in_line(world, nudged_source, to, |state| {
            Block::from_state_id(state.id)
                .has_tag(&tag::Block::MINECRAFT_OCCLUDES_VIBRATION_SIGNALS)
        })
    })
}

/// Vanilla `VibrationSystem.Listener` for a listener owned by something other than a block
/// entity: the allay and the warden hand it to a `DynamicGameEventListener`.
pub struct VibrationListener {
    system: Weak<dyn VibrationSystem>,
    /// The user's position source, which never changes.
    listener_source: PositionSource,
}

impl VibrationListener {
    #[must_use]
    pub const fn new(system: Weak<dyn VibrationSystem>, listener_source: PositionSource) -> Self {
        Self {
            system,
            listener_source,
        }
    }
}

impl GameEventListener for VibrationListener {
    fn get_listener_source(&self) -> &PositionSource {
        &self.listener_source
    }

    fn get_listener_radius(&self) -> i32 {
        self.system.upgrade().map_or(0, |system| {
            system.get_vibration_user().get_listener_radius()
        })
    }

    fn handle_game_event(
        &self,
        world: &World,
        event: GameEvent,
        context: &GameEventContext<'_>,
        source_position: Vector3<f64>,
    ) -> bool {
        self.system.upgrade().is_some_and(|system| {
            handle_game_event(system.as_ref(), world, event, context, source_position)
        })
    }
}

/// `ServerLevel.sendParticles(new VibrationParticleOption(destination, arrivalInTicks), x, y,
/// z, 1, 0, 0, 0, 0)`; whether any player got it.
fn send_vibration_particle(
    world: &World,
    position: Vector3<f64>,
    destination: &PositionSource,
    arrival_in_ticks: i32,
) -> bool {
    let mut data = Vec::new();
    let Some(Ok(())) = destination.write(&mut data) else {
        return false;
    };
    if data.write_var_int(&VarInt(arrival_in_ticks)).is_err() {
        return false;
    }
    world.send_particles(
        Particle::Vibration,
        &data,
        false,
        false,
        position,
        1,
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 0.0),
        ParticleRandomization::Default,
    ) > 0
}

/// Vanilla `VibrationSystem.Ticker.tick`.
pub fn tick(world: &Arc<World>, data: &Mutex<VibrationData>, user: &dyn VibrationUser) {
    let mut guard = data.lock().unwrap_or_else(PoisonError::into_inner);
    if guard.current_vibration.is_none() {
        try_select_and_schedule_vibration(world, &mut guard, user);
    }
    let Some(current_vibration) = guard.current_vibration.clone() else {
        return;
    };
    let mut has_changed = guard.travel_time_in_ticks > 0;
    try_reload_vibration_particle(world, &mut guard, user);
    guard.decrement_travel_time();
    if guard.travel_time_in_ticks <= 0 {
        // The user may post game events, which this listener's own `handleGameEvent` reads.
        drop(guard);
        has_changed = receive_vibration(world, data, user, &current_vibration);
    } else {
        drop(guard);
    }
    if has_changed {
        user.on_data_changed();
    }
}

/// Vanilla `VibrationSystem.Ticker.trySelectAndScheduleVibration`.
fn try_select_and_schedule_vibration(
    world: &World,
    data: &mut VibrationData,
    user: &dyn VibrationUser,
) {
    let Some(vibration) = data
        .selection_strategy
        .chosen_candidate(world.get_world_age())
    else {
        return;
    };
    let origin = vibration.pos;
    data.travel_time_in_ticks = user.calculate_travel_time_in_ticks(vibration.distance);
    data.current_vibration = Some(vibration);
    send_vibration_particle(
        world,
        origin,
        user.get_position_source(),
        data.travel_time_in_ticks,
    );
    user.on_data_changed();
    data.selection_strategy.start_over();
}

/// Vanilla `VibrationSystem.Ticker.tryReloadVibrationParticle`: resends the particle of a
/// vibration loaded mid-flight, from where it would be by now.
fn try_reload_vibration_particle(
    world: &World,
    data: &mut VibrationData,
    user: &dyn VibrationUser,
) {
    if !data.reload_vibration_particle {
        return;
    }
    let Some(current_vibration) = &data.current_vibration else {
        data.reload_vibration_particle = false;
        return;
    };
    let origin = current_vibration.pos;
    let position_source = user.get_position_source();
    let destination = position_source.get_position().unwrap_or(origin);
    let travel_time_in_ticks = data.travel_time_in_ticks;
    let initial_travel_time = user.calculate_travel_time_in_ticks(current_vibration.distance);
    let alpha = 1.0 - f64::from(travel_time_in_ticks) / f64::from(initial_travel_time);
    let new_initial = origin.lerp(&destination, alpha);
    if send_vibration_particle(world, new_initial, position_source, travel_time_in_ticks) {
        data.reload_vibration_particle = false;
    }
}

/// Vanilla `VibrationSystem.Ticker.receiveVibration`.
fn receive_vibration(
    world: &Arc<World>,
    data: &Mutex<VibrationData>,
    user: &dyn VibrationUser,
    current_vibration: &VibrationInfo,
) -> bool {
    let origin = BlockPos::floored_v(current_vibration.pos);
    let destination = user
        .get_position_source()
        .get_position()
        .map_or(origin, BlockPos::floored_v);
    if user.requires_adjacent_chunks_to_be_ticking()
        && !are_adjacent_chunks_ticking(world, destination)
    {
        return false;
    }
    let source_entity = current_vibration.get_entity(world);
    let projectile_owner = current_vibration.get_projectile_owner(world);
    user.on_receive_vibration(
        world,
        origin,
        current_vibration.game_event,
        source_entity.as_deref(),
        projectile_owner.as_deref(),
        distance_between_in_blocks(origin, destination),
    );
    data.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .current_vibration = None;
    true
}

/// Vanilla `VibrationSystem.Ticker.areAdjacentChunksTicking`. The tick loop already holds the
/// active chunks for reading, so this only tries to read them; failing means "not ticking",
/// and the vibration waits for the next tick as vanilla's would.
fn are_adjacent_chunks_ticking(world: &World, listener_pos: BlockPos) -> bool {
    let Ok(active_chunks) = world.active_chunks.try_read() else {
        return false;
    };
    let listener_chunk_pos = listener_pos.chunk_position();
    for x in listener_chunk_pos.x - 1..=listener_chunk_pos.x + 1 {
        for z in listener_chunk_pos.y - 1..=listener_chunk_pos.y + 1 {
            let chunk_pos = Vector2::new(x, z);
            if !active_chunks.contains(&chunk_pos) || !world.level.is_chunk_loaded(&chunk_pos) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vibration(game_event: GameEvent, distance: f32) -> VibrationInfo {
        VibrationInfo {
            game_event,
            distance,
            pos: Vector3::new(0.5, 64.0, 0.5),
            uuid: None,
            projectile_owner_uuid: None,
        }
    }

    #[test]
    fn selector_keeps_the_closest_then_most_frequent_vibration_of_a_tick() {
        let mut selector = VibrationSelector::default();
        selector.add_candidate(vibration(GameEvent::BlockPlace, 5.0), 10);
        selector.add_candidate(vibration(GameEvent::Step, 3.0), 10);
        selector.add_candidate(vibration(GameEvent::Eat, 3.0), 10);
        selector.add_candidate(vibration(GameEvent::Swim, 3.0), 10);
        // A later tick never replaces the pending candidate.
        selector.add_candidate(vibration(GameEvent::Explode, 0.0), 11);

        assert!(selector.chosen_candidate(10).is_none());
        let chosen = selector.chosen_candidate(11);
        assert_eq!(chosen.map(|v| v.game_event), Some(GameEvent::Eat));
    }

    #[test]
    fn data_round_trips_and_falls_back_without_a_selector() {
        let mut data = VibrationData {
            current_vibration: Some(vibration(GameEvent::NoteBlockPlay, 4.5)),
            travel_time_in_ticks: 3,
            ..Default::default()
        };
        data.selection_strategy
            .add_candidate(vibration(GameEvent::Step, 2.0), 7);
        let mut parent = NbtCompound::new();
        parent.put_compound(VibrationData::NBT_TAG_KEY, data.write_nbt());

        let read = VibrationData::read_nbt(&parent);
        assert_eq!(
            read.current_vibration.map(|v| (v.game_event, v.distance)),
            Some((GameEvent::NoteBlockPlay, 4.5))
        );
        assert_eq!(read.travel_time_in_ticks, 3);
        assert!(read.reload_vibration_particle);
        assert_eq!(
            read.selection_strategy
                .chosen_candidate(8)
                .map(|v| v.game_event),
            Some(GameEvent::Step)
        );

        let mut no_selector = NbtCompound::new();
        no_selector.put_compound(
            "event",
            vibration(GameEvent::NoteBlockPlay, 4.5).write_nbt(),
        );
        no_selector.put_int("event_delay", 3);
        let mut parent = NbtCompound::new();
        parent.put_compound(VibrationData::NBT_TAG_KEY, no_selector);
        assert!(VibrationData::read_nbt(&parent).current_vibration.is_none());
    }
}
