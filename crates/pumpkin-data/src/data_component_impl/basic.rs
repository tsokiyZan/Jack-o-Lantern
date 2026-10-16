use crate::banner_pattern::BannerPattern;
use crate::data_component::DataComponent;
use crate::data_component_impl::{
    DataComponentImpl, Holder, IDSet, IDSetContent, IdOr, MapHash, get_bool_hash,
    get_compound_hash, get_f32_hash, get_i32_hash, get_idor, get_idor_hash, get_idset_hash,
    get_item_stack_template_hash, get_list_hash, get_map_hash, get_str_hash, get_text_hash,
    item_stack_template_eq, namespaced, put_idor, read_item_stack_template, text_to_nbt,
    write_item_stack_template,
};
use crate::instrument::Instrument;
use crate::item_stack::ItemStack;
use crate::sound::Sound;
use crate::trim_material::TrimMaterial;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;
use std::borrow::Cow;

#[derive(Clone, Debug, PartialEq)]
pub struct CustomDataImpl {
    pub data: NbtCompound,
}
impl CustomDataImpl {
    #[must_use]
    pub const fn new(data: NbtCompound) -> Self {
        Self { data }
    }
    #[must_use]
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        if let NbtTag::Compound(c) = tag {
            Some(Self { data: c.clone() })
        } else {
            None
        }
    }
}
impl DataComponentImpl for CustomDataImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Compound(self.data.clone())
    }
    fn get_hash(&self) -> i32 {
        get_compound_hash(&self.data) as i32
    }
    default_impl!(CustomData);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct MaxStackSizeImpl {
    pub size: u8,
}
impl MaxStackSizeImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_int().map(|size| Self { size: size as u8 })
    }
}
impl DataComponentImpl for MaxStackSizeImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Int(self.size as i32)
    }
    fn get_hash(&self) -> i32 {
        get_i32_hash(self.size as i32) as i32
    }
    default_impl!(MaxStackSize);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct MaxDamageImpl {
    pub max_damage: i32,
}
impl MaxDamageImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_int().map(|max_damage| Self { max_damage })
    }
}
impl DataComponentImpl for MaxDamageImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Int(self.max_damage)
    }
    default_impl!(MaxDamage);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct DamageImpl {
    pub damage: i32,
}
impl DamageImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_int().map(|damage| Self { damage })
    }
}
impl DataComponentImpl for DamageImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Int(self.damage)
    }
    fn get_hash(&self) -> i32 {
        get_i32_hash(self.damage) as i32
    }
    default_impl!(Damage);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct UnbreakableImpl;
impl UnbreakableImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for UnbreakableImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Compound(NbtCompound::new())
    }
    fn get_hash(&self) -> i32 {
        get_map_hash([]) as i32
    }
    default_impl!(Unbreakable);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct CustomNameImpl {
    pub name: TextComponent,
}
impl CustomNameImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_string().map(|name| Self {
            name: TextComponent::text(name.to_string()),
        })
    }
}
impl DataComponentImpl for CustomNameImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.name.clone().get_text().into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(self.name.clone().get_text().as_str()) as i32
    }
    default_impl!(CustomName);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ItemNameImpl {
    pub name: Cow<'static, str>,
}
impl ItemNameImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let name = match data {
            NbtTag::String(name) => name.to_string(),
            NbtTag::Compound(component) => component
                .get_string("translate")
                .or_else(|| component.get_string("text"))?
                .to_owned(),
            _ => return None,
        };
        Some(Self {
            name: Cow::Owned(name),
        })
    }
}
impl DataComponentImpl for ItemNameImpl {
    fn write_data(&self) -> NbtTag {
        let mut component = NbtCompound::new();
        component.put_string("translate", self.name.to_string());
        NbtTag::Compound(component)
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(&self.name) as i32
    }
    default_impl!(ItemName);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ItemModelImpl {
    pub id: Cow<'static, str>,
}
impl ItemModelImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_string().map(|id| Self {
            id: Cow::Owned(id.to_string()),
        })
    }
}
impl DataComponentImpl for ItemModelImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.id.clone().into_owned().into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(self.id.as_ref()) as i32
    }
    default_impl!(ItemModel);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct LoreImpl {
    pub lines: Vec<TextComponent>,
}
impl LoreImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let NbtTag::List(lines) = data else {
            return None;
        };

        Some(Self {
            lines: lines
                .iter()
                .filter_map(NbtTag::extract_string)
                .map(|line| TextComponent::text(line.to_owned()))
                .collect(),
        })
    }
}
impl DataComponentImpl for LoreImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::List(
            self.lines
                .iter()
                .map(|line| NbtTag::String(line.clone().get_text().into_boxed_str()))
                .collect(),
        )
    }
    default_impl!(Lore);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Rarity {
    #[default]
    Common = 0,
    Uncommon = 1,
    Rare = 2,
    Epic = 3,
}

impl Rarity {
    #[must_use]
    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Self::Common),
            1 => Some(Self::Uncommon),
            2 => Some(Self::Rare),
            3 => Some(Self::Epic),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_id(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "common" => Some(Self::Common),
            "uncommon" => Some(Self::Uncommon),
            "rare" => Some(Self::Rare),
            "epic" => Some(Self::Epic),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_name(self) -> &'static str {
        match self {
            Self::Common => "common",
            Self::Uncommon => "uncommon",
            Self::Rare => "rare",
            Self::Epic => "epic",
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct RarityImpl {
    pub rarity: Rarity,
}

impl RarityImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let name = data.extract_string()?;
        Some(Self {
            rarity: Rarity::from_name(name)?,
        })
    }
}

impl DataComponentImpl for RarityImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.rarity.to_name().into())
    }

    fn get_hash(&self) -> i32 {
        crate::data_component_impl::get_i32_hash(self.rarity.to_id()) as i32
    }

    default_impl!(Rarity);
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomModelDataImpl {
    pub floats: Vec<f32>,
    pub flags: Vec<bool>,
    pub strings: Vec<String>,
    pub colors: Vec<i32>,
}
impl CustomModelDataImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let floats = compound
            .get_list("floats")
            .map(|l| l.iter().filter_map(NbtTag::extract_float).collect())
            .unwrap_or_default();
        let flags = compound
            .get_list("flags")
            .map(|l| l.iter().filter_map(NbtTag::extract_bool).collect())
            .unwrap_or_default();
        let strings = compound
            .get_list("strings")
            .map(|l| {
                l.iter()
                    .filter_map(|t| t.extract_string().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        // Vanilla encodes the color list as ints, but tolerate a packed int array too.
        let colors = if let Some(arr) = compound.get_int_array("colors") {
            arr.to_vec()
        } else if let Some(l) = compound.get_list("colors") {
            l.iter().filter_map(NbtTag::extract_int).collect()
        } else {
            Vec::new()
        };
        Some(Self {
            floats,
            flags,
            strings,
            colors,
        })
    }
}
impl DataComponentImpl for CustomModelDataImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_list(
            "floats",
            self.floats.iter().map(|f| NbtTag::Float(*f)).collect(),
        );
        compound.put_list(
            "flags",
            self.flags.iter().map(|b| NbtTag::Byte(*b as i8)).collect(),
        );
        compound.put_list(
            "strings",
            self.strings
                .iter()
                .map(|s| NbtTag::String(s.clone().into()))
                .collect(),
        );
        compound.put_list(
            "colors",
            self.colors.iter().map(|c| NbtTag::Int(*c)).collect(),
        );
        NbtTag::Compound(compound)
    }
    default_impl!(CustomModelData);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Default)]
pub struct TooltipDisplayImpl {
    pub hide_tooltip: bool,
    pub hidden_components: Vec<DataComponent>,
}
impl TooltipDisplayImpl {
    pub const DEFAULT: Self = Self {
        hide_tooltip: false,
        hidden_components: Vec::new(),
    };

    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let hide_tooltip = match compound.get("hide_tooltip") {
            Some(tag) => tag.extract_bool()?,
            None => false,
        };
        let hidden_components = match compound.get_list("hidden_components") {
            Some(list) => list
                .iter()
                .map(|tag| DataComponent::try_from_name(&namespaced(tag.extract_string()?)))
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        Some(Self {
            hide_tooltip,
            hidden_components,
        })
    }
}
impl DataComponentImpl for TooltipDisplayImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        if self.hide_tooltip {
            compound.put_bool("hide_tooltip", true);
        }
        if !self.hidden_components.is_empty() {
            compound.put_list(
                "hidden_components",
                self.hidden_components
                    .iter()
                    .map(|id| NbtTag::String(id.to_name().into()))
                    .collect(),
            );
        }
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        MapHash::new()
            .optional_field(
                "hide_tooltip",
                self.hide_tooltip.then(|| get_bool_hash(true)),
            )
            .optional_field(
                "hidden_components",
                (!self.hidden_components.is_empty()).then(|| {
                    get_list_hash(
                        self.hidden_components
                            .iter()
                            .map(|id| get_str_hash(id.to_name())),
                    )
                }),
            )
            .finish() as i32
    }
    default_impl!(TooltipDisplay);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct CreativeSlotLockImpl;
impl CreativeSlotLockImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for CreativeSlotLockImpl {
    default_impl!(CreativeSlotLock);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct EnchantmentGlintOverrideImpl {
    pub has_glint: bool,
}
impl EnchantmentGlintOverrideImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_bool().map(|has_glint| Self { has_glint })
    }
}
impl DataComponentImpl for EnchantmentGlintOverrideImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Byte(i8::from(self.has_glint))
    }
    fn get_hash(&self) -> i32 {
        get_bool_hash(self.has_glint) as i32
    }
    default_impl!(EnchantmentGlintOverride);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct TooltipStyleImpl {
    pub id: String,
}
impl TooltipStyleImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_string().map(|id| Self { id: id.to_string() })
    }
}
impl DataComponentImpl for TooltipStyleImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.id.clone().into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(&self.id) as i32
    }
    default_impl!(TooltipStyle);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct NoteBlockSoundImpl {
    pub sound: String,
}
impl NoteBlockSoundImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_string().map(|sound| Self {
            sound: sound.to_string(),
        })
    }
}
impl DataComponentImpl for NoteBlockSoundImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.sound.clone().into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(&self.sound) as i32
    }
    default_impl!(NoteBlockSound);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct BaseColorImpl {
    pub color: String,
}
impl BaseColorImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_string().map(|color| Self {
            color: color.to_string(),
        })
    }
}
impl DataComponentImpl for BaseColorImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.color.clone().into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(&self.color) as i32
    }
    default_impl!(BaseColor);
}

/// An inline instrument, `Instrument.DIRECT_CODEC`.
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentDefinition {
    pub sound_event: IdOr<SoundEvent>,
    pub use_duration: f32,
    pub range: f32,
    pub durability_damage: i32,
    pub description: TextComponent,
}

impl InstrumentDefinition {
    fn read_data(compound: &NbtCompound) -> Option<Self> {
        compound.get("sound_event")?;
        Some(Self {
            sound_event: get_idor(compound, "sound_event", Sound::IntentionallyEmpty),
            use_duration: compound.get("use_duration")?.as_numeric_float()?,
            range: compound.get("range")?.as_numeric_float()?,
            durability_damage: match compound.get("durability_damage") {
                Some(tag) => tag.as_numeric_int()?,
                None => 0,
            },
            description: TextComponent::from_nbt(compound.get("description")?),
        })
    }

    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        put_idor(&mut compound, "sound_event", &self.sound_event);
        compound.put_float("use_duration", self.use_duration);
        compound.put_float("range", self.range);
        if self.durability_damage != 0 {
            compound.put_int("durability_damage", self.durability_damage);
        }
        compound.put("description", text_to_nbt(&self.description));
        NbtTag::Compound(compound)
    }

    fn get_hash(&self) -> u32 {
        MapHash::new()
            .field("sound_event", get_idor_hash(&self.sound_event))
            .field("use_duration", get_f32_hash(self.use_duration))
            .field("range", get_f32_hash(self.range))
            .optional_field(
                "durability_damage",
                (self.durability_damage != 0).then(|| get_i32_hash(self.durability_damage)),
            )
            .field("description", get_text_hash(&self.description))
            .finish()
    }
}

/// `minecraft:instrument`, a holder that is either a registry entry or an inline instrument.
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentImpl {
    pub instrument: Holder<Instrument, Box<InstrumentDefinition>>,
}
impl InstrumentImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let instrument = match data {
            NbtTag::String(id) => Holder::Reference(Instrument::from_name(id)?),
            NbtTag::Compound(compound) => {
                Holder::Direct(Box::new(InstrumentDefinition::read_data(compound)?))
            }
            _ => return None,
        };
        Some(Self { instrument })
    }
}
impl DataComponentImpl for InstrumentImpl {
    fn write_data(&self) -> NbtTag {
        match &self.instrument {
            Holder::Reference(instrument) => NbtTag::String(instrument.asset_id().into()),
            Holder::Direct(definition) => definition.write_data(),
        }
    }
    fn get_hash(&self) -> i32 {
        (match &self.instrument {
            Holder::Reference(instrument) => get_str_hash(instrument.asset_id()),
            Holder::Direct(definition) => definition.get_hash(),
        }) as i32
    }
    default_impl!(Instrument);
}

/// An inline trim material, `TrimMaterial.DIRECT_CODEC`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TrimMaterialDefinition {
    pub palette_id: String,
    pub description: TextComponent,
}

/// `minecraft:provides_trim_material`, a holder that is either a registry entry
/// or an inline trim material.
#[derive(Clone, Debug, PartialEq)]
pub struct ProvidesTrimMaterialImpl {
    pub material: Holder<TrimMaterial, TrimMaterialDefinition>,
}
impl ProvidesTrimMaterialImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let material = match data {
            NbtTag::String(id) => Holder::Reference(TrimMaterial::from_name(id)?),
            NbtTag::Compound(compound) => Holder::Direct(TrimMaterialDefinition {
                palette_id: compound.get_string("palette_id")?.to_string(),
                description: TextComponent::from_nbt(compound.get("description")?),
            }),
            _ => return None,
        };
        Some(Self { material })
    }
}
impl DataComponentImpl for ProvidesTrimMaterialImpl {
    fn write_data(&self) -> NbtTag {
        match &self.material {
            Holder::Reference(material) => NbtTag::String(material.asset_id().into()),
            Holder::Direct(definition) => {
                let mut compound = NbtCompound::new();
                compound.put_string("palette_id", definition.palette_id.clone());
                compound.put("description", text_to_nbt(&definition.description));
                NbtTag::Compound(compound)
            }
        }
    }
    fn get_hash(&self) -> i32 {
        (match &self.material {
            Holder::Reference(material) => get_str_hash(material.asset_id()),
            Holder::Direct(definition) => MapHash::new()
                .field(
                    "palette_id",
                    get_str_hash(&namespaced(&definition.palette_id)),
                )
                .field("description", get_text_hash(&definition.description))
                .finish(),
        }) as i32
    }
    default_impl!(ProvidesTrimMaterial);
}

impl IDSetContent for BannerPattern {
    fn registry_id(&self) -> u16 {
        self.id() as u16
    }
    fn to_string(&self) -> String {
        self.asset_id().to_string()
    }
    fn from_id(id: u16) -> Option<&'static Self> {
        Self::all().get(usize::from(id))
    }
    fn from_str(name: &str) -> Option<&'static Self> {
        Self::from_id(Self::from_name(name)?.id() as u16)
    }
}

/// `minecraft:provides_banner_patterns`, a set of banner patterns given by tag
/// or by id.
#[derive(Clone, Debug, PartialEq)]
pub struct ProvidesBannerPatternsImpl {
    pub patterns: IDSet<BannerPattern>,
}
impl ProvidesBannerPatternsImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        Some(Self {
            patterns: IDSet::read(data)?,
        })
    }
}
impl DataComponentImpl for ProvidesBannerPatternsImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        self.patterns.write(&mut compound, "patterns");
        compound
            .child_tags
            .remove("patterns")
            .unwrap_or(NbtTag::End)
    }
    fn get_hash(&self) -> i32 {
        get_idset_hash(&self.patterns) as i32
    }
    default_impl!(ProvidesBannerPatterns);
}

/// An inline banner pattern, `BannerPattern.DIRECT_CODEC`.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct BannerPatternDefinition {
    pub asset_id: String,
    pub translation_key: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BannerPatternLayer {
    /// A `banner_pattern` registry id, or an inline pattern.
    pub pattern: Holder<String, BannerPatternDefinition>,
    pub color: crate::dye_color::DyeColor,
}

impl BannerPatternLayer {
    fn read_data(tag: &NbtTag) -> Option<Self> {
        let compound = tag.extract_compound()?;
        let pattern = match compound.get("pattern")? {
            NbtTag::String(id) => Holder::Reference(id.to_string()),
            NbtTag::Compound(pattern) => Holder::Direct(BannerPatternDefinition {
                asset_id: pattern.get_string("asset_id")?.to_string(),
                translation_key: pattern.get_string("translation_key")?.to_string(),
            }),
            _ => return None,
        };
        let color = crate::dye_color::DyeColor::by_name(compound.get_string("color")?)?;
        Some(Self { pattern, color })
    }

    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        match &self.pattern {
            Holder::Reference(id) => compound.put_string("pattern", id.clone()),
            Holder::Direct(definition) => {
                let mut pattern = NbtCompound::new();
                pattern.put_string("asset_id", definition.asset_id.clone());
                pattern.put_string("translation_key", definition.translation_key.clone());
                compound.put_compound("pattern", pattern);
            }
        }
        compound.put_string("color", self.color.name().to_string());
        NbtTag::Compound(compound)
    }

    fn get_hash(&self) -> u32 {
        let pattern = match &self.pattern {
            Holder::Reference(id) => get_str_hash(&namespaced(id)),
            Holder::Direct(definition) => MapHash::new()
                .field("asset_id", get_str_hash(&namespaced(&definition.asset_id)))
                .field("translation_key", get_str_hash(&definition.translation_key))
                .finish(),
        };
        MapHash::new()
            .field("pattern", pattern)
            .field("color", get_str_hash(self.color.name()))
            .finish()
    }
}

/// `minecraft:banner_patterns`: the layers painted on a banner or shield.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BannerPatternsImpl {
    pub layers: Vec<BannerPatternLayer>,
}

impl BannerPatternsImpl {
    pub const EMPTY: Self = Self { layers: Vec::new() };

    #[must_use]
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let layers = data
            .extract_list()?
            .iter()
            .map(BannerPatternLayer::read_data)
            .collect::<Option<Vec<_>>>()?;
        Some(Self { layers })
    }
}

impl DataComponentImpl for BannerPatternsImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::List(
            self.layers
                .iter()
                .map(BannerPatternLayer::write_data)
                .collect(),
        )
    }

    fn get_hash(&self) -> i32 {
        get_list_hash(self.layers.iter().map(BannerPatternLayer::get_hash)) as i32
    }

    default_impl!(BannerPatterns);
}

/// `minecraft:pot_decorations`: the item shown on each side of a decorated pot.
#[derive(Clone, Default)]
pub struct PotDecorationsImpl {
    pub back: Option<ItemStack>,
    pub left: Option<ItemStack>,
    pub right: Option<ItemStack>,
    pub front: Option<ItemStack>,
}
impl PotDecorationsImpl {
    const SIDES: [&'static str; 4] = ["back", "left", "right", "front"];

    #[must_use]
    pub const fn sides(&self) -> [&Option<ItemStack>; 4] {
        [&self.back, &self.left, &self.right, &self.front]
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.back.is_none() && self.left.is_none() && self.right.is_none() && self.front.is_none()
    }

    /// Also reads the flat list of item ids saved before 26.3, as vanilla's
    /// `PotDecorationsComponentUnflatteningFix.unpackList` upgrades it.
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        if let NbtTag::List(ids) = data {
            return Self::unpack_list(ids);
        }
        let compound = data.extract_compound()?;
        let side = |name: &str| match compound.get(name) {
            Some(tag) => read_item_stack_template(tag).map(Some),
            None => Some(None),
        };
        Some(Self {
            back: side("back")?,
            left: side("left")?,
            right: side("right")?,
            front: side("front")?,
        })
    }

    /// Missing or non-string entries are bricks; an empty id leaves the list unreadable.
    fn unpack_list(ids: &[NbtTag]) -> Option<Self> {
        let side = |i: usize| {
            let id = ids
                .get(i)
                .and_then(NbtTag::extract_string)
                .unwrap_or("minecraft:brick");
            if id.is_empty() {
                return None;
            }
            read_item_stack_template(&NbtTag::String(id.into())).map(Some)
        };
        Some(Self {
            back: side(0)?,
            left: side(1)?,
            right: side(2)?,
            front: side(3)?,
        })
    }
}
impl PartialEq for PotDecorationsImpl {
    fn eq(&self, other: &Self) -> bool {
        self.sides()
            .iter()
            .zip(other.sides())
            .all(|(a, b)| match (a, b) {
                (Some(a), Some(b)) => item_stack_template_eq(a, b),
                (None, None) => true,
                _ => false,
            })
    }
}
impl std::fmt::Debug for PotDecorationsImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = |side: &Option<ItemStack>| side.as_ref().map(|stack| stack.item.registry_key);
        f.debug_struct("PotDecorationsImpl")
            .field("back", &name(&self.back))
            .field("left", &name(&self.left))
            .field("right", &name(&self.right))
            .field("front", &name(&self.front))
            .finish()
    }
}
impl DataComponentImpl for PotDecorationsImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        for (name, side) in Self::SIDES.iter().zip(self.sides()) {
            if let Some(stack) = side {
                compound.put(name, write_item_stack_template(stack));
            }
        }
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        let mut fields = MapHash::new();
        for (name, side) in Self::SIDES.iter().zip(self.sides()) {
            fields = fields.optional_field(name, side.as_ref().map(get_item_stack_template_hash));
        }
        fields.finish() as i32
    }
    default_impl!(PotDecorations);
}

/// The lock's item predicate, kept as its raw NBT compound since Pumpkin does
/// not yet model item predicates.
// TODO: replace `predicate` with a typed item predicate once item predicates are modelled.
#[derive(Clone, Debug, PartialEq)]
pub struct LockImpl {
    pub predicate: NbtCompound,
}
impl LockImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_compound().map(|predicate| Self {
            predicate: predicate.clone(),
        })
    }
}
impl DataComponentImpl for LockImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Compound(self.predicate.clone())
    }
    default_impl!(Lock);
}

/// `minecraft:break_sound`, a sound event holder.
#[derive(Clone, Debug, Hash, PartialEq)]
pub struct BreakSoundImpl {
    pub sound: IdOr<SoundEvent>,
}
impl BreakSoundImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let mut compound = NbtCompound::new();
        compound.put("sound", data.clone());
        Some(Self {
            sound: get_idor(&compound, "sound", Sound::EntityItemBreak),
        })
    }
}
impl DataComponentImpl for BreakSoundImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        put_idor(&mut compound, "sound", &self.sound);
        compound.child_tags.remove("sound").unwrap_or(NbtTag::End)
    }
    fn get_hash(&self) -> i32 {
        get_idor_hash(&self.sound) as i32
    }
    default_impl!(BreakSound);
}

#[derive(Clone, Debug, PartialEq)]
pub struct SoundEvent {
    pub sound_name: String,
    pub range: Option<f32>,
}
impl std::hash::Hash for SoundEvent {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.sound_name.hash(state);
        if let Some(val) = self.range {
            true.hash(state);
            unsafe { (*(&raw const val).cast::<u32>()).hash(state) };
        } else {
            false.hash(state);
        }
    }
}
impl SoundEvent {
    pub const fn new(sound_name: String, range: Option<f32>) -> Self {
        Self { sound_name, range }
    }
}
