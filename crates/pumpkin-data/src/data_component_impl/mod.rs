#![allow(dead_code)]

use crate::Block;
use crate::BlockId;
use crate::data_component::DataComponent;
use crate::entity_type::EntityType;
use crate::sound::Sound;
use crate::tag::Taggable;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;
use pumpkin_util::version::JavaMinecraftVersion;
use std::any::Any;
use std::borrow::Cow;

/// CRC-32C state for vanilla's data component hashes.
#[derive(Default)]
pub struct Digest(u32);

impl Digest {
    #[must_use]
    pub const fn new() -> Self {
        Self(0)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0 = crc32c::crc32c_append(self.0, bytes);
    }

    #[must_use]
    pub const fn finalize(&self) -> u64 {
        self.0 as u64
    }
}

pub trait DataComponentImpl: Send + Sync {
    fn write_data(&self) -> NbtTag {
        NbtTag::End
    }
    fn get_hash(&self) -> i32 {
        0
    }
    /// make sure other is the same type component, or it will panic
    fn equal(&self, other: &dyn DataComponentImpl) -> bool;
    fn get_enum() -> DataComponent
    where
        Self: Sized;
    fn get_self_enum(&self) -> DataComponent; // only for debugging
    fn to_dyn(self) -> Box<dyn DataComponentImpl>;
    fn clone_dyn(&self) -> Box<dyn DataComponentImpl>;
    fn as_any(&self) -> &dyn Any;
    fn as_mut_any(&mut self) -> &mut dyn Any;
}

impl Clone for Box<dyn DataComponentImpl> {
    fn clone(&self) -> Self {
        self.clone_dyn()
    }
}

#[inline]
pub fn get<T: DataComponentImpl + 'static>(value: &dyn DataComponentImpl) -> &T {
    value.as_any().downcast_ref::<T>().unwrap_or_else(|| {
        panic!(
            "you are trying to cast {} ({}) to {} ({})",
            value.get_self_enum().to_name(),
            std::any::type_name_of_val(value),
            T::get_enum().to_name(),
            std::any::type_name::<T>()
        )
    })
}

#[inline]
pub fn get_mut<T: DataComponentImpl + 'static>(value: &mut dyn DataComponentImpl) -> &mut T {
    let name = value.get_self_enum().to_name();
    let val_type = std::any::type_name_of_val(value);
    value.as_mut_any().downcast_mut::<T>().unwrap_or_else(|| {
        panic!(
            "you are trying to cast {name} ({val_type}) to {} ({})",
            T::get_enum().to_name(),
            std::any::type_name::<T>()
        )
    })
}

macro_rules! default_impl {
    ($t: ident) => {
        fn equal(&self, other: &dyn crate::data_component_impl::DataComponentImpl) -> bool {
            if let Some(other) = other.as_any().downcast_ref::<Self>() {
                self == other
            } else {
                false
            }
        }
        #[inline]
        fn get_enum() -> crate::data_component::DataComponent
        where
            Self: Sized,
        {
            crate::data_component::DataComponent::$t
        }
        fn get_self_enum(&self) -> crate::data_component::DataComponent {
            crate::data_component::DataComponent::$t
        }
        fn to_dyn(self) -> Box<dyn crate::data_component_impl::DataComponentImpl> {
            Box::new(self)
        }
        fn clone_dyn(&self) -> Box<dyn crate::data_component_impl::DataComponentImpl> {
            Box::new(self.clone())
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_mut_any(&mut self) -> &mut dyn std::any::Any {
            self
        }
    };
}

// Type tags written by vanilla `HashOps` in front of each value.
const HASH_TAG_EMPTY: u8 = 1;
const HASH_TAG_MAP_START: u8 = 2;
const HASH_TAG_MAP_END: u8 = 3;
const HASH_TAG_LIST_START: u8 = 4;
const HASH_TAG_LIST_END: u8 = 5;
const HASH_TAG_BYTE: u8 = 6;
const HASH_TAG_SHORT: u8 = 7;
const HASH_TAG_INT: u8 = 8;
const HASH_TAG_LONG: u8 = 9;
const HASH_TAG_FLOAT: u8 = 10;
const HASH_TAG_DOUBLE: u8 = 11;
const HASH_TAG_STRING: u8 = 12;
const HASH_TAG_BOOLEAN: u8 = 13;
const HASH_TAG_BYTE_ARRAY_START: u8 = 14;
const HASH_TAG_BYTE_ARRAY_END: u8 = 15;
const HASH_TAG_INT_ARRAY_START: u8 = 16;
const HASH_TAG_INT_ARRAY_END: u8 = 17;
const HASH_TAG_LONG_ARRAY_START: u8 = 18;
const HASH_TAG_LONG_ARRAY_END: u8 = 19;

fn hash_bytes(parts: &[&[u8]]) -> u32 {
    let mut digest = Digest::new();
    for part in parts {
        digest.update(part);
    }
    digest.finalize() as u32
}

/// `HashOps.empty`.
#[must_use]
pub fn get_empty_hash() -> u32 {
    hash_bytes(&[&[HASH_TAG_EMPTY]])
}

/// `HashOps.createString`: the UTF-16 length, then the UTF-16 code units.
#[must_use]
pub fn get_str_hash(val: &str) -> u32 {
    let mut digest = Digest::new();
    digest.update(&[HASH_TAG_STRING]);
    digest.update(&(val.encode_utf16().count() as u32).to_le_bytes());
    for unit in val.encode_utf16() {
        digest.update(&unit.to_le_bytes());
    }
    digest.finalize() as u32
}

/// `HashOps.createByte`.
#[must_use]
pub fn get_byte_hash(val: i8) -> u32 {
    hash_bytes(&[&[HASH_TAG_BYTE], &val.to_le_bytes()])
}

/// `HashOps.createShort`.
#[must_use]
pub fn get_short_hash(val: i16) -> u32 {
    hash_bytes(&[&[HASH_TAG_SHORT], &val.to_le_bytes()])
}

/// `HashOps.createInt`.
#[must_use]
pub fn get_i32_hash(val: i32) -> u32 {
    hash_bytes(&[&[HASH_TAG_INT], &val.to_le_bytes()])
}

/// `HashOps.createLong`.
#[must_use]
pub fn get_i64_hash(val: i64) -> u32 {
    hash_bytes(&[&[HASH_TAG_LONG], &val.to_le_bytes()])
}

/// `HashOps.createFloat`.
#[must_use]
pub fn get_f32_hash(val: f32) -> u32 {
    hash_bytes(&[&[HASH_TAG_FLOAT], &val.to_bits().to_le_bytes()])
}

/// `HashOps.createDouble`.
#[must_use]
pub fn get_f64_hash(val: f64) -> u32 {
    hash_bytes(&[&[HASH_TAG_DOUBLE], &val.to_bits().to_le_bytes()])
}

/// `HashOps.createBoolean`.
#[must_use]
pub fn get_bool_hash(val: bool) -> u32 {
    hash_bytes(&[&[HASH_TAG_BOOLEAN, u8::from(val)]])
}

/// `HashOps.createList` over already hashed elements.
#[must_use]
pub fn get_list_hash(elements: impl IntoIterator<Item = u32>) -> u32 {
    let mut digest = Digest::new();
    digest.update(&[HASH_TAG_LIST_START]);
    for element in elements {
        digest.update(&element.to_le_bytes());
    }
    digest.update(&[HASH_TAG_LIST_END]);
    digest.finalize() as u32
}

/// `HashOps.createMap` over already hashed `(key, value)` entries. Vanilla
/// sorts the entries by key hash, then value hash, both compared unsigned.
#[must_use]
pub fn get_map_hash(entries: impl IntoIterator<Item = (u32, u32)>) -> u32 {
    let mut entries: Vec<(u32, u32)> = entries.into_iter().collect();
    entries.sort_unstable();
    let mut digest = Digest::new();
    digest.update(&[HASH_TAG_MAP_START]);
    for (key, value) in entries {
        digest.update(&key.to_le_bytes());
        digest.update(&value.to_le_bytes());
    }
    digest.update(&[HASH_TAG_MAP_END]);
    digest.finalize() as u32
}

/// Collects the fields of a record codec for `get_map_hash`. Fields a codec
/// omits (an `optionalFieldOf` at its default) are simply not added.
#[derive(Default)]
pub struct MapHash(Vec<(u32, u32)>);

impl MapHash {
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    #[must_use]
    pub fn field(mut self, key: &str, value: u32) -> Self {
        self.0.push((get_str_hash(key), value));
        self
    }

    #[must_use]
    pub fn optional_field(self, key: &str, value: Option<u32>) -> Self {
        match value {
            Some(value) => self.field(key, value),
            None => self,
        }
    }

    #[must_use]
    pub fn finish(self) -> u32 {
        get_map_hash(self.0)
    }
}

/// Hashes an NBT tag the way `NbtOps.convertTo(HashOps)` does, as used by
/// codecs that pass NBT through (`CustomData`, entity data).
#[must_use]
pub fn get_nbt_hash(tag: &NbtTag) -> u32 {
    match tag {
        NbtTag::End => get_empty_hash(),
        NbtTag::Byte(value) => get_byte_hash(*value),
        NbtTag::Short(value) => get_short_hash(*value),
        NbtTag::Int(value) => get_i32_hash(*value),
        NbtTag::Long(value) => get_i64_hash(*value),
        NbtTag::Float(value) => get_f32_hash(*value),
        NbtTag::Double(value) => get_f64_hash(*value),
        NbtTag::ByteArray(values) => {
            let mut digest = Digest::new();
            digest.update(&[HASH_TAG_BYTE_ARRAY_START]);
            for value in values {
                digest.update(&value.to_le_bytes());
            }
            digest.update(&[HASH_TAG_BYTE_ARRAY_END]);
            digest.finalize() as u32
        }
        NbtTag::String(value) => get_str_hash(value),
        NbtTag::List(values) => get_list_hash(values.iter().map(get_nbt_hash)),
        NbtTag::Compound(compound) => get_compound_hash(compound),
        NbtTag::IntArray(values) => {
            let mut digest = Digest::new();
            digest.update(&[HASH_TAG_INT_ARRAY_START]);
            for value in values {
                digest.update(&value.to_le_bytes());
            }
            digest.update(&[HASH_TAG_INT_ARRAY_END]);
            digest.finalize() as u32
        }
        NbtTag::LongArray(values) => {
            let mut digest = Digest::new();
            digest.update(&[HASH_TAG_LONG_ARRAY_START]);
            for value in values {
                digest.update(&value.to_le_bytes());
            }
            digest.update(&[HASH_TAG_LONG_ARRAY_END]);
            digest.finalize() as u32
        }
    }
}

/// `get_nbt_hash` for a compound.
#[must_use]
pub fn get_compound_hash(compound: &NbtCompound) -> u32 {
    get_map_hash(
        compound
            .child_tags
            .iter()
            .map(|(key, value)| (get_str_hash(key), get_nbt_hash(value))),
    )
}

/// Adds the `minecraft` namespace to a bare resource location, as vanilla's
/// `Identifier` parsing does.
#[must_use]
pub fn namespaced(id: &str) -> Cow<'_, str> {
    if id.contains(':') {
        Cow::Borrowed(id)
    } else {
        Cow::Owned(format!("minecraft:{id}"))
    }
}

/// Hash of a `SoundEvent.CODEC` holder: the sound id for a registry
/// reference, otherwise the inline `{sound_id, range}` record.
#[must_use]
pub fn get_idor_hash(val: &IdOr<basic::SoundEvent>) -> u32 {
    match val {
        IdOr::Id(sound) => get_str_hash(&namespaced(sound.to_name())),
        IdOr::Value(sound) => MapHash::new()
            .field("sound_id", get_str_hash(&namespaced(&sound.sound_name)))
            .optional_field("range", sound.range.map(get_f32_hash))
            .finish(),
    }
}

pub fn put_idor(nbt: &mut NbtCompound, key: &str, val: &IdOr<basic::SoundEvent>) {
    match val {
        IdOr::Id(id) => nbt.put_string(key, format!("minecraft:{}", id.to_name())),
        IdOr::Value(sound) => {
            let mut sound_compound = NbtCompound::new();

            sound_compound.put_string("sound_id", sound.sound_name.clone());
            if let Some(range) = sound.range {
                sound_compound.put_float("range", range);
            }
            nbt.put(key, NbtTag::Compound(sound_compound));
        }
    }
}

pub fn get_idor(nbt: &NbtCompound, key: &str, default: Sound) -> IdOr<basic::SoundEvent> {
    if let Some(sound) = nbt.get_string(key) {
        let sound = sound.strip_prefix("minecraft:").unwrap_or(sound);
        IdOr::Id(Sound::from_name(sound).unwrap_or(default))
    } else if let Some(sound_compound) = nbt.get_compound(key) {
        let Some(sound_name) = sound_compound.get_string("sound_id") else {
            return IdOr::Id(default);
        };
        let range = sound_compound.get_float("range");
        IdOr::Value(basic::SoundEvent {
            sound_name: sound_name.to_string(),
            range,
        })
    } else {
        IdOr::Id(default)
    }
}

/// Hash of a `RegistryCodecs.holderSet`: `#tag`, a single id, or a list of ids
/// (`ExtraCodecs.compactListCodec`).
#[must_use]
pub fn get_idset_hash<T: IDSetContent>(val: &IDSet<T>) -> u32 {
    match val {
        IDSet::Tag(tag) => get_str_hash(&format!("#{}", namespaced(tag))),
        IDSet::IDs(ids) => {
            let id_hash = |id: &&T| get_str_hash(&namespaced(&id.to_string()));
            if let [id] = ids.as_ref() {
                id_hash(id)
            } else {
                get_list_hash(ids.iter().map(id_hash))
            }
        }
    }
}

pub trait IDSetContent {
    fn registry_id(&self) -> u16;
    fn to_string(&self) -> String;
    fn from_id(id: u16) -> Option<&'static Self>;
    fn from_str(name: &str) -> Option<&'static Self>;
}

impl IDSetContent for Block {
    fn registry_id(&self) -> u16 {
        Taggable::registry_id(self)
    }

    fn from_id(id: u16) -> Option<&'static Self> {
        BlockId::new(id).map(Self::from_id)
    }

    fn from_str(name: &str) -> Option<&'static Self> {
        Block::from_name(name)
    }

    fn to_string(&self) -> String {
        self.name.to_string()
    }
}

impl IDSetContent for crate::item::Item {
    fn registry_id(&self) -> u16 {
        self.id
    }

    fn from_id(id: u16) -> Option<&'static Self> {
        crate::item::Item::from_id(id)
    }

    fn from_str(name: &str) -> Option<&'static Self> {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        crate::item::Item::from_registry_key(name)
    }

    fn to_string(&self) -> String {
        self.registry_key.to_string()
    }
}

#[derive(Clone, Hash, PartialEq, Debug)]
pub enum IDSet<T: IDSetContent + 'static> {
    Tag(Cow<'static, str>),
    IDs(Cow<'static, [&'static T]>),
}

impl<T: IDSetContent + 'static> IDSet<T> {
    pub fn read(data: &NbtTag) -> Option<Self> {
        match data.clone() {
            NbtTag::String(tag) => {
                if tag.starts_with("#") {
                    Some(Self::Tag(Cow::Owned(tag.strip_prefix("#")?.to_string())))
                } else {
                    Some(Self::IDs(Cow::Owned([T::from_str(tag.as_ref())?].to_vec())))
                }
            }
            NbtTag::List(nbt_tags) => {
                let mut ids = Vec::<&T>::new();
                for nbt in nbt_tags {
                    if let NbtTag::String(id) = nbt
                        && let Some(instance) = T::from_str(id.as_ref())
                    {
                        ids.push(instance);
                    }
                }
                Some(Self::IDs(Cow::Owned(ids)))
            }
            _ => None,
        }
    }

    pub fn write(&self, compound: &mut NbtCompound, key: &str) {
        match self {
            Self::Tag(cow) => {
                let mut tag = cow.to_string();
                tag.insert(0, '#');
                compound.put_string(key, tag);
            }
            // `ExtraCodecs.compactListCodec` writes a single id without the list.
            Self::IDs(arr) if arr.len() == 1 => {
                compound.put_string(key, namespaced(&arr[0].to_string()).into_owned());
            }
            Self::IDs(arr) => {
                let id_vec: Vec<NbtTag> = arr
                    .iter()
                    .map(|x| NbtTag::String(x.to_string().into()))
                    .collect();
                compound.put_list(key, id_vec);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum EquipmentType {
    Hand,
    HumanoidArmor,
    AnimalArmor,
    Saddle,
}

#[derive(Clone, Hash, Eq, PartialEq)]
pub struct EquipmentSlotData {
    pub slot_type: EquipmentType,
    pub entity_id: i32,
    pub max_count: i32,
    pub index: i32,
    pub name: Cow<'static, str>,
}

#[derive(Clone, Hash, Eq, PartialEq)]
#[repr(i8)]
pub enum EquipmentSlot {
    MainHand(EquipmentSlotData),
    OffHand(EquipmentSlotData),
    Feet(EquipmentSlotData),
    Legs(EquipmentSlotData),
    Chest(EquipmentSlotData),
    Head(EquipmentSlotData),
    Body(EquipmentSlotData),
    Saddle(EquipmentSlotData),
}

impl EquipmentSlot {
    pub const MAIN_HAND: Self = Self::MainHand(EquipmentSlotData {
        slot_type: EquipmentType::Hand,
        entity_id: 0,
        index: 0,
        max_count: 0,
        name: Cow::Borrowed("mainhand"),
    });
    pub const OFF_HAND: Self = Self::OffHand(EquipmentSlotData {
        slot_type: EquipmentType::Hand,
        entity_id: 1,
        index: 5,
        max_count: 0,
        name: Cow::Borrowed("offhand"),
    });
    pub const FEET: Self = Self::Feet(EquipmentSlotData {
        slot_type: EquipmentType::HumanoidArmor,
        entity_id: 0,
        index: 1,
        max_count: 1,
        name: Cow::Borrowed("feet"),
    });
    pub const LEGS: Self = Self::Legs(EquipmentSlotData {
        slot_type: EquipmentType::HumanoidArmor,
        entity_id: 1,
        index: 2,
        max_count: 1,
        name: Cow::Borrowed("legs"),
    });
    pub const CHEST: Self = Self::Chest(EquipmentSlotData {
        slot_type: EquipmentType::HumanoidArmor,
        entity_id: 2,
        index: 3,
        max_count: 1,
        name: Cow::Borrowed("chest"),
    });
    pub const HEAD: Self = Self::Head(EquipmentSlotData {
        slot_type: EquipmentType::HumanoidArmor,
        entity_id: 3,
        index: 4,
        max_count: 1,
        name: Cow::Borrowed("head"),
    });
    pub const BODY: Self = Self::Body(EquipmentSlotData {
        slot_type: EquipmentType::AnimalArmor,
        entity_id: 0,
        index: 6,
        max_count: 1,
        name: Cow::Borrowed("body"),
    });
    pub const SADDLE: Self = Self::Saddle(EquipmentSlotData {
        slot_type: EquipmentType::Saddle,
        entity_id: 0,
        index: 7,
        max_count: 1,
        name: Cow::Borrowed("saddle"),
    });
    /// Vanilla `EquipmentSlot.VALUES`, in declaration order.
    pub const VALUES: [Self; 8] = [
        Self::MAIN_HAND,
        Self::OFF_HAND,
        Self::FEET,
        Self::LEGS,
        Self::CHEST,
        Self::HEAD,
        Self::BODY,
        Self::SADDLE,
    ];

    #[must_use]
    pub const fn get_entity_slot_id(&self) -> i32 {
        match self {
            Self::MainHand(data) => data.entity_id,
            Self::OffHand(data) => data.entity_id,
            Self::Feet(data) => data.entity_id,
            Self::Legs(data) => data.entity_id,
            Self::Chest(data) => data.entity_id,
            Self::Head(data) => data.entity_id,
            Self::Body(data) => data.entity_id,
            Self::Saddle(data) => data.entity_id,
        }
    }

    #[must_use]
    pub const fn get_slot_index(&self) -> i32 {
        match self {
            Self::MainHand(data) => data.index,
            Self::OffHand(data) => data.index,
            Self::Feet(data) => data.index,
            Self::Legs(data) => data.index,
            Self::Chest(data) => data.index,
            Self::Head(data) => data.index,
            Self::Body(data) => data.index,
            Self::Saddle(data) => data.index,
        }
    }

    #[must_use]
    pub fn from_slot_index(name: i32) -> Option<&'static Self> {
        match name {
            0 => Some(&Self::MAIN_HAND),
            1 => Some(&Self::FEET),
            2 => Some(&Self::LEGS),
            3 => Some(&Self::CHEST),
            4 => Some(&Self::HEAD),
            5 => Some(&Self::OFF_HAND),
            6 => Some(&Self::BODY),
            7 => Some(&Self::SADDLE),
            _ => None,
        }
    }

    #[must_use]
    pub fn get_from_name(name: &str) -> Option<&'static Self> {
        match name {
            "mainhand" => Some(&Self::MAIN_HAND),
            "offhand" => Some(&Self::OFF_HAND),
            "feet" => Some(&Self::FEET),
            "legs" => Some(&Self::LEGS),
            "chest" => Some(&Self::CHEST),
            "head" => Some(&Self::HEAD),
            "body" => Some(&Self::BODY),
            "saddle" => Some(&Self::SADDLE),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_name(&self) -> &Cow<'static, str> {
        match self {
            Self::MainHand(data) => &data.name,
            Self::OffHand(data) => &data.name,
            Self::Feet(data) => &data.name,
            Self::Legs(data) => &data.name,
            Self::Chest(data) => &data.name,
            Self::Head(data) => &data.name,
            Self::Body(data) => &data.name,
            Self::Saddle(data) => &data.name,
        }
    }

    #[must_use]
    pub const fn get_offset_entity_slot_id(&self, offset: i32) -> i32 {
        self.get_entity_slot_id() + offset
    }

    #[must_use]
    pub const fn slot_type(&self) -> EquipmentType {
        match self {
            Self::MainHand(data) => data.slot_type,
            Self::OffHand(data) => data.slot_type,
            Self::Feet(data) => data.slot_type,
            Self::Legs(data) => data.slot_type,
            Self::Chest(data) => data.slot_type,
            Self::Head(data) => data.slot_type,
            Self::Body(data) => data.slot_type,
            Self::Saddle(data) => data.slot_type,
        }
    }

    #[must_use]
    pub const fn is_armor_slot(&self) -> bool {
        matches!(
            self.slot_type(),
            EquipmentType::HumanoidArmor | EquipmentType::AnimalArmor
        )
    }

    /// Vanilla `EquipmentSlot.canIncreaseExperience`.
    #[must_use]
    pub const fn can_increase_experience(&self) -> bool {
        !matches!(self.slot_type(), EquipmentType::Saddle)
    }

    #[must_use]
    pub const fn discriminant(&self) -> i8 {
        match self {
            Self::MainHand(_) => 0,
            Self::OffHand(_) => 1,
            Self::Feet(_) => 2,
            Self::Legs(_) => 3,
            Self::Chest(_) => 4,
            Self::Head(_) => 5,
            Self::Body(_) => 6,
            Self::Saddle(_) => 7,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum EntityTypeOrTag {
    Tag(&'static crate::tag::Tag),
    Single(&'static EntityType),
}

impl std::hash::Hash for EntityTypeOrTag {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Tag(tag) => {
                for x in tag.0 {
                    x.hash(state);
                }
            }
            Self::Single(entity_type) => {
                entity_type.id.hash(state);
            }
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum IdOr<T> {
    Id(Sound),
    Value(T),
}

/// A registry holder that may also carry its value inline, like a vanilla
/// `Holder` encoded with `ByteBufCodecs.holder` or `RegistryCodecs.holder`
/// with a direct codec.
#[derive(Clone, Debug, PartialEq)]
pub enum Holder<R, D> {
    Reference(R),
    Direct(D),
}

/// The id a client gives `name` in a synced registry, which is its position in
/// the registry data Pumpkin sends during configuration.
#[must_use]
pub fn synced_registry_id(registry: &str, name: &str) -> Option<u32> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    crate::registry::REGISTRY_V_26_3
        .iter()
        .find(|reg| reg.registry_id == registry)?
        .entries
        .iter()
        .position(|entry| entry.name == name)
        .and_then(|id| u32::try_from(id).ok())
}

/// The entry name at `id` in a synced registry, see `synced_registry_id`.
#[must_use]
pub fn synced_registry_entry(registry: &str, id: u32) -> Option<&'static str> {
    crate::registry::REGISTRY_V_26_3
        .iter()
        .find(|reg| reg.registry_id == registry)?
        .entries
        .get(usize::try_from(id).ok()?)
        .map(|entry| entry.name)
}

/// Hash of NBT written by a codec whose only byte fields are booleans, such
/// as text components or inline trim patterns: their bytes hash as booleans.
#[must_use]
pub fn get_codec_nbt_hash(tag: &NbtTag) -> u32 {
    match tag {
        NbtTag::Byte(value) => get_bool_hash(*value != 0),
        NbtTag::List(values) => get_list_hash(values.iter().map(get_codec_nbt_hash)),
        NbtTag::Compound(compound) => get_map_hash(
            compound
                .child_tags
                .iter()
                .map(|(key, value)| (get_str_hash(key), get_codec_nbt_hash(value))),
        ),
        other => get_nbt_hash(other),
    }
}

/// Hash of a text component as `ComponentSerialization.CODEC` encodes it.
#[must_use]
pub fn get_text_hash(text: &TextComponent) -> u32 {
    get_codec_nbt_hash(&text_to_nbt(text))
}

/// `ComponentSerialization.CODEC` NBT form of a text component.
#[must_use]
pub fn text_to_nbt(text: &TextComponent) -> NbtTag {
    text.to_nbt_tag_for_version(&JavaMinecraftVersion::V_26_3)
}

/// Reads an `ItemStackTemplate.CODEC` value: an item id, or `{id, count, components}`
/// where `count` defaults to 1.
#[must_use]
pub fn read_item_stack_template(tag: &NbtTag) -> Option<crate::item_stack::ItemStack> {
    match tag {
        NbtTag::String(id) => {
            let item =
                crate::item::Item::from_registry_key(id.strip_prefix("minecraft:").unwrap_or(id))?;
            Some(crate::item_stack::ItemStack::new(1, item))
        }
        NbtTag::Compound(compound) => {
            if compound.has("count") {
                crate::item_stack::ItemStack::read_item_stack(compound)
            } else {
                let mut compound = compound.clone();
                compound.put_int("count", 1);
                crate::item_stack::ItemStack::read_item_stack(&compound)
            }
        }
        _ => None,
    }
}

/// Writes an `ItemStackTemplate.CODEC` value, leaving out a count of 1 and an
/// empty component patch like vanilla's optional fields do.
#[must_use]
pub fn write_item_stack_template(stack: &crate::item_stack::ItemStack) -> NbtTag {
    let mut compound = NbtCompound::new();
    stack.write_item_stack(&mut compound);
    if compound.get_int("count") == Some(1) {
        compound.child_tags.remove("count");
    }
    if compound
        .get_compound("components")
        .is_some_and(|components| components.child_tags.is_empty())
    {
        compound.child_tags.remove("components");
    }
    NbtTag::Compound(compound)
}

/// Hash of an `ItemStackTemplate.CODEC` value.
#[must_use]
pub fn get_item_stack_template_hash(stack: &crate::item_stack::ItemStack) -> u32 {
    let mut fields = MapHash::new().field("id", get_str_hash(&namespaced(stack.item.registry_key)));
    if stack.item_count != 1 {
        fields = fields.field("count", get_i32_hash(i32::from(stack.item_count)));
    }
    let components: Vec<(u32, u32)> = stack
        .patch
        .iter()
        .map(|(id, data)| match data {
            Some(data) => (get_str_hash(id.to_name()), data.get_hash() as u32),
            None => (
                get_str_hash(&format!("!{}", id.to_name())),
                get_map_hash([]),
            ),
        })
        .collect();
    if !components.is_empty() {
        fields = fields.field("components", get_map_hash(components));
    }
    fields.finish()
}

/// Whether two item stack templates hold the same item, count and components.
#[must_use]
pub fn item_stack_template_eq(
    a: &crate::item_stack::ItemStack,
    b: &crate::item_stack::ItemStack,
) -> bool {
    a.item_count == b.item_count && a.are_items_and_components_equal(b)
}

pub mod basic;
pub mod block_entity;
pub mod book;
pub mod combat;
pub mod entity_variant;
pub mod food;
pub mod utility;

// Re-export all component implementations
pub use basic::*;
pub use block_entity::*;
pub use book::*;
pub use combat::*;
pub use entity_variant::*;
pub use food::*;
pub use utility::*;

#[must_use]
pub fn read_data(id: DataComponent, data: &NbtTag) -> Option<Box<dyn DataComponentImpl>> {
    match id {
        DataComponent::MaxStackSize => Some(MaxStackSizeImpl::read_data(data)?.to_dyn()),
        DataComponent::CustomData => Some(CustomDataImpl::read_data(data)?.to_dyn()),
        DataComponent::Enchantments => Some(EnchantmentsImpl::read_data(data)?.to_dyn()),
        DataComponent::Damage => Some(DamageImpl::read_data(data)?.to_dyn()),
        DataComponent::MaxDamage => Some(MaxDamageImpl::read_data(data)?.to_dyn()),
        DataComponent::Unbreakable => Some(UnbreakableImpl::read_data(data)?.to_dyn()),
        DataComponent::Food => Some(FoodImpl::read_data(data)?.to_dyn()),
        DataComponent::Tool => Some(ToolImpl::read_data(data)?.to_dyn()),
        DataComponent::Enchantable => Some(EnchantableImpl::read_data(data)?.to_dyn()),
        DataComponent::DamageResistant => Some(DamageResistantImpl::read_data(data)?.to_dyn()),
        DataComponent::PotionContents => Some(PotionContentsImpl::read_data(data)?.to_dyn()),
        DataComponent::PotionDurationScale => {
            Some(PotionDurationScaleImpl::read_data(data)?.to_dyn())
        }
        DataComponent::SuspiciousStewEffects => {
            Some(SuspiciousStewEffectsImpl::read_data(data)?.to_dyn())
        }
        DataComponent::Fireworks => Some(FireworksImpl::read_data(data)?.to_dyn()),
        DataComponent::FireworkExplosion => Some(FireworkExplosionImpl::read_data(data)?.to_dyn()),
        DataComponent::CustomName => Some(CustomNameImpl::read_data(data)?.to_dyn()),
        DataComponent::Lore => Some(LoreImpl::read_data(data)?.to_dyn()),
        DataComponent::ItemName => Some(ItemNameImpl::read_data(data)?.to_dyn()),
        DataComponent::ItemModel => Some(ItemModelImpl::read_data(data)?.to_dyn()),
        DataComponent::Consumable => Some(ConsumableImpl::read_data(data)?.to_dyn()),
        DataComponent::Equippable => Some(EquippableImpl::read_data(data)?.to_dyn()),
        DataComponent::AttackRange => Some(AttackRangeImpl::read_data(data)?.to_dyn()),
        DataComponent::KineticWeapon => Some(KineticWeaponImpl::read_data(data)?.to_dyn()),
        DataComponent::PiercingWeapon => Some(PiercingWeaponImpl::read_data(data)?.to_dyn()),
        DataComponent::MinimumAttackCharge => {
            Some(MinimumAttackChargeImpl::read_data(data)?.to_dyn())
        }
        DataComponent::DamageType => Some(DamageTypeImpl::read_data(data)?.to_dyn()),
        DataComponent::StoredEnchantments => {
            Some(StoredEnchantmentsImpl::read_data(data)?.to_dyn())
        }
        DataComponent::UseCooldown => Some(UseCooldownImpl::read_data(data)?.to_dyn()),
        DataComponent::RepairCost => Some(RepairCostImpl::read_data(data)?.to_dyn()),
        DataComponent::Repairable => Some(RepairableImpl::read_data(data)?.to_dyn()),
        DataComponent::MapId => Some(MapIdImpl::read_data(data)?.to_dyn()),
        DataComponent::MapPostProcessing => Some(MapPostProcessingImpl::read_data(data)?.to_dyn()),
        DataComponent::ChargedProjectiles => {
            Some(ChargedProjectilesImpl::read_data(data)?.to_dyn())
        }
        DataComponent::BlockEntityData => Some(BlockEntityDataImpl::read_data(data)?.to_dyn()),
        DataComponent::BundleContents => Some(BundleContentsImpl::read_data(data)?.to_dyn()),
        DataComponent::Container => Some(ContainerImpl::read_data(data)?.to_dyn()),
        DataComponent::WrittenBookContent => {
            Some(WrittenBookContentImpl::read_data(data)?.to_dyn())
        }
        DataComponent::WritableBookContent => {
            Some(WritableBookContentImpl::read_data(data)?.to_dyn())
        }
        DataComponent::OminousBottleAmplifier => {
            Some(OminousBottleAmplifierImpl::read_data(data)?.to_dyn())
        }
        DataComponent::BlockState => Some(BlockStateImpl::read_data(data)?.to_dyn()),
        DataComponent::Profile => Some(ProfileImpl::read_data(data)?.to_dyn()),
        DataComponent::ChickenVariant => Some(ChickenVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::VillagerVariant => Some(VillagerVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::WolfVariant => Some(WolfVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::WolfSoundVariant => Some(WolfSoundVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::WolfCollar => Some(WolfCollarImpl::read_data(data)?.to_dyn()),
        DataComponent::FoxVariant => Some(FoxVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::SalmonSize => Some(SalmonSizeImpl::read_data(data)?.to_dyn()),
        DataComponent::ParrotVariant => Some(ParrotVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::TropicalFishPattern => {
            Some(TropicalFishPatternImpl::read_data(data)?.to_dyn())
        }
        DataComponent::TropicalFishBaseColor => {
            Some(TropicalFishBaseColorImpl::read_data(data)?.to_dyn())
        }
        DataComponent::TropicalFishPatternColor => {
            Some(TropicalFishPatternColorImpl::read_data(data)?.to_dyn())
        }
        DataComponent::MooshroomVariant => Some(MooshroomVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::RabbitVariant => Some(RabbitVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::PigVariant => Some(PigVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::PigSoundVariant => Some(PigSoundVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::CowVariant => Some(CowVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::CowSoundVariant => Some(CowSoundVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::ChickenSoundVariant => {
            Some(ChickenSoundVariantImpl::read_data(data)?.to_dyn())
        }
        DataComponent::ZombieNautilusVariant => {
            Some(ZombieNautilusVariantImpl::read_data(data)?.to_dyn())
        }
        DataComponent::FrogVariant => Some(FrogVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::HorseVariant => Some(HorseVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::PaintingVariant => Some(PaintingVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::LlamaVariant => Some(LlamaVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::AxolotlVariant => Some(AxolotlVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::CatVariant => Some(CatVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::CatSoundVariant => Some(CatSoundVariantImpl::read_data(data)?.to_dyn()),
        DataComponent::CatCollar => Some(CatCollarImpl::read_data(data)?.to_dyn()),
        DataComponent::SheepColor => Some(SheepColorImpl::read_data(data)?.to_dyn()),
        DataComponent::ShulkerColor => Some(ShulkerColorImpl::read_data(data)?.to_dyn()),
        DataComponent::DyedColor => Some(DyedColorImpl::read_data(data)?.to_dyn()),
        DataComponent::BaseColor => Some(BaseColorImpl::read_data(data)?.to_dyn()),
        DataComponent::NoteBlockSound => Some(NoteBlockSoundImpl::read_data(data)?.to_dyn()),
        DataComponent::TooltipStyle => Some(TooltipStyleImpl::read_data(data)?.to_dyn()),
        DataComponent::Lock => Some(LockImpl::read_data(data)?.to_dyn()),
        DataComponent::ContainerLoot => Some(ContainerLootImpl::read_data(data)?.to_dyn()),
        DataComponent::CustomModelData => Some(CustomModelDataImpl::read_data(data)?.to_dyn()),
        DataComponent::LodestoneTracker => Some(LodestoneTrackerImpl::read_data(data)?.to_dyn()),
        DataComponent::Glider => Some(GliderImpl::read_data(data)?.to_dyn()),
        DataComponent::IntangibleProjectile => {
            Some(IntangibleProjectileImpl::read_data(data)?.to_dyn())
        }
        DataComponent::Trim => Some(TrimImpl::read_data(data)?.to_dyn()),
        DataComponent::CanPlaceOn => Some(CanPlaceOnImpl::read_data(data)?.to_dyn()),
        DataComponent::CanBreak => Some(CanBreakImpl::read_data(data)?.to_dyn()),
        DataComponent::AttackAnimation => Some(SwingAnimationImpl::read_data(data)?.to_dyn()),
        DataComponent::Rarity => Some(RarityImpl::read_data(data)?.to_dyn()),
        DataComponent::BannerPatterns => Some(BannerPatternsImpl::read_data(data)?.to_dyn()),
        DataComponent::UseEffects => Some(UseEffectsImpl::read_data(data)?.to_dyn()),
        DataComponent::UseRemainder => Some(UseRemainderImpl::read_data(data)?.to_dyn()),
        DataComponent::Weapon => Some(WeaponImpl::read_data(data)?.to_dyn()),
        DataComponent::TooltipDisplay => Some(TooltipDisplayImpl::read_data(data)?.to_dyn()),
        DataComponent::CreativeSlotLock => Some(CreativeSlotLockImpl::read_data(data)?.to_dyn()),
        DataComponent::EnchantmentGlintOverride => {
            Some(EnchantmentGlintOverrideImpl::read_data(data)?.to_dyn())
        }
        DataComponent::DeathProtection => Some(DeathProtectionImpl::read_data(data)?.to_dyn()),
        DataComponent::BlocksAttacks => Some(BlocksAttacksImpl::read_data(data)?.to_dyn()),
        DataComponent::AdditionalTradeCost => {
            Some(AdditionalTradeCostImpl::read_data(data)?.to_dyn())
        }
        DataComponent::Dye => Some(DyeImpl::read_data(data)?.to_dyn()),
        DataComponent::MapDecorations => Some(MapDecorationsImpl::read_data(data)?.to_dyn()),
        DataComponent::DebugStickState => Some(DebugStickStateImpl::read_data(data)?.to_dyn()),
        DataComponent::EntityData => Some(EntityDataImpl::read_data(data)?.to_dyn()),
        DataComponent::BucketEntityData => Some(BucketEntityDataImpl::read_data(data)?.to_dyn()),
        DataComponent::Instrument => Some(InstrumentImpl::read_data(data)?.to_dyn()),
        DataComponent::ProvidesTrimMaterial => {
            Some(ProvidesTrimMaterialImpl::read_data(data)?.to_dyn())
        }
        DataComponent::JukeboxPlayable => Some(JukeboxPlayableImpl::read_data(data)?.to_dyn()),
        DataComponent::ProvidesBannerPatterns => {
            Some(ProvidesBannerPatternsImpl::read_data(data)?.to_dyn())
        }
        DataComponent::Recipes => Some(RecipesImpl::read_data(data)?.to_dyn()),
        DataComponent::PotDecorations => Some(PotDecorationsImpl::read_data(data)?.to_dyn()),
        DataComponent::Bees => Some(BeesImpl::read_data(data)?.to_dyn()),
        DataComponent::SulfurCubeContent => Some(SulfurCubeContentImpl::read_data(data)?.to_dyn()),
        DataComponent::BreakSound => Some(BreakSoundImpl::read_data(data)?.to_dyn()),
        DataComponent::AttributeModifiers => {
            Some(AttributeModifiersImpl::read_data(data)?.to_dyn())
        }
        DataComponent::BrewingFuel => Some(BrewingFuelImpl::read_data(data)?.to_dyn()),
        DataComponent::CookingFuel => Some(CookingFuelImpl::read_data(data)?.to_dyn()),
        DataComponent::Compostable => Some(CompostableImpl::read_data(data)?.to_dyn()),
        DataComponent::Waxed => Some(WaxedImpl::read_data(data)?.to_dyn()),
        DataComponent::InteractAnimation => Some(InteractAnimationImpl::read_data(data)?.to_dyn()),
        DataComponent::BlockTransformer => Some(BlockTransformerImpl::read_data(data)?.to_dyn()),
        DataComponent::VillagerFood => Some(VillagerFoodImpl::read_data(data)?.to_dyn()),
        DataComponent::MobVisibility => Some(MobVisibilityImpl::read_data(data)?.to_dyn()),
        DataComponent::ProvidesPotteryPattern => {
            Some(ProvidesPotteryPatternImpl::read_data(data)?.to_dyn())
        }
        DataComponent::SignTextFront => Some(SignTextFrontImpl::read_data(data)?.to_dyn()),
        DataComponent::SignTextBack => Some(SignTextBackImpl::read_data(data)?.to_dyn()),
        DataComponent::CushionColor => Some(CushionColorImpl::read_data(data)?.to_dyn()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash() {
        assert_eq!(get_str_hash("minecraft:sharpness"), 2734053906u32);
        assert_eq!(get_i32_hash(3), 3795317917u32);
        assert_eq!(
            EnchantmentsImpl {
                enchantment: Cow::Borrowed(&[(&crate::Enchantment::SHARPNESS, 2)]),
            }
            .get_hash(),
            -1580618251i32
        );
        // Map entries are sorted by hash, so insertion order does not matter.
        let sharpness = (&crate::Enchantment::SHARPNESS, 5);
        let unbreaking = (&crate::Enchantment::UNBREAKING, 3);
        for enchantment in [vec![sharpness, unbreaking], vec![unbreaking, sharpness]] {
            assert_eq!(
                EnchantmentsImpl {
                    enchantment: Cow::Owned(enchantment),
                }
                .get_hash(),
                1_231_404_551
            );
        }
        assert_eq!(MaxStackSizeImpl { size: 99 }.get_hash(), -1632321551i32);
        assert_eq!(MapIdImpl { id: 10 }.get_hash(), -919192125i32);
    }

    // Expected hashes below come from vanilla 26.3: the component's codec
    // encoded with `HashOps.CRC32C_INSTANCE` through registry ops.

    #[test]
    fn hash_ops_scalars() {
        // `putUnencodedChars` hashes UTF-16 code units, not UTF-8 bytes.
        assert_eq!(get_str_hash("é𝄞") as i32, -165_708_647);
        // Floats carry the FLOAT tag (10), not SHORT.
        assert_eq!(
            PotionDurationScaleImpl { scale: 0.5 }.get_hash(),
            -1_631_070_359
        );
        assert_eq!(
            EnchantmentGlintOverrideImpl { has_glint: true }.get_hash(),
            -1_019_818_302
        );
        assert_eq!(
            EnchantmentGlintOverrideImpl { has_glint: false }.get_hash(),
            828_198_337
        );
        // `Unit.CODEC` encodes an empty map.
        assert_eq!(UnbreakableImpl.get_hash(), -982_207_288);
        assert_eq!(GliderImpl.get_hash(), -982_207_288);
    }

    #[test]
    fn hash_custom_data_like_nbt_ops() {
        let mut nbt = NbtCompound::new();
        nbt.put_int("Age", 5);
        nbt.put_float("Health", 10.0);
        nbt.put_string("Name", "é".to_string());
        assert_eq!(
            BucketEntityDataImpl {
                nbt: Some(nbt.clone())
            }
            .get_hash(),
            -1_842_861_489
        );
        assert_eq!(CustomDataImpl { data: nbt }.get_hash(), -1_842_861_489);
    }

    #[test]
    fn hash_records_omit_default_fields() {
        assert_eq!(
            UseEffectsImpl {
                can_sprint: true,
                interact_vibrations: false,
                speed_multiplier: 0.5,
            }
            .get_hash(),
            1_668_803_792
        );
        assert_eq!(UseEffectsImpl::DEFAULT.get_hash(), -982_207_288);
        assert_eq!(TooltipDisplayImpl::DEFAULT.get_hash(), -982_207_288);
        assert_eq!(
            TooltipDisplayImpl {
                hide_tooltip: true,
                hidden_components: vec![DataComponent::CustomName, DataComponent::Lore],
            }
            .get_hash(),
            -1_090_191_763
        );
        assert_eq!(SwingAnimationImpl::DEFAULT.get_hash(), -982_207_288);
        assert_eq!(
            InteractAnimationImpl(SwingAnimationImpl {
                animation_type: SwingAnimationType::Stab,
                duration: 10,
            })
            .get_hash(),
            -2_140_542_379
        );
    }

    #[test]
    fn hash_registry_values() {
        assert_eq!(
            DyeImpl {
                color: crate::dye_color::DyeColor::Red
            }
            .get_hash(),
            -1_939_582_294
        );
        assert_eq!(VillagerFoodImpl { nutrition: 4 }.get_hash(), -964_794_316);
        assert_eq!(
            MobVisibilityImpl {
                targeting_entity_types: IDSet::IDs(Cow::Borrowed(&[&EntityType::ZOMBIE])),
                visibility: 0.5,
            }
            .get_hash(),
            -407_642_323
        );
        assert_eq!(
            RecipesImpl {
                recipes: vec!["minecraft:stick".to_string()]
            }
            .get_hash(),
            -533_729_841
        );
    }

    fn custom_horn_sound() -> IdOr<SoundEvent> {
        IdOr::Value(SoundEvent {
            sound_name: "minecraft:custom.horn".to_string(),
            range: Some(32.0),
        })
    }

    #[test]
    fn hash_sound_holders() {
        assert_eq!(
            BreakSoundImpl {
                sound: IdOr::Id(Sound::EntityItemBreak)
            }
            .get_hash(),
            123_447_120
        );
        assert_eq!(
            BreakSoundImpl {
                sound: custom_horn_sound()
            }
            .get_hash(),
            786_858_243
        );
    }

    #[test]
    fn hash_inline_holders() {
        assert_eq!(
            InstrumentImpl {
                instrument: Holder::Direct(Box::new(InstrumentDefinition {
                    sound_event: custom_horn_sound(),
                    use_duration: 7.0,
                    range: 256.0,
                    durability_damage: 2,
                    description: TextComponent::text("Horn"),
                })),
            }
            .get_hash(),
            -1_895_168_128
        );
        assert_eq!(
            ProvidesTrimMaterialImpl {
                material: Holder::Direct(TrimMaterialDefinition {
                    palette_id: "minecraft:ruby".to_string(),
                    description: TextComponent::text("Ruby"),
                }),
            }
            .get_hash(),
            72_354_046
        );
    }

    #[test]
    fn hash_resolvable_numbers() {
        assert_eq!(
            CompostableImpl {
                layers: IntProvider::Id(Cow::Borrowed("minecraft:compostable/low")),
            }
            .get_hash(),
            2_075_224_535
        );
        assert_eq!(
            BrewingFuelImpl {
                uses: IntProvider::Inline(3),
                speed_multiplier: FloatProvider::Inline(1.5),
            }
            .get_hash(),
            1_256_775_528
        );
        assert_eq!(
            CookingFuelImpl {
                burn_time: IntProvider::Inline(1600),
                speed_multiplier: FloatProvider::Id(Cow::Borrowed(
                    "minecraft:cooking/speed_default"
                )),
            }
            .get_hash(),
            1_144_219_266
        );
    }

    fn strong_swiftness_contents() -> PotionContentsImpl {
        PotionContentsImpl {
            potion_id: Some(i32::from(crate::potion::Potion::STRONG_SWIFTNESS.id)),
            custom_color: Some(0xFF00FF),
            custom_effects: vec![StatusEffectInstance {
                effect_id: Cow::Borrowed("minecraft:speed"),
                amplifier: 1,
                duration: 200,
                ambient: false,
                show_particles: false,
                show_icon: true,
                hidden_effect: None,
            }],
            custom_name: None,
        }
    }

    #[test]
    fn potion_contents_hash() {
        assert_eq!(strong_swiftness_contents().get_hash(), -520_524_062);
    }

    #[test]
    fn potion_contents_nbt_is_vanilla_shaped() {
        // {custom_color:16711935,custom_effects:[{amplifier:1b,duration:200,
        //  id:"minecraft:speed",show_icon:1b,show_particles:0b}],potion:"minecraft:strong_swiftness"}
        let mut effect = NbtCompound::new();
        effect.put_byte("amplifier", 1);
        effect.put_int("duration", 200);
        effect.put_string("id", "minecraft:speed".to_string());
        effect.put_bool("show_icon", true);
        effect.put_bool("show_particles", false);
        let mut vanilla = NbtCompound::new();
        vanilla.put_int("custom_color", 0xFF00FF);
        vanilla.put_list("custom_effects", vec![NbtTag::Compound(effect)]);
        vanilla.put_string("potion", "minecraft:strong_swiftness".to_string());
        let vanilla = NbtTag::Compound(vanilla);

        assert_eq!(strong_swiftness_contents().write_data(), vanilla);
        assert_eq!(
            PotionContentsImpl::read_data(&vanilla).expect("vanilla potion NBT should read"),
            strong_swiftness_contents()
        );
        // The bare potion id form of `PotionContents.CODEC`.
        let bare = PotionContentsImpl::read_data(&NbtTag::String("minecraft:water".into()))
            .expect("a bare potion id should read");
        assert_eq!(
            bare.potion_id,
            Some(i32::from(crate::potion::Potion::WATER.id))
        );
    }

    #[test]
    fn status_effect_with_hidden_effect() {
        let effect = StatusEffectInstance {
            effect_id: Cow::Borrowed("minecraft:speed"),
            amplifier: 1,
            duration: 200,
            ambient: false,
            show_particles: false,
            show_icon: true,
            hidden_effect: Some(Box::new(StatusEffectInstance {
                effect_id: Cow::Borrowed("minecraft:speed"),
                amplifier: 0,
                duration: 100,
                ambient: false,
                show_particles: true,
                show_icon: true,
                hidden_effect: None,
            })),
        };
        assert_eq!(effect.get_hash(), 2_145_254_872);

        // {amplifier:1b,duration:200,hidden_effect:{duration:100,show_icon:1b},
        //  id:"minecraft:speed",show_icon:1b,show_particles:0b}
        let mut hidden = NbtCompound::new();
        hidden.put_int("duration", 100);
        hidden.put_bool("show_icon", true);
        let mut vanilla = NbtCompound::new();
        vanilla.put_byte("amplifier", 1);
        vanilla.put_int("duration", 200);
        vanilla.put_compound("hidden_effect", hidden);
        vanilla.put_string("id", "minecraft:speed".to_string());
        vanilla.put_bool("show_icon", true);
        vanilla.put_bool("show_particles", false);
        let vanilla = NbtTag::Compound(vanilla);

        assert_eq!(effect.as_nbt(), vanilla);
        assert_eq!(StatusEffectInstance::read_data(&vanilla), Some(effect));
    }

    #[test]
    fn status_effect_reads_missing_fields_as_defaults() {
        let mut nbt = NbtCompound::new();
        nbt.put_string("id", "minecraft:regeneration".to_string());
        nbt.put_bool("show_particles", false);
        let effect = StatusEffectInstance::read_data(&NbtTag::Compound(nbt))
            .expect("an effect with only an id should read");
        assert_eq!((effect.amplifier, effect.duration), (0, 0));
        assert!(!effect.ambient);
        // `show_icon` falls back to `show_particles`.
        assert!(!effect.show_icon);
    }

    #[test]
    fn hash_bees_and_templates() {
        let mut entity = NbtCompound::new();
        entity.put_int("Age", 5);
        entity.put_float("Health", 10.0);
        entity.put_string("Name", "é".to_string());
        entity.put_string("id", "minecraft:bee".to_string());
        assert_eq!(
            BeesImpl {
                bees: vec![BeeOccupant {
                    entity_data: entity,
                    ticks_in_hive: 100,
                    min_ticks_in_hive: 600,
                }],
            }
            .get_hash(),
            1_269_290_279
        );
        assert_eq!(
            PotDecorationsImpl {
                back: Some(crate::item_stack::ItemStack::new(
                    1,
                    &crate::item::Item::BRICK
                )),
                front: Some(crate::item_stack::ItemStack::new(
                    2,
                    &crate::item::Item::ARCHER_POTTERY_SHERD
                )),
                ..Default::default()
            }
            .get_hash(),
            -118_313_068
        );
        assert_eq!(
            SulfurCubeContentImpl {
                absorbed_block: crate::item_stack::ItemStack::new(3, &crate::item::Item::STONE),
            }
            .get_hash(),
            -1_751_786_989
        );
    }

    #[test]
    fn hash_sign_text_and_map_decorations() {
        let empty = || TextComponent::text("");
        assert_eq!(
            SignTextFrontImpl(SignText {
                messages: [TextComponent::text("Hi"), empty(), empty(), empty()],
                filtered_messages: None,
                color: crate::dye_color::DyeColor::Red,
                has_glowing_text: true,
            })
            .get_hash(),
            -1_435_795_150
        );
        assert_eq!(
            MapDecorationsImpl {
                decorations: vec![(
                    "+".to_string(),
                    MapDecorationEntry {
                        r#type: "minecraft:target_point".to_string(),
                        x: 12.5,
                        z: -3.0,
                        rotation: 180.0,
                    },
                )],
            }
            .get_hash(),
            1_654_822_948
        );
    }

    #[test]
    fn hash_consume_effect_components() {
        let regeneration = StatusEffectInstance {
            effect_id: Cow::Borrowed("minecraft:regeneration"),
            amplifier: 1,
            duration: 900,
            ambient: false,
            show_particles: true,
            show_icon: true,
            hidden_effect: None,
        };
        assert_eq!(
            DeathProtectionImpl {
                death_effects: Cow::Owned(vec![
                    ConsumeEffect::ClearAllEffects,
                    ConsumeEffect::ApplyEffects((Cow::Owned(vec![regeneration]), 1.0)),
                ]),
            }
            .get_hash(),
            -1_943_729_730
        );
        assert_eq!(
            DeathProtectionImpl {
                death_effects: Cow::Owned(vec![ConsumeEffect::TeleportRandomly {
                    diameter: 16.0,
                    directional_particles: false,
                }]),
            }
            .get_hash(),
            -2_127_496_444
        );
        assert_eq!(
            BlocksAttacksImpl {
                block_delay_seconds: 0.25,
                disable_cooldown_scale: 1.0,
                damage_reductions: Cow::Borrowed(BlocksAttacksImpl::DEFAULT_DAMAGE_REDUCTIONS),
                item_damage: ItemDamageFunction {
                    threshold: 3.0,
                    base: 1.0,
                    factor: 1.0,
                },
                bypassed_by: None,
                block_sound: Some(IdOr::Id(Sound::ItemShieldBlock)),
                disabled_sound: None,
            }
            .get_hash(),
            -1_772_717_597
        );
    }

    #[test]
    fn holders_round_trip_through_nbt() {
        assert_round_trip(
            InstrumentImpl {
                instrument: Holder::Reference(crate::instrument::Instrument::SeekGoatHorn),
            },
            InstrumentImpl::read_data,
        );
        assert_round_trip(
            JukeboxPlayableImpl {
                song: Holder::Reference(crate::jukebox_song::JukeboxSong::Pigstep),
            },
            JukeboxPlayableImpl::read_data,
        );
        assert_round_trip(
            ProvidesBannerPatternsImpl {
                patterns: IDSet::Tag(Cow::Borrowed("pattern_item/flower")),
            },
            ProvidesBannerPatternsImpl::read_data,
        );
    }

    #[test]
    fn components_round_trip_through_nbt() {
        assert_round_trip(
            SignTextBackImpl(SignText {
                messages: std::array::from_fn(|i| TextComponent::text(format!("line {i}"))),
                filtered_messages: Some(std::array::from_fn(|_| TextComponent::text("***"))),
                color: crate::dye_color::DyeColor::Cyan,
                has_glowing_text: false,
            }),
            SignTextBackImpl::read_data,
        );
        assert_round_trip(
            DebugStickStateImpl {
                properties: vec![("minecraft:oak_stairs".to_string(), "facing".to_string())],
            },
            DebugStickStateImpl::read_data,
        );
        assert_round_trip(
            BlockTransformerImpl {
                transformer: Cow::Borrowed("minecraft:axe"),
            },
            BlockTransformerImpl::read_data,
        );
    }

    #[test]
    fn hash_enum_variants_by_name() {
        assert_eq!(
            FoxVariantImpl {
                value: Cow::Borrowed("snow")
            }
            .get_hash(),
            -1_511_535_024
        );
        assert_eq!(
            RabbitVariantImpl {
                value: Cow::Borrowed("evil")
            }
            .get_hash(),
            1_619_861_300
        );
    }

    #[test]
    fn hash_fireworks() {
        assert_eq!(
            FireworksImpl {
                flight_duration: 2,
                explosions: vec![FireworkExplosionImpl {
                    shape: FireworkExplosionShape::Star,
                    colors: vec![0xFF0000],
                    fade_colors: Vec::new(),
                    has_trail: true,
                    has_twinkle: false,
                }],
            }
            .get_hash(),
            1_870_020_809
        );
    }

    fn assert_round_trip<T: DataComponentImpl + Clone + 'static>(
        value: T,
        read: impl Fn(&NbtTag) -> Option<T>,
    ) {
        let restored = read(&value.write_data()).expect("read_data returned None");
        assert!(value.equal(&restored));
    }

    #[test]
    fn max_damage_round_trip() {
        assert_round_trip(MaxDamageImpl { max_damage: 1561 }, MaxDamageImpl::read_data);
    }

    #[test]
    fn enchantable_round_trip() {
        assert_round_trip(EnchantableImpl { value: 14 }, EnchantableImpl::read_data);
    }

    #[test]
    fn food_round_trip() {
        assert_round_trip(
            FoodImpl {
                nutrition: 4,
                saturation: 2.4,
                can_always_eat: true,
            },
            FoodImpl::read_data,
        );
    }

    #[test]
    fn block_entity_data_round_trip() {
        let mut nbt = NbtCompound::new();
        nbt.put_string("id", "minecraft:chest".to_string());
        nbt.put_int("x", 12);
        assert_round_trip(BlockEntityDataImpl { nbt }, BlockEntityDataImpl::read_data);
    }

    #[test]
    fn entity_data_round_trip() {
        let mut nbt = NbtCompound::new();
        nbt.put_string("id", "minecraft:iron_golem".to_string());
        nbt.put_bool("PlayerCreated", true);
        assert_round_trip(EntityDataImpl { nbt: Some(nbt) }, EntityDataImpl::read_data);
    }

    #[test]
    fn tool_round_trip() {
        assert_round_trip(
            ToolImpl {
                rules: Cow::Owned(vec![
                    ToolRule {
                        blocks: IDSet::Tag(Cow::Borrowed("mineable/pickaxe")),
                        speed: Some(6.0),
                        correct_for_drops: Some(true),
                    },
                    ToolRule {
                        blocks: IDSet::Tag(Cow::Borrowed("incorrect_for_wooden_tool")),
                        speed: None,
                        correct_for_drops: Some(false),
                    },
                ]),
                default_mining_speed: 1.0,
                damage_per_block: 2,
                can_destroy_blocks_in_creative: false,
            },
            ToolImpl::read_data,
        );
    }

    #[test]
    fn dyed_color_round_trip() {
        assert_round_trip(DyedColorImpl { rgb: 0x00A0F0 }, DyedColorImpl::read_data);
    }

    #[test]
    fn base_color_round_trip() {
        assert_round_trip(
            BaseColorImpl {
                color: "light_blue".to_string(),
            },
            BaseColorImpl::read_data,
        );
    }

    #[test]
    fn note_block_sound_round_trip() {
        assert_round_trip(
            NoteBlockSoundImpl {
                sound: "minecraft:block.note_block.pling".to_string(),
            },
            NoteBlockSoundImpl::read_data,
        );
    }

    #[test]
    fn tooltip_style_round_trip() {
        assert_round_trip(
            TooltipStyleImpl {
                id: "minecraft:my_style".to_string(),
            },
            TooltipStyleImpl::read_data,
        );
    }

    #[test]
    fn container_loot_round_trip() {
        assert_round_trip(
            ContainerLootImpl {
                loot_table: "minecraft:chests/simple_dungeon".to_string(),
                seed: 123_456_789,
            },
            ContainerLootImpl::read_data,
        );
    }

    #[test]
    fn custom_model_data_round_trip() {
        assert_round_trip(
            CustomModelDataImpl {
                floats: vec![1.5, -2.0],
                flags: vec![true, false, true],
                strings: vec!["a".to_string(), "b".to_string()],
                colors: vec![0xFF0000, 0x00FF00],
            },
            CustomModelDataImpl::read_data,
        );
    }

    #[test]
    fn lodestone_tracker_round_trip() {
        assert_round_trip(
            LodestoneTrackerImpl {
                target: Some(LodestoneTarget {
                    dimension: "minecraft:overworld".to_string(),
                    x: 10,
                    y: 64,
                    z: -30,
                }),
                tracked: true,
            },
            LodestoneTrackerImpl::read_data,
        );
        assert_round_trip(
            LodestoneTrackerImpl {
                target: None,
                tracked: false,
            },
            LodestoneTrackerImpl::read_data,
        );
    }

    #[test]
    fn marker_components_round_trip() {
        assert_round_trip(GliderImpl, GliderImpl::read_data);
        assert_round_trip(
            IntangibleProjectileImpl,
            IntangibleProjectileImpl::read_data,
        );
        assert_round_trip(
            BrewingFuelImpl {
                uses: IntProvider::Id(Cow::Borrowed("minecraft:brewing/uses_default")),
                speed_multiplier: FloatProvider::Inline(2.0),
            },
            BrewingFuelImpl::read_data,
        );
        assert_round_trip(WaxedImpl, WaxedImpl::read_data);
        assert_round_trip(
            CookingFuelImpl {
                burn_time: IntProvider::Id(Cow::Borrowed("minecraft:cooking/time_coal")),
                speed_multiplier: FloatProvider::Id(Cow::Borrowed(
                    "minecraft:cooking/speed_default",
                )),
            },
            CookingFuelImpl::read_data,
        );
        assert_round_trip(
            CookingFuelImpl {
                burn_time: IntProvider::Inline(1600),
                speed_multiplier: FloatProvider::Inline(1.5),
            },
            CookingFuelImpl::read_data,
        );
    }

    #[test]
    fn raw_nbt_components_round_trip() {
        let mut material = NbtCompound::new();
        material.put_string("material", "minecraft:diamond".to_string());
        material.put_string("pattern", "minecraft:coast".to_string());
        assert_round_trip(
            TrimImpl::read_data(&NbtTag::Compound(material)).unwrap(),
            TrimImpl::read_data,
        );

        let mut predicate = NbtCompound::new();
        predicate.put_string("blocks", "minecraft:stone".to_string());
        assert_round_trip(
            CanPlaceOnImpl {
                predicate: NbtTag::Compound(predicate.clone()),
            },
            CanPlaceOnImpl::read_data,
        );
        assert_round_trip(LockImpl { predicate }, LockImpl::read_data);
    }

    #[test]
    fn equal_with_different_types_returns_false() {
        let enc = EnchantmentsImpl {
            enchantment: Cow::Borrowed(&[(&crate::Enchantment::SHARPNESS, 2)]),
        };
        let max_stack = MaxStackSizeImpl { size: 64 };
        assert!(!enc.equal(&max_stack));
    }

    #[test]
    fn enchantments_read_data_formats() {
        let mut direct = NbtCompound::new();
        direct.put_int("sharpness", 2);
        let enc1 = EnchantmentsImpl::read_data(&NbtTag::Compound(direct)).unwrap();
        assert!(enc1.enchantment[0].0 == &crate::Enchantment::SHARPNESS);
        assert_eq!(enc1.enchantment[0].1, 2);

        let mut levels = NbtCompound::new();
        levels.put_int("minecraft:sharpness", 2);
        let mut wrapped = NbtCompound::new();
        wrapped
            .child_tags
            .insert("levels".into(), NbtTag::Compound(levels));
        let enc2 = EnchantmentsImpl::read_data(&NbtTag::Compound(wrapped)).unwrap();
        assert_eq!(enc2.enchantment.len(), 1);
        assert!(enc2.enchantment[0].0 == &crate::Enchantment::SHARPNESS);
        assert_eq!(enc2.enchantment[0].1, 2);
    }

    #[test]
    fn swing_animation_round_trip() {
        assert_round_trip(
            SwingAnimationImpl {
                animation_type: SwingAnimationType::Stab,
                duration: 19,
            },
            SwingAnimationImpl::read_data,
        );
        assert_round_trip(SwingAnimationImpl::DEFAULT, SwingAnimationImpl::read_data);
    }
}
