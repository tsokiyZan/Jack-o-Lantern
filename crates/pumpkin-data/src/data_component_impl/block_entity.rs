use super::Digest;
use crate::data_component_impl::{
    DataComponentImpl, MapHash, get_bool_hash, get_compound_hash, get_i32_hash,
    get_item_stack_template_hash, get_list_hash, get_str_hash, get_text_hash,
    item_stack_template_eq, read_item_stack_template, text_to_nbt, write_item_stack_template,
};
use crate::decorated_pot_pattern::DecoratedPotPattern;
use crate::dye_color::DyeColor;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;

#[derive(Clone, Debug, PartialEq)]
pub struct BlockEntityDataImpl {
    pub nbt: NbtCompound,
}
impl BlockEntityDataImpl {
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        if let NbtTag::Compound(c) = tag {
            Some(Self { nbt: c.clone() })
        } else {
            None
        }
    }
}
impl DataComponentImpl for BlockEntityDataImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Compound(self.nbt.clone())
    }
    default_impl!(BlockEntityData);
}

/// Entity NBT carried by an item, e.g. a spawn egg. `None` for the item's default component.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityDataImpl {
    pub nbt: Option<NbtCompound>,
}
impl EntityDataImpl {
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        if let NbtTag::Compound(c) = tag {
            Some(Self {
                nbt: Some(c.clone()),
            })
        } else {
            None
        }
    }
}
impl DataComponentImpl for EntityDataImpl {
    fn write_data(&self) -> NbtTag {
        self.nbt.clone().map_or(NbtTag::End, NbtTag::Compound)
    }
    default_impl!(EntityData);
}

/// `minecraft:bucket_entity_data`: the NBT a bucketed mob keeps, as `CustomData`.
/// `None` stands for the empty compound of an item's default component.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BucketEntityDataImpl {
    pub nbt: Option<NbtCompound>,
}
impl BucketEntityDataImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        data.extract_compound().map(|nbt| Self {
            nbt: Some(nbt.clone()),
        })
    }

    #[must_use]
    pub fn nbt(&self) -> NbtCompound {
        self.nbt.clone().unwrap_or_default()
    }
}
impl DataComponentImpl for BucketEntityDataImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::Compound(self.nbt())
    }
    fn get_hash(&self) -> i32 {
        get_compound_hash(&self.nbt()) as i32
    }
    default_impl!(BucketEntityData);
}

#[derive(Clone)]
pub struct ContainerImpl {
    pub items: Vec<(u8, crate::item_stack::ItemStack)>,
}
/// Vanilla `ItemContainerContents.equals`: the same stacks in the same slots.
impl PartialEq for ContainerImpl {
    fn eq(&self, other: &Self) -> bool {
        fn filled(contents: &ContainerImpl) -> Vec<&(u8, crate::item_stack::ItemStack)> {
            let mut items = contents
                .items
                .iter()
                .filter(|(_, stack)| !stack.is_empty())
                .collect::<Vec<_>>();
            items.sort_by_key(|(slot, _)| *slot);
            items
        }
        let (mine, theirs) = (filled(self), filled(other));
        mine.len() == theirs.len()
            && mine
                .iter()
                .zip(&theirs)
                .all(|((slot, stack), (other_slot, other_stack))| {
                    slot == other_slot && stack.are_equal(other_stack)
                })
    }
}
impl Eq for ContainerImpl {}
impl std::fmt::Debug for ContainerImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ContainerImpl")
    }
}
impl ContainerImpl {
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        let mut items = Vec::new();
        if let NbtTag::List(l) = tag {
            for item_tag in l {
                if let NbtTag::Compound(c) = item_tag
                    && let Some(slot) = c.get_int("slot")
                    && let Some(item_compound) = c.get_compound("item")
                    && let Some(stack) =
                        crate::item_stack::ItemStack::read_item_stack(item_compound)
                {
                    items.push((slot as u8, stack));
                }
            }
        }
        Some(Self { items })
    }
}
impl DataComponentImpl for ContainerImpl {
    fn write_data(&self) -> NbtTag {
        let mut list = Vec::new();
        for (slot, stack) in &self.items {
            let mut entry = NbtCompound::new();
            entry.put_int("slot", *slot as i32);
            let mut item_compound = NbtCompound::new();
            stack.write_item_stack(&mut item_compound);
            entry.put_compound("item", item_compound);
            list.push(NbtTag::Compound(entry));
        }
        NbtTag::List(list)
    }
    default_impl!(Container);
}

use std::borrow::Cow;

#[derive(Clone, Debug)]
pub struct BlockStateImpl {
    pub properties: Cow<'static, [(Cow<'static, str>, Cow<'static, str>)]>,
}
impl PartialEq for BlockStateImpl {
    fn eq(&self, other: &Self) -> bool {
        let mut self_props = self.properties.to_vec();
        self_props.sort_by(|a, b| a.0.cmp(&b.0));
        let mut other_props = other.properties.to_vec();
        other_props.sort_by(|a, b| a.0.cmp(&b.0));
        self_props == other_props
    }
}
impl Eq for BlockStateImpl {}
impl std::hash::Hash for BlockStateImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let mut props = self.properties.to_vec();
        props.sort_by(|a, b| a.0.cmp(&b.0));
        for (k, v) in props {
            k.hash(state);
            v.hash(state);
        }
    }
}
impl BlockStateImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let mut properties = Vec::new();
        for (key, val) in compound.child_tags.iter() {
            if let Some(s) = val.extract_string() {
                properties.push((Cow::Owned(key.to_string()), Cow::Owned(s.to_string())));
            }
        }
        Some(Self {
            properties: Cow::Owned(properties),
        })
    }
}
impl DataComponentImpl for BlockStateImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        for (k, v) in self.properties.iter() {
            compound.put_string(k.as_ref(), v.to_string());
        }
        NbtTag::Compound(compound)
    }
    fn get_hash(&self) -> i32 {
        let mut digest = Digest::new();
        let mut props = self.properties.to_vec();
        props.sort_by(|a, b| a.0.cmp(&b.0));
        for (k, v) in props {
            digest.update(k.as_ref().as_bytes());
            digest.update(v.as_ref().as_bytes());
        }
        digest.finalize() as i32
    }
    default_impl!(BlockState);
}

/// One bee stored in a hive, `BeehiveBlockEntity.Occupant`.
#[derive(Clone, Debug, PartialEq)]
pub struct BeeOccupant {
    /// The bee's entity NBT, with its type in `id` like `minecraft:entity_data`.
    pub entity_data: NbtCompound,
    pub ticks_in_hive: i32,
    pub min_ticks_in_hive: i32,
}

/// `minecraft:bees`: the bees inside a beehive or bee nest item.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BeesImpl {
    pub bees: Vec<BeeOccupant>,
}
impl BeesImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let bees = data
            .extract_list()?
            .iter()
            .map(|tag| {
                let compound = tag.extract_compound()?;
                let entity_data = compound.get_compound("entity_data")?;
                entity_data.get_string("id")?;
                Some(BeeOccupant {
                    entity_data: entity_data.clone(),
                    ticks_in_hive: compound.get("ticks_in_hive")?.as_numeric_int()?,
                    min_ticks_in_hive: compound.get("min_ticks_in_hive")?.as_numeric_int()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { bees })
    }
}
impl DataComponentImpl for BeesImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::List(
            self.bees
                .iter()
                .map(|bee| {
                    let mut compound = NbtCompound::new();
                    compound.put_compound("entity_data", bee.entity_data.clone());
                    compound.put_int("ticks_in_hive", bee.ticks_in_hive);
                    compound.put_int("min_ticks_in_hive", bee.min_ticks_in_hive);
                    NbtTag::Compound(compound)
                })
                .collect(),
        )
    }
    fn get_hash(&self) -> i32 {
        get_list_hash(self.bees.iter().map(|bee| {
            MapHash::new()
                .field("entity_data", get_compound_hash(&bee.entity_data))
                .field("ticks_in_hive", get_i32_hash(bee.ticks_in_hive))
                .field("min_ticks_in_hive", get_i32_hash(bee.min_ticks_in_hive))
                .finish()
        })) as i32
    }
    default_impl!(Bees);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ContainerLootImpl {
    pub loot_table: String,
    pub seed: i64,
}
impl ContainerLootImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let loot_table = compound.get_string("loot_table")?.to_string();
        let seed = compound.get_long("seed").unwrap_or(0);
        Some(Self { loot_table, seed })
    }
}
impl DataComponentImpl for ContainerLootImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_string("loot_table", self.loot_table.clone());
        compound.put_long("seed", self.seed);
        NbtTag::Compound(compound)
    }
    default_impl!(ContainerLoot);
}

/// `minecraft:sulfur_cube_content`: the block item a sulfur cube absorbed.
#[derive(Clone)]
pub struct SulfurCubeContentImpl {
    pub absorbed_block: crate::item_stack::ItemStack,
}
impl SulfurCubeContentImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        read_item_stack_template(data).map(|absorbed_block| Self { absorbed_block })
    }
}
impl PartialEq for SulfurCubeContentImpl {
    fn eq(&self, other: &Self) -> bool {
        item_stack_template_eq(&self.absorbed_block, &other.absorbed_block)
    }
}
impl std::fmt::Debug for SulfurCubeContentImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SulfurCubeContentImpl")
            .field("absorbed_block", &self.absorbed_block.item.registry_key)
            .finish()
    }
}
impl DataComponentImpl for SulfurCubeContentImpl {
    fn write_data(&self) -> NbtTag {
        write_item_stack_template(&self.absorbed_block)
    }
    fn get_hash(&self) -> i32 {
        get_item_stack_template_hash(&self.absorbed_block) as i32
    }
    default_impl!(SulfurCubeContent);
}

/// The text on one side of a sign item, `SignText`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SignText {
    pub messages: [TextComponent; SignText::LINES],
    /// What players with chat filtering see; `None` when it matches `messages`.
    pub filtered_messages: Option<[TextComponent; SignText::LINES]>,
    pub color: DyeColor,
    pub has_glowing_text: bool,
}

impl SignText {
    pub const LINES: usize = 4;

    fn read_lines(tag: &NbtTag) -> Option<[TextComponent; Self::LINES]> {
        let lines = tag.extract_list()?;
        if lines.len() != Self::LINES {
            return None;
        }
        Some(std::array::from_fn(|i| TextComponent::from_nbt(&lines[i])))
    }

    fn write_lines(lines: &[TextComponent; Self::LINES]) -> NbtTag {
        NbtTag::List(lines.iter().map(text_to_nbt).collect())
    }

    fn lines_hash(lines: &[TextComponent; Self::LINES]) -> u32 {
        get_list_hash(lines.iter().map(get_text_hash))
    }

    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let messages = Self::read_lines(compound.get("messages")?)?;
        // `lenientOptionalFieldOf`: a malformed value counts as absent.
        let filtered_messages = compound
            .get("filtered_messages")
            .and_then(Self::read_lines)
            .filter(|filtered| *filtered != messages);
        let color = match compound.get("color") {
            Some(tag) => DyeColor::by_name(tag.extract_string()?)?,
            None => DyeColor::Black,
        };
        let has_glowing_text = match compound.get("has_glowing_text") {
            Some(tag) => tag.as_numeric_int()? != 0,
            None => false,
        };
        Some(Self {
            messages,
            filtered_messages,
            color,
            has_glowing_text,
        })
    }

    pub fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put("messages", Self::write_lines(&self.messages));
        if let Some(filtered) = &self.filtered_messages {
            compound.put("filtered_messages", Self::write_lines(filtered));
        }
        compound.put_string("color", self.color.name().to_string());
        compound.put_bool("has_glowing_text", self.has_glowing_text);
        NbtTag::Compound(compound)
    }

    pub fn get_hash(&self) -> u32 {
        MapHash::new()
            .field("messages", Self::lines_hash(&self.messages))
            .optional_field(
                "filtered_messages",
                self.filtered_messages.as_ref().map(Self::lines_hash),
            )
            .field("color", get_str_hash(self.color.name()))
            .field("has_glowing_text", get_bool_hash(self.has_glowing_text))
            .finish()
    }
}

macro_rules! sign_text_component {
    ($name:ident, $component:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name(pub SignText);

        impl $name {
            pub fn read_data(data: &NbtTag) -> Option<Self> {
                SignText::read_data(data).map(Self)
            }
        }

        impl DataComponentImpl for $name {
            fn write_data(&self) -> NbtTag {
                self.0.write_data()
            }
            fn get_hash(&self) -> i32 {
                self.0.get_hash() as i32
            }
            default_impl!($component);
        }
    };
}

sign_text_component!(SignTextFrontImpl, SignTextFront);
sign_text_component!(SignTextBackImpl, SignTextBack);

/// `minecraft:cushion/color`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CushionColorImpl {
    pub color: DyeColor,
}
impl CushionColorImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        Some(Self {
            color: DyeColor::by_name(data.extract_string()?)?,
        })
    }
}
impl DataComponentImpl for CushionColorImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(self.color.name().into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(self.color.name()) as i32
    }
    default_impl!(CushionColor);
}

/// `minecraft:provides_pottery_pattern`: the pattern a pottery sherd puts on a pot.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProvidesPotteryPatternImpl {
    pub pattern: DecoratedPotPattern,
}
impl ProvidesPotteryPatternImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        Some(Self {
            pattern: DecoratedPotPattern::from_name(data.extract_string()?)?,
        })
    }
}
impl DataComponentImpl for ProvidesPotteryPatternImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::String(format!("minecraft:{}", self.pattern.to_name()).into())
    }
    fn get_hash(&self) -> i32 {
        get_str_hash(&format!("minecraft:{}", self.pattern.to_name())) as i32
    }
    default_impl!(ProvidesPotteryPattern);
}
