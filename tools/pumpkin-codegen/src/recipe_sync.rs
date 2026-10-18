use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

const RECIPE_DIR: &str = "../../assets/datapack/data/minecraft/recipe";

/// Vanilla `RecipeManager.RECIPE_PROPERTY_SETS`, in declaration order: the property set and
/// the recipe fields its extractor takes an ingredient from.
const PROPERTY_SETS: &[(&str, &[(&str, &str)])] = &[
    (
        "minecraft:smithing_addition",
        &[
            ("minecraft:smithing_transform", "addition"),
            ("minecraft:smithing_trim", "addition"),
        ],
    ),
    (
        "minecraft:smithing_base",
        &[
            ("minecraft:smithing_transform", "base"),
            ("minecraft:smithing_trim", "base"),
        ],
    ),
    (
        "minecraft:smithing_template",
        &[
            ("minecraft:smithing_transform", "template"),
            ("minecraft:smithing_trim", "template"),
        ],
    ),
    (
        "minecraft:furnace_input",
        &[("minecraft:smelting", "ingredient")],
    ),
    (
        "minecraft:blast_furnace_input",
        &[("minecraft:blasting", "ingredient")],
    ),
    (
        "minecraft:smoker_input",
        &[("minecraft:smoking", "ingredient")],
    ),
    (
        "minecraft:campfire_input",
        &[("minecraft:campfire_cooking", "ingredient")],
    ),
    (
        "minecraft:brewing_input",
        &[("minecraft:brewing", "input.item")],
    ),
    (
        "minecraft:brewing_reagent",
        &[("minecraft:brewing", "reagent.item")],
    ),
];

fn read_recipes(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Value>) {
    for entry in fs::read_dir(dir).expect("vanilla datapack recipes missing") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        if path.is_dir() {
            read_recipes(&path, &format!("{prefix}{name}/"), out);
        } else if let Some(stem) = name.strip_suffix(".json") {
            let recipe = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            out.insert(format!("minecraft:{prefix}{stem}"), recipe);
        }
    }
}

fn field<'a>(recipe: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(recipe, |v, key| v.get(key))
}

/// A vanilla `Ingredient` (an item, a list of items or `#tag`) as a `SyncIngredient`.
fn ingredient(value: &Value, item_ids: &BTreeMap<String, u16>) -> TokenStream {
    let id = |name: &str| {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        *item_ids
            .get(name)
            .unwrap_or_else(|| panic!("unknown item {name}"))
    };
    match value {
        Value::String(s) if s.starts_with('#') => {
            let tag = &s[1..];
            quote! { SyncIngredient::Tag(#tag) }
        }
        Value::String(s) => {
            let item = id(s);
            quote! { SyncIngredient::Items(&[#item]) }
        }
        Value::Array(items) => {
            let items = items
                .iter()
                .map(|i| id(i.as_str().expect("ingredient list entry")));
            quote! { SyncIngredient::Items(&[#(#items),*]) }
        }
        other => panic!("unhandled ingredient {other}"),
    }
}

/// Generates what vanilla's `ClientboundUpdateRecipesPacket` carries for the vanilla recipes:
/// the ingredients behind each synchronized `RecipePropertySet` and the stonecutter entries
/// of `RecipeManager.finalizeRecipeLoading`, in the recipe registry's (sorted id) order.
pub fn build() -> TokenStream {
    let items: BTreeMap<String, Value> =
        serde_json::from_str(&fs::read_to_string("../../assets/items.json").unwrap()).unwrap();
    let item_ids: BTreeMap<String, u16> = items
        .iter()
        .map(|(name, item)| (name.clone(), item["id"].as_u64().unwrap() as u16))
        .collect();
    let mut recipes = BTreeMap::new();
    read_recipes(Path::new(RECIPE_DIR), "", &mut recipes);

    let property_sets = PROPERTY_SETS.iter().map(|(key, sources)| {
        let ingredients = recipes.values().filter_map(|recipe| {
            let kind = recipe["type"].as_str()?;
            let (_, path) = sources.iter().find(|(t, _)| *t == kind)?;
            field(recipe, path).map(|v| ingredient(v, &item_ids))
        });
        quote! { (#key, &[#(#ingredients),*]) }
    });

    let stonecutter = recipes
        .values()
        .filter(|recipe| recipe["type"] == "minecraft:stonecutting")
        .map(|recipe| {
            let input = ingredient(&recipe["ingredient"], &item_ids);
            let result = &recipe["result"];
            assert!(
                result.get("components").is_none(),
                "stonecutter result components aren't synced yet"
            );
            let name = result["id"].as_str().unwrap();
            let item = item_ids[name.strip_prefix("minecraft:").unwrap_or(name)];
            let count = result.get("count").and_then(Value::as_i64).unwrap_or(1) as i32;
            quote! { StonecutterEntry { input: #input, result: #item, count: #count } }
        });

    quote! {
        /// A vanilla `Ingredient` on the wire: a direct item list or a tag.
        #[derive(Clone, Copy, Debug)]
        pub enum SyncIngredient {
            Items(&'static [u16]),
            Tag(&'static str),
        }

        /// A stonecutter `SingleInputEntry`: its input and the `item_stack` slot display of
        /// its result.
        #[derive(Clone, Copy, Debug)]
        pub struct StonecutterEntry {
            pub input: SyncIngredient,
            pub result: u16,
            pub count: i32,
        }

        /// The ingredients behind each synchronized `RecipePropertySet`.
        pub static PROPERTY_SET_INGREDIENTS: &[(&str, &[SyncIngredient])] = &[#(#property_sets),*];

        /// `RecipeManager.stonecutterRecipes`, in recipe order.
        pub static STONECUTTER_ENTRIES: &[StonecutterEntry] = &[#(#stonecutter),*];
    }
}
