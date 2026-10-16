//! Vanilla `net.minecraft.world.level.gameevent`: game event listeners, the per-section
//! listener registries and the dispatcher `ServerLevel.gameEvent` posts to.

pub mod vibrations;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};

use pumpkin_data::BlockState;
use pumpkin_data::game_event::GameEvent;
use pumpkin_protocol::ser::NetworkWriteExt;
use pumpkin_protocol::{VarInt, ser::WritingError};
use pumpkin_util::math::position::{BlockPos, chunk_section_from_pos};
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::math::vector3::Vector3;
use rustc_hash::FxHashMap;

use crate::entity::EntityBase;
use crate::world::World;

/// Vanilla `GameEvent.DEFAULT_NOTIFICATION_RADIUS`.
pub const DEFAULT_NOTIFICATION_RADIUS: i32 = 16;

/// Vanilla `GameEvent.notificationRadius`, as each event is registered with.
#[must_use]
pub const fn notification_radius(event: GameEvent) -> i32 {
    match event {
        GameEvent::JukeboxPlay | GameEvent::JukeboxStopPlay => 10,
        GameEvent::Shriek => 32,
        _ => DEFAULT_NOTIFICATION_RADIUS,
    }
}

/// Vanilla `GameEvent.Context`.
#[derive(Clone, Copy, Default)]
pub struct GameEventContext<'a> {
    pub source_entity: Option<&'a dyn EntityBase>,
    pub affected_state: Option<&'static BlockState>,
}

impl<'a> GameEventContext<'a> {
    /// Vanilla `GameEvent.Context.of(Entity, BlockState)`.
    #[must_use]
    pub const fn of(
        source_entity: Option<&'a dyn EntityBase>,
        affected_state: Option<&'static BlockState>,
    ) -> Self {
        Self {
            source_entity,
            affected_state,
        }
    }

    /// Vanilla `GameEvent.Context.of(Entity)`.
    #[must_use]
    pub const fn of_entity(source_entity: &'a dyn EntityBase) -> Self {
        Self::of(Some(source_entity), None)
    }

    /// Vanilla `GameEvent.Context.of(BlockState)`.
    #[must_use]
    pub const fn of_state(affected_state: &'static BlockState) -> Self {
        Self::of(None, Some(affected_state))
    }
}

/// Vanilla `PositionSourceType.BLOCK`'s registry id.
const BLOCK_POSITION_SOURCE_TYPE_ID: i32 = 0;
/// Vanilla `PositionSourceType.ENTITY`'s registry id.
const ENTITY_POSITION_SOURCE_TYPE_ID: i32 = 1;

/// Vanilla `PositionSource`.
#[derive(Clone)]
pub enum PositionSource {
    /// Vanilla `BlockPositionSource`.
    Block(BlockPos),
    /// Vanilla `EntityPositionSource`.
    Entity {
        entity: Weak<dyn EntityBase>,
        y_offset: f32,
    },
}

impl PositionSource {
    /// Vanilla `PositionSource.getPosition`.
    #[must_use]
    pub fn get_position(&self) -> Option<Vector3<f64>> {
        match self {
            Self::Block(pos) => Some(pos.to_centered_f64()),
            Self::Entity { entity, y_offset } => entity.upgrade().map(|entity| {
                entity
                    .get_entity()
                    .pos
                    .load()
                    .add_raw(0.0, f64::from(*y_offset), 0.0)
            }),
        }
    }

    /// Vanilla `PositionSource.STREAM_CODEC`. `None` when the entity is gone, where vanilla
    /// throws.
    pub fn write(&self, buf: &mut Vec<u8>) -> Option<Result<(), WritingError>> {
        Some(match self {
            Self::Block(pos) => buf
                .write_var_int(&VarInt(BLOCK_POSITION_SOURCE_TYPE_ID))
                .and_then(|()| buf.write_i64_be(pos.as_long())),
            Self::Entity { entity, y_offset } => {
                let entity_id = entity.upgrade()?.get_entity().entity_id;
                buf.write_var_int(&VarInt(ENTITY_POSITION_SOURCE_TYPE_ID))
                    .and_then(|()| buf.write_var_int(&VarInt(entity_id)))
                    .and_then(|()| buf.write_f32_be(*y_offset))
            }
        })
    }
}

/// Vanilla `GameEventListener.DeliveryMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeliveryMode {
    #[default]
    Unspecified,
    ByDistance,
}

/// Vanilla `GameEventListener`.
pub trait GameEventListener: Send + Sync {
    fn get_listener_source(&self) -> &PositionSource;

    fn get_listener_radius(&self) -> i32;

    fn handle_game_event(
        &self,
        world: &World,
        event: GameEvent,
        context: &GameEventContext<'_>,
        source_position: Vector3<f64>,
    ) -> bool;

    fn get_delivery_mode(&self) -> DeliveryMode {
        DeliveryMode::Unspecified
    }
}

/// Vanilla `EuclideanGameEventListenerRegistry`: one chunk section's listeners.
type SectionListeners = Vec<Arc<dyn GameEventListener>>;

/// Every loaded chunk's `LevelChunk.gameEventListenerRegistrySections`, keyed by chunk, then
/// section Y.
#[derive(Default)]
pub struct GameEventListenerRegistries {
    chunks: RwLock<FxHashMap<Vector2<i32>, FxHashMap<i32, SectionListeners>>>,
    /// Listeners registered anywhere, so posting skips the lookups in a world without any.
    listener_count: AtomicUsize,
}

impl GameEventListenerRegistries {
    /// Vanilla `EuclideanGameEventListenerRegistry.register` on `section`'s registry.
    pub fn register(&self, section: Vector3<i32>, listener: Arc<dyn GameEventListener>) {
        self.chunks
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(Vector2::new(section.x, section.z))
            .or_default()
            .entry(section.y)
            .or_default()
            .push(listener);
        self.listener_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Vanilla `EuclideanGameEventListenerRegistry.unregister`, which drops the section's
    /// registry once it is empty (`LevelChunk.removeGameEventListenerRegistry`).
    pub fn unregister(&self, section: Vector3<i32>, listener: &Arc<dyn GameEventListener>) {
        let mut chunks = self.chunks.write().unwrap_or_else(PoisonError::into_inner);
        let chunk_pos = Vector2::new(section.x, section.z);
        let Some(sections) = chunks.get_mut(&chunk_pos) else {
            return;
        };
        let Some(listeners) = sections.get_mut(&section.y) else {
            return;
        };
        if let Some(index) = listeners.iter().position(|registered| {
            std::ptr::addr_eq(Arc::as_ptr(registered), Arc::as_ptr(listener))
        }) {
            listeners.remove(index);
            self.listener_count.fetch_sub(1, Ordering::Relaxed);
        }
        if listeners.is_empty() {
            sections.remove(&section.y);
            if sections.is_empty() {
                chunks.remove(&chunk_pos);
            }
        }
    }

    /// Drops a chunk's registries with the chunk.
    pub fn remove_chunk(&self, chunk_pos: Vector2<i32>) {
        let removed = self
            .chunks
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&chunk_pos);
        if let Some(sections) = removed {
            let count: usize = sections.values().map(Vec::len).sum();
            self.listener_count.fetch_sub(count, Ordering::Relaxed);
        }
    }

    /// Vanilla `GameEventDispatcher.post`'s walk over the sections in `radius` of `center`,
    /// with `EuclideanGameEventListenerRegistry.visitInRangeListeners` for each, in vanilla's
    /// order. Collected first so no listener runs with the registry locked.
    fn in_range_listeners(
        &self,
        center: BlockPos,
        radius: i32,
        source_position: Vector3<f64>,
    ) -> Vec<(Arc<dyn GameEventListener>, Vector3<f64>)> {
        let mut visited = Vec::new();
        if self.listener_count.load(Ordering::Relaxed) == 0 {
            return visited;
        }
        let min = chunk_section_from_pos(&center.add(-radius, -radius, -radius));
        let max = chunk_section_from_pos(&center.add(radius, radius, radius));
        let chunks = self.chunks.read().unwrap_or_else(PoisonError::into_inner);
        for chunk_x in min.x..=max.x {
            for chunk_z in min.z..=max.z {
                let Some(sections) = chunks.get(&Vector2::new(chunk_x, chunk_z)) else {
                    continue;
                };
                for section_y in min.y..=max.y {
                    let Some(listeners) = sections.get(&section_y) else {
                        continue;
                    };
                    for listener in listeners {
                        if let Some(position) =
                            get_postable_listener_position(source_position, listener.as_ref())
                        {
                            visited.push((listener.clone(), position));
                        }
                    }
                }
            }
        }
        visited
    }
}

/// Vanilla `EuclideanGameEventListenerRegistry.getPostableListenerPosition`.
fn get_postable_listener_position(
    source_position: Vector3<f64>,
    listener: &dyn GameEventListener,
) -> Option<Vector3<f64>> {
    let position = listener.get_listener_source().get_position()?;
    let listener_pos = BlockPos::floored_v(position).0;
    let source_pos = BlockPos::floored_v(source_position).0;
    let dx = i64::from(listener_pos.x - source_pos.x);
    let dy = i64::from(listener_pos.y - source_pos.y);
    let dz = i64::from(listener_pos.z - source_pos.z);
    let radius = i64::from(listener.get_listener_radius());
    (dx * dx + dy * dy + dz * dz <= radius * radius).then_some(position)
}

/// Vanilla `GameEventDispatcher.post`.
pub fn post(
    world: &World,
    event: GameEvent,
    position: Vector3<f64>,
    context: &GameEventContext<'_>,
) {
    let listeners = world.game_event_listeners.in_range_listeners(
        BlockPos::floored_v(position),
        notification_radius(event),
        position,
    );
    if listeners.is_empty() {
        return;
    }
    // Vanilla handles `UNSPECIFIED` listeners while visiting, then the `BY_DISTANCE` ones
    // nearest first (`GameEvent.ListenerInfo.compareTo`).
    let mut by_distance = Vec::new();
    for (listener, listener_position) in listeners {
        if listener.get_delivery_mode() == DeliveryMode::ByDistance {
            by_distance.push((
                position.squared_distance_to_vec(&listener_position),
                listener,
            ));
        } else {
            listener.handle_game_event(world, event, context, position);
        }
    }
    by_distance.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, listener) in by_distance {
        listener.handle_game_event(world, event, context, position);
    }
}

/// Vanilla `DynamicGameEventListener`: a listener that follows a moving entity from section to
/// section.
pub struct DynamicGameEventListener {
    listener: Arc<dyn GameEventListener>,
    last_section: Mutex<Option<Vector3<i32>>>,
}

impl DynamicGameEventListener {
    #[must_use]
    pub fn new(listener: Arc<dyn GameEventListener>) -> Self {
        Self {
            listener,
            last_section: Mutex::new(None),
        }
    }

    #[must_use]
    pub const fn get_listener(&self) -> &Arc<dyn GameEventListener> {
        &self.listener
    }

    pub fn add(&self, world: &World) {
        self.r#move(world);
    }

    pub fn remove(&self, world: &World) {
        // Vanilla keeps `lastSection`, but its entities are never added again after removal;
        // Pumpkin can re-add the same entity, which must register anew.
        let last_section = self
            .last_section
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(section) = last_section {
            Self::if_chunk_exists(world, section, |registries, section| {
                registries.unregister(section, &self.listener);
            });
        }
    }

    pub fn r#move(&self, world: &World) {
        let Some(position) = self.listener.get_listener_source().get_position() else {
            return;
        };
        let current_section = chunk_section_from_pos(&BlockPos::floored_v(position));
        let mut last_section = self
            .last_section
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *last_section == Some(current_section) {
            return;
        }
        if let Some(section) = *last_section {
            Self::if_chunk_exists(world, section, |registries, section| {
                registries.unregister(section, &self.listener);
            });
        }
        *last_section = Some(current_section);
        Self::if_chunk_exists(world, current_section, |registries, section| {
            registries.register(section, self.listener.clone());
        });
    }

    /// `r#move` for an entity in a world: vanilla's section change callback only exists while
    /// the entity is in a level, and Pumpkin moves entities before adding them.
    pub fn on_section_change(&self, world: &World) {
        let added = self
            .last_section
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if added {
            self.r#move(world);
        }
    }

    fn if_chunk_exists(
        world: &World,
        section: Vector3<i32>,
        action: impl FnOnce(&GameEventListenerRegistries, Vector3<i32>),
    ) {
        if world
            .level
            .is_chunk_loaded(&Vector2::new(section.x, section.z))
        {
            action(&world.game_event_listeners, section);
        }
    }
}
