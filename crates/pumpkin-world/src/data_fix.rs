//! The 26.3 data fixes that depend on the version a save was written at.
//!
//! A reader can't tell these old forms from valid 26.3 data, so they are applied here, keyed on
//! the `DataVersion` of the file being read. Fixes whose old form can't occur in 26.3 data are
//! handled by the readers themselves.

use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

/// Schema version of vanilla's `SignsOpFlagFix`.
const SIGNS_OP_FLAG_FIX_VERSION: i32 = 5002;
/// Schema version of vanilla's `ExplorerMapItemFix`; the item renames at 5012 are folded in.
const EXPLORER_MAP_ITEM_FIX_VERSION: i32 = 5008;
/// Schema version of vanilla's `UnfilledBuriedTreasureMapFix`.
const UNFILLED_BURIED_TREASURE_MAP_FIX_VERSION: i32 = 5016;

/// `ExplorerMapItemFix`'s decoration types, mapped to the item the 5012 rename settles on and
/// the item name the map was given before it had its own item.
const EXPLORER_MAPS: [(&str, &str, &str); 11] = [
    (
        "minecraft:mansion",
        "minecraft:woodland_mansion_map",
        "filled_map.mansion",
    ),
    (
        "minecraft:monument",
        "minecraft:ocean_monument_map",
        "filled_map.monument",
    ),
    (
        "minecraft:trial_chambers",
        "minecraft:buried_trial_chambers_map",
        "filled_map.trial_chambers",
    ),
    (
        "minecraft:jungle_temple",
        "minecraft:jungle_pyramid_map",
        "filled_map.explorer_jungle",
    ),
    (
        "minecraft:swamp_hut",
        "minecraft:swamp_hut_map",
        "filled_map.explorer_swamp",
    ),
    (
        "minecraft:village_desert",
        "minecraft:desert_village_map",
        "filled_map.village_desert",
    ),
    (
        "minecraft:village_plains",
        "minecraft:plains_village_map",
        "filled_map.village_plains",
    ),
    (
        "minecraft:village_savanna",
        "minecraft:savanna_village_map",
        "filled_map.village_savanna",
    ),
    (
        "minecraft:village_snowy",
        "minecraft:snowy_village_map",
        "filled_map.village_snowy",
    ),
    (
        "minecraft:village_taiga",
        "minecraft:taiga_village_map",
        "filled_map.village_taiga",
    ),
    (
        "minecraft:red_x",
        "minecraft:buried_treasure_map",
        "filled_map.buried_treasure",
    ),
];

/// The `DataVersion` of a save, or what vanilla assumes when it is missing: older than any fix.
#[must_use]
pub fn data_version(nbt: &NbtCompound) -> i32 {
    nbt.get_int("DataVersion").unwrap_or(-1)
}

/// Upgrades a chunk's block entity saved at `data_version`.
pub fn upgrade_block_entity(nbt: &mut NbtCompound, data_version: i32) {
    if data_version < SIGNS_OP_FLAG_FIX_VERSION
        && matches!(
            nbt.get_string("id"),
            Some("minecraft:sign" | "minecraft:hanging_sign")
        )
    {
        nbt.put_bool("allow_op_features", true);
    }
    upgrade_item_stacks(nbt, data_version);
}

/// Upgrades every item stack inside `nbt`, an entity, block entity or player saved at
/// `data_version`.
pub fn upgrade_item_stacks(nbt: &mut NbtCompound, data_version: i32) {
    if data_version >= UNFILLED_BURIED_TREASURE_MAP_FIX_VERSION {
        return;
    }
    upgrade_compound(nbt, data_version);
}

fn upgrade_compound(nbt: &mut NbtCompound, data_version: i32) {
    if data_version < EXPLORER_MAP_ITEM_FIX_VERSION {
        explorer_map_item_fix(nbt);
    }
    unfilled_buried_treasure_map_fix(nbt);
    for tag in nbt.child_tags.values_mut() {
        upgrade_tag(tag, data_version);
    }
}

fn upgrade_tag(tag: &mut NbtTag, data_version: i32) {
    match tag {
        NbtTag::Compound(compound) => upgrade_compound(compound, data_version),
        NbtTag::List(list) => {
            for tag in list {
                upgrade_tag(tag, data_version);
            }
        }
        _ => {}
    }
}

fn is_item(nbt: &NbtCompound, id: &str) -> bool {
    nbt.get_string("id")
        .is_some_and(|item| item.strip_prefix("minecraft:").unwrap_or(item) == id)
}

/// Vanilla `ExtraDataFixUtils.getPlainTranslationKey`.
fn plain_translation_key(text: &NbtTag) -> Option<&str> {
    let NbtTag::Compound(text) = text else {
        return None;
    };
    if text.child_tags.len() != 1 {
        return None;
    }
    text.get_string("translate")
}

/// Vanilla `ExplorerMapItemFix`, with the item renames of schema 5012.
fn explorer_map_item_fix(item: &mut NbtCompound) {
    if !is_item(item, "filled_map") {
        return;
    }
    let Some(components) = item.get_compound("components") else {
        return;
    };
    let Some(decoration_type) = components
        .get_compound("minecraft:map_decorations")
        .and_then(|decorations| decorations.get_compound("+"))
        .and_then(|decoration| decoration.get_string("type"))
    else {
        return;
    };
    let decoration_type = if decoration_type.contains(':') {
        decoration_type.to_string()
    } else {
        format!("minecraft:{decoration_type}")
    };
    let Some((_, new_id, legacy_name)) = EXPLORER_MAPS
        .iter()
        .find(|(decoration, _, _)| *decoration == decoration_type)
    else {
        return;
    };
    item.put_string("id", (*new_id).to_string());
    if let Some(NbtTag::Compound(components)) = item.child_tags.get_mut("components")
        && components
            .get("minecraft:item_name")
            .and_then(plain_translation_key)
            == Some(*legacy_name)
    {
        components.child_tags.remove("minecraft:item_name");
    }
}

/// Vanilla `UnfilledBuriedTreasureMapFix`.
fn unfilled_buried_treasure_map_fix(item: &mut NbtCompound) {
    if !is_item(item, "map") {
        return;
    }
    if let Some(NbtTag::Compound(components)) = item.child_tags.get_mut("components")
        && let Some(item_name) = components.child_tags.get_mut("minecraft:item_name")
        && plain_translation_key(item_name) == Some("filled_map.buried_treasure")
        && let NbtTag::Compound(item_name) = item_name
    {
        item_name.put_string(
            "translate",
            "item.minecraft.buried_treasure_map".to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn explorer_map(decoration_type: &str, item_name: &str) -> NbtCompound {
        let mut decoration = NbtCompound::new();
        decoration.put_string("type", decoration_type.to_string());
        let mut decorations = NbtCompound::new();
        decorations.put_compound("+", decoration);
        let mut name = NbtCompound::new();
        name.put_string("translate", item_name.to_string());
        let mut components = NbtCompound::new();
        components.put_compound("minecraft:map_decorations", decorations);
        components.put_compound("minecraft:item_name", name);
        let mut item = NbtCompound::new();
        item.put_string("id", "minecraft:filled_map".to_string());
        item.put_int("count", 1);
        item.put_compound("components", components);
        item
    }

    fn chest_with(item: NbtCompound) -> NbtCompound {
        let mut chest = NbtCompound::new();
        chest.put_string("id", "minecraft:chest".to_string());
        chest.put_list("Items", vec![NbtTag::Compound(item)]);
        chest
    }

    fn first_item(chest: &NbtCompound) -> &NbtCompound {
        match chest.get_list("Items").and_then(|items| items.first()) {
            Some(NbtTag::Compound(item)) => item,
            _ => panic!("chest lost its item"),
        }
    }

    #[test]
    fn explorer_maps_get_their_26_3_item() {
        let mut chest = chest_with(explorer_map("mansion", "filled_map.mansion"));
        upgrade_block_entity(&mut chest, 4903);
        let item = first_item(&chest);
        assert_eq!(
            item.get_string("id"),
            Some("minecraft:woodland_mansion_map")
        );
        let components = item.get_compound("components").expect("components");
        assert!(!components.has("minecraft:item_name"));
        assert!(components.has("minecraft:map_decorations"));

        // A renamed map keeps its name, and 26.3 saves are left alone.
        let mut chest = chest_with(explorer_map("minecraft:monument", "Ocean"));
        upgrade_block_entity(&mut chest, 4903);
        let components = first_item(&chest)
            .get_compound("components")
            .expect("components");
        assert!(components.has("minecraft:item_name"));
        let mut chest = chest_with(explorer_map("mansion", "filled_map.mansion"));
        upgrade_block_entity(&mut chest, 5023);
        assert_eq!(
            first_item(&chest).get_string("id"),
            Some("minecraft:filled_map")
        );
    }

    #[test]
    fn unfilled_buried_treasure_map_gets_its_26_3_name() {
        let mut map = explorer_map("red_x", "filled_map.buried_treasure");
        map.put_string("id", "minecraft:map".to_string());
        let mut chest = chest_with(map);
        upgrade_block_entity(&mut chest, 5008);
        let name = first_item(&chest)
            .get_compound("components")
            .and_then(|components| components.get_compound("minecraft:item_name"))
            .and_then(|name| name.get_string("translate"));
        assert_eq!(name, Some("item.minecraft.buried_treasure_map"));
    }

    #[test]
    fn signs_from_before_the_op_flag_keep_their_click_events() {
        let mut sign = NbtCompound::new();
        sign.put_string("id", "minecraft:sign".to_string());
        upgrade_block_entity(&mut sign, 4903);
        assert_eq!(sign.get_bool("allow_op_features"), Some(true));
        let mut sign = NbtCompound::new();
        sign.put_string("id", "minecraft:hanging_sign".to_string());
        upgrade_block_entity(&mut sign, 5002);
        assert_eq!(sign.get_bool("allow_op_features"), None);
    }
}
