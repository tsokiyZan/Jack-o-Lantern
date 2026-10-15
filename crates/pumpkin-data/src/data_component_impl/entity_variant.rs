use crate::data_component_impl::{
    DataComponentImpl, Holder, MapHash, get_i32_hash, get_str_hash, get_text_hash, namespaced,
    text_to_nbt,
};
use crate::painting_variant::PaintingVariant;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;
use std::borrow::Cow;

/// What happens to an id a `ByIdMap.continuous` map has no entry for.
#[derive(Clone, Copy, Debug)]
pub enum OutOfBoundsStrategy {
    Zero,
    Wrap,
    Clamp,
}

/// How a variant component goes over the network and into a hash.
#[derive(Clone, Copy, Debug)]
pub enum VariantCodec {
    /// `ByteBufCodecs.holderRegistry` over a registry sent during configuration.
    SyncedRegistry(&'static str),
    /// `ByteBufCodecs.holderRegistry` over a built-in registry, in registration order.
    BuiltinRegistry(&'static [&'static str]),
    /// `ByteBufCodecs.idMapper` over a `ByIdMap.continuous` enum, names by id.
    Continuous(&'static [&'static str], OutOfBoundsStrategy),
    /// `ByteBufCodecs.idMapper` over a `ByIdMap.sparse` enum, with its default.
    Sparse(&'static [(i32, &'static str)], &'static str),
    /// `DyeColor.STREAM_CODEC`.
    DyeColor,
}

impl VariantCodec {
    /// Registry entries are identifiers; enums and dye colors are bare names.
    #[must_use]
    pub fn persistent_name<'a>(&self, value: &'a str) -> Cow<'a, str> {
        match self {
            Self::SyncedRegistry(_) | Self::BuiltinRegistry(_) => namespaced(value),
            _ => Cow::Borrowed(value.strip_prefix("minecraft:").unwrap_or(value)),
        }
    }
}

/// `VillagerType` registration order.
pub const VILLAGER_TYPES: &[&str] = &[
    "desert", "jungle", "plains", "savanna", "snow", "swamp", "taiga",
];
/// `Fox.Variant`.
pub const FOX_VARIANTS: &[&str] = &["red", "snow"];
/// `Salmon.Variant`.
pub const SALMON_VARIANTS: &[&str] = &["small", "medium", "large"];
/// `Parrot.Variant`.
pub const PARROT_VARIANTS: &[&str] = &["red_blue", "blue", "green", "yellow_blue", "gray"];
/// `TropicalFish.Pattern`, keyed by `base.id | index << 8`.
pub const TROPICAL_FISH_PATTERNS: &[(i32, &str)] = &[
    (0, "kob"),
    (1 << 8, "sunstreak"),
    (2 << 8, "snooper"),
    (3 << 8, "dasher"),
    (4 << 8, "brinely"),
    (5 << 8, "spotty"),
    (1, "flopper"),
    (1 | 1 << 8, "stripey"),
    (1 | 2 << 8, "glitter"),
    (1 | 3 << 8, "blockfish"),
    (1 | 4 << 8, "betty"),
    (1 | 5 << 8, "clayfish"),
];
/// `MushroomCow.Variant`.
pub const MOOSHROOM_VARIANTS: &[&str] = &["red", "brown"];
/// `Rabbit.Variant`.
pub const RABBIT_VARIANTS: &[(i32, &str)] = &[
    (0, "brown"),
    (1, "white"),
    (2, "black"),
    (3, "white_splotched"),
    (4, "gold"),
    (5, "salt"),
    (99, "evil"),
];
/// `equine.Variant`.
pub const HORSE_VARIANTS: &[&str] = &[
    "white",
    "creamy",
    "chestnut",
    "brown",
    "black",
    "gray",
    "dark_brown",
];
/// `Llama.Variant`.
pub const LLAMA_VARIANTS: &[&str] = &["creamy", "white", "brown", "gray"];
/// `Axolotl.Variant`.
pub const AXOLOTL_VARIANTS: &[&str] = &["lucy", "wild", "gold", "cyan", "blue"];

macro_rules! string_variant {
    ($name:ident, $struct_name:ident, $codec:expr) => {
        #[derive(Clone, Debug, Hash, PartialEq, Eq)]
        pub struct $struct_name {
            pub value: Cow<'static, str>,
        }
        impl $struct_name {
            pub const CODEC: VariantCodec = $codec;

            pub fn read_data(data: &NbtTag) -> Option<Self> {
                data.extract_string().map(|v| Self {
                    value: Cow::Owned(v.to_string()),
                })
            }
        }
        impl DataComponentImpl for $struct_name {
            fn write_data(&self) -> NbtTag {
                NbtTag::String(Self::CODEC.persistent_name(&self.value).into())
            }
            fn get_hash(&self) -> i32 {
                get_str_hash(&Self::CODEC.persistent_name(&self.value)) as i32
            }
            default_impl!($name);
        }
    };
}

string_variant!(
    VillagerVariant,
    VillagerVariantImpl,
    VariantCodec::BuiltinRegistry(VILLAGER_TYPES)
);
string_variant!(
    WolfVariant,
    WolfVariantImpl,
    VariantCodec::SyncedRegistry("wolf_variant")
);
string_variant!(
    WolfSoundVariant,
    WolfSoundVariantImpl,
    VariantCodec::SyncedRegistry("wolf_sound_variant")
);
string_variant!(WolfCollar, WolfCollarImpl, VariantCodec::DyeColor);
string_variant!(
    FoxVariant,
    FoxVariantImpl,
    VariantCodec::Continuous(FOX_VARIANTS, OutOfBoundsStrategy::Zero)
);
string_variant!(
    SalmonSize,
    SalmonSizeImpl,
    VariantCodec::Continuous(SALMON_VARIANTS, OutOfBoundsStrategy::Clamp)
);
string_variant!(
    ParrotVariant,
    ParrotVariantImpl,
    VariantCodec::Continuous(PARROT_VARIANTS, OutOfBoundsStrategy::Clamp)
);
string_variant!(
    TropicalFishPattern,
    TropicalFishPatternImpl,
    VariantCodec::Sparse(TROPICAL_FISH_PATTERNS, "kob")
);
string_variant!(
    TropicalFishBaseColor,
    TropicalFishBaseColorImpl,
    VariantCodec::DyeColor
);
string_variant!(
    TropicalFishPatternColor,
    TropicalFishPatternColorImpl,
    VariantCodec::DyeColor
);
string_variant!(
    MooshroomVariant,
    MooshroomVariantImpl,
    VariantCodec::Continuous(MOOSHROOM_VARIANTS, OutOfBoundsStrategy::Clamp)
);
string_variant!(
    RabbitVariant,
    RabbitVariantImpl,
    VariantCodec::Sparse(RABBIT_VARIANTS, "brown")
);
string_variant!(
    PigVariant,
    PigVariantImpl,
    VariantCodec::SyncedRegistry("pig_variant")
);
string_variant!(
    PigSoundVariant,
    PigSoundVariantImpl,
    VariantCodec::SyncedRegistry("pig_sound_variant")
);
string_variant!(
    CowVariant,
    CowVariantImpl,
    VariantCodec::SyncedRegistry("cow_variant")
);
string_variant!(
    CowSoundVariant,
    CowSoundVariantImpl,
    VariantCodec::SyncedRegistry("cow_sound_variant")
);
string_variant!(
    ChickenVariant,
    ChickenVariantImpl,
    VariantCodec::SyncedRegistry("chicken_variant")
);
string_variant!(
    ChickenSoundVariant,
    ChickenSoundVariantImpl,
    VariantCodec::SyncedRegistry("chicken_sound_variant")
);
string_variant!(
    ZombieNautilusVariant,
    ZombieNautilusVariantImpl,
    VariantCodec::SyncedRegistry("zombie_nautilus_variant")
);
string_variant!(
    FrogVariant,
    FrogVariantImpl,
    VariantCodec::SyncedRegistry("frog_variant")
);
string_variant!(
    HorseVariant,
    HorseVariantImpl,
    VariantCodec::Continuous(HORSE_VARIANTS, OutOfBoundsStrategy::Wrap)
);
string_variant!(
    LlamaVariant,
    LlamaVariantImpl,
    VariantCodec::Continuous(LLAMA_VARIANTS, OutOfBoundsStrategy::Clamp)
);
string_variant!(
    AxolotlVariant,
    AxolotlVariantImpl,
    VariantCodec::Continuous(AXOLOTL_VARIANTS, OutOfBoundsStrategy::Zero)
);
string_variant!(
    CatVariant,
    CatVariantImpl,
    VariantCodec::SyncedRegistry("cat_variant")
);
string_variant!(
    CatSoundVariant,
    CatSoundVariantImpl,
    VariantCodec::SyncedRegistry("cat_sound_variant")
);
string_variant!(CatCollar, CatCollarImpl, VariantCodec::DyeColor);
string_variant!(SheepColor, SheepColorImpl, VariantCodec::DyeColor);
string_variant!(ShulkerColor, ShulkerColorImpl, VariantCodec::DyeColor);

/// An inline painting, `PaintingVariant.DIRECT_CODEC`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaintingVariantDefinition {
    pub width: i32,
    pub height: i32,
    pub asset_id: String,
    pub title: Option<TextComponent>,
    pub author: Option<TextComponent>,
}

/// `minecraft:painting/variant`, a holder that is either a registry entry or an inline painting.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintingVariantImpl {
    pub variant: Holder<PaintingVariant, Box<PaintingVariantDefinition>>,
}
impl PaintingVariantImpl {
    // `PaintingVariant.CODEC` only stores registry paintings. The inline form is
    // read back too so a painting that arrived over the network survives a save.
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let variant = match data {
            NbtTag::String(id) => Holder::Reference(PaintingVariant::from_name(id)?),
            NbtTag::Compound(compound) => {
                let text = |name: &str| compound.get(name).map(TextComponent::from_nbt);
                Holder::Direct(Box::new(PaintingVariantDefinition {
                    width: compound.get("width")?.as_numeric_int()?,
                    height: compound.get("height")?.as_numeric_int()?,
                    asset_id: compound.get_string("asset_id")?.to_string(),
                    title: text("title"),
                    author: text("author"),
                }))
            }
            _ => return None,
        };
        Some(Self { variant })
    }
}
impl DataComponentImpl for PaintingVariantImpl {
    fn write_data(&self) -> NbtTag {
        match &self.variant {
            Holder::Reference(variant) => NbtTag::String(variant.asset_id().into()),
            Holder::Direct(definition) => {
                let mut compound = NbtCompound::new();
                compound.put_int("width", definition.width);
                compound.put_int("height", definition.height);
                compound.put_string("asset_id", definition.asset_id.clone());
                if let Some(title) = &definition.title {
                    compound.put("title", text_to_nbt(title));
                }
                if let Some(author) = &definition.author {
                    compound.put("author", text_to_nbt(author));
                }
                NbtTag::Compound(compound)
            }
        }
    }
    fn get_hash(&self) -> i32 {
        (match &self.variant {
            Holder::Reference(variant) => get_str_hash(variant.asset_id()),
            Holder::Direct(definition) => MapHash::new()
                .field("width", get_i32_hash(definition.width))
                .field("height", get_i32_hash(definition.height))
                .field("asset_id", get_str_hash(&namespaced(&definition.asset_id)))
                .optional_field("title", definition.title.as_ref().map(get_text_hash))
                .optional_field("author", definition.author.as_ref().map(get_text_hash))
                .finish(),
        }) as i32
    }
    default_impl!(PaintingVariant);
}
