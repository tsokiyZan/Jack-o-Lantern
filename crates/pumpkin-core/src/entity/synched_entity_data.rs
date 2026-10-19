use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::BufMut;
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::tracked_data::{TrackedData, TrackedId};
use pumpkin_protocol::java::client::play::{Metadata, MetadataSerializer};
use pumpkin_protocol::ser::WritingError;
use pumpkin_util::version::JavaMinecraftVersion;

pub trait ErasedSerializer: Send + Sync {
    fn write(
        &self,
        index: TrackedId,
        r#type: MetaDataType,
        writer: &mut dyn std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), WritingError>;

    fn write_canonical(&self, index: TrackedId, r#type: MetaDataType) -> Vec<u8>;
}

struct SerializerHolder<T> {
    value: T,
}

impl<T: MetadataSerializer + Clone + Send + Sync + 'static> ErasedSerializer
    for SerializerHolder<T>
{
    fn write(
        &self,
        index: TrackedId,
        r#type: MetaDataType,
        writer: &mut dyn std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), WritingError> {
        let meta = Metadata::new_raw(index, r#type, &self.value);
        meta.write(writer, version)
    }

    fn write_canonical(&self, index: TrackedId, r#type: MetaDataType) -> Vec<u8> {
        let mut buf = Vec::new();
        let meta = Metadata::new_raw(index, r#type, &self.value);
        let _ = meta.write(&mut buf, &JavaMinecraftVersion::V_26_3);
        buf
    }
}

pub struct DataItem {
    pub tracked: TrackedData,
    pub serializer: Box<dyn ErasedSerializer>,
    pub canonical_bytes: Vec<u8>,
    pub dirty: bool,
    pub is_default: bool,
}

pub struct SynchedEntityData {
    /// By index, so packets list the values in vanilla's `itemsById` order.
    items: Mutex<BTreeMap<u8, DataItem>>,
    is_dirty: AtomicBool,
    /// Whose vanilla `defineSynchedData` defaults apply, if known.
    entity_type_id: Option<u16>,
}

impl Default for SynchedEntityData {
    fn default() -> Self {
        Self::new()
    }
}

impl SynchedEntityData {
    /// An empty tracker; values are added by [`Self::define`] and [`Self::set`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            items: Mutex::new(BTreeMap::new()),
            is_dirty: AtomicBool::new(false),
            entity_type_id: None,
        }
    }

    /// A tracker that knows vanilla's defaults for `entity_type_id`, so values still at
    /// their default are neither sent nor counted as set.
    #[must_use]
    pub const fn for_entity_type(entity_type_id: u16) -> Self {
        Self {
            items: Mutex::new(BTreeMap::new()),
            is_dirty: AtomicBool::new(false),
            entity_type_id: Some(entity_type_id),
        }
    }

    /// Whether `canonical` is vanilla's default for its index on this entity type.
    fn is_vanilla_default(&self, index: u8, canonical: &[u8]) -> bool {
        self.entity_type_id.is_some_and(|type_id| {
            pumpkin_data::synched_data_defaults::synched_data_default(type_id, index)
                == Some(canonical)
        })
    }

    /// Registers a tracked value with the default the client already assumes, so it
    /// is only sent once something actually sets it.
    pub fn define<T: MetadataSerializer + Clone + Send + Sync + 'static>(
        &self,
        tracked: TrackedData,
        value: T,
    ) {
        let holder = SerializerHolder { value };
        let canonical_bytes = holder.write_canonical(tracked.id, tracked.r#type);
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items.insert(
            tracked.id.0,
            DataItem {
                tracked,
                serializer: Box::new(holder),
                canonical_bytes,
                dirty: false,
                is_default: true,
            },
        );
    }

    /// Stores a tracked value and marks it dirty, returning whether it differs from
    /// the value that was already stored.
    pub fn set<T: MetadataSerializer + Clone + Send + Sync + 'static>(
        &self,
        tracked: TrackedData,
        value: T,
    ) -> bool {
        let holder = SerializerHolder { value };
        let new_canonical = holder.write_canonical(tracked.id, tracked.r#type);

        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(item) = items.get_mut(&tracked.id.0) {
            if item.canonical_bytes == new_canonical {
                return false;
            }
            // Vanilla `DataItem.isSetToDefault`: back at the initial value, it leaves the
            // spawn data again.
            item.is_default = self.is_vanilla_default(tracked.id.0, &new_canonical);
            item.canonical_bytes = new_canonical;
            item.serializer = Box::new(holder);
            item.dirty = true;
        } else if self.is_vanilla_default(tracked.id.0, &new_canonical) {
            // Vanilla's item already holds this value from `defineSynchedData`: no change.
            items.insert(
                tracked.id.0,
                DataItem {
                    tracked,
                    serializer: Box::new(holder),
                    canonical_bytes: new_canonical,
                    dirty: false,
                    is_default: true,
                },
            );
            return false;
        } else {
            items.insert(
                tracked.id.0,
                DataItem {
                    tracked,
                    serializer: Box::new(holder),
                    canonical_bytes: new_canonical,
                    dirty: true,
                    is_default: false,
                },
            );
        }
        self.is_dirty.store(true, Ordering::Release);
        true
    }

    /// Whether any value changed since the last [`Self::clear_dirty`].
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.is_dirty.load(Ordering::Acquire)
    }

    /// Serializes the values that changed since the last [`Self::clear_dirty`], or
    /// `None` when nothing changed.
    pub fn pack_dirty_for_version(&self, version: &JavaMinecraftVersion) -> Option<Box<[u8]>> {
        if !self.is_dirty.load(Ordering::Acquire) {
            return None;
        }

        let items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut buf = Vec::new();
        let mut has_any = false;

        for item in items.values() {
            if item.dirty {
                let before_len = buf.len();
                if item
                    .serializer
                    .write(item.tracked.id, item.tracked.r#type, &mut buf, version)
                    .is_ok()
                    && buf.len() > before_len
                {
                    has_any = true;
                }
            }
        }

        if !has_any {
            return None;
        }

        buf.put_u8(255);
        Some(buf.into_boxed_slice())
    }

    /// Marks every value as sent.
    pub fn clear_dirty(&self) {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for item in items.values_mut() {
            item.dirty = false;
        }
        self.is_dirty.store(false, Ordering::Release);
    }

    /// Serializes every value that differs from the default the client assumes, or
    /// `None` when they are all still at their default.
    pub fn get_non_default_values_for_version(
        &self,
        version: &JavaMinecraftVersion,
    ) -> Option<Box<[u8]>> {
        let items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut buf = Vec::new();
        let mut has_any = false;

        for item in items.values() {
            if !item.is_default {
                let before_len = buf.len();
                if item
                    .serializer
                    .write(item.tracked.id, item.tracked.r#type, &mut buf, version)
                    .is_ok()
                    && buf.len() > before_len
                {
                    has_any = true;
                }
            }
        }

        if !has_any {
            return None;
        }

        buf.put_u8(255);
        Some(buf.into_boxed_slice())
    }
}

#[cfg(test)]
mod test {
    use pumpkin_data::tracked_data::player::PLAYER_MODE_CUSTOMISATION;

    use super::*;

    /// Metadata ids no longer vary per version: core serializes the current format for
    /// every client and `pumpkin-java-multiversion` translates. So the senders need no
    /// version check of their own to decide whether a value reaches a client.
    #[test]
    fn a_set_value_is_serialized_for_every_version() {
        let data = SynchedEntityData::new();
        data.define(PLAYER_MODE_CUSTOMISATION, 0u8);
        assert!(data.set(PLAYER_MODE_CUSTOMISATION, 0x7Fu8));

        let current = data.get_non_default_values_for_version(&JavaMinecraftVersion::V_26_3);
        let legacy = data.get_non_default_values_for_version(&JavaMinecraftVersion::V_1_20_5);
        assert!(current.is_some());
        assert_eq!(current, legacy);
        assert_eq!(
            data.pack_dirty_for_version(&JavaMinecraftVersion::V_1_20_5),
            current
        );
    }

    /// A value still at the default the client assumes is not sent.
    #[test]
    fn default_values_are_not_serialized() {
        let data = SynchedEntityData::new();
        data.define(PLAYER_MODE_CUSTOMISATION, 0u8);

        assert!(
            data.get_non_default_values_for_version(&JavaMinecraftVersion::V_26_3)
                .is_none()
        );
    }
}
