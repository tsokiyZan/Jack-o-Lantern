use super::Digest;
use crate::data_component_impl::basic::SoundEvent;
use crate::data_component_impl::combat::SwingAnimationImpl;
use crate::data_component_impl::{
    DataComponentImpl, Holder, IDSet, IdOr, MapHash, get_bool_hash, get_byte_hash, get_f32_hash,
    get_f64_hash, get_i32_hash, get_idor, get_idor_hash, get_idset_hash, get_list_hash,
    get_map_hash, get_str_hash, get_text_hash, namespaced, put_idor, synced_registry_id,
    text_to_nbt,
};
use crate::dye_color::DyeColor;
use crate::entity_type::EntityType;
use crate::jukebox_song::JukeboxSong;
use crate::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;
use std::borrow::Cow;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntProvider {
    Id(Cow<'static, str>),
    Inline(i32),
}

impl std::hash::Hash for IntProvider {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Id(s) => {
                0u8.hash(state);
                s.hash(state);
            }
            Self::Inline(v) => {
                1u8.hash(state);
                v.hash(state);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum FloatProvider {
    Id(Cow<'static, str>),
    Inline(f32),
}

impl PartialEq for FloatProvider {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Id(a), Self::Id(b)) => a == b,
            (Self::Inline(a), Self::Inline(b)) => a.to_bits() == b.to_bits(),
            _ => false,
        }
    }
}

impl Eq for FloatProvider {}

impl std::hash::Hash for FloatProvider {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Id(s) => {
                0u8.hash(state);
                s.hash(state);
            }
            Self::Inline(v) => {
                1u8.hash(state);
                v.to_bits().hash(state);
            }
        }
    }
}

// `ResolvableInt.CODEC`/`ResolvableFloat.CODEC` are `Codec.either(constant, key)`.
impl IntProvider {
    #[must_use]
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        match tag {
            NbtTag::String(id) => Some(Self::Id(Cow::Owned(id.to_string()))),
            other => other.as_numeric_int().map(Self::Inline),
        }
    }

    #[must_use]
    pub fn write_data(&self) -> NbtTag {
        match self {
            Self::Id(id) => NbtTag::String(id.as_ref().into()),
            Self::Inline(value) => NbtTag::Int(*value),
        }
    }

    #[must_use]
    pub fn get_hash(&self) -> u32 {
        match self {
            Self::Id(id) => get_str_hash(&namespaced(id)),
            Self::Inline(value) => get_i32_hash(*value),
        }
    }
}

impl FloatProvider {
    #[must_use]
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        match tag {
            NbtTag::String(id) => Some(Self::Id(Cow::Owned(id.to_string()))),
            other => other.as_numeric_float().map(Self::Inline),
        }
    }

    #[must_use]
    pub fn write_data(&self) -> NbtTag {
        match self {
            Self::Id(id) => NbtTag::String(id.as_ref().into()),
            Self::Inline(value) => NbtTag::Float(*value),
        }
    }

    #[must_use]
    pub fn get_hash(&self) -> u32 {
        match self {
            Self::Id(id) => get_str_hash(&namespaced(id)),
            Self::Inline(value) => get_f32_hash(*value),
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct CookingFuelImpl {
    pub burn_time: IntProvider,
    pub speed_multiplier: FloatProvider,
}

/// `minecraft:compostable`: how many layers the item adds to a composter.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct CompostableImpl {
    pub layers: IntProvider,
}

impl CompostableImpl {
    /// The chance that composting the item adds a layer. The vanilla
    /// `compostable/*` providers are weighted lists of 1 and 0 layers.
    #[must_use]
    pub fn chance(&self) -> f32 {
        match &self.layers {
            IntProvider::Inline(layers) => {
                if *layers > 0 {
                    1.0
                } else {
                    0.0
                }
            }
            IntProvider::Id(id) => match id.strip_prefix("minecraft:").unwrap_or(id) {
                "compostable/low" => 0.3,
                "compostable/low_medium" => 0.5,
                "compostable/medium" => 0.65,
                "compostable/medium_high" => 0.85,
                "compostable/always_add_one" => 1.0,
                _ => 0.0,
            },
        }
    }

    pub fn read_data(data: &NbtTag) -> Option<Self> {
        Some(Self {
            layers: IntProvider::read_data(data.extract_compound()?.get("layers")?)?,
        })
    }
}

impl DataComponentImpl for CompostableImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put("layers", self.layers.write_data());
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        MapHash::new()
            .field("layers", self.layers.get_hash())
            .finish() as i32
    }
    default_impl!(Compostable);
}

impl CookingFuelImpl {
    #[must_use]
    pub fn get_burn_ticks(&self, is_fast_cooking: bool) -> u16 {
        let base_ticks: u32 = match &self.burn_time {
            IntProvider::Inline(v) => (*v).max(0) as u32,
            IntProvider::Id(id) => {
                let name = id.strip_prefix("minecraft:").unwrap_or(id.as_ref());
                match name {
                    "cooking/time_lava_bucket" => 20000,
                    "cooking/time_coal_block" => 16000,
                    "cooking/time_dried_kelp_block" => 4001,
                    "cooking/time_blaze_rod" => 2400,
                    "cooking/time_coal" => 1600,
                    "cooking/time_boats" => 1200,
                    "cooking/time_hanging_signs" => 800,
                    "cooking/time_roots"
                    | "cooking/time_wood_blocks"
                    | "cooking/time_wood_items_small" => 300,
                    "cooking/time_wood_items_large" => 200,
                    "cooking/time_wood_slabs" => 150,
                    "cooking/time_dry_plants"
                    | "cooking/time_wood_items_extra_small"
                    | "cooking/time_wool" => 100,
                    "cooking/time_wool_carpets" => 67,
                    "cooking/time_bamboo" | "cooking/time_wool_slabs" => 50,
                    _ => 0,
                }
            }
        };

        let factor = if is_fast_cooking { 2 } else { 1 };
        (base_ticks / factor) as u16
    }

    #[must_use]
    pub fn get_speed_multiplier(&self, is_fast_cooking: bool) -> f32 {
        match &self.speed_multiplier {
            FloatProvider::Inline(v) => *v,
            FloatProvider::Id(id) => {
                let name = id.strip_prefix("minecraft:").unwrap_or(id.as_ref());
                match name {
                    "cooking/fast_speed_multiplier" => 2.0,
                    "cooking/normal_speed_multiplier" => 1.0,
                    "cooking/speed_default" => {
                        if is_fast_cooking {
                            2.0
                        } else {
                            1.0
                        }
                    }
                    _ => 1.0,
                }
            }
        }
    }

    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        Some(Self {
            burn_time: IntProvider::read_data(compound.get("burn_time")?)?,
            speed_multiplier: FloatProvider::read_data(compound.get("speed_multiplier")?)?,
        })
    }
}

impl DataComponentImpl for CookingFuelImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put("burn_time", self.burn_time.write_data());
        compound.put("speed_multiplier", self.speed_multiplier.write_data());
        NbtTag::Compound(compound)
    }

    fn get_hash(&self) -> i32 {
        MapHash::new()
            .field("burn_time", self.burn_time.get_hash())
            .field("speed_multiplier", self.speed_multiplier.get_hash())
            .finish() as i32
    }

    default_impl!(CookingFuel);
}

/// 26.3 `minecraft:waxed` marker.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct WaxedImpl;

impl WaxedImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}

impl DataComponentImpl for WaxedImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Compound(NbtCompound::new())
    }
    fn get_hash(&self) -> i32 {
        get_map_hash([]) as i32
    }
    default_impl!(Waxed);
}

/// `minecraft:dye`: the dye color an item applies.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct DyeImpl {
    pub color: DyeColor,
}
impl DyeImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        Some(Self {
            color: DyeColor::by_name(data.extract_string()?)?,
        })
    }
}
impl DataComponentImpl for DyeImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.color.name().into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(self.color.name()) as i32
    }
    default_impl!(Dye);
}

/// 26.3 `minecraft:brewing_fuel`: how many brews the fuel lasts and how fast it brews.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct BrewingFuelImpl {
    pub uses: IntProvider,
    pub speed_multiplier: FloatProvider,
}
impl BrewingFuelImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        Some(Self {
            uses: IntProvider::read_data(compound.get("uses")?)?,
            speed_multiplier: FloatProvider::read_data(compound.get("speed_multiplier")?)?,
        })
    }
}
impl DataComponentImpl for BrewingFuelImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put("uses", self.uses.write_data());
        compound.put("speed_multiplier", self.speed_multiplier.write_data());
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        MapHash::new()
            .field("uses", self.uses.get_hash())
            .field("speed_multiplier", self.speed_multiplier.get_hash())
            .finish() as i32
    }
    default_impl!(BrewingFuel);
}

/// `minecraft:mob_visibility`: how well the listed mobs can see the item's wearer.
#[derive(Clone, Debug, PartialEq)]
pub struct MobVisibilityImpl {
    pub targeting_entity_types: IDSet<EntityType>,
    pub visibility: f32,
}
impl MobVisibilityImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        Some(Self {
            targeting_entity_types: IDSet::read(compound.get("targeting_entity_types")?)?,
            visibility: compound.get("visibility")?.as_numeric_float()?,
        })
    }
}
impl DataComponentImpl for MobVisibilityImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        self.targeting_entity_types
            .write(&mut compound, "targeting_entity_types");
        compound.put_float("visibility", self.visibility);
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        MapHash::new()
            .field(
                "targeting_entity_types",
                get_idset_hash(&self.targeting_entity_types),
            )
            .field("visibility", get_f32_hash(self.visibility))
            .finish() as i32
    }
    default_impl!(MobVisibility);
}

/// `minecraft:block_transformer`: the `block_transformer` registry entry the
/// item applies when used on a block.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct BlockTransformerImpl {
    pub transformer: Cow<'static, str>,
}
impl BlockTransformerImpl {
    pub const REGISTRY: &'static str = "block_transformer";

    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let id = data.extract_string()?;
        synced_registry_id(Self::REGISTRY, id)?;
        Some(Self {
            transformer: Cow::Owned(namespaced(id).into_owned()),
        })
    }
}
impl DataComponentImpl for BlockTransformerImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(namespaced(&self.transformer).into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(&namespaced(&self.transformer)) as i32
    }
    default_impl!(BlockTransformer);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct DyedColorImpl {
    pub rgb: i32,
}
impl DyedColorImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_int().map(|rgb| Self { rgb })
    }
}
impl DataComponentImpl for DyedColorImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Int(self.rgb)
    }
    fn get_hash(&self) -> i32 {
        get_i32_hash(self.rgb) as i32
    }
    default_impl!(DyedColor);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct MapIdImpl {
    pub id: i32,
}
impl MapIdImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_int().map(|id| Self { id })
    }
}
impl DataComponentImpl for MapIdImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Int(self.id)
    }
    fn get_hash(&self) -> i32 {
        get_i32_hash(self.id) as i32
    }
    default_impl!(MapId);
}

/// One `MapDecorations.Entry`: a marker drawn on a map item.
#[derive(Clone, Debug, PartialEq)]
pub struct MapDecorationEntry {
    pub r#type: String,
    pub x: f64,
    pub z: f64,
    pub rotation: f32,
}

/// `minecraft:map_decorations`: markers keyed by id.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MapDecorationsImpl {
    pub decorations: Vec<(String, MapDecorationEntry)>,
}
impl MapDecorationsImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let decorations = data
            .extract_compound()?
            .child_tags
            .iter()
            .map(|(key, entry)| {
                let entry = entry.extract_compound()?;
                Some((
                    key.to_string(),
                    MapDecorationEntry {
                        r#type: entry.get_string("type")?.to_string(),
                        x: entry.get_double("x")?,
                        z: entry.get_double("z")?,
                        rotation: entry.get_float("rotation")?,
                    },
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { decorations })
    }
}
impl DataComponentImpl for MapDecorationsImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        for (key, entry) in &self.decorations {
            let mut entry_compound = NbtCompound::new();
            entry_compound.put_string("type", entry.r#type.clone());
            entry_compound.put_double("x", entry.x);
            entry_compound.put_double("z", entry.z);
            entry_compound.put_float("rotation", entry.rotation);
            compound.put_compound(key, entry_compound);
        }
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        get_map_hash(self.decorations.iter().map(|(key, entry)| {
            (
                get_str_hash(key),
                MapHash::new()
                    .field("type", get_str_hash(&namespaced(&entry.r#type)))
                    .field("x", get_f64_hash(entry.x))
                    .field("z", get_f64_hash(entry.z))
                    .field("rotation", get_f32_hash(entry.rotation))
                    .finish(),
            )
        })) as i32
    }
    default_impl!(MapDecorations);
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum MapPostProcessing {
    Lock = 0,
    Scale = 1,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Default)]
pub struct MapPostProcessingImpl {
    pub processing: Option<MapPostProcessing>,
}

impl MapPostProcessingImpl {
    pub const SCALE: Self = Self {
        processing: Some(MapPostProcessing::Scale),
    };
    pub const LOCK: Self = Self {
        processing: Some(MapPostProcessing::Lock),
    };

    #[must_use]
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        if let Some(val) = data.extract_int() {
            let processing = match val {
                0 => Some(MapPostProcessing::Lock),
                1 => Some(MapPostProcessing::Scale),
                _ => None,
            };
            Some(Self { processing })
        } else if let Some(val) = data.extract_string() {
            let processing = match val {
                "lock" => Some(MapPostProcessing::Lock),
                "scale" => Some(MapPostProcessing::Scale),
                _ => None,
            };
            Some(Self { processing })
        } else {
            None
        }
    }
}

impl DataComponentImpl for MapPostProcessingImpl {
    fn write_data(&self) -> NbtTag {
        match self.processing {
            Some(MapPostProcessing::Lock) => NbtTag::Int(0),
            Some(MapPostProcessing::Scale) => NbtTag::Int(1),
            None => NbtTag::Int(0),
        }
    }
    default_impl!(MapPostProcessing);
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChargedProjectilesImpl {
    pub projectiles: Vec<NbtCompound>,
}
impl ChargedProjectilesImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let list = data.extract_list()?;
        let mut projectiles = Vec::new();
        for item in list {
            projectiles.push(item.extract_compound()?.clone());
        }
        Some(Self { projectiles })
    }
}
impl DataComponentImpl for ChargedProjectilesImpl {
    fn write_data(&self) -> NbtTag {
        let mut list = Vec::new();
        for item in &self.projectiles {
            list.push(NbtTag::Compound(item.clone()));
        }
        NbtTag::List(list)
    }
    fn get_hash(&self) -> i32 {
        0
    }
    default_impl!(ChargedProjectiles);
}

#[derive(Clone)]
pub struct BundleContentsImpl {
    pub items: Vec<crate::item_stack::ItemStack>,
    /// Vanilla `BundleContents.selectedItem`: the item the player picked in the bundle's
    /// tooltip, -1 for none. Neither saved nor sent, nor part of equality.
    pub selected_item: i32,
}
impl PartialEq for BundleContentsImpl {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}
impl Eq for BundleContentsImpl {}
impl std::fmt::Debug for BundleContentsImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BundleContentsImpl")
    }
}
impl BundleContentsImpl {
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        let mut items = Vec::new();
        if let NbtTag::List(l) = tag {
            for item_tag in l {
                if let NbtTag::Compound(c) = item_tag
                    && let Some(stack) = crate::item_stack::ItemStack::read_item_stack(c)
                {
                    items.push(stack);
                }
            }
        }
        Some(Self {
            items,
            selected_item: -1,
        })
    }

    /// Vanilla `indexIsOutsideAllowedBounds`.
    const fn index_is_outside_allowed_bounds(&self, index: i32) -> bool {
        index < 0 || index as usize >= self.items.len()
    }

    /// Vanilla `BundleContents.Mutable.toggleSelectedItem`: picking the selected item again,
    /// or one that isn't there, clears the selection.
    pub const fn toggle_selected_item(&mut self, selected_item: i32) {
        self.selected_item = if self.selected_item != selected_item
            && !self.index_is_outside_allowed_bounds(selected_item)
        {
            selected_item
        } else {
            -1
        };
    }

    pub fn get_weight(&self) -> u32 {
        self.items
            .iter()
            .map(|item| item.item_count as u32 * (64 / item.get_max_stack_size() as u32).max(1))
            .sum()
    }
    pub fn try_insert(&mut self, stack: &mut crate::item_stack::ItemStack) -> bool {
        if stack.is_empty() || stack.get_data_component::<BundleContentsImpl>().is_some() {
            return false;
        }
        let weight_per_item = (64 / stack.get_max_stack_size() as u32).max(1);
        let mut inserted_anything = false;
        while stack.item_count > 0 && self.get_weight() + weight_per_item <= 64 {
            if let Some(top) = self.items.first_mut()
                && crate::item_stack::ItemStack::are_items_and_components_equal(top, stack)
                && top.item_count < top.get_max_stack_size()
            {
                top.item_count += 1;
                stack.item_count -= 1;
                inserted_anything = true;
                continue;
            }
            self.items.insert(0, stack.copy_with_count(1));
            stack.item_count -= 1;
            inserted_anything = true;
        }
        inserted_anything
    }
    /// Vanilla `BundleContents.Mutable.removeOne`: the selected item, else the first; the
    /// selection is cleared.
    pub fn try_extract(&mut self) -> Option<crate::item_stack::ItemStack> {
        if self.items.is_empty() {
            return None;
        }
        let index = if self.index_is_outside_allowed_bounds(self.selected_item) {
            0
        } else {
            self.selected_item as usize
        };
        let stack = self.items.remove(index);
        self.toggle_selected_item(-1);
        Some(stack)
    }
}
impl DataComponentImpl for BundleContentsImpl {
    fn write_data(&self) -> NbtTag {
        let mut list = Vec::new();
        for stack in &self.items {
            let mut item_compound = NbtCompound::new();
            stack.write_item_stack(&mut item_compound);
            list.push(NbtTag::Compound(item_compound));
        }
        NbtTag::List(list)
    }
    default_impl!(BundleContents);
}

/// The dimension and block position a lodestone compass points to.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct LodestoneTarget {
    pub dimension: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct LodestoneTrackerImpl {
    pub target: Option<LodestoneTarget>,
    pub tracked: bool,
}
impl LodestoneTrackerImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let tracked = compound.get_bool("tracked").unwrap_or(true);
        let target = compound.get_compound("target").and_then(|target| {
            let dimension = target.get_string("dimension")?.to_string();
            let pos = target.get_int_array("pos")?;
            if pos.len() != 3 {
                return None;
            }
            Some(LodestoneTarget {
                dimension,
                x: pos[0],
                y: pos[1],
                z: pos[2],
            })
        });
        Some(Self { target, tracked })
    }
}
impl DataComponentImpl for LodestoneTrackerImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        if let Some(target) = &self.target {
            let mut target_compound = NbtCompound::new();
            target_compound.put_string("dimension", target.dimension.clone());
            target_compound.put("pos", NbtTag::IntArray(vec![target.x, target.y, target.z]));
            compound.put_compound("target", target_compound);
        }
        compound.put_bool("tracked", self.tracked);
        NbtTag::Compound(compound)
    }
    default_impl!(LodestoneTracker);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FireworkExplosionShape {
    SmallBall = 0,
    LargeBall = 1,
    Star = 2,
    Creeper = 3,
    Burst = 4,
}
impl FireworkExplosionShape {
    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Self::SmallBall),
            1 => Some(Self::LargeBall),
            2 => Some(Self::Star),
            3 => Some(Self::Creeper),
            4 => Some(Self::Burst),
            _ => None,
        }
    }
    pub fn to_id(&self) -> i32 {
        *self as i32
    }
    pub fn to_name(&self) -> &str {
        match self {
            Self::SmallBall => "small_ball",
            Self::LargeBall => "large_ball",
            Self::Star => "star",
            Self::Creeper => "creeper",
            Self::Burst => "burst",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "small_ball" => Some(Self::SmallBall),
            "large_ball" => Some(Self::LargeBall),
            "star" => Some(Self::Star),
            "creeper" => Some(Self::Creeper),
            "burst" => Some(Self::Burst),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct FireworkExplosionImpl {
    pub shape: FireworkExplosionShape,
    pub colors: Vec<i32>,
    pub fade_colors: Vec<i32>,
    pub has_trail: bool,
    pub has_twinkle: bool,
}
impl FireworkExplosionImpl {
    pub fn new(
        shape: FireworkExplosionShape,
        colors: Vec<i32>,
        fade_colors: Vec<i32>,
        has_trail: bool,
        has_twinkle: bool,
    ) -> Self {
        Self {
            shape,
            colors,
            fade_colors,
            has_trail,
            has_twinkle,
        }
    }
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        let compound = tag.extract_compound()?;
        let shape = FireworkExplosionShape::from_name(compound.get_string("shape")?)?;
        // `Codec.INT.listOf()` reads an int list or an int array.
        let colors = |name: &str| match compound.get(name) {
            Some(NbtTag::IntArray(colors)) => Some(colors.clone()),
            Some(NbtTag::List(colors)) => colors.iter().map(NbtTag::as_numeric_int).collect(),
            Some(_) => None,
            None => Some(Vec::new()),
        };
        let colors_list = colors("colors")?;
        let fade_colors = colors("fade_colors")?;
        let has_trail = compound.get_bool("has_trail").unwrap_or(false);
        let has_twinkle = compound.get_bool("has_twinkle").unwrap_or(false);
        Some(Self {
            shape,
            colors: colors_list,
            fade_colors,
            has_trail,
            has_twinkle,
        })
    }
}
impl DataComponentImpl for FireworkExplosionImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_string("shape", self.shape.to_name().to_string());
        let colors = |colors: &[i32]| colors.iter().map(|color| NbtTag::Int(*color)).collect();
        if !self.colors.is_empty() {
            compound.put_list("colors", colors(&self.colors));
        }
        if !self.fade_colors.is_empty() {
            compound.put_list("fade_colors", colors(&self.fade_colors));
        }
        if self.has_trail {
            compound.put_bool("has_trail", true);
        }
        if self.has_twinkle {
            compound.put_bool("has_twinkle", true);
        }
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        // `COLOR_LIST_CODEC` is `Codec.INT.listOf()`, a plain list of ints.
        let colors = |colors: &[i32]| {
            (!colors.is_empty()).then(|| get_list_hash(colors.iter().map(|c| get_i32_hash(*c))))
        };
        MapHash::new()
            .field("shape", get_str_hash(self.shape.to_name()))
            .optional_field("colors", colors(&self.colors))
            .optional_field("fade_colors", colors(&self.fade_colors))
            .optional_field("has_trail", self.has_trail.then(|| get_bool_hash(true)))
            .optional_field("has_twinkle", self.has_twinkle.then(|| get_bool_hash(true)))
            .finish() as i32
    }
    default_impl!(FireworkExplosion);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct FireworksImpl {
    pub flight_duration: i32,
    pub explosions: Vec<FireworkExplosionImpl>,
}
impl FireworksImpl {
    pub fn new(flight_duration: i32, explosions: Vec<FireworkExplosionImpl>) -> Self {
        Self {
            flight_duration,
            explosions,
        }
    }
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        let compound = tag.extract_compound()?;
        // `ExtraCodecs.UNSIGNED_BYTE`, 0 when absent.
        let flight_duration = match compound.get("flight_duration") {
            Some(tag) => i32::from(tag.as_numeric_int()? as u8),
            None => 0,
        };
        let mut explosions = Vec::new();
        if let Some(list) = compound.get_list("explosions") {
            for item in list {
                if let Some(explosion) = FireworkExplosionImpl::read_data(item) {
                    explosions.push(explosion);
                }
            }
        }
        Some(Self {
            flight_duration,
            explosions,
        })
    }
}
impl DataComponentImpl for FireworksImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        if self.flight_duration != 0 {
            compound.put_byte("flight_duration", self.flight_duration as u8 as i8);
        }
        if !self.explosions.is_empty() {
            let explosions_list: Vec<NbtTag> =
                self.explosions.iter().map(|e| e.write_data()).collect();
            compound.put_list("explosions", explosions_list);
        }
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        MapHash::new()
            .optional_field(
                "flight_duration",
                (self.flight_duration != 0)
                    .then(|| get_byte_hash(self.flight_duration as u8 as i8)),
            )
            .optional_field(
                "explosions",
                (!self.explosions.is_empty())
                    .then(|| get_list_hash(self.explosions.iter().map(|e| e.get_hash() as u32))),
            )
            .finish() as i32
    }
    default_impl!(Fireworks);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ProfileProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Default)]
pub struct ProfileImpl {
    pub name: Option<String>,
    pub id: Option<[i32; 4]>,
    pub properties: Vec<ProfileProperty>,
    pub texture: Option<String>,
    pub cape: Option<String>,
    pub elytra: Option<String>,
    pub model: Option<String>,
}
impl ProfileImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        match data {
            NbtTag::String(name) => Some(Self {
                name: Some(name.to_string()),
                ..Default::default()
            }),
            NbtTag::Compound(compound) => {
                let name = compound.get_string("name").map(String::from);
                let id = compound.get_int_array("id").and_then(|arr| {
                    if arr.len() == 4 {
                        Some([arr[0], arr[1], arr[2], arr[3]])
                    } else {
                        None
                    }
                });
                let mut properties = Vec::new();
                if let Some(props_list) = compound.get_list("properties") {
                    for prop_tag in props_list {
                        if let Some(prop_comp) = prop_tag.extract_compound()
                            && let (Some(prop_name), Some(prop_value)) =
                                (prop_comp.get_string("name"), prop_comp.get_string("value"))
                        {
                            properties.push(ProfileProperty {
                                name: prop_name.to_string(),
                                value: prop_value.to_string(),
                                signature: prop_comp.get_string("signature").map(String::from),
                            });
                        }
                    }
                }
                let texture = compound.get_string("texture").map(String::from);
                let cape = compound.get_string("cape").map(String::from);
                let elytra = compound.get_string("elytra").map(String::from);
                let model = compound.get_string("model").map(String::from);
                Some(Self {
                    name,
                    id,
                    properties,
                    texture,
                    cape,
                    elytra,
                    model,
                })
            }
            _ => None,
        }
    }
}
impl DataComponentImpl for ProfileImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        if let Some(name) = &self.name {
            compound.put_string("name", name.clone());
        }
        if let Some(id) = &self.id {
            compound.put("id", NbtTag::IntArray(id.to_vec()));
        }
        if !self.properties.is_empty() {
            let mut props_list = Vec::new();
            for prop in &self.properties {
                let mut prop_comp = NbtCompound::new();
                prop_comp.put_string("name", prop.name.clone());
                prop_comp.put_string("value", prop.value.clone());
                if let Some(sig) = &prop.signature {
                    prop_comp.put_string("signature", sig.clone());
                }
                props_list.push(NbtTag::Compound(prop_comp));
            }
            compound.put_list("properties", props_list);
        }
        if let Some(texture) = &self.texture {
            compound.put_string("texture", texture.clone());
        }
        if let Some(cape) = &self.cape {
            compound.put_string("cape", cape.clone());
        }
        if let Some(elytra) = &self.elytra {
            compound.put_string("elytra", elytra.clone());
        }
        if let Some(model) = &self.model {
            compound.put_string("model", model.clone());
        }
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        if let Some(name) = &self.name {
            digest.update(&[1u8]);
            digest.update(&get_str_hash(name).to_le_bytes());
        }
        if let Some(id) = &self.id {
            digest.update(&[2u8]);
            for val in id {
                digest.update(&get_i32_hash(*val).to_le_bytes());
            }
        }
        if !self.properties.is_empty() {
            digest.update(&[3u8]);
            for prop in &self.properties {
                digest.update(&get_str_hash(&prop.name).to_le_bytes());
                digest.update(&get_str_hash(&prop.value).to_le_bytes());
                if let Some(sig) = &prop.signature {
                    digest.update(&[1u8]);
                    digest.update(&get_str_hash(sig).to_le_bytes());
                } else {
                    digest.update(&[0u8]);
                }
            }
        }
        if let Some(texture) = &self.texture {
            digest.update(&[4u8]);
            digest.update(&get_str_hash(texture).to_le_bytes());
        }
        if let Some(cape) = &self.cape {
            digest.update(&[5u8]);
            digest.update(&get_str_hash(cape).to_le_bytes());
        }
        if let Some(elytra) = &self.elytra {
            digest.update(&[6u8]);
            digest.update(&get_str_hash(elytra).to_le_bytes());
        }
        if let Some(model) = &self.model {
            digest.update(&[7u8]);
            digest.update(&get_str_hash(model).to_le_bytes());
        }
        digest.finalize() as i32
    }
    default_impl!(Profile);
}

/// An inline jukebox song, `JukeboxSong.DIRECT_CODEC`.
#[derive(Clone, Debug, PartialEq)]
pub struct JukeboxSongDefinition {
    pub sound_event: IdOr<SoundEvent>,
    pub description: TextComponent,
    pub length_in_seconds: f32,
    pub comparator_output: i32,
}

/// `minecraft:jukebox_playable`, a holder of the song the item plays.
#[derive(Clone, Debug, PartialEq)]
pub struct JukeboxPlayableImpl {
    pub song: Holder<JukeboxSong, Box<JukeboxSongDefinition>>,
}
impl JukeboxPlayableImpl {
    // `JukeboxSong.CODEC` only stores registry songs. The inline form is read
    // back too so a song that arrived over the network survives a save.
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let song = match data {
            NbtTag::String(id) => Holder::Reference(JukeboxSong::from_name(
                id.strip_prefix("minecraft:").unwrap_or(id),
            )?),
            NbtTag::Compound(compound) => {
                compound.get("sound_event")?;
                Holder::Direct(Box::new(JukeboxSongDefinition {
                    sound_event: get_idor(compound, "sound_event", Sound::IntentionallyEmpty),
                    description: TextComponent::from_nbt(compound.get("description")?),
                    length_in_seconds: compound.get("length_in_seconds")?.as_numeric_float()?,
                    comparator_output: compound.get("comparator_output")?.as_numeric_int()?,
                }))
            }
            _ => return None,
        };
        Some(Self { song })
    }
}
impl DataComponentImpl for JukeboxPlayableImpl {
    fn write_data(&self) -> NbtTag {
        match &self.song {
            Holder::Reference(song) => {
                NbtTag::String(format!("minecraft:{}", song.to_name()).into())
            }
            Holder::Direct(definition) => {
                let mut compound = NbtCompound::new();
                put_idor(&mut compound, "sound_event", &definition.sound_event);
                compound.put("description", text_to_nbt(&definition.description));
                compound.put_float("length_in_seconds", definition.length_in_seconds);
                compound.put_int("comparator_output", definition.comparator_output);
                NbtTag::Compound(compound)
            }
        }
    }
    fn get_hash(&self) -> i32 {
        (match &self.song {
            Holder::Reference(song) => get_str_hash(&format!("minecraft:{}", song.to_name())),
            Holder::Direct(definition) => MapHash::new()
                .field("sound_event", get_idor_hash(&definition.sound_event))
                .field("description", get_text_hash(&definition.description))
                .field(
                    "length_in_seconds",
                    get_f32_hash(definition.length_in_seconds),
                )
                .field(
                    "comparator_output",
                    get_i32_hash(definition.comparator_output),
                )
                .finish(),
        }) as i32
    }
    default_impl!(JukeboxPlayable);
}

/// `minecraft:recipes`: the recipe keys a knowledge book unlocks.
#[derive(Clone, Debug, Hash, PartialEq, Eq, Default)]
pub struct RecipesImpl {
    pub recipes: Vec<String>,
}
impl RecipesImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let recipes = data
            .extract_list()?
            .iter()
            .map(|tag| tag.extract_string().map(str::to_string))
            .collect::<Option<Vec<_>>>()?;
        Some(Self { recipes })
    }
}
impl DataComponentImpl for RecipesImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::List(
            self.recipes
                .iter()
                .map(|recipe| NbtTag::String(recipe.as_str().into()))
                .collect(),
        )
    }
    fn get_hash(&self) -> i32 {
        get_list_hash(
            self.recipes
                .iter()
                .map(|recipe| get_str_hash(&namespaced(recipe))),
        ) as i32
    }
    default_impl!(Recipes);
}
