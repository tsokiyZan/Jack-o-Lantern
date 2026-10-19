use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pumpkin_data::Block;
use pumpkin_data::block_properties::JukeboxLikeProperties;
use pumpkin_data::data_component_impl::{Holder, JukeboxPlayableImpl};
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::jukebox_song::JukeboxSong;
use pumpkin_data::particle::Particle;
use pumpkin_data::world::WorldEvent;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;

use crate::block::entities::BlockEntity;
use crate::entity::Entity;
use crate::entity::item::ItemEntity;
use crate::world::World;
use crate::world::game_event::GameEventContext;
use pumpkin_data::entity::EntityType;
use pumpkin_inventory::{Clearable, Inventory};

/// Vanilla `JukeboxSongPlayer.PLAY_EVENT_INTERVAL_TICKS`.
const PLAY_EVENT_INTERVAL_TICKS: u64 = 20;
/// Vanilla `JukeboxSong.hasFinished`'s grace after the song's length.
const SONG_FINISH_GRACE_TICKS: u64 = 20;

/// Vanilla `JukeboxSong.fromStack`, for the registry songs Pumpkin plays.
fn song_from_stack(stack: &ItemStack) -> Option<JukeboxSong> {
    match stack.get_data_component::<JukeboxPlayableImpl>()?.song {
        Holder::Reference(song) => Some(song),
        Holder::Direct(_) => None,
    }
}

/// Vanilla `JukeboxSong.hasFinished`, for a song `length_in_ticks` long.
const fn has_finished(length_in_ticks: u64, ticks_since_song_started: u64) -> bool {
    ticks_since_song_started >= length_in_ticks + SONG_FINISH_GRACE_TICKS
}

/// Matches vanilla's `JukeboxBlockEntity`
pub struct JukeboxBlockEntity {
    position: BlockPos,
    /// The record item stored in the jukebox (`RecordItem` in NBT)
    record_stack: Arc<Mutex<ItemStack>>,
    /// Vanilla `JukeboxSongPlayer.ticksSinceSongStarted`.
    ticks_since_song_started: AtomicU64,
    /// Vanilla `JukeboxSongPlayer.song`, by the length `hasFinished` checks; 0 for none.
    song_length_ticks: AtomicU64,
    dirty: AtomicBool,
    comparator_dirty: AtomicBool,
}

const RECORD_ITEM_NBT_KEY: &str = "RecordItem";
const TICKS_SINCE_SONG_STARTED_NBT_KEY: &str = "ticks_since_song_started";

impl BlockEntity for JukeboxBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let record_stack = nbt
            .get_compound(RECORD_ITEM_NBT_KEY)
            .and_then(ItemStack::read_item_stack)
            .unwrap_or_else(|| ItemStack::EMPTY.clone());

        // Vanilla `JukeboxSongPlayer.setSongWithoutPlaying`.
        let (song_length_ticks, ticks_since_song_started) = nbt
            .get_long(TICKS_SINCE_SONG_STARTED_NBT_KEY)
            .map(|ticks| ticks as u64)
            .and_then(|ticks| {
                song_from_stack(&record_stack)
                    .map(|song| song.length_in_ticks())
                    .filter(|length| !has_finished(*length, ticks))
                    .map(|length| (length, ticks))
            })
            .unwrap_or((0, 0));

        Self {
            position,
            record_stack: Arc::new(Mutex::new(record_stack)),
            ticks_since_song_started: AtomicU64::new(ticks_since_song_started),
            song_length_ticks: AtomicU64::new(song_length_ticks),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        let record = self
            .record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !record.is_empty() {
            let mut record_nbt = NbtCompound::new();
            record.write_item_stack(&mut record_nbt);
            nbt.put(RECORD_ITEM_NBT_KEY, record_nbt);
        }

        if self.is_playing() {
            nbt.put_long(
                TICKS_SINCE_SONG_STARTED_NBT_KEY,
                self.ticks_since_song_started.load(Ordering::Relaxed) as i64,
            );
        }
    }

    /// Vanilla `JukeboxSongPlayer.tick`.
    fn tick(&self, world: &Arc<World>) {
        let song_length = self.song_length_ticks.load(Ordering::Relaxed);
        if song_length == 0 {
            return;
        }
        let ticks = self.ticks_since_song_started.load(Ordering::Relaxed);
        if has_finished(song_length, ticks) {
            self.stop(world);
            return;
        }
        if ticks.is_multiple_of(PLAY_EVENT_INTERVAL_TICKS) {
            world.game_event(
                GameEvent::JukeboxPlay,
                self.position.to_centered_f64(),
                GameEventContext::of_state(world.get_block_state(&self.position)),
            );
            self.spawn_music_particles(world);
        }
        self.ticks_since_song_started
            .store(ticks + 1, Ordering::Relaxed);
    }

    /// Vanilla `JukeboxBlockEntity.preRemoveSideEffects`, which flag 256 skips.
    fn on_block_replaced(self: Arc<Self>, world: &Arc<World>, _position: &BlockPos) {
        self.pop_out_the_item(world);
    }

    /// Vanilla `JukeboxBlockEntity.setRemoved`.
    fn set_removed(&self, world: &World) {
        // The jukebox is already gone, so its old state is unknown; listeners only check
        // the block.
        world.game_event(
            GameEvent::JukeboxStopPlay,
            self.position.to_centered_f64(),
            GameEventContext::of_state(Block::JUKEBOX.default_state),
        );
        world.sync_world_event(WorldEvent::SoundStopJukeboxSong, self.position, 0);
    }

    fn is_comparator_dirty(&self) -> bool {
        self.comparator_dirty.load(Ordering::Relaxed)
    }

    fn clear_comparator_dirty(&self) {
        self.comparator_dirty.store(false, Ordering::Relaxed);
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        if let Ok(record) = self.record_stack.try_lock()
            && !record.is_empty()
        {
            let mut record_nbt = NbtCompound::new();
            record.write_item_stack(&mut record_nbt);
            nbt.put("RecordItem", NbtTag::Compound(record_nbt));
        }
        nbt.put_long(
            "ticks_since_song_started",
            self.ticks_since_song_started.load(Ordering::Relaxed) as i64,
        );
        Some(nbt)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        Some(self)
    }
}

impl JukeboxBlockEntity {
    pub const ID: &'static str = "minecraft:jukebox";

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            record_stack: Arc::new(Mutex::new(ItemStack::EMPTY.clone())),
            ticks_since_song_started: AtomicU64::new(0),
            song_length_ticks: AtomicU64::new(0),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
        }
    }

    /// Get the current record stack
    pub fn get_record(&self) -> ItemStack {
        self.record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Vanilla `JukeboxSongPlayer.isPlaying`.
    pub fn is_playing(&self) -> bool {
        self.song_length_ticks.load(Ordering::Relaxed) > 0
    }

    /// Starts a song `length_in_ticks` long for plugins, without vanilla's sound and events.
    pub fn start_playing(&self, length_in_ticks: u64) {
        self.ticks_since_song_started.store(0, Ordering::Relaxed);
        self.song_length_ticks
            .store(length_in_ticks, Ordering::Relaxed);
        self.mark_dirty();
    }

    /// Stops the song for plugins and containers, without vanilla's sound and events.
    pub fn stop_playing(&self) {
        self.ticks_since_song_started.store(0, Ordering::Relaxed);
        self.song_length_ticks.store(0, Ordering::Relaxed);
        self.mark_dirty();
    }

    /// Vanilla `JukeboxBlockEntity.setTheItem`.
    pub fn set_the_item(&self, world: &Arc<World>, stack: ItemStack) {
        let song = song_from_stack(&stack);
        let item_was_inserted = !stack.is_empty();
        *self
            .record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = stack;
        self.mark_dirty();
        self.notify_item_changed_in_jukebox(world, item_was_inserted);
        match song {
            Some(song) if item_was_inserted => self.play(world, song),
            _ => self.stop(world),
        }
    }

    /// Vanilla `JukeboxBlockEntity.notifyItemChangedInJukebox`.
    fn notify_item_changed_in_jukebox(&self, world: &Arc<World>, was_inserted: bool) {
        let (block, state) = world.get_block_and_state_id(&self.position);
        if block != &Block::JUKEBOX {
            return;
        }
        let mut props = JukeboxLikeProperties::from_state_id(state);
        props.has_record = was_inserted;
        world.set_block_state(
            &self.position,
            props.to_state_id(block),
            BlockFlags::NOTIFY_LISTENERS,
        );
        world.game_event(
            GameEvent::BlockChange,
            self.position.to_centered_f64(),
            GameEventContext::of_state(world.get_block_state(&self.position)),
        );
    }

    /// Vanilla `JukeboxBlockEntity.popOutTheItem`.
    pub fn pop_out_the_item(&self, world: &Arc<World>) {
        let item_before_popping_out = self.get_record();
        if item_before_popping_out.is_empty() {
            return;
        }
        self.set_the_item(world, ItemStack::EMPTY.clone());
        let mut rng = rand::rng();
        // `Vec3.atLowerCornerWithOffset(pos, 0.5, 1.01, 0.5).offsetRandomXZ(random, 0.7F)`.
        let spawn_pos = Vector3::new(
            f64::from(self.position.0.x) + 0.5 + f64::from((rng.random::<f32>() - 0.5) * 0.7),
            f64::from(self.position.0.y) + 1.01,
            f64::from(self.position.0.z) + 0.5 + f64::from((rng.random::<f32>() - 0.5) * 0.7),
        );
        let entity = Entity::new(world.clone(), spawn_pos, &EntityType::ITEM);
        world.spawn_entity(Arc::new(ItemEntity::new(entity, item_before_popping_out)));
        self.on_song_changed(world);
    }

    /// Vanilla `JukeboxSongPlayer.play`.
    fn play(&self, world: &Arc<World>, song: JukeboxSong) {
        self.start_playing(song.length_in_ticks());
        world.sync_world_event(
            WorldEvent::SoundPlayJukeboxSong,
            self.position,
            song.get_id() as i32,
        );
        self.on_song_changed(world);
    }

    /// Vanilla `JukeboxSongPlayer.stop`.
    fn stop(&self, world: &Arc<World>) {
        if !self.is_playing() {
            return;
        }
        self.stop_playing();
        world.game_event(
            GameEvent::JukeboxStopPlay,
            self.position.to_centered_f64(),
            GameEventContext::of_state(world.get_block_state(&self.position)),
        );
        world.sync_world_event(WorldEvent::SoundStopJukeboxSong, self.position, 0);
        self.on_song_changed(world);
    }

    /// Vanilla `JukeboxBlockEntity.onSongChanged`.
    fn on_song_changed(&self, world: &Arc<World>) {
        world.update_neighbors_at(&self.position, &Block::JUKEBOX, None);
        self.mark_dirty();
    }

    /// Vanilla `JukeboxSongPlayer.spawnMusicParticles`.
    fn spawn_music_particles(&self, world: &World) {
        let random_color = rand::rng().random_range(0..4) as f32 / 24.0;
        let pos = self.position;
        world.spawn_particle(
            Vector3::new(
                f64::from(pos.0.x) + 0.5,
                f64::from(pos.0.y) + f64::from(1.2f32),
                f64::from(pos.0.z) + 0.5,
            ),
            Vector3::new(random_color, 0.0, 0.0),
            1.0,
            0,
            Particle::Note,
        );
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
        self.comparator_dirty.store(true, Ordering::Relaxed);
    }
}

/// Implements single-slot inventory for jukebox (matches vanilla's `SingleStackInventory`)
impl Inventory for JukeboxBlockEntity {
    fn size(&self) -> usize {
        1
    }

    fn is_empty(&self) -> bool {
        self.record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    fn get_stack(&self, _slot: usize) -> ItemStack {
        self.record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn remove_stack(&self, _slot: usize) -> ItemStack {
        // TODO: vanilla goes through `setTheItem`, which stops the song with its events; this
        // has no world to post them to.
        self.stop_playing();
        let mut record = self
            .record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let taken = record.clone();
        *record = ItemStack::EMPTY.clone();
        self.mark_dirty();
        taken
    }

    fn remove_stack_specific(&self, _slot: usize, _amount: u8) -> ItemStack {
        // Jukebox only holds one item, so remove the whole stack
        self.remove_stack(0)
    }

    fn set_stack(&self, _slot: usize, stack: ItemStack) {
        *self
            .record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = stack;
        self.mark_dirty();
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
        self.comparator_dirty.store(true, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Clearable for JukeboxBlockEntity {
    fn clear(&self) {
        self.stop_playing();
        *self
            .record_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = ItemStack::EMPTY.clone();
        self.mark_dirty();
    }
}
