use std::collections::BTreeSet;
use std::sync::RwLock;

use pumpkin_data::item::Item;
use pumpkin_data::recipe_sync::{PROPERTY_SET_INGREDIENTS, STONECUTTER_ENTRIES, SyncIngredient};
use pumpkin_data::tag::{RegistryKey, get_tag_ids};
use pumpkin_protocol::codec::recipe::{OwnedCookingRecipeType, OwnedRecipeIngredient};
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::ser::{NetworkWriteExt, WritingError};

use pumpkin_inventory::crafting::recipe_provider::RecipeProvider;
pub use pumpkin_protocol::codec::recipe::DynamicRecipe;

pub struct RecipeManager {
    dynamic_recipes: RwLock<Vec<DynamicRecipe>>,
}

impl Default for RecipeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RecipeManager {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            dynamic_recipes: RwLock::new(Vec::new()),
        }
    }

    pub fn add_recipe(&self, recipe: DynamicRecipe) {
        let mut recipes = self
            .dynamic_recipes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        recipes.push(recipe);
    }

    pub fn add_recipes(&self, new_recipes: impl IntoIterator<Item = DynamicRecipe>) {
        let mut recipes = self
            .dynamic_recipes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        recipes.extend(new_recipes);
    }

    pub fn set_recipes(&self, new_recipes: Vec<DynamicRecipe>) {
        let mut recipes = self
            .dynamic_recipes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *recipes = new_recipes;
    }

    pub fn clear(&self) {
        let mut recipes = self
            .dynamic_recipes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        recipes.clear();
    }

    pub fn get_dynamic_recipes_internal(&self) -> Vec<DynamicRecipe> {
        self.dynamic_recipes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The body of vanilla `ClientboundUpdateRecipesPacket` with this manager's
    /// `getSynchronizedItemProperties` and `getSynchronizedStonecutterRecipes`: the items each
    /// property set accepts (the vanilla recipes', plus datapack cooking and brewing recipes'),
    /// then the stonecutter entries.
    #[must_use]
    pub fn update_recipes_payload(&self) -> Vec<u8> {
        let mut sets: Vec<(&str, BTreeSet<u16>)> = PROPERTY_SET_INGREDIENTS
            .iter()
            .map(|(key, ingredients)| {
                let mut items = BTreeSet::new();
                for ingredient in *ingredients {
                    items.extend(sync_ingredient_items(*ingredient));
                }
                (*key, items)
            })
            .collect();
        let mut add = |key: &str, ingredient: &OwnedRecipeIngredient| {
            if let Some((_, items)) = sets.iter_mut().find(|(k, _)| *k == key) {
                items.extend(owned_ingredient_items(ingredient));
            }
        };
        for recipe in self.get_dynamic_recipes_internal() {
            match recipe {
                DynamicRecipe::Cooking(cooking) => {
                    let (key, recipe) = match &cooking {
                        OwnedCookingRecipeType::Smelting(r) => ("minecraft:furnace_input", r),
                        OwnedCookingRecipeType::Blasting(r) => ("minecraft:blast_furnace_input", r),
                        OwnedCookingRecipeType::Smoking(r) => ("minecraft:smoker_input", r),
                        OwnedCookingRecipeType::CampfireCooking(r) => {
                            ("minecraft:campfire_input", r)
                        }
                    };
                    add(key, &recipe.ingredient);
                }
                DynamicRecipe::Brewing(brewing) => {
                    add(
                        "minecraft:brewing_input",
                        &OwnedRecipeIngredient::Simple(brewing.input_item),
                    );
                    add(
                        "minecraft:brewing_reagent",
                        &OwnedRecipeIngredient::Simple(brewing.reagent),
                    );
                }
                DynamicRecipe::Crafting(_) => {}
            }
        }

        let mut out = Vec::new();
        // Writing to a `Vec` can't fail.
        let _ = write_payload(&mut out, &sets);
        out
    }
}

fn write_payload(out: &mut Vec<u8>, sets: &[(&str, BTreeSet<u16>)]) -> Result<(), WritingError> {
    let len = |n: usize| VarInt(n as i32);
    out.write_var_int(&len(sets.len()))?;
    for (key, items) in sets {
        // `RecipePropertySet.STREAM_CODEC`: the key, then the items as a list.
        out.write_string(key)?;
        out.write_var_int(&len(items.len()))?;
        for item in items {
            out.write_var_int(&VarInt(i32::from(*item)))?;
        }
    }
    // `SelectableRecipe.SingleInputSet.noRecipeCodec`.
    out.write_var_int(&len(STONECUTTER_ENTRIES.len()))?;
    for entry in STONECUTTER_ENTRIES {
        // `Ingredient.CONTENTS_STREAM_CODEC`, a `ByteBufCodecs.holderSet`: 0 and the tag, or
        // the size plus one and the items.
        match entry.input {
            SyncIngredient::Tag(tag) => {
                out.write_var_int(&VarInt(0))?;
                out.write_string(tag)?;
            }
            SyncIngredient::Items(items) => {
                out.write_var_int(&len(items.len() + 1))?;
                for item in items {
                    out.write_var_int(&VarInt(i32::from(*item)))?;
                }
            }
        }
        // `SlotDisplay.ItemStackSlotDisplay`: its type, then `ItemStackTemplate` (item, count
        // and an empty component patch).
        out.write_var_int(&VarInt(SLOT_DISPLAY_ITEM_STACK))?;
        out.write_var_int(&VarInt(i32::from(entry.result)))?;
        out.write_var_int(&VarInt(entry.count))?;
        out.write_var_int(&VarInt(0))?;
        out.write_var_int(&VarInt(0))?;
    }
    Ok(())
}

/// `item_stack`'s id in vanilla's `slot_display` registry (`SlotDisplays.bootstrap`).
const SLOT_DISPLAY_ITEM_STACK: i32 = 5;

/// `Ingredient.items` for a generated ingredient.
fn sync_ingredient_items(ingredient: SyncIngredient) -> Vec<u16> {
    match ingredient {
        SyncIngredient::Items(items) => items.to_vec(),
        SyncIngredient::Tag(tag) => get_tag_ids(RegistryKey::Item, tag)
            .map(<[u16]>::to_vec)
            .unwrap_or_default(),
    }
}

/// `Ingredient.items` for a datapack recipe's ingredient.
fn owned_ingredient_items(ingredient: &OwnedRecipeIngredient) -> Vec<u16> {
    let item = |name: &str| {
        Item::from_registry_key(name.strip_prefix("minecraft:").unwrap_or(name)).map(|i| i.id)
    };
    match ingredient {
        OwnedRecipeIngredient::Simple(name) => item(name).into_iter().collect(),
        OwnedRecipeIngredient::OneOf(names) => names.iter().filter_map(|n| item(n)).collect(),
        OwnedRecipeIngredient::Tagged(tag) => {
            get_tag_ids(RegistryKey::Item, tag.strip_prefix('#').unwrap_or(tag))
                .map(<[u16]>::to_vec)
                .unwrap_or_default()
        }
    }
}

impl RecipeProvider for RecipeManager {
    fn get_dynamic_recipes(&self) -> Vec<DynamicRecipe> {
        self.dynamic_recipes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}
