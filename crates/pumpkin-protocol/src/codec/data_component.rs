#![allow(clippy::wildcard_imports)]

use std::borrow::Cow;

use crate::codec::item_stack_seralizer::ItemStackSerializer;
use crate::codec::var_int::VarInt;
use crate::ser::{NetworkReadExt, NetworkWriteExt, ReadingError, WritingError};
use pumpkin_data::Enchantment;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::*;
use pumpkin_data::decorated_pot_pattern::DecoratedPotPattern;
use pumpkin_data::dye_color::DyeColor;
use pumpkin_data::instrument::Instrument;
use pumpkin_data::jukebox_song::JukeboxSong;
use pumpkin_data::painting_variant::PaintingVariant;
use pumpkin_data::trim_material::TrimMaterial;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::text::TextComponent;

use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::{serializer::NbtWriteHelperJava, tag::NbtTag};
use pumpkin_util::version::JavaMinecraftVersion;

const MAX_STATUS_EFFECTS: usize = 128;

#[must_use]
pub fn data_to_proto_sound(id_or: &IdOr<SoundEvent>) -> crate::IdOr<crate::SoundEvent> {
    match id_or {
        IdOr::Id(id) => crate::IdOr::Id(*id as u16),
        IdOr::Value(sound) => crate::IdOr::Value(crate::SoundEvent {
            sound_name: sound.sound_name.clone(),
            range: sound.range,
        }),
    }
}

#[must_use]
pub fn proto_to_data_sound(id_or: &crate::IdOr<crate::SoundEvent>) -> Option<IdOr<SoundEvent>> {
    match id_or {
        crate::IdOr::Id(id) => {
            let name = Sound::NAMES.get(*id as usize)?;
            Some(IdOr::Id(Sound::from_name(name)?))
        }
        crate::IdOr::Value(sound) => Some(IdOr::Value(SoundEvent {
            sound_name: sound.sound_name.clone(),
            range: sound.range,
        })),
    }
}

fn deserialize_idset<T: IDSetContent>(
    seq: &mut impl NetworkReadExt,
) -> Result<IDSet<T>, ReadingError> {
    let id_type = seq.get_var_int()?.0;

    match id_type.cmp(&0) {
        std::cmp::Ordering::Equal => {
            let tag = seq.get_str()?;
            Ok(IDSet::Tag(Cow::Owned(tag.into())))
        }
        std::cmp::Ordering::Greater => {
            let len = id_type - 1;
            let mut content_vec = Vec::with_capacity((len as usize).min(MAX_LIST_LENGTH));

            for _ in 0..len {
                let varint_id = seq.get_var_int()?.0;

                let elmt = T::from_id(varint_id as u16).ok_or(ReadingError::Message(
                    "Invalid registry id VarInt in IDSet".into(),
                ))?;
                content_vec.push(elmt);
            }
            Ok(IDSet::IDs(Cow::Owned(content_vec)))
        }
        std::cmp::Ordering::Less => Result::Err(ReadingError::Message(
            "Negative type/len VarInt in IDSet".into(),
        )),
    }
}

fn serialize_idset<C: IDSetContent>(
    idset: &IDSet<C>,
    seq: &mut impl NetworkWriteExt,
) -> Result<(), WritingError> {
    match idset {
        IDSet::Tag(tag) => {
            seq.write_var_int(&VarInt(0))?;
            seq.write_string(tag)
        }
        IDSet::IDs(elements) => {
            seq.write_var_int(&VarInt(elements.len() as i32 + 1))?;
            for elmt in elements.iter() {
                seq.write_var_int(&VarInt(elmt.registry_id() as i32))?;
            }
            Ok(())
        }
    }
}

/// Upper bound for lists vanilla reads without a size limit.
const MAX_LIST_LENGTH: usize = 256;

fn read_list_length(seq: &mut impl NetworkReadExt, what: &str) -> Result<usize, ReadingError> {
    let len = seq.get_var_int()?.0;
    usize::try_from(len)
        .ok()
        .filter(|len| *len <= MAX_LIST_LENGTH)
        .ok_or_else(|| ReadingError::Message(format!("Invalid {what} count {len}")))
}

fn read_registry_id(seq: &mut impl NetworkReadExt, what: &str) -> Result<u32, ReadingError> {
    let id = seq.get_var_int()?.0;
    u32::try_from(id).map_err(|_| ReadingError::Message(format!("Invalid {what} id {id}")))
}

/// Reads a `ByteBufCodecs.holder` id: 0 for an inline value, otherwise the
/// registry id plus one.
fn read_holder_id(seq: &mut impl NetworkReadExt, what: &str) -> Result<Option<u32>, ReadingError> {
    let id = read_registry_id(seq, what)?;
    Ok(id.checked_sub(1))
}

fn write_holder_reference(seq: &mut impl NetworkWriteExt, id: u32) -> Result<(), WritingError> {
    seq.write_var_int(&VarInt(id as i32 + 1))
}

/// `SoundEvent.STREAM_CODEC`.
fn read_sound(seq: &mut impl NetworkReadExt) -> Result<IdOr<SoundEvent>, ReadingError> {
    let proto = crate::IdOr::<crate::SoundEvent>::read(seq, |r| {
        let sound_name = r.get_str()?.into();
        let range = r.get_option(NetworkReadExt::get_f32)?;
        Ok(crate::SoundEvent { sound_name, range })
    })?;
    proto_to_data_sound(&proto).ok_or_else(|| ReadingError::Message("Invalid sound id".into()))
}

fn write_sound(
    seq: &mut impl NetworkWriteExt,
    sound: &IdOr<SoundEvent>,
) -> Result<(), WritingError> {
    crate::IdOr::<crate::SoundEvent>::write(&data_to_proto_sound(sound), seq, |w, e| {
        w.write_string(&e.sound_name)?;
        w.write_option(&e.range, |w2, r| w2.write_f32(*r))
    })
}

/// `ComponentSerialization.STREAM_CODEC`.
fn read_text(seq: &mut impl NetworkReadExt) -> Result<TextComponent, ReadingError> {
    let tag = seq
        .get_nbt_with_version(&JavaMinecraftVersion::V_26_2)?
        .ok_or_else(|| ReadingError::Message("Missing text component".into()))?;
    Ok(TextComponent::from_nbt(&tag))
}

fn write_text(seq: &mut impl NetworkWriteExt, text: &TextComponent) -> Result<(), WritingError> {
    seq.write_slice(&text.encode_for_version(&JavaMinecraftVersion::V_26_2))
}

/// `ByteBufCodecs.COMPOUND_TAG`, which rejects a missing tag.
fn read_compound(seq: &mut impl NetworkReadExt) -> Result<NbtCompound, ReadingError> {
    seq.get_compound_nbt_with_version(&JavaMinecraftVersion::V_26_2)?
        .ok_or_else(|| ReadingError::Message("Expected non-null compound tag".into()))
}

/// The wire form of components vanilla only gives a persistent codec:
/// `ByteBufCodecs.fromCodecWithRegistries` sends the codec's NBT.
fn read_codec_nbt<T>(
    seq: &mut impl NetworkReadExt,
    read: impl FnOnce(&NbtTag) -> Option<T>,
    what: &str,
) -> Result<T, ReadingError> {
    let tag = seq
        .get_nbt_with_version(&JavaMinecraftVersion::V_26_2)?
        .unwrap_or(NbtTag::End);
    read(&tag).ok_or_else(|| ReadingError::Message(format!("Failed to decode {what}")))
}

/// `DyeColor.STREAM_CODEC`: an id map that falls back to white.
fn read_dye_color(seq: &mut impl NetworkReadExt) -> Result<DyeColor, ReadingError> {
    let id = seq.get_var_int()?.0;
    Ok(u8::try_from(id)
        .ok()
        .and_then(DyeColor::by_id)
        .unwrap_or(DyeColor::White))
}

fn write_dye_color(seq: &mut impl NetworkWriteExt, color: DyeColor) -> Result<(), WritingError> {
    seq.write_var_int(&VarInt(i32::from(color.id())))
}

/// `ResolvableInt.STREAM_CODEC`: `true` then an int constant, or `false` then a key.
fn read_int_provider(seq: &mut impl NetworkReadExt) -> Result<IntProvider, ReadingError> {
    Ok(if seq.get_bool()? {
        IntProvider::Inline(seq.get_i32()?)
    } else {
        IntProvider::Id(Cow::Owned(seq.get_str()?.into()))
    })
}

fn write_int_provider(
    seq: &mut impl NetworkWriteExt,
    provider: &IntProvider,
) -> Result<(), WritingError> {
    match provider {
        IntProvider::Inline(value) => {
            seq.write_bool(true)?;
            seq.write_i32(*value)
        }
        IntProvider::Id(id) => {
            seq.write_bool(false)?;
            seq.write_string(id)
        }
    }
}

/// `ResolvableFloat.STREAM_CODEC`.
fn read_float_provider(seq: &mut impl NetworkReadExt) -> Result<FloatProvider, ReadingError> {
    Ok(if seq.get_bool()? {
        FloatProvider::Inline(seq.get_f32()?)
    } else {
        FloatProvider::Id(Cow::Owned(seq.get_str()?.into()))
    })
}

fn write_float_provider(
    seq: &mut impl NetworkWriteExt,
    provider: &FloatProvider,
) -> Result<(), WritingError> {
    match provider {
        FloatProvider::Inline(value) => {
            seq.write_bool(true)?;
            seq.write_f32(*value)
        }
        FloatProvider::Id(id) => {
            seq.write_bool(false)?;
            seq.write_string(id)
        }
    }
}

/// `TypedEntityData.streamCodec(EntityType.STREAM_CODEC)`. Pumpkin keeps the
/// type as `id` inside the NBT.
fn read_typed_entity_data(seq: &mut impl NetworkReadExt) -> Result<NbtCompound, ReadingError> {
    let type_id = seq.get_var_int()?.0;
    let entity_type = u16::try_from(type_id)
        .ok()
        .and_then(EntityType::from_raw)
        .ok_or_else(|| ReadingError::Message(format!("Unknown entity type id {type_id}")))?;
    let mut nbt = read_compound(seq)?;
    nbt.put_string("id", format!("minecraft:{}", entity_type.resource_name));
    Ok(nbt)
}

fn write_typed_entity_data(
    seq: &mut impl NetworkWriteExt,
    nbt: &NbtCompound,
) -> Result<(), WritingError> {
    let id = nbt
        .get_string("id")
        .ok_or_else(|| WritingError::Message("Entity data has no 'id'".into()))?;
    let type_id = EntityType::from_name(id)
        .map(|entity_type| i32::from(entity_type.id))
        .ok_or_else(|| WritingError::Message(format!("Unknown entity type {id}")))?;
    let mut nbt = nbt.clone();
    nbt.child_tags.remove("id");
    seq.write_var_int(&VarInt(type_id))?;
    seq.write_nbt(NbtTag::Compound(nbt))
}

fn deserialize_status_effects(
    seq: &mut impl NetworkReadExt,
) -> Result<Vec<StatusEffectInstance>, ReadingError> {
    let effects_len = seq.get_var_int()?.0 as usize;
    if effects_len > MAX_STATUS_EFFECTS {
        return Err(ReadingError::Message("Too many status effects".into()));
    }
    let mut custom_effects = Vec::with_capacity(effects_len);
    for _ in 0..effects_len {
        let effect_registry_id = seq.get_var_int()?.0;
        let effect_name = StatusEffect::from_id(effect_registry_id as u16)
            .ok_or(ReadingError::Message("Invalid effect_id!".into()))?
            .minecraft_name;
        custom_effects.push(deserialize_effect_details(
            seq,
            Cow::Borrowed(effect_name),
            StatusEffectInstance::MAX_HIDDEN_EFFECT_DEPTH,
        )?);
    }

    Ok(custom_effects)
}

/// `MobEffectInstance.Details.STREAM_CODEC`, which nests the hidden effect.
fn deserialize_effect_details(
    seq: &mut impl NetworkReadExt,
    effect_id: Cow<'static, str>,
    depth: usize,
) -> Result<StatusEffectInstance, ReadingError> {
    let amplifier = seq.get_var_int()?.0;
    let duration = seq.get_var_int()?.0;
    let ambient = seq.get_bool()?;
    let show_particles = seq.get_bool()?;
    let show_icon = seq.get_bool()?;
    let hidden_effect = if seq.get_bool()? {
        let depth = depth
            .checked_sub(1)
            .ok_or_else(|| ReadingError::TooLarge("Potion effect hidden depth exceeded".into()))?;
        Some(Box::new(deserialize_effect_details(
            seq,
            effect_id.clone(),
            depth,
        )?))
    } else {
        None
    };
    Ok(StatusEffectInstance {
        effect_id,
        amplifier,
        duration,
        ambient,
        show_particles,
        show_icon,
        hidden_effect,
    })
}

fn serialize_effect_details(
    seq: &mut impl NetworkWriteExt,
    effect: &StatusEffectInstance,
) -> Result<(), WritingError> {
    seq.write_var_int(&VarInt::from(effect.amplifier))?;
    seq.write_var_int(&VarInt::from(effect.duration))?;
    seq.write_bool(effect.ambient)?;
    seq.write_bool(effect.show_particles)?;
    seq.write_bool(effect.show_icon)?;
    seq.write_option(&effect.hidden_effect, |seq, hidden| {
        serialize_effect_details(seq, hidden)
    })
}

fn serialize_status_effects(
    effects: &[StatusEffectInstance],
    seq: &mut impl NetworkWriteExt,
) -> Result<(), WritingError> {
    seq.write_var_int(&VarInt(effects.len() as i32))?;

    for effect in effects {
        let effect_id = StatusEffect::from_minecraft_name(&namespaced(&effect.effect_id))
            .ok_or_else(|| {
                WritingError::Message(format!("Invalid status effect: {}", effect.effect_id))
            })?
            .registry_id();
        seq.write_var_int(&VarInt(effect_id as i32))?;
        serialize_effect_details(seq, effect)?;
    }
    Ok(())
}

fn deserialize_consume_effect(
    seq: &mut impl NetworkReadExt,
) -> Result<ConsumeEffect, ReadingError> {
    let effect_type = seq.get_var_int()?.0;
    match effect_type {
        0 => {
            let effects = deserialize_status_effects(seq)?;
            let probability = seq.get_f32()?;
            Ok(ConsumeEffect::ApplyEffects((
                Cow::Owned(effects),
                probability,
            )))
        }
        1 => {
            let idset = deserialize_idset(seq)?;
            Ok(ConsumeEffect::RemoveEffects(idset))
        }
        2 => Ok(ConsumeEffect::ClearAllEffects),
        3 => {
            let diameter = seq.get_f32()?;
            let directional_particles = seq.get_bool()?;
            Ok(ConsumeEffect::TeleportRandomly {
                diameter,
                directional_particles,
            })
        }
        4 => Ok(ConsumeEffect::PlaySound(read_sound(seq)?)),
        _ => Err(ReadingError::Message(
            "Invalid effect_type in ConsumeEffect".into(),
        )),
    }
}

fn serialize_consume_effect(
    consume_effect: &ConsumeEffect,
    seq: &mut impl NetworkWriteExt,
) -> Result<(), WritingError> {
    seq.write_var_int(&VarInt(consume_effect.registry_id() as i32))?;
    match consume_effect {
        ConsumeEffect::ApplyEffects((effects, probability)) => {
            serialize_status_effects(effects, seq)?;
            seq.write_f32(*probability)?;
        }
        ConsumeEffect::RemoveEffects(idset) => serialize_idset(idset, seq)?,
        ConsumeEffect::ClearAllEffects => (),
        ConsumeEffect::TeleportRandomly {
            diameter,
            directional_particles,
        } => {
            seq.write_f32(*diameter)?;
            seq.write_bool(*directional_particles)?;
        }
        ConsumeEffect::PlaySound(id_or) => write_sound(seq, id_or)?,
    }
    Ok(())
}

fn deserialize_consume_effects(
    seq: &mut impl NetworkReadExt,
) -> Result<Vec<ConsumeEffect>, ReadingError> {
    let len = read_list_length(seq, "consume effect")?;
    (0..len).map(|_| deserialize_consume_effect(seq)).collect()
}

fn serialize_consume_effects(
    effects: &[ConsumeEffect],
    seq: &mut impl NetworkWriteExt,
) -> Result<(), WritingError> {
    seq.write_var_int(&VarInt(effects.len() as i32))?;
    for effect in effects {
        serialize_consume_effect(effect, seq)?;
    }
    Ok(())
}

pub(crate) trait DataComponentCodec<Impl: DataComponentImpl> {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError>;
    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Impl, ReadingError>;
}

impl DataComponentCodec<Self> for MaxStackSizeImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.size))
    }
    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let size = u8::try_from(seq.get_var_int()?.0)
            .map_err(|_| ReadingError::Message("No MaxStackSize VarInt!".into()))?;
        Ok(Self { size })
    }
}

impl DataComponentCodec<Self> for DamageImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.damage))
    }
    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let damage = seq.get_var_int()?.0;
        Ok(Self { damage })
    }
}

impl DataComponentCodec<Self> for RepairCostImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.cost))
    }
    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let cost = seq.get_var_int()?.0;
        Ok(Self { cost })
    }
}

impl DataComponentCodec<Self> for EnchantmentsImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.enchantment.len() as i32))?;
        for (enc, level) in self.enchantment.iter() {
            seq.write_var_int(&VarInt::from(enc.id))?;
            seq.write_var_int(&VarInt::from(*level))?;
        }
        Ok(())
    }
    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        const MAX_ENCHANTMENTS: usize = 256;

        let len = seq.get_var_int()?.0 as usize;
        if len > MAX_ENCHANTMENTS {
            return Err(ReadingError::Message("Too many enchantments".into()));
        }
        let mut enc = Vec::with_capacity(len);
        for _ in 0..len {
            let id = seq.get_var_int()?.0 as u8;
            let level = seq.get_var_int()?.0;
            enc.push((
                Enchantment::from_id(id).ok_or(ReadingError::Message(
                    "EnchantmentsImpl Enchantment VarInt Incorrect!".into(),
                ))?,
                level,
            ));
        }
        Ok(Self {
            enchantment: Cow::from(enc),
        })
    }
}

impl DataComponentCodec<Self> for UnbreakableImpl {
    fn serialize(&self, _seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        Ok(())
    }
    fn deserialize(_seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self)
    }
}

impl DataComponentCodec<Self> for ItemModelImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_string(&self.id)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let id = seq.get_str()?;
        Ok(Self {
            id: Cow::Owned(id.into()),
        })
    }
}

impl DataComponentCodec<Self> for CustomNameImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        let mut bytes = Vec::new();
        NbtTag::String(self.name.clone().get_text().into_boxed_str())
            .serialize(&mut NbtWriteHelperJava::new(&mut bytes))
            .map_err(|e| WritingError::Message(e.to_string()))?;
        seq.write_slice(&bytes)?;
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let tag = seq.get_nbt_with_version(&pumpkin_util::version::JavaMinecraftVersion::V_26_2)?;
        let name = tag.as_ref().map_or_else(
            pumpkin_util::text::TextComponent::empty,
            pumpkin_util::text::TextComponent::from_nbt,
        );
        Ok(Self { name })
    }
}

impl DataComponentCodec<Self> for LoreImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(self.lines.len() as i32))?;
        for line in &self.lines {
            seq.write_slice(
                &line.encode_for_version(&pumpkin_util::version::JavaMinecraftVersion::V_26_2),
            )?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        // TODO: Could probably be extracted?
        const MAX_LORE_LINES: i32 = 256;

        let count = seq.get_var_int()?.0;
        if !(0..=MAX_LORE_LINES).contains(&count) {
            return Err(ReadingError::Message(format!(
                "LoreImpl line count {count} is out of bounds (0-{MAX_LORE_LINES})"
            )));
        }

        let mut lines = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let tag =
                seq.get_nbt_with_version(&pumpkin_util::version::JavaMinecraftVersion::V_26_2)?;
            let text = tag.as_ref().map_or_else(
                pumpkin_util::text::TextComponent::empty,
                pumpkin_util::text::TextComponent::from_nbt,
            );
            lines.push(text);
        }
        Ok(Self { lines })
    }
}

impl DataComponentCodec<Self> for ItemNameImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        let mut name = pumpkin_nbt::compound::NbtCompound::new();
        name.put_string("translate", self.name.to_string());
        let mut bytes = Vec::new();
        NbtTag::Compound(name)
            .serialize(&mut NbtWriteHelperJava::new(&mut bytes))
            .map_err(|error| WritingError::Message(error.to_string()))?;
        seq.write_slice(&bytes)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let name = seq.get_str()?;
        Ok(Self {
            name: Cow::Owned(name.into()),
        })
    }
}

impl DataComponentCodec<Self> for DyedColorImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_i32(self.rgb)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            rgb: seq.get_i32()?,
        })
    }
}

impl DataComponentCodec<Self> for SuspiciousStewEffectsImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        let effect_count = i32::try_from(self.effects.len())
            .map_err(|_| WritingError::Message("Too many suspicious stew effects".into()))?;
        seq.write_var_int(&VarInt(effect_count))?;
        for effect in self.effects.iter() {
            let id = StatusEffect::from_minecraft_name(&effect.effect)
                .ok_or_else(|| WritingError::Message("Unknown suspicious stew effect".into()))?
                .id;
            seq.write_var_int(&VarInt(i32::from(id)))?;
            seq.write_var_int(&VarInt(effect.duration))?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        const MAX_EFFECTS: i32 = 128;

        let count = seq.get_var_int()?.0;
        if !(0..=MAX_EFFECTS).contains(&count) {
            return Err(ReadingError::Message(
                "Invalid suspicious stew effect count".into(),
            ));
        }

        let mut effects =
            Vec::with_capacity(usize::try_from(count).map_err(|_| {
                ReadingError::Message("Invalid suspicious stew effect count".into())
            })?);
        for _ in 0..count {
            let id = u16::try_from(seq.get_var_int()?.0)
                .map_err(|_| ReadingError::Message("Invalid suspicious stew effect id".into()))?;
            let effect = StatusEffect::from_id(id)
                .ok_or_else(|| ReadingError::Message("Unknown suspicious stew effect id".into()))?;
            let duration = seq.get_var_int()?.0;
            effects.push(SuspiciousStewEffect {
                effect: Cow::Borrowed(effect.minecraft_name),
                duration,
            });
        }
        Ok(Self {
            effects: Cow::Owned(effects),
        })
    }
}

impl DataComponentCodec<Self> for CustomDataImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        let mut bytes = Vec::new();
        NbtTag::Compound(self.data.clone())
            .serialize(&mut NbtWriteHelperJava::new(&mut bytes))
            .map_err(|e| WritingError::Message(e.to_string()))?;
        seq.write_slice(&bytes)?;
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let data = seq
            .get_compound_nbt_with_version(&pumpkin_util::version::JavaMinecraftVersion::V_26_2)?
            .unwrap_or_else(pumpkin_nbt::compound::NbtCompound::new);
        Ok(Self { data })
    }
}

impl DataComponentCodec<Self> for ConsumableImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_f32(self.consume_seconds)?;
        seq.write_var_int(&VarInt(self.animation as i32))?;
        crate::IdOr::<crate::SoundEvent>::write(
            &data_to_proto_sound(&self.sound_event),
            seq,
            |w, e| {
                w.write_string(&e.sound_name)?;
                w.write_option(&e.range, |w2, r| w2.write_f32(*r))
            },
        )?;
        seq.write_bool(self.consume_particles)?;
        seq.write_var_int(&VarInt(self.effects.len() as i32))?;

        for effect in self.effects.iter() {
            serialize_consume_effect(effect, seq)?;
        }

        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        const MAX_CONSUME_EFFECTS: i32 = 256;

        let consume_seconds = seq.get_f32()?;
        let animation_id = seq.get_var_int()?;

        let animation: ConsumeAnimation = animation_id
            .0
            .try_into()
            .map_err(|()| ReadingError::Message("Invalid ConsumableImpl animation id!".into()))?;
        let proto_sound_event = crate::IdOr::<crate::SoundEvent>::read(seq, |r| {
            let sound_name = r.get_str()?.into();
            let range = r.get_option(NetworkReadExt::get_f32)?;
            Ok(crate::SoundEvent { sound_name, range })
        })?;
        let consume_particles = seq.get_bool()?;

        let sound_event = proto_to_data_sound(&proto_sound_event).ok_or(ReadingError::Message(
            "Invalid sound in ConsumableImpl".into(),
        ))?;
        let effects_len = seq.get_var_int()?.0;
        if !(0..=MAX_CONSUME_EFFECTS).contains(&effects_len) {
            return Err(ReadingError::Message("Invalid consume effect count".into()));
        }

        let mut effects_vec = Vec::with_capacity(effects_len as usize);

        for _ in 0..effects_len {
            effects_vec.push(deserialize_consume_effect(seq)?);
        }

        let effects: Cow<'static, [ConsumeEffect]> = Cow::Owned(effects_vec);

        Ok(Self {
            consume_seconds,
            animation,
            sound_event,
            consume_particles,
            effects,
        })
    }
}

impl DataComponentCodec<Self> for EquippableImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(self.slot.get_slot_index()))?;
        crate::IdOr::<crate::SoundEvent>::write(
            &data_to_proto_sound(&self.equip_sound),
            seq,
            |w, e| {
                w.write_string(&e.sound_name)?;
                w.write_option(&e.range, |w2, r| w2.write_f32(*r))
            },
        )?;

        seq.write_bool(self.asset_id.is_some())?;
        if let Some(asset) = &self.asset_id {
            seq.write_string(asset)?;
        }

        seq.write_bool(self.camera_overlay.is_some())?;
        if let Some(overlay) = &self.camera_overlay {
            seq.write_string(overlay)?;
        }

        seq.write_bool(self.allowed_entities.is_some())?;
        if let Some(allowed) = &self.allowed_entities {
            serialize_idset(allowed, seq)?;
        }

        seq.write_bool(self.dispensable)?;
        seq.write_bool(self.swappable)?;
        seq.write_bool(self.damage_on_hurt)?;
        seq.write_bool(self.equip_on_interact)?;
        seq.write_bool(self.can_be_sheared)?;
        crate::IdOr::<crate::SoundEvent>::write(
            &data_to_proto_sound(&self.shearing_sound),
            seq,
            |w, e| {
                w.write_string(&e.sound_name)?;
                w.write_option(&e.range, |w2, r| w2.write_f32(*r))
            },
        )
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let slot_index = seq.get_var_int()?.0;
        let slot = EquipmentSlot::from_slot_index(slot_index).ok_or(ReadingError::Message(
            format!("Invalid equipment slot index {slot_index}"),
        ))?;
        let equip_sound = proto_to_data_sound(&crate::IdOr::<crate::SoundEvent>::read(seq, |r| {
            let sound_name = r.get_str()?.into();
            let range = r.get_option(NetworkReadExt::get_f32)?;
            Ok(crate::SoundEvent { sound_name, range })
        })?)
        .ok_or(ReadingError::Message(
            "Invalid sound in EquippableImpl".into(),
        ))?;

        let asset_id = if seq.get_bool()? {
            Some(Cow::Owned(seq.get_str()?.into()))
        } else {
            None
        };

        let camera_overlay = if seq.get_bool()? {
            Some(Cow::Owned(seq.get_str()?.into()))
        } else {
            None
        };

        let has_allowed_entities = seq.get_bool()?;

        let allowed_entities: Option<IDSet<EntityType>> = if has_allowed_entities {
            Some(deserialize_idset(seq)?)
        } else {
            None
        };

        let dispensable = seq.get_bool()?;
        let swappable = seq.get_bool()?;
        let damage_on_hurt = seq.get_bool()?;
        let equip_on_interact = seq.get_bool()?;
        let can_be_sheared = seq.get_bool()?;
        let shearing_sound =
            proto_to_data_sound(&crate::IdOr::<crate::SoundEvent>::read(seq, |r| {
                let sound_name = r.get_str()?.into();
                let range = r.get_option(NetworkReadExt::get_f32)?;
                Ok(crate::SoundEvent { sound_name, range })
            })?)
            .ok_or(ReadingError::Message(
                "Invalid shearing sound in EquippableImpl".into(),
            ))?;

        Ok(Self {
            slot,
            equip_sound,
            asset_id,
            camera_overlay,
            allowed_entities,
            dispensable,
            swappable,
            damage_on_hurt,
            equip_on_interact,
            can_be_sheared,
            shearing_sound,
        })
    }
}

impl DataComponentCodec<Self> for PotionContentsImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // Potion ID (optional)
        if let Some(potion_id) = self.potion_id {
            seq.write_bool(true)?;
            seq.write_var_int(&VarInt::from(potion_id))?;
        } else {
            seq.write_bool(false)?;
        }

        // Custom color (optional)
        if let Some(color) = self.custom_color {
            seq.write_bool(true)?;
            seq.write_i32(color)?;
        } else {
            seq.write_bool(false)?;
        }

        // Custom effects list
        serialize_status_effects(&self.custom_effects, seq)?;

        // Custom name (optional)
        if let Some(name) = &self.custom_name {
            seq.write_bool(true)?;
            seq.write_string(name.as_str())?;
        } else {
            seq.write_bool(false)?;
        }

        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        // Potion ID (optional)
        let has_potion = seq.get_bool()?;
        let potion_id = has_potion
            .then(|| seq.get_var_int().map(|value| value.0))
            .transpose()?;

        // Custom color (optional)
        let has_color = seq.get_bool()?;
        let custom_color = has_color.then(|| seq.get_i32()).transpose()?;

        // Custom effects list
        let custom_effects = deserialize_status_effects(seq)?;

        // Custom name (optional)
        let has_name = seq.get_bool()?;
        let custom_name = has_name
            .then(|| seq.get_str().map(String::from))
            .transpose()?;

        Ok(Self {
            potion_id,
            custom_color,
            custom_effects,
            custom_name,
        })
    }
}

impl DataComponentCodec<Self> for FireworkExplosionImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // Shape (VarInt enum)
        seq.write_var_int(&VarInt::from(self.shape.to_id()))?;
        // Colors list
        seq.write_var_int(&VarInt::from(self.colors.len() as i32))?;
        for color in &self.colors {
            seq.write_i32(*color)?;
        }
        // Fade colors list
        seq.write_var_int(&VarInt::from(self.fade_colors.len() as i32))?;
        for color in &self.fade_colors {
            seq.write_i32(*color)?;
        }
        // hasTrail
        seq.write_bool(self.has_trail)?;
        // hasTwinkle
        seq.write_bool(self.has_twinkle)?;
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        // Needs a length cap during deserialization to prevent OOM from malicious packets
        // Vanilla doesn't have any limits (Integer.MAX_VALUE is technically a limit but not enforced in practice)
        const MAX_COLORS: usize = 256;
        const MAX_FADE_COLORS: usize = 256;

        // Shape (VarInt enum)
        let shape_id = seq.get_var_int()?.0;
        let shape = FireworkExplosionShape::from_id(shape_id).ok_or(ReadingError::Message(
            "Invalid FireworkExplosionShape id!".into(),
        ))?;

        // Colors list
        let colors_len = seq.get_var_int()?.0 as usize;
        if colors_len > MAX_COLORS {
            return Err(ReadingError::Message(format!(
                "FireworkExplosionImpl colors_len {colors_len} exceeds maximum of {MAX_COLORS}"
            )));
        }
        let mut colors = Vec::with_capacity(colors_len);
        for _ in 0..colors_len {
            let color = seq.get_i32()?;
            colors.push(color);
        }

        // Fade colors list
        let fade_colors_len = seq.get_var_int()?.0 as usize;
        if fade_colors_len > MAX_FADE_COLORS {
            return Err(ReadingError::Message(format!(
                "FireworkExplosionImpl fade_colors_len {fade_colors_len} exceeds maximum of {MAX_FADE_COLORS}"
            )));
        }
        let mut fade_colors = Vec::with_capacity(fade_colors_len);
        for _ in 0..fade_colors_len {
            let color = seq.get_i32()?;
            fade_colors.push(color);
        }

        // hasTrail
        let has_trail = seq.get_bool()?;

        // hasTwinkle
        let has_twinkle = seq.get_bool()?;

        Ok(Self::new(
            shape,
            colors,
            fade_colors,
            has_trail,
            has_twinkle,
        ))
    }
}

impl DataComponentCodec<Self> for FireworksImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // Flight duration (VarInt)
        seq.write_var_int(&VarInt::from(self.flight_duration))?;
        // Explosions list
        seq.write_var_int(&VarInt::from(self.explosions.len() as i32))?;
        for explosion in &self.explosions {
            explosion.serialize(seq)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        // Needs a length cap during deserialization to prevent OOM from malicious packets
        // Vanilla doesn't have any limits
        const MAX_EXPLOSIONS: usize = 256;
        // Vanilla restricts to 0-255 (UNSIGNED_BYTE in data component codec) (do not trust client NBT to limit it)
        const MAX_FLIGHT_DURATION: i32 = 255;

        // Flight duration
        let flight_duration = seq.get_var_int()?.0;
        if !(0..=MAX_FLIGHT_DURATION).contains(&flight_duration) {
            return Err(ReadingError::Message(format!(
                "FireworksImpl flight_duration {flight_duration} is out of bounds (0-{MAX_FLIGHT_DURATION})"
            )));
        }

        // Explosions list
        let explosions_len = seq.get_var_int()?.0 as usize;
        if explosions_len > MAX_EXPLOSIONS {
            return Err(ReadingError::Message(format!(
                "FireworksImpl explosions_len {explosions_len} exceeds maximum of {MAX_EXPLOSIONS}"
            )));
        }
        let mut explosions = Vec::with_capacity(explosions_len);
        for _ in 0..explosions_len {
            // Recursively deserialize each explosion
            let explosion = FireworkExplosionImpl::deserialize(seq)?;
            explosions.push(explosion);
        }

        Ok(Self::new(flight_duration, explosions))
    }
}

impl DataComponentCodec<Self> for StoredEnchantmentsImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.enchantment.len() as i32))?;
        for (enc, level) in self.enchantment.iter() {
            seq.write_var_int(&VarInt::from(enc.id))?;
            seq.write_var_int(&VarInt::from(*level))?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        const MAX_ENCHANTMENTS: usize = 256;

        let len = seq.get_var_int()?.0 as usize;

        if len > MAX_ENCHANTMENTS {
            return Err(ReadingError::Message("Too many enchantments".into()));
        }

        let mut stored_enchantments = Vec::with_capacity(len);
        for _ in 0..len {
            let id = seq.get_var_int()?.0 as u8;
            let level = seq.get_var_int()?.0;
            stored_enchantments.push((
                Enchantment::from_id(id).ok_or(ReadingError::Message(
                    "StoredEnchantmentsImpl Enchantment VarInt Incorrect!".into(),
                ))?,
                level,
            ));
        }
        Ok(Self {
            enchantment: Cow::from(stored_enchantments),
        })
    }
}

impl DataComponentCodec<Self> for RepairableImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        serialize_idset(&self.items, seq)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            items: deserialize_idset(seq)?,
        })
    }
}

impl DataComponentCodec<Self> for SwingAnimationImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.animation_type.to_id()))?;
        seq.write_var_int(&VarInt::from(self.duration))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let type_id = seq.get_var_int()?.0;
        let animation_type = SwingAnimationType::from_id(type_id).ok_or_else(|| {
            ReadingError::Message(format!("Invalid SwingAnimationType id {type_id}"))
        })?;
        let duration = seq.get_var_int()?.0;
        Ok(Self {
            animation_type,
            duration,
        })
    }
}

impl DataComponentCodec<Self> for RarityImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.rarity.to_id()))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let id = seq.get_var_int()?.0;
        let rarity = Rarity::from_id(id)
            .ok_or_else(|| ReadingError::Message(format!("Invalid Rarity id {id}")))?;
        Ok(Self { rarity })
    }
}

#[allow(clippy::too_many_lines)]
pub fn deserialize(
    id: DataComponent,
    seq: &mut impl NetworkReadExt,
) -> Result<Box<dyn DataComponentImpl>, ReadingError> {
    match id {
        DataComponent::CustomData => Ok(CustomDataImpl::deserialize(seq)?.to_dyn()),
        DataComponent::MaxStackSize => Ok(MaxStackSizeImpl::deserialize(seq)?.to_dyn()),
        DataComponent::MaxDamage => Ok(MaxDamageImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Damage => Ok(DamageImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Unbreakable => Ok(UnbreakableImpl::deserialize(seq)?.to_dyn()),
        DataComponent::UseEffects => Ok(UseEffectsImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CustomName => Ok(CustomNameImpl::deserialize(seq)?.to_dyn()),
        DataComponent::MinimumAttackCharge => {
            Ok(MinimumAttackChargeImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::DamageType => Ok(DamageTypeImpl::deserialize(seq)?.to_dyn()),
        DataComponent::ItemName => Ok(ItemNameImpl::deserialize(seq)?.to_dyn()),
        DataComponent::ItemModel => Ok(ItemModelImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Lore => Ok(LoreImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Rarity => Ok(RarityImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Enchantments => Ok(EnchantmentsImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CanPlaceOn => Ok(CanPlaceOnImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CanBreak => Ok(CanBreakImpl::deserialize(seq)?.to_dyn()),
        DataComponent::AttributeModifiers => Ok(AttributeModifiersImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CustomModelData => Ok(CustomModelDataImpl::deserialize(seq)?.to_dyn()),
        DataComponent::TooltipDisplay => Ok(TooltipDisplayImpl::deserialize(seq)?.to_dyn()),
        DataComponent::RepairCost => Ok(RepairCostImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CreativeSlotLock => Ok(CreativeSlotLockImpl::deserialize(seq)?.to_dyn()),
        DataComponent::EnchantmentGlintOverride => {
            Ok(EnchantmentGlintOverrideImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::IntangibleProjectile => {
            Ok(IntangibleProjectileImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::Food => Ok(FoodImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Consumable => Ok(ConsumableImpl::deserialize(seq)?.to_dyn()),
        DataComponent::UseRemainder => Ok(UseRemainderImpl::deserialize(seq)?.to_dyn()),
        DataComponent::UseCooldown => Ok(UseCooldownImpl::deserialize(seq)?.to_dyn()),
        DataComponent::DamageResistant => Ok(DamageResistantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Tool => Ok(ToolImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Weapon => Ok(WeaponImpl::deserialize(seq)?.to_dyn()),
        DataComponent::AttackRange => Ok(AttackRangeImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Enchantable => Ok(EnchantableImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Equippable => Ok(EquippableImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Repairable => Ok(RepairableImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Glider => Ok(GliderImpl::deserialize(seq)?.to_dyn()),
        DataComponent::TooltipStyle => Ok(TooltipStyleImpl::deserialize(seq)?.to_dyn()),
        DataComponent::DeathProtection => Ok(DeathProtectionImpl::deserialize(seq)?.to_dyn()),
        DataComponent::BlocksAttacks => Ok(BlocksAttacksImpl::deserialize(seq)?.to_dyn()),
        DataComponent::PiercingWeapon => Ok(PiercingWeaponImpl::deserialize(seq)?.to_dyn()),
        DataComponent::KineticWeapon => Ok(KineticWeaponImpl::deserialize(seq)?.to_dyn()),
        DataComponent::AttackAnimation => Ok(SwingAnimationImpl::deserialize(seq)?.to_dyn()),
        DataComponent::AdditionalTradeCost => {
            Ok(AdditionalTradeCostImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::StoredEnchantments => Ok(StoredEnchantmentsImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Dye => Ok(DyeImpl::deserialize(seq)?.to_dyn()),
        DataComponent::DyedColor => Ok(DyedColorImpl::deserialize(seq)?.to_dyn()),
        DataComponent::MapId => Ok(MapIdImpl::deserialize(seq)?.to_dyn()),
        DataComponent::MapDecorations => Ok(MapDecorationsImpl::deserialize(seq)?.to_dyn()),
        DataComponent::MapPostProcessing => Ok(MapPostProcessingImpl::deserialize(seq)?.to_dyn()),
        DataComponent::ChargedProjectiles => Ok(ChargedProjectilesImpl::deserialize(seq)?.to_dyn()),
        DataComponent::BundleContents => Ok(BundleContentsImpl::deserialize(seq)?.to_dyn()),
        DataComponent::PotionContents => Ok(PotionContentsImpl::deserialize(seq)?.to_dyn()),
        DataComponent::PotionDurationScale => {
            Ok(PotionDurationScaleImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::SuspiciousStewEffects => {
            Ok(SuspiciousStewEffectsImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::WritableBookContent => {
            Ok(WritableBookContentImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::WrittenBookContent => Ok(WrittenBookContentImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Trim => Ok(TrimImpl::deserialize(seq)?.to_dyn()),
        DataComponent::DebugStickState => Ok(DebugStickStateImpl::deserialize(seq)?.to_dyn()),
        DataComponent::EntityData => Ok(EntityDataImpl::deserialize(seq)?.to_dyn()),
        DataComponent::BucketEntityData => Ok(BucketEntityDataImpl::deserialize(seq)?.to_dyn()),
        DataComponent::BlockEntityData => Ok(BlockEntityDataImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Instrument => Ok(InstrumentImpl::deserialize(seq)?.to_dyn()),
        DataComponent::ProvidesTrimMaterial => {
            Ok(ProvidesTrimMaterialImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::OminousBottleAmplifier => {
            Ok(OminousBottleAmplifierImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::JukeboxPlayable => Ok(JukeboxPlayableImpl::deserialize(seq)?.to_dyn()),
        DataComponent::ProvidesBannerPatterns => {
            Ok(ProvidesBannerPatternsImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::Recipes => Ok(RecipesImpl::deserialize(seq)?.to_dyn()),
        DataComponent::LodestoneTracker => Ok(LodestoneTrackerImpl::deserialize(seq)?.to_dyn()),
        DataComponent::FireworkExplosion => Ok(FireworkExplosionImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Fireworks => Ok(FireworksImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Profile => Ok(ProfileImpl::deserialize(seq)?.to_dyn()),
        DataComponent::NoteBlockSound => Ok(NoteBlockSoundImpl::deserialize(seq)?.to_dyn()),
        DataComponent::BannerPatterns => Ok(BannerPatternsImpl::deserialize(seq)?.to_dyn()),
        DataComponent::BaseColor => Ok(BaseColorImpl::deserialize(seq)?.to_dyn()),
        DataComponent::PotDecorations => Ok(PotDecorationsImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Container => Ok(ContainerImpl::deserialize(seq)?.to_dyn()),
        DataComponent::BlockState => Ok(BlockStateImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Bees => Ok(BeesImpl::deserialize(seq)?.to_dyn()),
        DataComponent::SulfurCubeContent => Ok(SulfurCubeContentImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Lock => Ok(LockImpl::deserialize(seq)?.to_dyn()),
        DataComponent::ContainerLoot => Ok(ContainerLootImpl::deserialize(seq)?.to_dyn()),
        DataComponent::BreakSound => Ok(BreakSoundImpl::deserialize(seq)?.to_dyn()),
        DataComponent::VillagerVariant => Ok(VillagerVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::WolfVariant => Ok(WolfVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::WolfSoundVariant => Ok(WolfSoundVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::WolfCollar => Ok(WolfCollarImpl::deserialize(seq)?.to_dyn()),
        DataComponent::FoxVariant => Ok(FoxVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::SalmonSize => Ok(SalmonSizeImpl::deserialize(seq)?.to_dyn()),
        DataComponent::ParrotVariant => Ok(ParrotVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::TropicalFishPattern => {
            Ok(TropicalFishPatternImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::TropicalFishBaseColor => {
            Ok(TropicalFishBaseColorImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::TropicalFishPatternColor => {
            Ok(TropicalFishPatternColorImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::MooshroomVariant => Ok(MooshroomVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::RabbitVariant => Ok(RabbitVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::PigVariant => Ok(PigVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::PigSoundVariant => Ok(PigSoundVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CowVariant => Ok(CowVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CowSoundVariant => Ok(CowSoundVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::ChickenVariant => Ok(ChickenVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::ChickenSoundVariant => {
            Ok(ChickenSoundVariantImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::ZombieNautilusVariant => {
            Ok(ZombieNautilusVariantImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::FrogVariant => Ok(FrogVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::HorseVariant => Ok(HorseVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::PaintingVariant => Ok(PaintingVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::LlamaVariant => Ok(LlamaVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::AxolotlVariant => Ok(AxolotlVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CatVariant => Ok(CatVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CatSoundVariant => Ok(CatSoundVariantImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CatCollar => Ok(CatCollarImpl::deserialize(seq)?.to_dyn()),
        DataComponent::SheepColor => Ok(SheepColorImpl::deserialize(seq)?.to_dyn()),
        DataComponent::ShulkerColor => Ok(ShulkerColorImpl::deserialize(seq)?.to_dyn()),
        DataComponent::InteractAnimation => Ok(InteractAnimationImpl::deserialize(seq)?.to_dyn()),
        DataComponent::BlockTransformer => Ok(BlockTransformerImpl::deserialize(seq)?.to_dyn()),
        DataComponent::VillagerFood => Ok(VillagerFoodImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Compostable => Ok(CompostableImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CookingFuel => Ok(CookingFuelImpl::deserialize(seq)?.to_dyn()),
        DataComponent::BrewingFuel => Ok(BrewingFuelImpl::deserialize(seq)?.to_dyn()),
        DataComponent::MobVisibility => Ok(MobVisibilityImpl::deserialize(seq)?.to_dyn()),
        DataComponent::ProvidesPotteryPattern => {
            Ok(ProvidesPotteryPatternImpl::deserialize(seq)?.to_dyn())
        }
        DataComponent::SignTextFront => Ok(SignTextFrontImpl::deserialize(seq)?.to_dyn()),
        DataComponent::SignTextBack => Ok(SignTextBackImpl::deserialize(seq)?.to_dyn()),
        DataComponent::Waxed => Ok(WaxedImpl::deserialize(seq)?.to_dyn()),
        DataComponent::CushionColor => Ok(CushionColorImpl::deserialize(seq)?.to_dyn()),
    }
}

#[allow(clippy::too_many_lines)]
pub fn serialize(
    id: DataComponent,
    value: &dyn DataComponentImpl,
    seq: &mut impl NetworkWriteExt,
) -> Result<(), WritingError> {
    match id {
        DataComponent::CustomData => get::<CustomDataImpl>(value).serialize(seq),
        DataComponent::MaxStackSize => get::<MaxStackSizeImpl>(value).serialize(seq),
        DataComponent::MaxDamage => get::<MaxDamageImpl>(value).serialize(seq),
        DataComponent::Damage => get::<DamageImpl>(value).serialize(seq),
        DataComponent::Unbreakable => get::<UnbreakableImpl>(value).serialize(seq),
        DataComponent::UseEffects => get::<UseEffectsImpl>(value).serialize(seq),
        DataComponent::CustomName => get::<CustomNameImpl>(value).serialize(seq),
        DataComponent::MinimumAttackCharge => get::<MinimumAttackChargeImpl>(value).serialize(seq),
        DataComponent::DamageType => get::<DamageTypeImpl>(value).serialize(seq),
        DataComponent::ItemName => get::<ItemNameImpl>(value).serialize(seq),
        DataComponent::ItemModel => get::<ItemModelImpl>(value).serialize(seq),
        DataComponent::Lore => get::<LoreImpl>(value).serialize(seq),
        DataComponent::Rarity => get::<RarityImpl>(value).serialize(seq),
        DataComponent::Enchantments => get::<EnchantmentsImpl>(value).serialize(seq),
        DataComponent::CanPlaceOn => get::<CanPlaceOnImpl>(value).serialize(seq),
        DataComponent::CanBreak => get::<CanBreakImpl>(value).serialize(seq),
        DataComponent::AttributeModifiers => get::<AttributeModifiersImpl>(value).serialize(seq),
        DataComponent::CustomModelData => get::<CustomModelDataImpl>(value).serialize(seq),
        DataComponent::TooltipDisplay => get::<TooltipDisplayImpl>(value).serialize(seq),
        DataComponent::RepairCost => get::<RepairCostImpl>(value).serialize(seq),
        DataComponent::CreativeSlotLock => get::<CreativeSlotLockImpl>(value).serialize(seq),
        DataComponent::EnchantmentGlintOverride => {
            get::<EnchantmentGlintOverrideImpl>(value).serialize(seq)
        }
        DataComponent::IntangibleProjectile => {
            get::<IntangibleProjectileImpl>(value).serialize(seq)
        }
        DataComponent::Food => get::<FoodImpl>(value).serialize(seq),
        DataComponent::Consumable => get::<ConsumableImpl>(value).serialize(seq),
        DataComponent::UseRemainder => get::<UseRemainderImpl>(value).serialize(seq),
        DataComponent::UseCooldown => get::<UseCooldownImpl>(value).serialize(seq),
        DataComponent::DamageResistant => get::<DamageResistantImpl>(value).serialize(seq),
        DataComponent::Tool => get::<ToolImpl>(value).serialize(seq),
        DataComponent::Weapon => get::<WeaponImpl>(value).serialize(seq),
        DataComponent::AttackRange => get::<AttackRangeImpl>(value).serialize(seq),
        DataComponent::Enchantable => get::<EnchantableImpl>(value).serialize(seq),
        DataComponent::Equippable => get::<EquippableImpl>(value).serialize(seq),
        DataComponent::Repairable => get::<RepairableImpl>(value).serialize(seq),
        DataComponent::Glider => get::<GliderImpl>(value).serialize(seq),
        DataComponent::TooltipStyle => get::<TooltipStyleImpl>(value).serialize(seq),
        DataComponent::DeathProtection => get::<DeathProtectionImpl>(value).serialize(seq),
        DataComponent::BlocksAttacks => get::<BlocksAttacksImpl>(value).serialize(seq),
        DataComponent::PiercingWeapon => get::<PiercingWeaponImpl>(value).serialize(seq),
        DataComponent::KineticWeapon => get::<KineticWeaponImpl>(value).serialize(seq),
        DataComponent::AttackAnimation => get::<SwingAnimationImpl>(value).serialize(seq),
        DataComponent::AdditionalTradeCost => get::<AdditionalTradeCostImpl>(value).serialize(seq),
        DataComponent::StoredEnchantments => get::<StoredEnchantmentsImpl>(value).serialize(seq),
        DataComponent::Dye => get::<DyeImpl>(value).serialize(seq),
        DataComponent::DyedColor => get::<DyedColorImpl>(value).serialize(seq),
        DataComponent::MapId => get::<MapIdImpl>(value).serialize(seq),
        DataComponent::MapDecorations => get::<MapDecorationsImpl>(value).serialize(seq),
        DataComponent::MapPostProcessing => get::<MapPostProcessingImpl>(value).serialize(seq),
        DataComponent::ChargedProjectiles => get::<ChargedProjectilesImpl>(value).serialize(seq),
        DataComponent::BundleContents => get::<BundleContentsImpl>(value).serialize(seq),
        DataComponent::PotionContents => get::<PotionContentsImpl>(value).serialize(seq),
        DataComponent::PotionDurationScale => get::<PotionDurationScaleImpl>(value).serialize(seq),
        DataComponent::SuspiciousStewEffects => {
            get::<SuspiciousStewEffectsImpl>(value).serialize(seq)
        }
        DataComponent::WritableBookContent => get::<WritableBookContentImpl>(value).serialize(seq),
        DataComponent::WrittenBookContent => get::<WrittenBookContentImpl>(value).serialize(seq),
        DataComponent::Trim => get::<TrimImpl>(value).serialize(seq),
        DataComponent::DebugStickState => get::<DebugStickStateImpl>(value).serialize(seq),
        DataComponent::EntityData => get::<EntityDataImpl>(value).serialize(seq),
        DataComponent::BucketEntityData => get::<BucketEntityDataImpl>(value).serialize(seq),
        DataComponent::BlockEntityData => get::<BlockEntityDataImpl>(value).serialize(seq),
        DataComponent::Instrument => get::<InstrumentImpl>(value).serialize(seq),
        DataComponent::ProvidesTrimMaterial => {
            get::<ProvidesTrimMaterialImpl>(value).serialize(seq)
        }
        DataComponent::OminousBottleAmplifier => {
            get::<OminousBottleAmplifierImpl>(value).serialize(seq)
        }
        DataComponent::JukeboxPlayable => get::<JukeboxPlayableImpl>(value).serialize(seq),
        DataComponent::ProvidesBannerPatterns => {
            get::<ProvidesBannerPatternsImpl>(value).serialize(seq)
        }
        DataComponent::Recipes => get::<RecipesImpl>(value).serialize(seq),
        DataComponent::LodestoneTracker => get::<LodestoneTrackerImpl>(value).serialize(seq),
        DataComponent::FireworkExplosion => get::<FireworkExplosionImpl>(value).serialize(seq),
        DataComponent::Fireworks => get::<FireworksImpl>(value).serialize(seq),
        DataComponent::Profile => get::<ProfileImpl>(value).serialize(seq),
        DataComponent::NoteBlockSound => get::<NoteBlockSoundImpl>(value).serialize(seq),
        DataComponent::BannerPatterns => get::<BannerPatternsImpl>(value).serialize(seq),
        DataComponent::BaseColor => get::<BaseColorImpl>(value).serialize(seq),
        DataComponent::PotDecorations => get::<PotDecorationsImpl>(value).serialize(seq),
        DataComponent::Container => get::<ContainerImpl>(value).serialize(seq),
        DataComponent::BlockState => get::<BlockStateImpl>(value).serialize(seq),
        DataComponent::Bees => get::<BeesImpl>(value).serialize(seq),
        DataComponent::SulfurCubeContent => get::<SulfurCubeContentImpl>(value).serialize(seq),
        DataComponent::Lock => get::<LockImpl>(value).serialize(seq),
        DataComponent::ContainerLoot => get::<ContainerLootImpl>(value).serialize(seq),
        DataComponent::BreakSound => get::<BreakSoundImpl>(value).serialize(seq),
        DataComponent::VillagerVariant => get::<VillagerVariantImpl>(value).serialize(seq),
        DataComponent::WolfVariant => get::<WolfVariantImpl>(value).serialize(seq),
        DataComponent::WolfSoundVariant => get::<WolfSoundVariantImpl>(value).serialize(seq),
        DataComponent::WolfCollar => get::<WolfCollarImpl>(value).serialize(seq),
        DataComponent::FoxVariant => get::<FoxVariantImpl>(value).serialize(seq),
        DataComponent::SalmonSize => get::<SalmonSizeImpl>(value).serialize(seq),
        DataComponent::ParrotVariant => get::<ParrotVariantImpl>(value).serialize(seq),
        DataComponent::TropicalFishPattern => get::<TropicalFishPatternImpl>(value).serialize(seq),
        DataComponent::TropicalFishBaseColor => {
            get::<TropicalFishBaseColorImpl>(value).serialize(seq)
        }
        DataComponent::TropicalFishPatternColor => {
            get::<TropicalFishPatternColorImpl>(value).serialize(seq)
        }
        DataComponent::MooshroomVariant => get::<MooshroomVariantImpl>(value).serialize(seq),
        DataComponent::RabbitVariant => get::<RabbitVariantImpl>(value).serialize(seq),
        DataComponent::PigVariant => get::<PigVariantImpl>(value).serialize(seq),
        DataComponent::PigSoundVariant => get::<PigSoundVariantImpl>(value).serialize(seq),
        DataComponent::CowVariant => get::<CowVariantImpl>(value).serialize(seq),
        DataComponent::CowSoundVariant => get::<CowSoundVariantImpl>(value).serialize(seq),
        DataComponent::ChickenVariant => get::<ChickenVariantImpl>(value).serialize(seq),
        DataComponent::ChickenSoundVariant => get::<ChickenSoundVariantImpl>(value).serialize(seq),
        DataComponent::ZombieNautilusVariant => {
            get::<ZombieNautilusVariantImpl>(value).serialize(seq)
        }
        DataComponent::FrogVariant => get::<FrogVariantImpl>(value).serialize(seq),
        DataComponent::HorseVariant => get::<HorseVariantImpl>(value).serialize(seq),
        DataComponent::PaintingVariant => get::<PaintingVariantImpl>(value).serialize(seq),
        DataComponent::LlamaVariant => get::<LlamaVariantImpl>(value).serialize(seq),
        DataComponent::AxolotlVariant => get::<AxolotlVariantImpl>(value).serialize(seq),
        DataComponent::CatVariant => get::<CatVariantImpl>(value).serialize(seq),
        DataComponent::CatSoundVariant => get::<CatSoundVariantImpl>(value).serialize(seq),
        DataComponent::CatCollar => get::<CatCollarImpl>(value).serialize(seq),
        DataComponent::SheepColor => get::<SheepColorImpl>(value).serialize(seq),
        DataComponent::ShulkerColor => get::<ShulkerColorImpl>(value).serialize(seq),
        DataComponent::InteractAnimation => get::<InteractAnimationImpl>(value).serialize(seq),
        DataComponent::BlockTransformer => get::<BlockTransformerImpl>(value).serialize(seq),
        DataComponent::VillagerFood => get::<VillagerFoodImpl>(value).serialize(seq),
        DataComponent::Compostable => get::<CompostableImpl>(value).serialize(seq),
        DataComponent::CookingFuel => get::<CookingFuelImpl>(value).serialize(seq),
        DataComponent::BrewingFuel => get::<BrewingFuelImpl>(value).serialize(seq),
        DataComponent::MobVisibility => get::<MobVisibilityImpl>(value).serialize(seq),
        DataComponent::ProvidesPotteryPattern => {
            get::<ProvidesPotteryPatternImpl>(value).serialize(seq)
        }
        DataComponent::SignTextFront => get::<SignTextFrontImpl>(value).serialize(seq),
        DataComponent::SignTextBack => get::<SignTextBackImpl>(value).serialize(seq),
        DataComponent::Waxed => get::<WaxedImpl>(value).serialize(seq),
        DataComponent::CushionColor => get::<CushionColorImpl>(value).serialize(seq),
    }
}

impl DataComponentCodec<Self> for MapIdImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.id))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let id = seq.get_var_int()?.0;
        Ok(Self { id })
    }
}

impl DataComponentCodec<Self> for UseCooldownImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_f32(self.seconds)?;
        seq.write_bool(self.cooldown_group.is_some())?;
        if let Some(group) = &self.cooldown_group {
            seq.write_string(group)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let seconds = seq.get_f32()?;
        let cooldown_group = if seq.get_bool()? {
            Some(seq.get_str()?.into())
        } else {
            None
        };
        Ok(Self {
            seconds,
            cooldown_group,
        })
    }
}

/// `ItemStackTemplate.STREAM_CODEC`.
fn deserialize_item_stack_template(
    seq: &mut impl NetworkReadExt,
) -> Result<pumpkin_data::item_stack::ItemStack, ReadingError> {
    ItemStackSerializer::read_template0(seq, &JavaMinecraftVersion::V_26_3)
        .map(ItemStackSerializer::to_stack)
}

fn serialize_item_stack_template(
    stack: &pumpkin_data::item_stack::ItemStack,
    seq: &mut impl NetworkWriteExt,
) -> Result<(), WritingError> {
    ItemStackSerializer(Cow::Borrowed(stack)).write_template0(seq, &JavaMinecraftVersion::V_26_3)
}

impl DataComponentCodec<Self> for BundleContentsImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.items.len() as i32))?;
        for item in &self.items {
            serialize_item_stack_template(item, seq)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        const MAX_BUNDLE_ITEMS: usize = 64;

        let len = seq.get_var_int()?.0 as usize;

        if len > MAX_BUNDLE_ITEMS {
            return Err(ReadingError::Message(
                "Too many items in BundleContents".into(),
            ));
        }

        let mut items = Vec::with_capacity(len);
        for _ in 0..len {
            items.push(deserialize_item_stack_template(seq)?);
        }
        Ok(Self { items })
    }
}

fn serialize_variant(
    codec: VariantCodec,
    value: &str,
    seq: &mut impl NetworkWriteExt,
) -> Result<(), WritingError> {
    let name = value.strip_prefix("minecraft:").unwrap_or(value);
    let unknown = || WritingError::Message(format!("Unknown variant {value}"));
    let id = match codec {
        VariantCodec::SyncedRegistry(registry) => {
            synced_registry_id(registry, name).ok_or_else(unknown)? as i32
        }
        VariantCodec::BuiltinRegistry(names) | VariantCodec::Continuous(names, _) => names
            .iter()
            .position(|entry| *entry == name)
            .ok_or_else(unknown)?
            as i32,
        VariantCodec::Sparse(entries, _) => {
            entries
                .iter()
                .find(|(_, entry)| *entry == name)
                .ok_or_else(unknown)?
                .0
        }
        VariantCodec::DyeColor => i32::from(DyeColor::by_name(name).ok_or_else(unknown)?.id()),
    };
    seq.write_var_int(&VarInt(id))
}

fn deserialize_variant(
    codec: VariantCodec,
    seq: &mut impl NetworkReadExt,
) -> Result<Cow<'static, str>, ReadingError> {
    let id = seq.get_var_int()?.0;
    let unknown = || ReadingError::Message(format!("Unknown variant id {id}"));
    Ok(match codec {
        VariantCodec::SyncedRegistry(registry) => {
            let name = u32::try_from(id)
                .ok()
                .and_then(|id| synced_registry_entry(registry, id))
                .ok_or_else(unknown)?;
            Cow::Owned(format!("minecraft:{name}"))
        }
        VariantCodec::BuiltinRegistry(names) => {
            let name = usize::try_from(id)
                .ok()
                .and_then(|id| names.get(id))
                .ok_or_else(unknown)?;
            Cow::Owned(format!("minecraft:{name}"))
        }
        // `ByIdMap.continuous`
        VariantCodec::Continuous(names, strategy) => {
            let len = names.len() as i32;
            let index = match strategy {
                OutOfBoundsStrategy::Zero => {
                    if (0..len).contains(&id) {
                        id
                    } else {
                        0
                    }
                }
                OutOfBoundsStrategy::Wrap => id.rem_euclid(len),
                OutOfBoundsStrategy::Clamp => id.clamp(0, len - 1),
            };
            Cow::Borrowed(names[index as usize])
        }
        // `ByIdMap.sparse`
        VariantCodec::Sparse(entries, default) => Cow::Borrowed(
            entries
                .iter()
                .find(|(entry_id, _)| *entry_id == id)
                .map_or(default, |(_, name)| *name),
        ),
        VariantCodec::DyeColor => Cow::Borrowed(
            u8::try_from(id)
                .ok()
                .and_then(DyeColor::by_id)
                .unwrap_or(DyeColor::White)
                .name(),
        ),
    })
}

macro_rules! codec_string_variant {
    ($struct_name:ident) => {
        impl DataComponentCodec<Self> for $struct_name {
            fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
                serialize_variant(Self::CODEC, &self.value, seq)
            }
            fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
                Ok(Self {
                    value: deserialize_variant(Self::CODEC, seq)?,
                })
            }
        }
    };
}

codec_string_variant!(VillagerVariantImpl);
codec_string_variant!(WolfVariantImpl);
codec_string_variant!(WolfSoundVariantImpl);
codec_string_variant!(WolfCollarImpl);
codec_string_variant!(FoxVariantImpl);
codec_string_variant!(SalmonSizeImpl);
codec_string_variant!(ParrotVariantImpl);
codec_string_variant!(TropicalFishPatternImpl);
codec_string_variant!(TropicalFishBaseColorImpl);
codec_string_variant!(TropicalFishPatternColorImpl);
codec_string_variant!(MooshroomVariantImpl);
codec_string_variant!(RabbitVariantImpl);
codec_string_variant!(PigVariantImpl);
codec_string_variant!(PigSoundVariantImpl);
codec_string_variant!(CowVariantImpl);
codec_string_variant!(CowSoundVariantImpl);
codec_string_variant!(ChickenVariantImpl);
codec_string_variant!(ChickenSoundVariantImpl);
codec_string_variant!(ZombieNautilusVariantImpl);
codec_string_variant!(FrogVariantImpl);
codec_string_variant!(HorseVariantImpl);
codec_string_variant!(LlamaVariantImpl);
codec_string_variant!(AxolotlVariantImpl);
codec_string_variant!(CatVariantImpl);
codec_string_variant!(CatSoundVariantImpl);
codec_string_variant!(CatCollarImpl);
codec_string_variant!(SheepColorImpl);
codec_string_variant!(ShulkerColorImpl);

impl DataComponentCodec<Self> for PaintingVariantImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        match &self.variant {
            Holder::Reference(variant) => write_holder_reference(seq, variant.id()),
            Holder::Direct(definition) => {
                seq.write_var_int(&VarInt(0))?;
                seq.write_var_int(&VarInt(definition.width))?;
                seq.write_var_int(&VarInt(definition.height))?;
                seq.write_string(&definition.asset_id)?;
                seq.write_option(&definition.title, write_text)?;
                seq.write_option(&definition.author, write_text)
            }
        }
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let variant = match read_holder_id(seq, "painting variant")? {
            Some(id) => Holder::Reference(
                PaintingVariant::all()
                    .get(id as usize)
                    .copied()
                    .ok_or_else(|| {
                        ReadingError::Message(format!("Unknown painting variant id {id}"))
                    })?,
            ),
            None => Holder::Direct(Box::new(PaintingVariantDefinition {
                width: seq.get_var_int()?.0,
                height: seq.get_var_int()?.0,
                asset_id: seq.get_str()?.into(),
                title: seq.get_option(read_text)?,
                author: seq.get_option(read_text)?,
            })),
        };
        Ok(Self { variant })
    }
}

impl DataComponentCodec<Self> for MaxDamageImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.max_damage))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let max_damage = seq.get_var_int()?.0;
        Ok(Self { max_damage })
    }
}

impl DataComponentCodec<Self> for UseEffectsImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_bool(self.can_sprint)?;
        seq.write_bool(self.interact_vibrations)?;
        seq.write_f32(self.speed_multiplier)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            can_sprint: seq.get_bool()?,
            interact_vibrations: seq.get_bool()?,
            speed_multiplier: seq.get_f32()?,
        })
    }
}

impl DataComponentCodec<Self> for MinimumAttackChargeImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_f32(self.charge)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let charge = seq.get_f32()?;
        Ok(Self { charge })
    }
}

impl DataComponentCodec<Self> for DamageTypeImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.damage_type.id as i32))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let id = seq.get_var_int()?.0 as u8;
        let damage_type = pumpkin_data::damage::DamageType::from_id(id)
            .ok_or_else(|| ReadingError::Message(format!("Invalid DamageType id {id}")))?;
        Ok(Self { damage_type })
    }
}

impl DataComponentCodec<Self> for CanPlaceOnImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(0))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let count = seq.get_var_int()?.0;
        for _ in 0..count {
            let has_blocks = seq.get_bool()?;
            if has_blocks {
                let id_type = seq.get_var_int()?.0;
                if id_type == 0 {
                    let _ = seq.get_str()?;
                } else if id_type > 0 {
                    for _ in 0..(id_type - 1) {
                        let _ = seq.get_var_int()?;
                    }
                }
            }
            let has_props = seq.get_bool()?;
            if has_props {
                let props_len = seq.get_var_int()?.0;
                for _ in 0..props_len {
                    let _ = seq.get_str()?;
                    let is_exact = seq.get_bool()?;
                    if is_exact {
                        let _ = seq.get_str()?;
                    } else {
                        if seq.get_bool()? {
                            let _ = seq.get_str()?;
                        }
                        if seq.get_bool()? {
                            let _ = seq.get_str()?;
                        }
                    }
                }
            }
            let has_nbt = seq.get_bool()?;
            if has_nbt {
                let _ = seq.get_nbt_with_version(&JavaMinecraftVersion::V_26_2)?;
            }
            let exact_len = seq.get_var_int()?.0;
            for _ in 0..exact_len {
                let comp_id = seq.get_var_int()?.0 as u8;
                if let Some(comp) = DataComponent::try_from_id(comp_id) {
                    let _ = deserialize(comp, seq)?;
                }
            }
            let partial_len = seq.get_var_int()?.0;
            for _ in 0..partial_len {
                let _ = seq.get_var_int()?;
            }
        }
        Ok(Self {
            predicate: NbtTag::List(Vec::new()),
        })
    }
}

impl DataComponentCodec<Self> for CanBreakImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(0))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let count = seq.get_var_int()?.0;
        for _ in 0..count {
            let has_blocks = seq.get_bool()?;
            if has_blocks {
                let id_type = seq.get_var_int()?.0;
                if id_type == 0 {
                    let _ = seq.get_str()?;
                } else if id_type > 0 {
                    for _ in 0..(id_type - 1) {
                        let _ = seq.get_var_int()?;
                    }
                }
            }
            let has_props = seq.get_bool()?;
            if has_props {
                let props_len = seq.get_var_int()?.0;
                for _ in 0..props_len {
                    let _ = seq.get_str()?;
                    let is_exact = seq.get_bool()?;
                    if is_exact {
                        let _ = seq.get_str()?;
                    } else {
                        if seq.get_bool()? {
                            let _ = seq.get_str()?;
                        }
                        if seq.get_bool()? {
                            let _ = seq.get_str()?;
                        }
                    }
                }
            }
            let has_nbt = seq.get_bool()?;
            if has_nbt {
                let _ = seq.get_nbt_with_version(&JavaMinecraftVersion::V_26_2)?;
            }
            let exact_len = seq.get_var_int()?.0;
            for _ in 0..exact_len {
                let comp_id = seq.get_var_int()?.0 as u8;
                if let Some(comp) = DataComponent::try_from_id(comp_id) {
                    let _ = deserialize(comp, seq)?;
                }
            }
            let partial_len = seq.get_var_int()?.0;
            for _ in 0..partial_len {
                let _ = seq.get_var_int()?;
            }
        }
        Ok(Self {
            predicate: NbtTag::List(Vec::new()),
        })
    }
}

impl DataComponentCodec<Self> for AttributeModifiersImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.attribute_modifiers.len() as i32))?;
        for modifier in self.attribute_modifiers.iter() {
            seq.write_var_int(&VarInt::from(modifier.r#type.id as i32))?;
            seq.write_string(modifier.id)?;
            seq.write_f64(modifier.amount)?;
            seq.write_var_int(&VarInt::from(modifier.operation as i32))?;
            let slot_id = match modifier.slot {
                pumpkin_data::enchantment::AttributeModifierSlot::Any => 0,
                pumpkin_data::enchantment::AttributeModifierSlot::MainHand => 1,
                pumpkin_data::enchantment::AttributeModifierSlot::OffHand => 2,
                pumpkin_data::enchantment::AttributeModifierSlot::Hand => 3,
                pumpkin_data::enchantment::AttributeModifierSlot::Feet => 4,
                pumpkin_data::enchantment::AttributeModifierSlot::Legs => 5,
                pumpkin_data::enchantment::AttributeModifierSlot::Chest => 6,
                pumpkin_data::enchantment::AttributeModifierSlot::Head => 7,
                pumpkin_data::enchantment::AttributeModifierSlot::Armor => 8,
                pumpkin_data::enchantment::AttributeModifierSlot::Body => 9,
                pumpkin_data::enchantment::AttributeModifierSlot::Saddle => 10,
            };
            seq.write_var_int(&VarInt(slot_id))?;
            seq.write_var_int(&VarInt(0))?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let len = seq.get_var_int()?.0 as usize;
        for _ in 0..len {
            let _attr_id = seq.get_var_int()?;
            let _id = seq.get_str()?;
            let _amount = seq.get_f64()?;
            let _operation = seq.get_var_int()?;
            let _slot = seq.get_var_int()?;
            let display_type = seq.get_var_int()?.0;
            if display_type == 2 {
                let _ = seq.get_nbt_with_version(&JavaMinecraftVersion::V_26_2)?;
            }
        }
        Ok(Self {
            attribute_modifiers: Cow::Borrowed(&[]),
        })
    }
}

impl DataComponentCodec<Self> for CustomModelDataImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.floats.len() as i32))?;
        for f in &self.floats {
            seq.write_f32(*f)?;
        }
        seq.write_var_int(&VarInt::from(self.flags.len() as i32))?;
        for b in &self.flags {
            seq.write_bool(*b)?;
        }
        seq.write_var_int(&VarInt::from(self.strings.len() as i32))?;
        for s in &self.strings {
            seq.write_string(s)?;
        }
        seq.write_var_int(&VarInt::from(self.colors.len() as i32))?;
        for c in &self.colors {
            seq.write_i32(*c)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let floats_len = seq.get_var_int()?.0 as usize;
        let mut floats = Vec::with_capacity(floats_len);
        for _ in 0..floats_len {
            floats.push(seq.get_f32()?);
        }
        let flags_len = seq.get_var_int()?.0 as usize;
        let mut flags = Vec::with_capacity(flags_len);
        for _ in 0..flags_len {
            flags.push(seq.get_bool()?);
        }
        let strings_len = seq.get_var_int()?.0 as usize;
        let mut strings = Vec::with_capacity(strings_len);
        for _ in 0..strings_len {
            strings.push(seq.get_str()?.to_string());
        }
        let colors_len = seq.get_var_int()?.0 as usize;
        let mut colors = Vec::with_capacity(colors_len);
        for _ in 0..colors_len {
            colors.push(seq.get_i32()?);
        }
        Ok(Self {
            floats,
            flags,
            strings,
            colors,
        })
    }
}

impl DataComponentCodec<Self> for TooltipDisplayImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_bool(self.hide_tooltip)?;
        seq.write_var_int(&VarInt(self.hidden_components.len() as i32))?;
        for id in &self.hidden_components {
            seq.write_var_int(&VarInt(i32::from(id.to_id())))?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let hide_tooltip = seq.get_bool()?;
        let len = read_list_length(seq, "hidden component")?;
        let mut hidden_components = Vec::with_capacity(len);
        for _ in 0..len {
            let id = seq.get_var_int()?.0;
            let component = u8::try_from(id)
                .ok()
                .and_then(DataComponent::try_from_id)
                .ok_or_else(|| ReadingError::Message(format!("Unknown component ID: {id}")))?;
            // `ByteBufCodecs.collection` into a set keeps the first of a duplicate.
            if !hidden_components.contains(&component) {
                hidden_components.push(component);
            }
        }
        Ok(Self {
            hide_tooltip,
            hidden_components,
        })
    }
}

impl DataComponentCodec<Self> for CreativeSlotLockImpl {
    fn serialize(&self, _seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        Ok(())
    }

    fn deserialize(_seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self)
    }
}

impl DataComponentCodec<Self> for EnchantmentGlintOverrideImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_bool(self.has_glint)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            has_glint: seq.get_bool()?,
        })
    }
}

impl DataComponentCodec<Self> for IntangibleProjectileImpl {
    // No stream codec in vanilla, so `Unit.CODEC` is sent as NBT: an empty compound.
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_nbt(self.write_data())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        read_codec_nbt(seq, Self::read_data, "intangible_projectile")
    }
}

impl DataComponentCodec<Self> for FoodImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.nutrition))?;
        seq.write_f32(self.saturation)?;
        seq.write_bool(self.can_always_eat)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let nutrition = seq.get_var_int()?.0;
        let saturation = seq.get_f32()?;
        let can_always_eat = seq.get_bool()?;
        Ok(Self {
            nutrition,
            saturation,
            can_always_eat,
        })
    }
}

impl DataComponentCodec<Self> for UseRemainderImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(0))?;
        seq.write_var_int(&VarInt(0))?;
        seq.write_var_int(&VarInt(0))?;
        seq.write_var_int(&VarInt(0))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let _ = deserialize_item_stack_template(seq)?;
        Ok(Self { remainder: None })
    }
}

impl DataComponentCodec<Self> for DamageResistantImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_string(self.res_type.as_str())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let tag = seq.get_str()?;
        Ok(Self {
            res_type: DamageResistantType::from_tag(&tag),
        })
    }
}

impl DataComponentCodec<Self> for ToolImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.rules.len() as i32))?;
        for rule in self.rules.iter() {
            serialize_idset(&rule.blocks, seq)?;
            seq.write_bool(rule.speed.is_some())?;
            if let Some(speed) = rule.speed {
                seq.write_f32(speed)?;
            }
            seq.write_bool(rule.correct_for_drops.is_some())?;
            if let Some(correct) = rule.correct_for_drops {
                seq.write_bool(correct)?;
            }
        }
        seq.write_f32(self.default_mining_speed)?;
        seq.write_var_int(&VarInt::from(self.damage_per_block as i32))?;
        seq.write_bool(self.can_destroy_blocks_in_creative)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let rules_len = seq.get_var_int()?.0 as usize;
        let mut rules = Vec::with_capacity(rules_len);
        for _ in 0..rules_len {
            let blocks = deserialize_idset(seq)?;
            let speed = if seq.get_bool()? {
                Some(seq.get_f32()?)
            } else {
                None
            };
            let correct_for_drops = if seq.get_bool()? {
                Some(seq.get_bool()?)
            } else {
                None
            };
            rules.push(pumpkin_data::data_component_impl::ToolRule {
                blocks,
                speed,
                correct_for_drops,
            });
        }
        let default_mining_speed = seq.get_f32()?;
        let damage_per_block = seq.get_var_int()?.0 as u32;
        let can_destroy_blocks_in_creative = seq.get_bool()?;
        Ok(Self {
            rules: Cow::Owned(rules),
            default_mining_speed,
            damage_per_block,
            can_destroy_blocks_in_creative,
        })
    }
}

impl DataComponentCodec<Self> for WeaponImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.item_damage_per_attack as i32))?;
        seq.write_f32(0.0)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let item_damage_per_attack = seq.get_var_int()?.0 as u32;
        let _disable_blocking_for_seconds = seq.get_f32()?;
        Ok(Self {
            item_damage_per_attack,
        })
    }
}

impl DataComponentCodec<Self> for AttackRangeImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_f32(self.min_reach)?;
        seq.write_f32(self.max_reach)?;
        seq.write_f32(self.min_creative_reach)?;
        seq.write_f32(self.max_creative_reach)?;
        seq.write_f32(self.hitbox_margin)?;
        seq.write_f32(self.mob_factor)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let min_reach = seq.get_f32()?;
        let max_reach = seq.get_f32()?;
        let min_creative_reach = seq.get_f32()?;
        let max_creative_reach = seq.get_f32()?;
        let hitbox_margin = seq.get_f32()?;
        let mob_factor = seq.get_f32()?;
        Ok(Self {
            min_reach,
            max_reach,
            min_creative_reach,
            max_creative_reach,
            hitbox_margin,
            mob_factor,
        })
    }
}

impl DataComponentCodec<Self> for EnchantableImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.value))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let value = seq.get_var_int()?.0;
        Ok(Self { value })
    }
}

impl DataComponentCodec<Self> for GliderImpl {
    fn serialize(&self, _seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        Ok(())
    }

    fn deserialize(_seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self)
    }
}

impl DataComponentCodec<Self> for TooltipStyleImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_string(&self.id)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let id = seq.get_str()?.to_string();
        Ok(Self { id })
    }
}

impl DataComponentCodec<Self> for DeathProtectionImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        serialize_consume_effects(&self.death_effects, seq)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            death_effects: Cow::Owned(deserialize_consume_effects(seq)?),
        })
    }
}

/// `ByteBufCodecs.holderSet(Registries.DAMAGE_TYPE)`.
fn read_damage_type_set(seq: &mut impl NetworkReadExt) -> Result<DamageTypeSet, ReadingError> {
    let id_type = seq.get_var_int()?.0;
    if id_type == 0 {
        return Ok(DamageTypeSet::Tag(Cow::Owned(seq.get_str()?.into())));
    }
    let len = usize::try_from(id_type - 1)
        .ok()
        .filter(|len| *len <= MAX_LIST_LENGTH)
        .ok_or_else(|| ReadingError::Message(format!("Invalid damage type count {id_type}")))?;
    let mut entries = Vec::with_capacity(len);
    for _ in 0..len {
        let id = read_registry_id(seq, "damage type")?;
        let name = synced_registry_entry(DAMAGE_TYPE_REGISTRY, id)
            .ok_or_else(|| ReadingError::Message(format!("Unknown damage type id {id}")))?;
        entries.push(Cow::Borrowed(name));
    }
    Ok(DamageTypeSet::Entries(Cow::Owned(entries)))
}

fn write_damage_type_set(
    seq: &mut impl NetworkWriteExt,
    set: &DamageTypeSet,
) -> Result<(), WritingError> {
    match set {
        DamageTypeSet::Tag(tag) => {
            seq.write_var_int(&VarInt(0))?;
            seq.write_string(tag)
        }
        DamageTypeSet::Entries(entries) => {
            seq.write_var_int(&VarInt(entries.len() as i32 + 1))?;
            for name in entries.iter() {
                let id = synced_registry_id(DAMAGE_TYPE_REGISTRY, name)
                    .ok_or_else(|| WritingError::Message(format!("Unknown damage type {name}")))?;
                seq.write_var_int(&VarInt(id as i32))?;
            }
            Ok(())
        }
    }
}

const DAMAGE_TYPE_REGISTRY: &str = "damage_type";

impl DataComponentCodec<Self> for BlocksAttacksImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_f32(self.block_delay_seconds)?;
        seq.write_f32(self.disable_cooldown_scale)?;
        seq.write_var_int(&VarInt(self.damage_reductions.len() as i32))?;
        for reduction in self.damage_reductions.iter() {
            seq.write_f32(reduction.horizontal_blocking_angle)?;
            seq.write_option(&reduction.r#type, write_damage_type_set)?;
            seq.write_f32(reduction.base)?;
            seq.write_f32(reduction.factor)?;
        }
        seq.write_f32(self.item_damage.threshold)?;
        seq.write_f32(self.item_damage.base)?;
        seq.write_f32(self.item_damage.factor)?;
        seq.write_option(&self.bypassed_by, write_damage_type_set)?;
        seq.write_option(&self.block_sound, write_sound)?;
        seq.write_option(&self.disabled_sound, write_sound)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let block_delay_seconds = seq.get_f32()?;
        let disable_cooldown_scale = seq.get_f32()?;
        let len = read_list_length(seq, "damage reduction")?;
        let mut damage_reductions = Vec::with_capacity(len);
        for _ in 0..len {
            damage_reductions.push(DamageReduction {
                horizontal_blocking_angle: seq.get_f32()?,
                r#type: seq.get_option(read_damage_type_set)?,
                base: seq.get_f32()?,
                factor: seq.get_f32()?,
            });
        }
        let item_damage = ItemDamageFunction {
            threshold: seq.get_f32()?,
            base: seq.get_f32()?,
            factor: seq.get_f32()?,
        };
        Ok(Self {
            block_delay_seconds,
            disable_cooldown_scale,
            damage_reductions: Cow::Owned(damage_reductions),
            item_damage,
            bypassed_by: seq.get_option(read_damage_type_set)?,
            block_sound: seq.get_option(read_sound)?,
            disabled_sound: seq.get_option(read_sound)?,
        })
    }
}

impl DataComponentCodec<Self> for PiercingWeaponImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_bool(self.deals_knockback)?;
        seq.write_bool(self.dismounts)?;
        if let Some(sound) = &self.sound {
            seq.write_bool(true)?;
            let proto_sound = data_to_proto_sound(sound);
            crate::IdOr::<crate::SoundEvent>::write(&proto_sound, seq, |w, e| {
                w.write_string(&e.sound_name)?;
                w.write_option(&e.range, |w2, r| w2.write_f32(*r))
            })?;
        } else {
            seq.write_bool(false)?;
        }
        if let Some(sound) = &self.hit_sound {
            seq.write_bool(true)?;
            let proto_sound = data_to_proto_sound(sound);
            crate::IdOr::<crate::SoundEvent>::write(&proto_sound, seq, |w, e| {
                w.write_string(&e.sound_name)?;
                w.write_option(&e.range, |w2, r| w2.write_f32(*r))
            })?;
        } else {
            seq.write_bool(false)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let deals_knockback = seq.get_bool()?;
        let dismounts = seq.get_bool()?;
        let sound = if seq.get_bool()? {
            let proto = crate::IdOr::<crate::SoundEvent>::read(seq, |r| {
                let sound_name = r.get_str()?.to_string();
                let range = r.get_option(NetworkReadExt::get_f32)?;
                Ok(crate::SoundEvent { sound_name, range })
            })
            .map_err(|e| ReadingError::Message(format!("No sound: {e}")))?;
            proto_to_data_sound(&proto)
        } else {
            None
        };
        let hit_sound = if seq.get_bool()? {
            let proto = crate::IdOr::<crate::SoundEvent>::read(seq, |r| {
                let sound_name = r.get_str()?.to_string();
                let range = r.get_option(NetworkReadExt::get_f32)?;
                Ok(crate::SoundEvent { sound_name, range })
            })
            .map_err(|e| ReadingError::Message(format!("No sound: {e}")))?;
            proto_to_data_sound(&proto)
        } else {
            None
        };
        Ok(Self {
            deals_knockback,
            dismounts,
            sound,
            hit_sound,
        })
    }
}

fn serialize_kinetic_condition(
    cond: &pumpkin_data::data_component_impl::KineticConditionImpl,
    seq: &mut impl NetworkWriteExt,
) -> Result<(), WritingError> {
    seq.write_var_int(&VarInt::from(cond.max_duration_ticks))?;
    seq.write_f32(cond.min_speed)?;
    seq.write_f32(cond.min_relative_speed)
}

fn deserialize_kinetic_condition(
    seq: &mut impl NetworkReadExt,
) -> Result<pumpkin_data::data_component_impl::KineticConditionImpl, ReadingError> {
    let max_duration_ticks = seq.get_var_int()?.0;
    let min_speed = seq.get_f32()?;
    let min_relative_speed = seq.get_f32()?;
    Ok(pumpkin_data::data_component_impl::KineticConditionImpl {
        max_duration_ticks,
        min_speed,
        min_relative_speed,
    })
}

impl DataComponentCodec<Self> for KineticWeaponImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.contact_cooldown_ticks))?;
        seq.write_var_int(&VarInt::from(self.delay_ticks))?;
        if let Some(cond) = &self.dismount_conditions {
            seq.write_bool(true)?;
            serialize_kinetic_condition(cond, seq)?;
        } else {
            seq.write_bool(false)?;
        }
        if let Some(cond) = &self.knockback_conditions {
            seq.write_bool(true)?;
            serialize_kinetic_condition(cond, seq)?;
        } else {
            seq.write_bool(false)?;
        }
        if let Some(cond) = &self.damage_conditions {
            seq.write_bool(true)?;
            serialize_kinetic_condition(cond, seq)?;
        } else {
            seq.write_bool(false)?;
        }
        seq.write_f32(self.forward_movement)?;
        seq.write_f32(self.damage_multiplier)?;
        if let Some(sound) = &self.sound {
            seq.write_bool(true)?;
            let proto = data_to_proto_sound(sound);
            crate::IdOr::<crate::SoundEvent>::write(&proto, seq, |w, e| {
                w.write_string(&e.sound_name)?;
                w.write_option(&e.range, |w2, r| w2.write_f32(*r))
            })?;
        } else {
            seq.write_bool(false)?;
        }
        if let Some(sound) = &self.hit_sound {
            seq.write_bool(true)?;
            let proto = data_to_proto_sound(sound);
            crate::IdOr::<crate::SoundEvent>::write(&proto, seq, |w, e| {
                w.write_string(&e.sound_name)?;
                w.write_option(&e.range, |w2, r| w2.write_f32(*r))
            })?;
        } else {
            seq.write_bool(false)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let contact_cooldown_ticks = seq.get_var_int()?.0;
        let delay_ticks = seq.get_var_int()?.0;
        let dismount_conditions = if seq.get_bool()? {
            Some(deserialize_kinetic_condition(seq)?)
        } else {
            None
        };
        let knockback_conditions = if seq.get_bool()? {
            Some(deserialize_kinetic_condition(seq)?)
        } else {
            None
        };
        let damage_conditions = if seq.get_bool()? {
            Some(deserialize_kinetic_condition(seq)?)
        } else {
            None
        };
        let forward_movement = seq.get_f32()?;
        let damage_multiplier = seq.get_f32()?;
        let sound = if seq.get_bool()? {
            let proto = crate::IdOr::<crate::SoundEvent>::read(seq, |r| {
                let sound_name = r.get_str()?.to_string();
                let range = r.get_option(NetworkReadExt::get_f32)?;
                Ok(crate::SoundEvent { sound_name, range })
            })
            .map_err(|e| ReadingError::Message(format!("No sound: {e}")))?;
            proto_to_data_sound(&proto)
        } else {
            None
        };
        let hit_sound = if seq.get_bool()? {
            let proto = crate::IdOr::<crate::SoundEvent>::read(seq, |r| {
                let sound_name = r.get_str()?.to_string();
                let range = r.get_option(NetworkReadExt::get_f32)?;
                Ok(crate::SoundEvent { sound_name, range })
            })
            .map_err(|e| ReadingError::Message(format!("No sound: {e}")))?;
            proto_to_data_sound(&proto)
        } else {
            None
        };
        Ok(Self {
            contact_cooldown_ticks,
            delay_ticks,
            dismount_conditions,
            knockback_conditions,
            damage_conditions,
            forward_movement,
            damage_multiplier,
            sound,
            hit_sound,
        })
    }
}

impl DataComponentCodec<Self> for AdditionalTradeCostImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(self.cost))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            cost: seq.get_var_int()?.0,
        })
    }
}

impl DataComponentCodec<Self> for DyeImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        write_dye_color(seq, self.color)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            color: read_dye_color(seq)?,
        })
    }
}

impl DataComponentCodec<Self> for MapDecorationsImpl {
    // No stream codec in vanilla, so the persistent codec is sent as NBT.
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_nbt(self.write_data())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        read_codec_nbt(seq, Self::read_data, "map_decorations")
    }
}

impl DataComponentCodec<Self> for MapPostProcessingImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.processing.map_or(0, |p| p as i32)))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let val = seq.get_var_int()?.0;
        let processing = match val {
            0 => Some(pumpkin_data::data_component_impl::MapPostProcessing::Lock),
            1 => Some(pumpkin_data::data_component_impl::MapPostProcessing::Scale),
            _ => None,
        };
        Ok(Self { processing })
    }
}

impl DataComponentCodec<Self> for ChargedProjectilesImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.projectiles.len() as i32))?;
        for _ in &self.projectiles {
            seq.write_var_int(&VarInt(0))?;
            seq.write_var_int(&VarInt(0))?;
            seq.write_var_int(&VarInt(0))?;
            seq.write_var_int(&VarInt(0))?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let len = seq.get_var_int()?.0 as usize;
        let mut projectiles = Vec::with_capacity(len);
        for _ in 0..len {
            let _ = deserialize_item_stack_template(seq)?;
            projectiles.push(pumpkin_nbt::compound::NbtCompound::new());
        }
        Ok(Self { projectiles })
    }
}

impl DataComponentCodec<Self> for PotionDurationScaleImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_f32(self.scale)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let scale = seq.get_f32()?;
        Ok(Self { scale })
    }
}

impl DataComponentCodec<Self> for WritableBookContentImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.pages.len() as i32))?;
        for page in &self.pages {
            seq.write_string(page)?;
            seq.write_bool(false)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let len = seq.get_var_int()?.0 as usize;
        let mut pages = Vec::with_capacity(len);
        for _ in 0..len {
            let raw = seq.get_str()?.to_string();
            let has_filtered = seq.get_bool()?;
            if has_filtered {
                let _ = seq.get_str()?;
            }
            pages.push(raw);
        }
        Ok(Self { pages })
    }
}

impl DataComponentCodec<Self> for WrittenBookContentImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_string(&self.title)?;
        seq.write_bool(false)?;
        seq.write_string(&self.author)?;
        seq.write_var_int(&VarInt(0))?;
        seq.write_var_int(&VarInt::from(self.pages.len() as i32))?;
        for page in &self.pages {
            seq.write_slice(&page.encode_for_version(&JavaMinecraftVersion::V_26_2))?;
            seq.write_bool(false)?;
        }
        seq.write_bool(true)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let title = seq.get_str()?.to_string();
        if seq.get_bool()? {
            let _ = seq.get_str()?;
        }
        let author = seq.get_str()?.to_string();
        let _generation = seq.get_var_int()?.0;
        let pages_len = seq.get_var_int()?.0 as usize;
        let mut pages = Vec::with_capacity(pages_len);
        for _ in 0..pages_len {
            let tag = seq.get_nbt_with_version(&JavaMinecraftVersion::V_26_2)?;
            let comp = tag.as_ref().map_or_else(
                pumpkin_util::text::TextComponent::empty,
                pumpkin_util::text::TextComponent::from_nbt,
            );
            if seq.get_bool()? {
                let _ = seq.get_nbt_with_version(&JavaMinecraftVersion::V_26_2)?;
            }
            pages.push(comp);
        }
        let _resolved = seq.get_bool()?;
        Ok(Self {
            title,
            author,
            pages,
        })
    }
}

impl DataComponentCodec<Self> for TrimImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(0))?;
        seq.write_var_int(&VarInt(0))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let _material = seq.get_var_int()?;
        let _pattern = seq.get_var_int()?;
        Ok(Self {
            material: NbtTag::String("minecraft:quartz".into()),
            pattern: NbtTag::String("minecraft:coast".into()),
        })
    }
}

impl DataComponentCodec<Self> for DebugStickStateImpl {
    // No stream codec in vanilla, so the persistent codec is sent as NBT.
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_nbt(self.write_data())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        read_codec_nbt(seq, Self::read_data, "debug_stick_state")
    }
}

impl DataComponentCodec<Self> for EntityDataImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        let mut nbt = self.nbt.clone().unwrap_or_default();
        // Vanilla TypedEntityData always carries its type, there is no fallback.
        let id = nbt
            .get_string("id")
            .ok_or_else(|| WritingError::Message("entity_data has no 'id'".into()))?;
        let type_id = EntityType::from_name(id.strip_prefix("minecraft:").unwrap_or(id))
            .map(|entity_type| i32::from(entity_type.id))
            .ok_or_else(|| WritingError::Message(format!("Unknown entity type {id}")))?;
        nbt.child_tags.remove("id");
        seq.write_var_int(&VarInt(type_id))?;
        seq.write_nbt(NbtTag::Compound(nbt))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let type_id = seq.get_var_int()?.0;
        let tag = seq.get_nbt_with_version(&JavaMinecraftVersion::V_26_2)?;
        // Vanilla's TypedEntityData keeps the type apart from the tag.
        //Pumpkin keeps it as "id" in the NBT.
        let entity_type = u16::try_from(type_id)
            .ok()
            .and_then(EntityType::from_raw)
            .ok_or_else(|| ReadingError::Message(format!("Unknown entity type id {type_id}")))?;
        let mut nbt = match tag {
            Some(NbtTag::Compound(c)) => c,
            _ => pumpkin_nbt::compound::NbtCompound::new(),
        };
        nbt.put_string("id", format!("minecraft:{}", entity_type.resource_name));
        Ok(Self { nbt: Some(nbt) })
    }
}

impl DataComponentCodec<Self> for BucketEntityDataImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_nbt(NbtTag::Compound(self.nbt()))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            nbt: Some(read_compound(seq)?),
        })
    }
}

impl DataComponentCodec<Self> for BlockEntityDataImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(0))?;
        seq.write_nbt(NbtTag::Compound(self.nbt.clone()))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let _type_id = seq.get_var_int()?;
        let tag = seq.get_nbt_with_version(&JavaMinecraftVersion::V_26_2)?;
        let nbt = if let Some(NbtTag::Compound(c)) = tag {
            c
        } else {
            pumpkin_nbt::compound::NbtCompound::new()
        };
        Ok(Self { nbt })
    }
}

impl DataComponentCodec<Self> for InstrumentImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        match &self.instrument {
            Holder::Reference(instrument) => write_holder_reference(seq, instrument.id()),
            Holder::Direct(definition) => {
                seq.write_var_int(&VarInt(0))?;
                write_sound(seq, &definition.sound_event)?;
                seq.write_f32(definition.use_duration)?;
                seq.write_f32(definition.range)?;
                seq.write_var_int(&VarInt(definition.durability_damage))?;
                write_text(seq, &definition.description)
            }
        }
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let instrument = match read_holder_id(seq, "instrument")? {
            Some(id) => Holder::Reference(
                Instrument::all()
                    .get(id as usize)
                    .copied()
                    .ok_or_else(|| ReadingError::Message(format!("Unknown instrument id {id}")))?,
            ),
            None => Holder::Direct(Box::new(InstrumentDefinition {
                sound_event: read_sound(seq)?,
                use_duration: seq.get_f32()?,
                range: seq.get_f32()?,
                durability_damage: seq.get_var_int()?.0,
                description: read_text(seq)?,
            })),
        };
        Ok(Self { instrument })
    }
}

impl DataComponentCodec<Self> for ProvidesTrimMaterialImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        match &self.material {
            Holder::Reference(material) => write_holder_reference(seq, material.id()),
            Holder::Direct(definition) => {
                seq.write_var_int(&VarInt(0))?;
                seq.write_string(&definition.palette_id)?;
                write_text(seq, &definition.description)
            }
        }
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let material = match read_holder_id(seq, "trim material")? {
            Some(id) => {
                Holder::Reference(TrimMaterial::all().get(id as usize).copied().ok_or_else(
                    || ReadingError::Message(format!("Unknown trim material id {id}")),
                )?)
            }
            None => Holder::Direct(TrimMaterialDefinition {
                palette_id: seq.get_str()?.into(),
                description: read_text(seq)?,
            }),
        };
        Ok(Self { material })
    }
}

impl DataComponentCodec<Self> for OminousBottleAmplifierImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.amplifier))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let amplifier = seq.get_var_int()?.0;
        Ok(Self { amplifier })
    }
}

impl DataComponentCodec<Self> for JukeboxPlayableImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        match &self.song {
            Holder::Reference(song) => write_holder_reference(seq, song.get_id()),
            Holder::Direct(definition) => {
                seq.write_var_int(&VarInt(0))?;
                write_sound(seq, &definition.sound_event)?;
                write_text(seq, &definition.description)?;
                seq.write_f32(definition.length_in_seconds)?;
                seq.write_var_int(&VarInt(definition.comparator_output))
            }
        }
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let song =
            match read_holder_id(seq, "jukebox song")? {
                Some(id) => Holder::Reference(JukeboxSong::from_id(id).ok_or_else(|| {
                    ReadingError::Message(format!("Unknown jukebox song id {id}"))
                })?),
                None => Holder::Direct(Box::new(JukeboxSongDefinition {
                    sound_event: read_sound(seq)?,
                    description: read_text(seq)?,
                    length_in_seconds: seq.get_f32()?,
                    comparator_output: seq.get_var_int()?.0,
                })),
            };
        Ok(Self { song })
    }
}

impl DataComponentCodec<Self> for ProvidesBannerPatternsImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        serialize_idset(&self.patterns, seq)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            patterns: deserialize_idset(seq)?,
        })
    }
}

impl DataComponentCodec<Self> for RecipesImpl {
    // No stream codec in vanilla, so the persistent codec is sent as NBT.
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_nbt(self.write_data())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        read_codec_nbt(seq, Self::read_data, "recipes")
    }
}

impl DataComponentCodec<Self> for LodestoneTrackerImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        if let Some(target) = &self.target {
            seq.write_bool(true)?;
            seq.write_string(&target.dimension)?;
            let pos = pumpkin_util::math::position::BlockPos::new(target.x, target.y, target.z);
            seq.write_block_pos(&pos, &JavaMinecraftVersion::V_26_2)?;
        } else {
            seq.write_bool(false)?;
        }
        seq.write_bool(self.tracked)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let target = if seq.get_bool()? {
            let dimension = seq.get_str()?.to_string();
            let pos = seq.get_block_pos(&JavaMinecraftVersion::V_26_2)?;
            Some(pumpkin_data::data_component_impl::LodestoneTarget {
                dimension,
                x: pos.0.x,
                y: pos.0.y,
                z: pos.0.z,
            })
        } else {
            None
        };
        let tracked = seq.get_bool()?;
        Ok(Self { target, tracked })
    }
}

impl DataComponentCodec<Self> for ProfileImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_bool(false)?;
        if let Some(name) = &self.name {
            seq.write_bool(true)?;
            seq.write_string(name)?;
        } else {
            seq.write_bool(false)?;
        }
        if let Some(id) = &self.id {
            seq.write_bool(true)?;
            let uuid = uuid::Uuid::from_u128(
                ((id[0] as u128) << 96)
                    | ((id[1] as u128 & 0xFFFFFFFF) << 64)
                    | ((id[2] as u128 & 0xFFFFFFFF) << 32)
                    | (id[3] as u128 & 0xFFFFFFFF),
            );
            seq.write_uuid(&uuid)?;
        } else {
            seq.write_bool(false)?;
        }
        seq.write_var_int(&VarInt::from(self.properties.len() as i32))?;
        for prop in &self.properties {
            seq.write_string(&prop.name)?;
            seq.write_string(&prop.value)?;
            if let Some(sig) = &prop.signature {
                seq.write_bool(true)?;
                seq.write_string(sig)?;
            } else {
                seq.write_bool(false)?;
            }
        }
        if let Some(texture) = &self.texture {
            seq.write_bool(true)?;
            seq.write_string(texture)?;
        } else {
            seq.write_bool(false)?;
        }
        if let Some(cape) = &self.cape {
            seq.write_bool(true)?;
            seq.write_string(cape)?;
        } else {
            seq.write_bool(false)?;
        }
        if let Some(elytra) = &self.elytra {
            seq.write_bool(true)?;
            seq.write_string(elytra)?;
        } else {
            seq.write_bool(false)?;
        }
        if self.model.is_some() {
            seq.write_bool(true)?;
            seq.write_var_int(&VarInt(0))?;
        } else {
            seq.write_bool(false)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let full_profile = seq.get_bool()?;
        let mut name = None;
        let mut id = None;
        let mut properties = Vec::new();
        if full_profile {
            let uuid = seq.get_uuid()?;
            let u = uuid.as_u128();
            id = Some([
                (u >> 96) as i32,
                (u >> 64) as i32,
                (u >> 32) as i32,
                u as i32,
            ]);
            name = Some(seq.get_str()?.to_string());
        } else {
            if seq.get_bool()? {
                name = Some(seq.get_str()?.to_string());
            }
            if seq.get_bool()? {
                let uuid = seq.get_uuid()?;
                let u = uuid.as_u128();
                id = Some([
                    (u >> 96) as i32,
                    (u >> 64) as i32,
                    (u >> 32) as i32,
                    u as i32,
                ]);
            }
        }
        let props_len = seq.get_var_int()?.0 as usize;
        for _ in 0..props_len {
            let prop_name = seq.get_str()?.to_string();
            let prop_value = seq.get_str()?.to_string();
            let sig = if seq.get_bool()? {
                Some(seq.get_str()?.to_string())
            } else {
                None
            };
            properties.push(pumpkin_data::data_component_impl::ProfileProperty {
                name: prop_name,
                value: prop_value,
                signature: sig,
            });
        }
        let texture = if seq.get_bool()? {
            Some(seq.get_str()?.to_string())
        } else {
            None
        };
        let cape = if seq.get_bool()? {
            Some(seq.get_str()?.to_string())
        } else {
            None
        };
        let elytra = if seq.get_bool()? {
            Some(seq.get_str()?.to_string())
        } else {
            None
        };
        let model = if seq.get_bool()? {
            let _ = seq.get_var_int()?;
            Some("wide".to_string())
        } else {
            None
        };
        Ok(Self {
            name,
            id,
            properties,
            texture,
            cape,
            elytra,
            model,
        })
    }
}

impl DataComponentCodec<Self> for NoteBlockSoundImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_string(&self.sound)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let sound = seq.get_str()?.to_string();
        Ok(Self { sound })
    }
}

impl DataComponentCodec<Self> for BannerPatternsImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.layers.len() as i32))?;
        for layer in &self.layers {
            seq.write_var_int(&VarInt(0))?;
            seq.write_var_int(&VarInt::from(layer.color.id() as i32))?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let len = seq.get_var_int()?.0 as usize;
        let mut layers = Vec::with_capacity(len);
        for _ in 0..len {
            let _pattern = seq.get_var_int()?.0;
            let color_id = seq.get_var_int()?.0 as u8;
            let color = pumpkin_data::dye_color::DyeColor::by_id(color_id).unwrap_or_default();
            layers.push(pumpkin_data::data_component_impl::BannerPatternLayer {
                pattern: String::new(),
                color,
            });
        }
        Ok(Self { layers })
    }
}

impl DataComponentCodec<Self> for BaseColorImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        let color_id =
            pumpkin_data::dye_color::DyeColor::by_name(&self.color).map_or(0, |c| c.id() as i32);
        seq.write_var_int(&VarInt::from(color_id))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let id = seq.get_var_int()?.0 as u8;
        let color = pumpkin_data::dye_color::DyeColor::by_id(id)
            .map_or("white", |c| c.name())
            .to_string();
        Ok(Self { color })
    }
}

impl DataComponentCodec<Self> for PotDecorationsImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        for side in self.sides() {
            seq.write_option(side, |seq, stack| serialize_item_stack_template(stack, seq))?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            back: seq.get_option(deserialize_item_stack_template)?,
            left: seq.get_option(deserialize_item_stack_template)?,
            right: seq.get_option(deserialize_item_stack_template)?,
            front: seq.get_option(deserialize_item_stack_template)?,
        })
    }
}

impl DataComponentCodec<Self> for ContainerImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.items.len() as i32))?;
        for (_slot, stack) in &self.items {
            seq.write_bool(true)?;
            serialize_item_stack_template(stack, seq)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let len = seq.get_var_int()?.0 as usize;
        let mut items = Vec::with_capacity(len);
        for slot in 0..len {
            if seq.get_bool()? {
                let stack = deserialize_item_stack_template(seq)?;
                items.push((slot as u8, stack));
            }
        }
        Ok(Self { items })
    }
}

impl DataComponentCodec<Self> for BlockStateImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.properties.len() as i32))?;
        for (k, v) in self.properties.iter() {
            seq.write_string(k)?;
            seq.write_string(v)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let len = seq.get_var_int()?.0 as usize;
        let mut properties = Vec::with_capacity(len);
        for _ in 0..len {
            let k = seq.get_str()?.to_string();
            let v = seq.get_str()?.to_string();
            properties.push((Cow::Owned(k), Cow::Owned(v)));
        }
        Ok(Self {
            properties: Cow::Owned(properties),
        })
    }
}

impl DataComponentCodec<Self> for BeesImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(self.bees.len() as i32))?;
        for bee in &self.bees {
            write_typed_entity_data(seq, &bee.entity_data)?;
            seq.write_var_int(&VarInt(bee.ticks_in_hive))?;
            seq.write_var_int(&VarInt(bee.min_ticks_in_hive))?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let len = read_list_length(seq, "bee")?;
        let mut bees = Vec::with_capacity(len);
        for _ in 0..len {
            bees.push(BeeOccupant {
                entity_data: read_typed_entity_data(seq)?,
                ticks_in_hive: seq.get_var_int()?.0,
                min_ticks_in_hive: seq.get_var_int()?.0,
            });
        }
        Ok(Self { bees })
    }
}

impl DataComponentCodec<Self> for SulfurCubeContentImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        serialize_item_stack_template(&self.absorbed_block, seq)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            absorbed_block: deserialize_item_stack_template(seq)?,
        })
    }
}

impl DataComponentCodec<Self> for LockImpl {
    // No stream codec in vanilla, so the persistent codec is sent as NBT.
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_nbt(self.write_data())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        read_codec_nbt(seq, Self::read_data, "lock")
    }
}

impl DataComponentCodec<Self> for ContainerLootImpl {
    // No stream codec in vanilla, so the persistent codec is sent as NBT.
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_nbt(self.write_data())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        read_codec_nbt(seq, Self::read_data, "container_loot")
    }
}

impl DataComponentCodec<Self> for BreakSoundImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        write_sound(seq, &self.sound)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            sound: read_sound(seq)?,
        })
    }
}

impl DataComponentCodec<Self> for InteractAnimationImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        self.0.serialize(seq)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        SwingAnimationImpl::deserialize(seq).map(Self)
    }
}

impl DataComponentCodec<Self> for BlockTransformerImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        let id = synced_registry_id(Self::REGISTRY, &self.transformer).ok_or_else(|| {
            WritingError::Message(format!("Unknown block transformer {}", self.transformer))
        })?;
        seq.write_var_int(&VarInt(id as i32))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let id = read_registry_id(seq, "block transformer")?;
        let name = synced_registry_entry(Self::REGISTRY, id)
            .ok_or_else(|| ReadingError::Message(format!("Unknown block transformer id {id}")))?;
        Ok(Self {
            transformer: Cow::Owned(format!("minecraft:{name}")),
        })
    }
}

impl DataComponentCodec<Self> for VillagerFoodImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(self.nutrition))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            nutrition: seq.get_var_int()?.0,
        })
    }
}

impl DataComponentCodec<Self> for CompostableImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        write_int_provider(seq, &self.layers)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            layers: read_int_provider(seq)?,
        })
    }
}

impl DataComponentCodec<Self> for CookingFuelImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        write_int_provider(seq, &self.burn_time)?;
        write_float_provider(seq, &self.speed_multiplier)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            burn_time: read_int_provider(seq)?,
            speed_multiplier: read_float_provider(seq)?,
        })
    }
}

impl DataComponentCodec<Self> for BrewingFuelImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        write_int_provider(seq, &self.uses)?;
        write_float_provider(seq, &self.speed_multiplier)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            uses: read_int_provider(seq)?,
            speed_multiplier: read_float_provider(seq)?,
        })
    }
}

impl DataComponentCodec<Self> for MobVisibilityImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        serialize_idset(&self.targeting_entity_types, seq)?;
        seq.write_f32(self.visibility)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            targeting_entity_types: deserialize_idset(seq)?,
            visibility: seq.get_f32()?,
        })
    }
}

impl DataComponentCodec<Self> for ProvidesPotteryPatternImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(self.pattern.id() as i32))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let id = read_registry_id(seq, "pottery pattern")?;
        let pattern = DecoratedPotPattern::all()
            .get(id as usize)
            .copied()
            .ok_or_else(|| ReadingError::Message(format!("Unknown pottery pattern id {id}")))?;
        Ok(Self { pattern })
    }
}

/// `SignText.STREAM_CODEC`.
fn serialize_sign_text(
    text: &SignText,
    seq: &mut impl NetworkWriteExt,
) -> Result<(), WritingError> {
    for line in &text.messages {
        write_text(seq, line)?;
    }
    seq.write_option(&text.filtered_messages, |seq, lines| {
        lines.iter().try_for_each(|line| write_text(seq, line))
    })?;
    write_dye_color(seq, text.color)?;
    seq.write_bool(text.has_glowing_text)
}

fn deserialize_sign_text(seq: &mut impl NetworkReadExt) -> Result<SignText, ReadingError> {
    fn read_lines(
        seq: &mut impl NetworkReadExt,
    ) -> Result<[TextComponent; SignText::LINES], ReadingError> {
        Ok([
            read_text(seq)?,
            read_text(seq)?,
            read_text(seq)?,
            read_text(seq)?,
        ])
    }
    let messages = read_lines(seq)?;
    let filtered_messages = seq
        .get_option(read_lines)?
        .filter(|filtered| *filtered != messages);
    Ok(SignText {
        messages,
        filtered_messages,
        color: read_dye_color(seq)?,
        has_glowing_text: seq.get_bool()?,
    })
}

impl DataComponentCodec<Self> for SignTextFrontImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        serialize_sign_text(&self.0, seq)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        deserialize_sign_text(seq).map(Self)
    }
}

impl DataComponentCodec<Self> for SignTextBackImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        serialize_sign_text(&self.0, seq)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        deserialize_sign_text(seq).map(Self)
    }
}

impl DataComponentCodec<Self> for WaxedImpl {
    fn serialize(&self, _seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        Ok(())
    }

    fn deserialize(_seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self)
    }
}

impl DataComponentCodec<Self> for CushionColorImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        write_dye_color(seq, self.color)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            color: read_dye_color(seq)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn textured_profile() -> ProfileImpl {
        ProfileImpl {
            name: Some("Notch".to_string()),
            properties: vec![ProfileProperty {
                name: "textures".to_string(),
                value: "eyJ0ZXh0dXJlcyI6e319".to_string(),
                signature: None,
            }],
            ..Default::default()
        }
    }

    /// The wire form vanilla expects for `textured_profile`.
    fn textured_profile_bytes() -> Vec<u8> {
        let mut bytes = vec![
            0, // partial profile tag
            1, 5, // name present, 5 bytes
        ];
        bytes.extend_from_slice(b"Notch");
        bytes.push(0); // no id
        bytes.push(1); // one property
        bytes.push(8); // property name, 8 bytes
        bytes.extend_from_slice(b"textures");
        bytes.push(20); // property value, 20 bytes
        bytes.extend_from_slice(b"eyJ0ZXh0dXJlcyI6e319");
        bytes.push(0); // no signature
        bytes.extend_from_slice(&[0, 0, 0, 0]); // no texture, cape, elytra, model
        bytes
    }

    #[test]
    fn partial_profile_is_written_with_a_false_tag() {
        let mut encoded = Vec::new();
        textured_profile().serialize(&mut encoded).unwrap();
        assert_eq!(encoded, textured_profile_bytes());
    }

    #[test]
    fn partial_profile_is_read_as_name_then_id() {
        let decoded = ProfileImpl::deserialize(&mut textured_profile_bytes().as_slice()).unwrap();
        assert_eq!(decoded, textured_profile());
    }

    #[test]
    fn full_profile_is_read_as_id_then_name() {
        let id = uuid::Uuid::from_u128(0x0000_0001_0000_0002_0000_0003_0000_0004);
        let mut encoded = vec![1];
        encoded.extend_from_slice(id.as_bytes());
        encoded.push(5);
        encoded.extend_from_slice(b"Notch");
        encoded.extend_from_slice(&[0, 0, 0, 0, 0]);

        let decoded = ProfileImpl::deserialize(&mut encoded.as_slice()).unwrap();
        assert_eq!(decoded.name.as_deref(), Some("Notch"));
        assert_eq!(decoded.id, Some([1, 2, 3, 4]));
    }

    // Wire bytes below were produced by vanilla 26.3's stream codecs.

    fn encode<T: DataComponentImpl + DataComponentCodec<T>>(value: &T) -> Vec<u8> {
        let mut bytes = Vec::new();
        value.serialize(&mut bytes).unwrap();
        bytes
    }

    /// Decodes `bytes` and checks nothing is left over for the next component.
    fn decode<T: DataComponentImpl + DataComponentCodec<T>>(bytes: &[u8]) -> T {
        let mut slice = bytes;
        let value = T::deserialize(&mut slice).unwrap();
        assert!(slice.is_empty(), "{} bytes left unread", slice.len());
        value
    }

    fn assert_wire<T>(value: &T, bytes: &[u8])
    where
        T: DataComponentImpl + DataComponentCodec<T> + PartialEq + std::fmt::Debug,
    {
        assert_eq!(encode(value), bytes);
        assert_eq!(&decode::<T>(bytes), value);
    }

    fn custom_horn_sound() -> IdOr<SoundEvent> {
        IdOr::Value(SoundEvent {
            sound_name: "minecraft:custom.horn".to_string(),
            range: Some(32.0),
        })
    }

    /// `SoundEvent.DIRECT_STREAM_CODEC` of `custom_horn_sound` behind a holder id of 0.
    const CUSTOM_HORN_SOUND: &[u8] = &[
        0x00, 0x15, b'm', b'i', b'n', b'e', b'c', b'r', b'a', b'f', b't', b':', b'c', b'u', b's',
        b't', b'o', b'm', b'.', b'h', b'o', b'r', b'n', 0x01, 0x42, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn instrument_reference_is_holder_id_plus_one() {
        // ponder_goat_horn is id 4 in the instrument registry.
        assert_wire(
            &InstrumentImpl {
                instrument: Holder::Reference(Instrument::PonderGoatHorn),
            },
            &[0x05],
        );
    }

    #[test]
    fn instrument_inline_value_is_fully_read() {
        let mut bytes = vec![0x00];
        bytes.extend_from_slice(CUSTOM_HORN_SOUND);
        bytes.extend_from_slice(&[0x40, 0xe0, 0x00, 0x00, 0x43, 0x80, 0x00, 0x00, 0x02]);
        bytes.extend_from_slice(&[0x08, 0x00, 0x04, b'H', b'o', b'r', b'n']);
        let decoded = decode::<InstrumentImpl>(&bytes);
        let Holder::Direct(definition) = &decoded.instrument else {
            panic!("expected an inline instrument");
        };
        assert_eq!(definition.sound_event, custom_horn_sound());
        assert_eq!(
            (
                definition.use_duration,
                definition.range,
                definition.durability_damage
            ),
            (7.0, 256.0, 2)
        );
        assert_eq!(definition.description.clone().get_text(), "Horn");
        assert_eq!(encode(&decoded), bytes);
    }

    #[test]
    fn instrument_rejects_truncated_inline_and_unknown_ids() {
        // A lone 0 announces an inline instrument; this is what used to kick goat horns.
        assert!(InstrumentImpl::deserialize(&mut [0x00u8].as_slice()).is_err());
        assert!(InstrumentImpl::deserialize(&mut [0x7fu8].as_slice()).is_err());
    }

    #[test]
    fn break_sound_holder() {
        // entity.item.break is sound 914.
        assert_wire(
            &BreakSoundImpl {
                sound: IdOr::Id(Sound::EntityItemBreak),
            },
            &[0x93, 0x07],
        );
        assert_wire(
            &BreakSoundImpl {
                sound: custom_horn_sound(),
            },
            CUSTOM_HORN_SOUND,
        );
    }

    #[test]
    fn trim_material_inline_value() {
        let mut bytes = vec![0x00, 0x0e];
        bytes.extend_from_slice(b"minecraft:ruby");
        bytes.extend_from_slice(&[0x08, 0x00, 0x04, b'R', b'u', b'b', b'y']);
        let decoded = decode::<ProvidesTrimMaterialImpl>(&bytes);
        assert!(matches!(
            &decoded.material,
            Holder::Direct(definition) if definition.palette_id == "minecraft:ruby"
        ));
        assert_eq!(encode(&decoded), bytes);
        // diamond is id 2 in the trim material registry.
        assert_wire(
            &ProvidesTrimMaterialImpl {
                material: Holder::Reference(TrimMaterial::Diamond),
            },
            &[0x03],
        );
    }

    #[test]
    fn jukebox_playable_holder() {
        // pigstep is id 14 in the jukebox song registry.
        assert_wire(
            &JukeboxPlayableImpl {
                song: Holder::Reference(JukeboxSong::Pigstep),
            },
            &[0x0f],
        );
        let mut bytes = vec![0x00];
        bytes.extend_from_slice(CUSTOM_HORN_SOUND);
        bytes.extend_from_slice(&[0x08, 0x00, 0x04, b'S', b'o', b'n', b'g']);
        bytes.extend_from_slice(&[0x42, 0xb4, 0x00, 0x00, 0x07]);
        let decoded = decode::<JukeboxPlayableImpl>(&bytes);
        assert!(matches!(
            &decoded.song,
            Holder::Direct(song) if song.length_in_seconds == 90.0 && song.comparator_output == 7
        ));
        assert_eq!(encode(&decoded), bytes);
    }

    #[test]
    fn provides_banner_patterns_tag() {
        let mut bytes = vec![0x00, 0x13];
        bytes.extend_from_slice(b"pattern_item/flower");
        assert_wire(
            &ProvidesBannerPatternsImpl {
                patterns: IDSet::Tag(Cow::Borrowed("pattern_item/flower")),
            },
            &bytes,
        );
    }

    #[test]
    fn dye_and_glint() {
        assert_wire(
            &DyeImpl {
                color: DyeColor::Red,
            },
            &[0x0e],
        );
        // `ByteIdMap` with `ZERO`: unknown colors read as white.
        assert_eq!(decode::<DyeImpl>(&[0x63]).color, DyeColor::White);
        assert_wire(&EnchantmentGlintOverrideImpl { has_glint: false }, &[0x00]);
    }

    #[test]
    fn tooltip_display_lists_hidden_components() {
        assert_wire(
            &TooltipDisplayImpl {
                hide_tooltip: true,
                hidden_components: vec![DataComponent::CustomName, DataComponent::Lore],
            },
            &[0x01, 0x02, 0x06, 0x0b],
        );
        assert!(TooltipDisplayImpl::deserialize(&mut [0x00u8, 0x01, 0x7f].as_slice()).is_err());
    }

    #[test]
    fn potion_contents_with_hidden_effect() {
        let speed = |amplifier, duration, show_particles, hidden_effect| StatusEffectInstance {
            effect_id: Cow::Borrowed("minecraft:speed"),
            amplifier,
            duration,
            ambient: false,
            show_particles,
            show_icon: true,
            hidden_effect,
        };
        assert_wire(
            &PotionContentsImpl {
                potion_id: Some(15),
                custom_color: Some(0xFF00FF),
                custom_effects: vec![speed(
                    1,
                    200,
                    false,
                    Some(Box::new(speed(0, 100, true, None))),
                )],
                custom_name: None,
            },
            &[
                0x01, 0x0f, 0x01, 0x00, 0xff, 0x00, 0xff, 0x01, 0x00, 0x01, 0xc8, 0x01, 0x00, 0x00,
                0x01, 0x01, 0x00, 0x64, 0x00, 0x01, 0x01, 0x00, 0x00,
            ],
        );
    }

    #[test]
    fn resolvable_numbers() {
        let mut compostable = vec![0x00, 0x19];
        compostable.extend_from_slice(b"minecraft:compostable/low");
        assert_wire(
            &CompostableImpl {
                layers: IntProvider::Id(Cow::Borrowed("minecraft:compostable/low")),
            },
            &compostable,
        );
        assert_wire(
            &BrewingFuelImpl {
                uses: IntProvider::Inline(3),
                speed_multiplier: FloatProvider::Inline(1.5),
            },
            &[0x01, 0x00, 0x00, 0x00, 0x03, 0x01, 0x3f, 0xc0, 0x00, 0x00],
        );
        let mut cooking = vec![0x01, 0x00, 0x00, 0x06, 0x40, 0x00, 0x1f];
        cooking.extend_from_slice(b"minecraft:cooking/speed_default");
        assert_wire(
            &CookingFuelImpl {
                burn_time: IntProvider::Inline(1600),
                speed_multiplier: FloatProvider::Id(Cow::Borrowed(
                    "minecraft:cooking/speed_default",
                )),
            },
            &cooking,
        );
    }

    #[test]
    fn bees_carry_typed_entity_data() {
        let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
        nbt.put_int("Age", 5);
        let bees = BeesImpl {
            bees: vec![BeeOccupant {
                entity_data: {
                    let mut data = nbt.clone();
                    data.put_string("id", "minecraft:bee".to_string());
                    data
                },
                ticks_in_hive: 100,
                min_ticks_in_hive: 600,
            }],
        };
        // One bee, entity type 11, {Age:5}, then 100 and 600 ticks.
        assert_wire(
            &bees,
            &[
                0x01, 0x0b, 0x0a, 0x03, 0x00, 0x03, b'A', b'g', b'e', 0x00, 0x00, 0x00, 0x05, 0x00,
                0x64, 0xd8, 0x04,
            ],
        );
        assert!(BeesImpl::deserialize(&mut [0x01u8, 0x7f, 0x0a, 0x00].as_slice()).is_err());
    }

    #[test]
    fn sign_text() {
        let empty = || TextComponent::text("");
        assert_wire(
            &SignTextFrontImpl(SignText {
                messages: [TextComponent::text("Hi"), empty(), empty(), empty()],
                filtered_messages: None,
                color: DyeColor::Red,
                has_glowing_text: true,
            }),
            &[
                0x08, 0x00, 0x02, b'H', b'i', 0x08, 0x00, 0x00, 0x08, 0x00, 0x00, 0x08, 0x00, 0x00,
                0x00, 0x0e, 0x01,
            ],
        );
    }

    #[test]
    fn mob_visibility() {
        // A one-entry holder set (count + 1), zombie (154), then 0.5.
        assert_wire(
            &MobVisibilityImpl {
                targeting_entity_types: IDSet::IDs(Cow::Borrowed(&[&EntityType::ZOMBIE])),
                visibility: 0.5,
            },
            &[0x02, 0x9a, 0x01, 0x3f, 0x00, 0x00, 0x00],
        );
    }

    #[test]
    fn death_protection_effects() {
        let regeneration = StatusEffectInstance {
            effect_id: Cow::Borrowed("minecraft:regeneration"),
            amplifier: 1,
            duration: 900,
            ambient: false,
            show_particles: true,
            show_icon: true,
            hidden_effect: None,
        };
        assert_wire(
            &DeathProtectionImpl {
                death_effects: Cow::Owned(vec![
                    ConsumeEffect::ClearAllEffects,
                    ConsumeEffect::ApplyEffects((Cow::Owned(vec![regeneration]), 1.0)),
                ]),
            },
            &[
                0x02, 0x02, 0x00, 0x01, 0x09, 0x01, 0x84, 0x07, 0x00, 0x01, 0x01, 0x00, 0x3f, 0x80,
                0x00, 0x00,
            ],
        );
        assert_wire(
            &DeathProtectionImpl {
                death_effects: Cow::Owned(vec![ConsumeEffect::TeleportRandomly {
                    diameter: 16.0,
                    directional_particles: false,
                }]),
            },
            &[0x01, 0x03, 0x41, 0x80, 0x00, 0x00, 0x00],
        );
    }

    #[test]
    fn blocks_attacks() {
        assert_wire(
            &BlocksAttacksImpl {
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
            },
            &[
                0x3e, 0x80, 0x00, 0x00, 0x3f, 0x80, 0x00, 0x00, 0x01, 0x42, 0xb4, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x3f, 0x80, 0x00, 0x00, 0x40, 0x40, 0x00, 0x00, 0x3f, 0x80,
                0x00, 0x00, 0x3f, 0x80, 0x00, 0x00, 0x00, 0x01, 0xb4, 0x0b, 0x00,
            ],
        );
    }

    #[test]
    fn pot_decorations_are_optional_templates() {
        let decorations = PotDecorationsImpl {
            back: Some(pumpkin_data::item_stack::ItemStack::new(
                1,
                &pumpkin_data::item::Item::BRICK,
            )),
            front: Some(pumpkin_data::item_stack::ItemStack::new(
                2,
                &pumpkin_data::item::Item::ARCHER_POTTERY_SHERD,
            )),
            ..Default::default()
        };
        assert_wire(
            &decorations,
            &[
                0x01, 0xf6, 0x08, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0xbf, 0x0c, 0x02, 0x00, 0x00,
            ],
        );
    }

    #[test]
    fn persistent_only_components_travel_as_nbt() {
        // `Unit.CODEC` through `fromCodecWithRegistries`: an empty compound.
        assert_eq!(encode(&IntangibleProjectileImpl), [0x0a, 0x00]);
        decode::<IntangibleProjectileImpl>(&[0x0a, 0x00]);
        let mut recipes = vec![0x09, 0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x0f];
        recipes.extend_from_slice(b"minecraft:stick");
        assert_wire(
            &RecipesImpl {
                recipes: vec!["minecraft:stick".to_string()],
            },
            &recipes,
        );
    }

    #[test]
    fn unit_components_write_nothing() {
        assert!(encode(&UnbreakableImpl).is_empty());
        assert!(encode(&GliderImpl).is_empty());
        assert!(encode(&CreativeSlotLockImpl).is_empty());
        assert!(encode(&WaxedImpl).is_empty());
        decode::<WaxedImpl>(&[]);
    }

    #[test]
    fn variants_use_vanilla_id_maps() {
        // `ByIdMap.sparse`: evil rabbits are 99, unknown ids fall back to brown.
        assert_wire(
            &RabbitVariantImpl {
                value: Cow::Borrowed("evil"),
            },
            &[0x63],
        );
        assert_eq!(decode::<RabbitVariantImpl>(&[0x07]).value, "brown");
        // Tropical fish patterns use `base | index << 8`.
        assert_wire(
            &TropicalFishPatternImpl {
                value: Cow::Borrowed("betty"),
            },
            &[0x81, 0x08],
        );
        // `ByIdMap.continuous` with `WRAP`.
        assert_eq!(decode::<HorseVariantImpl>(&[0x08]).value, "creamy");
        // Synced registries send the entry's position: alban is the first painting.
        assert_wire(
            &PaintingVariantImpl {
                variant: Holder::Reference(PaintingVariant::Alban),
            },
            &[0x01],
        );
        assert_eq!(
            encode(&WolfVariantImpl {
                value: Cow::Borrowed("minecraft:ashen"),
            }),
            [0x00]
        );
    }

    #[test]
    fn painting_variant_inline_value() {
        let mut bytes = vec![0x00, 0x02, 0x01, 0x10];
        bytes.extend_from_slice(b"minecraft:sunset");
        bytes.extend_from_slice(&[0x01, 0x08, 0x00, 0x06]);
        bytes.extend_from_slice(b"Sunset");
        bytes.push(0x00);
        let decoded = decode::<PaintingVariantImpl>(&bytes);
        let Holder::Direct(painting) = &decoded.variant else {
            panic!("expected an inline painting");
        };
        assert_eq!((painting.width, painting.height), (2, 1));
        assert_eq!(painting.asset_id, "minecraft:sunset");
        assert!(painting.author.is_none());
        assert_eq!(encode(&decoded), bytes);
    }
}
