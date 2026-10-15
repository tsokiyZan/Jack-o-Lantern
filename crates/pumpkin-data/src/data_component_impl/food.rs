use super::Digest;
use crate::data_component_impl::basic::SoundEvent;
use crate::data_component_impl::{
    DataComponentImpl, IDSet, IdOr, MapHash, get_bool_hash, get_byte_hash, get_f32_hash,
    get_i32_hash, get_idor, get_idor_hash, get_idset_hash, get_list_hash, get_str_hash, namespaced,
    put_idor,
};
use crate::effect::StatusEffect;
use crate::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq)]
pub struct FoodImpl {
    pub nutrition: i32,
    pub saturation: f32,
    pub can_always_eat: bool,
}
impl FoodImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let nutrition = compound.get_int("nutrition")?;
        let saturation = compound.get_float("saturation")?;
        let can_always_eat = compound.get_bool("can_always_eat").unwrap_or(false);
        Some(Self {
            nutrition,
            saturation,
            can_always_eat,
        })
    }
}
impl DataComponentImpl for FoodImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_int("nutrition", self.nutrition);
        compound.put_float("saturation", self.saturation);
        compound.put_bool("can_always_eat", self.can_always_eat);
        NbtTag::Compound(compound)
    }
    default_impl!(Food);
}
impl Hash for FoodImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.nutrition.hash(state);
        unsafe { (*(&raw const self.saturation).cast::<u32>()).hash(state) };
        self.can_always_eat.hash(state);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct StatusEffectInstance {
    pub effect_id: Cow<'static, str>,
    pub amplifier: i32,
    pub duration: i32,
    pub ambient: bool,
    pub show_particles: bool,
    pub show_icon: bool,
    /// The weaker effect that comes back when this one runs out.
    pub hidden_effect: Option<Box<Self>>,
}

impl StatusEffectInstance {
    /// Bounds the `hidden_effect` chain read from NBT or the network.
    pub const MAX_HIDDEN_EFFECT_DEPTH: usize = 32;

    /// Reads `MobEffectInstance.CODEC`.
    pub fn read_data(nbt: &NbtTag) -> Option<Self> {
        let compound = nbt.extract_compound()?;
        let effect_id = Cow::Owned(compound.get_string("id")?.to_string());
        Self::read_details(effect_id, compound, Self::MAX_HIDDEN_EFFECT_DEPTH)
    }

    /// Reads `MobEffectInstance.Details.MAP_CODEC`, where every field is optional.
    fn read_details(
        effect_id: Cow<'static, str>,
        compound: &NbtCompound,
        depth: usize,
    ) -> Option<Self> {
        let amplifier = match compound.get("amplifier") {
            // `ExtraCodecs.UNSIGNED_BYTE`
            Some(tag) => i32::from(tag.as_numeric_int()? as u8),
            None => 0,
        };
        let duration = match compound.get("duration") {
            Some(tag) => tag.as_numeric_int()?,
            None => 0,
        };
        let flag = |name: &str, default: bool| match compound.get(name) {
            Some(tag) => tag.as_numeric_int().map(|value| value != 0),
            None => Some(default),
        };
        let ambient = flag("ambient", false)?;
        let show_particles = flag("show_particles", true)?;
        let show_icon = flag("show_icon", show_particles)?;
        let hidden_effect = match compound.get("hidden_effect") {
            Some(hidden) => {
                let depth = depth.checked_sub(1)?;
                Some(Box::new(Self::read_details(
                    effect_id.clone(),
                    hidden.extract_compound()?,
                    depth,
                )?))
            }
            None => None,
        };
        Some(Self {
            effect_id,
            amplifier,
            duration,
            ambient,
            show_particles,
            show_icon,
            hidden_effect,
        })
    }

    /// Writes `MobEffectInstance.CODEC`.
    pub fn as_nbt(&self) -> NbtTag {
        let mut compound = self.details_nbt();
        compound.put_string("id", self.effect_id.to_string());
        NbtTag::Compound(compound)
    }

    fn details_nbt(&self) -> NbtCompound {
        let mut compound = NbtCompound::new();
        if self.amplifier != 0 {
            compound.put_byte("amplifier", self.amplifier as u8 as i8);
        }
        if self.duration != 0 {
            compound.put_int("duration", self.duration);
        }
        if self.ambient {
            compound.put_bool("ambient", true);
        }
        if !self.show_particles {
            compound.put_bool("show_particles", false);
        }
        compound.put_bool("show_icon", self.show_icon);
        if let Some(hidden) = &self.hidden_effect {
            compound.put_compound("hidden_effect", hidden.details_nbt());
        }
        compound
    }

    pub fn get_hash(&self) -> i32 {
        self.details_hash()
            .field("id", get_str_hash(&namespaced(&self.effect_id)))
            .finish() as i32
    }

    fn details_hash(&self) -> MapHash {
        MapHash::new()
            .optional_field(
                "amplifier",
                (self.amplifier != 0).then(|| get_byte_hash(self.amplifier as u8 as i8)),
            )
            .optional_field(
                "duration",
                (self.duration != 0).then(|| get_i32_hash(self.duration)),
            )
            .optional_field("ambient", self.ambient.then(|| get_bool_hash(true)))
            .optional_field(
                "show_particles",
                (!self.show_particles).then(|| get_bool_hash(false)),
            )
            .field("show_icon", get_bool_hash(self.show_icon))
            .optional_field(
                "hidden_effect",
                self.hidden_effect
                    .as_ref()
                    .map(|hidden| hidden.details_hash().finish()),
            )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConsumeAnimation {
    None,
    Eat,
    Drink,
    Block,
    Bow,
    Spear,
    Crossbow,
    Spyglass,
    Horn,
    Brush,
}

impl ConsumeAnimation {
    #[must_use]
    pub const fn to_str(&self) -> &'static str {
        match self {
            ConsumeAnimation::None => "none",
            ConsumeAnimation::Eat => "eat",
            ConsumeAnimation::Drink => "drink",
            ConsumeAnimation::Block => "block",
            ConsumeAnimation::Bow => "bow",
            ConsumeAnimation::Spear => "spear",
            ConsumeAnimation::Crossbow => "crossbow",
            ConsumeAnimation::Spyglass => "spyglass",
            ConsumeAnimation::Horn => "horn",
            ConsumeAnimation::Brush => "brush",
        }
    }
}
impl TryFrom<i32> for ConsumeAnimation {
    type Error = ();
    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Eat),
            2 => Ok(Self::Drink),
            3 => Ok(Self::Block),
            4 => Ok(Self::Bow),
            5 => Ok(Self::Spear),
            6 => Ok(Self::Crossbow),
            7 => Ok(Self::Spyglass),
            8 => Ok(Self::Horn),
            9 => Ok(Self::Brush),
            _ => Err(()),
        }
    }
}
impl FromStr for ConsumeAnimation {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Self::None),
            "eat" => Ok(Self::Eat),
            "drink" => Ok(Self::Drink),
            "block" => Ok(Self::Block),
            "bow" => Ok(Self::Bow),
            "spear" => Ok(Self::Spear),
            "crossbow" => Ok(Self::Crossbow),
            "spyglass" => Ok(Self::Spyglass),
            "horn" => Ok(Self::Horn),
            "brush" => Ok(Self::Brush),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConsumeEffect {
    ApplyEffects((Cow<'static, [StatusEffectInstance]>, f32)),
    RemoveEffects(IDSet<StatusEffect>),
    ClearAllEffects,
    TeleportRandomly(f32),
    PlaySound(IdOr<SoundEvent>),
}
impl Hash for ConsumeEffect {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.to_str().hash(state);
        match self {
            ConsumeEffect::ApplyEffects(tuple) => {
                tuple.0.hash(state);
                unsafe { (*(&raw const tuple.1).cast::<u32>()).hash(state) };
            }
            ConsumeEffect::RemoveEffects(status_effect_instances) => {
                status_effect_instances.hash(state)
            }
            ConsumeEffect::ClearAllEffects => (),
            ConsumeEffect::TeleportRandomly(dst) => unsafe {
                (*(&raw const dst).cast::<u32>()).hash(state)
            },
            ConsumeEffect::PlaySound(id_or) => id_or.hash(state),
        }
    }
}
impl ConsumeEffect {
    pub fn to_str(&self) -> &str {
        match self {
            ConsumeEffect::ApplyEffects(_) => "apply_effects",
            ConsumeEffect::RemoveEffects(_) => "remove_effects",
            ConsumeEffect::ClearAllEffects => "clear_all_effects",
            ConsumeEffect::TeleportRandomly(_) => "teleport_randomly",
            ConsumeEffect::PlaySound(_) => "play_sound",
        }
    }
    pub fn registry_id(&self) -> u8 {
        match self {
            ConsumeEffect::ApplyEffects(_) => 0,
            ConsumeEffect::RemoveEffects(_) => 1,
            ConsumeEffect::ClearAllEffects => 2,
            ConsumeEffect::TeleportRandomly(_) => 3,
            ConsumeEffect::PlaySound(_) => 4,
        }
    }
    pub fn read_data(nbt: &NbtTag) -> Option<Self> {
        let compound = nbt.extract_compound()?;
        let r#type = compound.get_string("type")?;
        match r#type {
            "remove_effects" => {
                let idset = IDSet::read(compound.get("effects")?)?;
                Some(Self::RemoveEffects(idset))
            }
            "clear_all_effects" => Some(Self::ClearAllEffects),
            "teleport_randomly" => {
                let dst = compound.get_float("diameter")?;
                Some(Self::TeleportRandomly(dst))
            }
            "play_sound" => {
                let sound = get_idor(compound, "sound", Sound::EntityGenericEat);
                Some(Self::PlaySound(sound))
            }
            "apply_effects" => {
                let probability = compound.get_float("probability")?;
                let effects_vec: Vec<StatusEffectInstance> = compound
                    .get_list("effects")?
                    .iter()
                    .filter_map(StatusEffectInstance::read_data)
                    .collect();
                let effects: Cow<'static, [StatusEffectInstance]> = Cow::Owned(effects_vec);
                Some(Self::ApplyEffects((effects, probability)))
            }
            _ => None,
        }
    }
    pub fn as_nbt(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_string("type", self.to_str().to_string());
        match self {
            ConsumeEffect::ApplyEffects(data) => {
                let nbt_arr = data.0.iter().map(|x| x.as_nbt()).collect();
                compound.put_list("effects", nbt_arr);
                compound.put_float("probability", data.1);
            }
            ConsumeEffect::RemoveEffects(idset) => idset.write(&mut compound, "effects"),
            ConsumeEffect::ClearAllEffects => (),
            ConsumeEffect::TeleportRandomly(dst) => compound.put_float("diameter", *dst),
            ConsumeEffect::PlaySound(id_or) => {
                put_idor(&mut compound, "sound", id_or);
            }
        }
        NbtTag::Compound(compound)
    }
    pub fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        match self {
            ConsumeEffect::ApplyEffects((effects, probability)) => {
                digest.update(&[1u8]);
                for effect in effects.iter() {
                    digest.update(&effect.get_hash().to_le_bytes());
                }
                digest.update(&[13u8]);
                digest.update(&get_f32_hash(*probability).to_le_bytes());
            }
            ConsumeEffect::RemoveEffects(idset) => {
                digest.update(&[2u8]);
                digest.update(&get_idset_hash(idset).to_le_bytes());
            }
            ConsumeEffect::ClearAllEffects => {
                digest.update(&[3u8]);
            }
            ConsumeEffect::TeleportRandomly(dst) => {
                digest.update(&[4u8]);
                digest.update(&get_f32_hash(*dst).to_le_bytes());
            }
            ConsumeEffect::PlaySound(id_or) => {
                digest.update(&[5u8]);
                digest.update(&get_idor_hash(id_or).to_le_bytes());
            }
        }
        digest.finalize() as i32
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConsumableImpl {
    pub consume_seconds: f32,
    pub animation: ConsumeAnimation,
    pub sound_event: IdOr<SoundEvent>,
    pub consume_particles: bool,
    pub effects: Cow<'static, [ConsumeEffect]>,
}
impl ConsumableImpl {
    pub const fn new(
        consume_seconds: f32,
        animation: ConsumeAnimation,
        sound_event: IdOr<SoundEvent>,
        consume_particles: bool,
        effects: Cow<'static, [ConsumeEffect]>,
    ) -> Self {
        Self {
            consume_seconds,
            animation,
            sound_event,
            consume_particles,
            effects,
        }
    }
    #[must_use]
    pub fn consume_ticks(&self) -> i32 {
        (self.consume_seconds * 20.0) as i32
    }
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let consume_seconds = compound.get_float("consume_seconds")?;
        let animation = compound
            .get_string("animation")?
            .parse::<ConsumeAnimation>()
            .ok()?;
        let sound_event = get_idor(compound, "sound", Sound::EntityGenericEat);
        let consume_particles = compound.get_bool("has_consume_particles").unwrap_or(false);
        let opt_list = compound.get_list("on_consume_effects");
        let effects: Cow<'static, [ConsumeEffect]> = if let Some(effect_list) = opt_list {
            effect_list
                .iter()
                .filter_map(ConsumeEffect::read_data)
                .collect()
        } else {
            Cow::Borrowed(&[])
        };
        Some(Self {
            consume_seconds,
            animation,
            sound_event,
            consume_particles,
            effects,
        })
    }
}
impl DataComponentImpl for ConsumableImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_float("consume_seconds", self.consume_seconds);
        compound.put_string("animation", self.animation.to_str().to_string());
        put_idor(&mut compound, "sound", &self.sound_event);
        compound.put_bool("has_consume_particles", self.consume_particles);
        let nbt_vec = self.effects.iter().map(|x| x.as_nbt()).collect();
        compound.put_list("on_consume_effects", nbt_vec);
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        digest.update(&[2u8]);
        digest.update(&get_f32_hash(self.consume_seconds).to_le_bytes());
        digest.update(&get_i32_hash(self.animation as i32).to_le_bytes());
        digest.update(&get_idor_hash(&self.sound_event).to_be_bytes());
        digest.update(&[self.consume_particles as u8]);
        for effect in self.effects.iter() {
            digest.update(&effect.get_hash().to_le_bytes());
        }
        digest.update(&[3u8]);
        digest.finalize() as i32
    }
    default_impl!(Consumable);
}
impl Hash for ConsumableImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        unsafe { (*(&raw const self.consume_seconds).cast::<u32>()).hash(state) };
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct UseEffectsImpl;
impl UseEffectsImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for UseEffectsImpl {
    default_impl!(UseEffects);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct UseRemainderImpl {
    pub remainder: Option<Cow<'static, str>>,
}
impl UseRemainderImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self { remainder: None })
    }
}
impl DataComponentImpl for UseRemainderImpl {
    default_impl!(UseRemainder);
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UseCooldownImpl {
    pub seconds: f32,
    pub cooldown_group: Option<String>,
}
impl UseCooldownImpl {
    pub fn new(seconds: f32, cooldown_group: Option<String>) -> Self {
        Self {
            seconds,
            cooldown_group,
        }
    }
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let seconds = compound.get_float("seconds")?;
        let cooldown_group = compound.get_string("cooldown_group").map(|s| s.to_string());
        Some(Self {
            seconds,
            cooldown_group,
        })
    }
}
impl DataComponentImpl for UseCooldownImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_float("seconds", self.seconds);
        if let Some(group) = &self.cooldown_group {
            compound.put_string("cooldown_group", group.clone());
        }
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        digest.update(&get_f32_hash(self.seconds).to_le_bytes());
        if let Some(group) = &self.cooldown_group {
            digest.update(&get_str_hash(group).to_le_bytes());
        }
        digest.finalize() as i32
    }
    default_impl!(UseCooldown);
}
impl Eq for UseCooldownImpl {}
impl Hash for UseCooldownImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.seconds.to_bits().hash(state);
        self.cooldown_group.hash(state);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct PotionContentsImpl {
    pub potion_id: Option<i32>,
    pub custom_color: Option<i32>,
    pub custom_effects: Vec<StatusEffectInstance>,
    pub custom_name: Option<String>,
}
impl PotionContentsImpl {
    /// Reads `PotionContents.CODEC`: the full record, or just a potion id.
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        let potion_from_name = |name: &str| {
            crate::potion::Potion::from_name(name.strip_prefix("minecraft:").unwrap_or(name))
                .map(|potion| i32::from(potion.id))
        };
        let compound = match tag {
            NbtTag::String(name) => {
                return Some(Self {
                    potion_id: Some(potion_from_name(name)?),
                    custom_color: None,
                    custom_effects: Vec::new(),
                    custom_name: None,
                });
            }
            NbtTag::Compound(compound) => compound,
            _ => return None,
        };
        let potion_id = match compound.get("potion") {
            Some(NbtTag::String(name)) => Some(potion_from_name(name)?),
            // Older Pumpkin saves stored the registry id.
            Some(NbtTag::Int(id)) => Some(*id),
            Some(_) => return None,
            None => None,
        };
        let custom_color = match compound.get("custom_color") {
            Some(tag) => Some(tag.as_numeric_int()?),
            None => None,
        };
        let custom_effects = match compound.get_list("custom_effects") {
            Some(list) => list
                .iter()
                .map(StatusEffectInstance::read_data)
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        let custom_name = match compound.get("custom_name") {
            Some(tag) => Some(tag.extract_string()?.to_string()),
            None => None,
        };
        Some(Self {
            potion_id,
            custom_color,
            custom_effects,
            custom_name,
        })
    }

    fn potion_name(&self) -> Option<String> {
        let potion = crate::potion::Potion::from_id(u8::try_from(self.potion_id?).ok()?)?;
        Some(format!("minecraft:{}", potion.name))
    }
}
impl DataComponentImpl for PotionContentsImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        if let Some(potion) = self.potion_name() {
            compound.put_string("potion", potion);
        }
        if let Some(color) = self.custom_color {
            compound.put_int("custom_color", color);
        }
        if !self.custom_effects.is_empty() {
            compound.put_list(
                "custom_effects",
                self.custom_effects
                    .iter()
                    .map(StatusEffectInstance::as_nbt)
                    .collect(),
            );
        }
        if let Some(name) = &self.custom_name {
            compound.put_string("custom_name", name.clone());
        }
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        MapHash::new()
            .optional_field("potion", self.potion_name().map(|name| get_str_hash(&name)))
            .optional_field("custom_color", self.custom_color.map(get_i32_hash))
            .optional_field(
                "custom_effects",
                (!self.custom_effects.is_empty()).then(|| {
                    get_list_hash(
                        self.custom_effects
                            .iter()
                            .map(|effect| effect.get_hash() as u32),
                    )
                }),
            )
            .optional_field("custom_name", self.custom_name.as_deref().map(get_str_hash))
            .finish() as i32
    }
    default_impl!(PotionContents);
}

#[derive(Clone, Debug, PartialEq)]
pub struct PotionDurationScaleImpl {
    pub scale: f32,
}
impl PotionDurationScaleImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_float().map(|scale| Self { scale })
    }
}
impl DataComponentImpl for PotionDurationScaleImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Float(self.scale)
    }
    fn get_hash(&self) -> i32 {
        get_f32_hash(self.scale) as i32
    }
    default_impl!(PotionDurationScale);
}
impl Hash for PotionDurationScaleImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.scale.to_bits().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::{DataComponentImpl, PotionDurationScaleImpl};
    use crate::item::Item;

    #[test]
    fn potion_duration_scale_round_trips_as_a_float() {
        let scale = PotionDurationScaleImpl { scale: 0.125 };
        let encoded = scale.write_data();
        let decoded = PotionDurationScaleImpl::read_data(&encoded).expect("scale should decode");

        assert_eq!(decoded, scale);
    }

    #[test]
    fn generated_arrow_duration_scale_is_data_driven() {
        let scale = Item::TIPPED_ARROW
            .components
            .iter()
            .find_map(|(id, component)| {
                (*id == crate::data_component::DataComponent::PotionDurationScale)
                    .then(|| component.as_any().downcast_ref::<PotionDurationScaleImpl>())
                    .flatten()
            })
            .expect("tipped arrows should have a duration scale");

        assert_eq!(scale.scale, 0.125);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct SuspiciousStewEffect {
    pub effect: Cow<'static, str>,
    pub duration: i32,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct SuspiciousStewEffectsImpl {
    pub effects: Cow<'static, [SuspiciousStewEffect]>,
}
impl SuspiciousStewEffectsImpl {
    pub const EMPTY: Self = Self {
        effects: Cow::Borrowed(&[]),
    };

    pub fn read_data(data: &NbtTag) -> Option<Self> {
        Some(Self {
            effects: Cow::Owned(
                data.extract_list()?
                    .iter()
                    .filter_map(|tag| {
                        let effect = tag.extract_compound()?;
                        Some(SuspiciousStewEffect {
                            effect: Cow::Owned(effect.get_string("id")?.to_owned()),
                            duration: effect.get_int("duration").unwrap_or(160),
                        })
                    })
                    .collect(),
            ),
        })
    }
}
impl DataComponentImpl for SuspiciousStewEffectsImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::List(
            self.effects
                .iter()
                .map(|effect| {
                    let mut nbt = NbtCompound::new();
                    nbt.put_string("id", effect.effect.to_string());
                    nbt.put_int("duration", effect.duration);
                    NbtTag::Compound(nbt)
                })
                .collect(),
        )
    }
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        for effect in self.effects.iter() {
            digest.update(&get_str_hash(&effect.effect).to_le_bytes());
            digest.update(&get_i32_hash(effect.duration).to_le_bytes());
        }
        digest.finalize() as i32
    }
    default_impl!(SuspiciousStewEffects);
}
