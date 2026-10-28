//! The loot functions the evaluator reads from their JSON (`LootFunctionKind::Json`).

use std::borrow::Cow;

use pumpkin_data::{
    Enchantment,
    data_component::DataComponent,
    data_component_impl::{
        CustomNameImpl, EnchantmentsImpl, Holder, InstrumentImpl, ItemNameImpl,
        OminousBottleAmplifierImpl, PotionContentsImpl, StoredEnchantmentsImpl,
        SuspiciousStewEffect, SuspiciousStewEffectsImpl, read_data,
    },
    effect::StatusEffect,
    instrument::Instrument,
    item::Item,
    item_stack::ItemStack,
    tag::{RegistryKey, get_tag_values},
};
use pumpkin_util::{
    loot_table::{LootNumberProvider, parser::provider},
    text::TextComponent,
};
use serde_json::Value;

use super::{Generator, RandomImpl, Stack};
use crate::enchantment::helper::EnchantmentHelper;
use crate::world::loot::predicates::{item_matches, json_to_nbt};

fn kind(function: &Value) -> &str {
    function
        .get("type")
        .or_else(|| function.get("function"))
        .and_then(Value::as_str)
        .map_or("", |name| name.trim_start_matches("minecraft:"))
}

fn number(function: &Value, field: &str) -> Option<LootNumberProvider> {
    provider(function.get(field)?)
}

/// A registry `HolderSet` in its JSON forms (`#tag`, one id, or a list), as names in order.
fn holder_names(value: &Value, registry: RegistryKey) -> Vec<&str> {
    match value {
        Value::String(name) => name.strip_prefix('#').map_or_else(
            || vec![name.as_str()],
            |tag| get_tag_values(registry, tag).unwrap_or_default().to_vec(),
        ),
        Value::Array(names) => names.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// The enchantments `options` names, or the whole registry in its order when it is absent.
fn enchantment_options(options: Option<&Value>) -> Vec<&'static Enchantment> {
    options.map_or_else(
        || Enchantment::all().copied().collect(),
        |options| {
            holder_names(options, RegistryKey::Enchantment)
                .into_iter()
                .filter_map(Enchantment::from_name)
                .collect()
        },
    )
}

/// Vanilla `EnchantmentHelper.updateEnchantments`: an enchanted book keeps its enchantments in
/// `stored_enchantments`.
fn update_enchantments(
    stack: &mut ItemStack,
    update: impl FnOnce(&mut Vec<(&'static Enchantment, i32)>),
) {
    let stored = stack.item == &Item::ENCHANTED_BOOK;
    let mut enchantments = if stored {
        stack
            .get_data_component::<StoredEnchantmentsImpl>()
            .map(|component| component.enchantment.to_vec())
    } else {
        stack
            .get_data_component::<EnchantmentsImpl>()
            .map(|component| component.enchantment.to_vec())
    }
    .unwrap_or_default();
    update(&mut enchantments);
    let id = if stored {
        DataComponent::StoredEnchantments
    } else {
        DataComponent::Enchantments
    };
    if enchantments.is_empty() {
        // The prototype's own (empty) value: no patch entry.
        stack.patch.retain(|(patched, _)| *patched != id);
    } else if stored {
        stack.set_data_component(StoredEnchantmentsImpl {
            enchantment: Cow::Owned(enchantments),
        });
    } else {
        stack.set_data_component(EnchantmentsImpl {
            enchantment: Cow::Owned(enchantments),
        });
    }
}

/// Vanilla `ItemEnchantments.Mutable.upgrade`.
fn upgrade(
    enchantments: &mut Vec<(&'static Enchantment, i32)>,
    enchantment: &'static Enchantment,
    level: i32,
) {
    if level <= 0 {
        return;
    }
    let level = level.min(255);
    if let Some((_, old)) = enchantments.iter_mut().find(|(e, _)| *e == enchantment) {
        *old = (*old).max(level);
    } else {
        enchantments.push((enchantment, level));
    }
}

/// Vanilla `ItemEnchantments.Mutable.set`.
fn set_level(
    enchantments: &mut Vec<(&'static Enchantment, i32)>,
    enchantment: &'static Enchantment,
    level: i32,
) {
    if level <= 0 {
        enchantments.retain(|(e, _)| *e != enchantment);
    } else if let Some((_, old)) = enchantments.iter_mut().find(|(e, _)| *e == enchantment) {
        *old = level.min(255);
    } else {
        enchantments.push((enchantment, level.min(255)));
    }
}

impl<R: RandomImpl> Generator<'_, '_, R> {
    /// Runs one JSON function on `stack`. Its `conditions` were tested by the caller.
    pub(super) fn run_json(&mut self, stack: &mut Stack, function: &Value) {
        match kind(function) {
            "enchant_randomly" => self.enchant_randomly(stack, function),
            "enchant_with_levels" => self.enchant_with_levels(stack, function),
            "set_enchantments" => self.set_enchantments(stack, function),
            "set_damage" => self.set_damage(stack, function),
            "set_potion" => set_potion(stack, function),
            "set_stew_effect" => self.set_stew_effect(stack, function),
            "set_ominous_bottle_amplifier" => {
                // Vanilla `SetOminousBottleAmplifierFunction.run`.
                if let Some(amplifier) = number(function, "amplifier") {
                    let amplifier = self.number_int(amplifier).clamp(0, 4);
                    stack
                        .stack
                        .set_data_component(OminousBottleAmplifierImpl { amplifier });
                }
            }
            "set_instrument" => self.set_instrument(stack, function),
            "set_name" => set_name(stack, function),
            "set_components" => set_components(stack, function),
            "copy_state" => self.copy_state(stack, function),
            "filtered" => self.filtered(stack, function),
            // Vanilla `DiscardItem.run`: `ItemStack.EMPTY`.
            "discard" => {
                stack.stack = ItemStack::EMPTY.clone();
                stack.count = 0;
            }
            // TODO: `ExplorationMapFunction` needs `findNearestMapStructure` and map rendering.
            // Until then the blank map stays, as when vanilla finds no structure; the `filtered`
            // after it in the vanilla tables then discards it.
            "exploration_map" => {}
            other => tracing::warn!("Unsupported loot function {other}"),
        }
    }

    /// Vanilla `EnchantRandomlyFunction.run`.
    fn enchant_randomly(&mut self, stack: &mut Stack, function: &Value) {
        let is_book = stack.stack.item == &Item::BOOK;
        let only_compatible = function
            .get("only_compatible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let check_compatibility = !is_book && only_compatible;
        let candidates: Vec<_> = enchantment_options(function.get("options"))
            .into_iter()
            .filter(|candidate| !check_compatibility || candidate.can_enchant(stack.stack.item))
            .collect();
        // `Util.getRandomSafe`.
        if candidates.is_empty() {
            tracing::warn!(
                "Couldn't find a compatible enchantment for {}",
                stack.stack.item.registry_key
            );
            return;
        }
        let enchantment = candidates[self.rng.next_bounded_i32(candidates.len() as i32) as usize];
        let level = self.rng.next_inbetween_i32(1, enchantment.max_level);
        if is_book {
            *stack = Stack::new(&Item::ENCHANTED_BOOK);
        }
        update_enchantments(&mut stack.stack, |enchantments| {
            upgrade(enchantments, enchantment, level);
        });
    }

    /// Vanilla `EnchantWithLevelsFunction.run` and `EnchantmentHelper.enchantItem`.
    fn enchant_with_levels(&mut self, stack: &mut Stack, function: &Value) {
        let Some(levels) = number(function, "levels") else {
            return;
        };
        let cost = self.number_int(levels);
        let source = enchantment_options(function.get("options"));
        let enchantments = if stack.is_empty() {
            Vec::new()
        } else {
            EnchantmentHelper::select_enchantment(self.rng, &stack.stack, cost, &source)
        };
        if stack.stack.item == &Item::BOOK {
            *stack = Stack::new(&Item::ENCHANTED_BOOK);
        }
        update_enchantments(&mut stack.stack, |current| {
            for (enchantment, level) in enchantments {
                upgrade(current, enchantment, level);
            }
        });
    }

    /// Vanilla `SetEnchantmentsFunction.run`: a book becomes an enchanted book, keeping its count
    /// and components (`transmuteCopy`).
    fn set_enchantments(&mut self, stack: &mut Stack, function: &Value) {
        if stack.stack.item == &Item::BOOK {
            let mut book = ItemStack::new(1, &Item::ENCHANTED_BOOK);
            book.patch = std::mem::take(&mut stack.stack.patch);
            stack.stack = book;
        }
        let add = function
            .get("add")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut levels = Vec::new();
        if let Some(enchantments) = function.get("enchantments").and_then(Value::as_object) {
            for (name, level) in enchantments {
                let (Some(enchantment), Some(level)) =
                    (Enchantment::from_name(name), provider(level))
                else {
                    continue;
                };
                levels.push((enchantment, self.number_int(level)));
            }
        }
        update_enchantments(&mut stack.stack, |current| {
            for (enchantment, level) in levels {
                let old = current
                    .iter()
                    .find(|(e, _)| *e == enchantment)
                    .map_or(0, |(_, level)| *level);
                let level = if add { old + level } else { level };
                set_level(current, enchantment, level.clamp(0, 255));
            }
        });
    }

    /// Vanilla `SetItemDamageFunction.run`.
    fn set_damage(&mut self, stack: &mut Stack, function: &Value) {
        let Some(damage) = number(function, "damage") else {
            return;
        };
        // `ItemStack.isDamageableItem`.
        let damageable = stack.stack.is_damageable()
            && !stack.stack.is_unbreakable()
            && stack.stack.has_data_component(DataComponent::Damage);
        if !damageable {
            tracing::warn!(
                "Couldn't set damage of loot item {}",
                stack.stack.item.registry_key
            );
            return;
        }
        let max_damage = stack.stack.get_max_damage().unwrap_or(0);
        let add = function
            .get("add")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let base = if add {
            1.0 - stack.stack.get_damage() as f32 / max_damage as f32
        } else {
            0.0
        };
        let pct = 1.0 - (self.number_float(damage) + base).clamp(0.0, 1.0);
        stack
            .stack
            .set_damage((pct * max_damage as f32).floor() as i32);
    }

    /// Vanilla `SetStewEffectFunction.run`.
    fn set_stew_effect(&mut self, stack: &mut Stack, function: &Value) {
        let effects = function
            .get("effects")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        if stack.stack.item != &Item::SUSPICIOUS_STEW || effects.is_empty() {
            return;
        }
        // `Util.getRandom`.
        let entry = &effects[self.rng.next_bounded_i32(effects.len() as i32) as usize];
        let Some(name) = entry.get("type").and_then(Value::as_str) else {
            return;
        };
        let mut duration =
            number(entry, "duration").map_or(0, |duration| self.number_int(duration));
        let effect = StatusEffect::from_name(name.trim_start_matches("minecraft:"));
        if !effect.is_some_and(crate::entity::effect::is_instantaneous) {
            duration *= 20;
        }
        let mut effects = stack
            .stack
            .get_data_component::<SuspiciousStewEffectsImpl>()
            .map_or_else(Vec::new, |component| component.effects.to_vec());
        let name = if name.contains(':') {
            name.to_owned()
        } else {
            format!("minecraft:{name}")
        };
        effects.push(SuspiciousStewEffect {
            effect: Cow::Owned(name),
            duration,
        });
        stack.stack.set_data_component(SuspiciousStewEffectsImpl {
            effects: Cow::Owned(effects),
        });
    }

    /// Vanilla `SetInstrumentFunction.run`: `HolderSet.getRandomElement`.
    fn set_instrument(&mut self, stack: &mut Stack, function: &Value) {
        let options: Vec<_> = function
            .get("options")
            .map(|options| holder_names(options, RegistryKey::Instrument))
            .unwrap_or_default()
            .into_iter()
            .filter_map(Instrument::from_name)
            .collect();
        if options.is_empty() {
            return;
        }
        let instrument = options[self.rng.next_bounded_i32(options.len() as i32) as usize];
        stack.stack.set_data_component(InstrumentImpl {
            instrument: Holder::Reference(instrument),
        });
    }

    /// Vanilla `CopyBlockState.run`: the listed properties the broken state has.
    fn copy_state(&self, stack: &mut Stack, function: &Value) {
        let Some(state) = self.params.block_state else {
            return;
        };
        let names: Vec<&str> = function
            .get("properties")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let block = pumpkin_data::Block::from_state_id(state.id);
        let Some(properties) = block.properties(state.id) else {
            return;
        };
        let mut copied = stack
            .stack
            .get_data_component::<pumpkin_data::data_component_impl::BlockStateImpl>()
            .map_or_else(Vec::new, |component| component.properties.to_vec());
        for property in properties.to_props() {
            let key = property.key_str();
            if !names.contains(&key) {
                continue;
            }
            let value = Cow::Borrowed(property.value_str());
            if let Some((_, old)) = copied.iter_mut().find(|(name, _)| name == key) {
                *old = value;
            } else {
                copied.push((Cow::Borrowed(key), value));
            }
        }
        stack
            .stack
            .set_data_component(pumpkin_data::data_component_impl::BlockStateImpl {
                properties: Cow::Owned(copied),
            });
    }

    /// Vanilla `FilteredFunction.run`: `on_pass` or `on_fail` by the item predicate.
    fn filtered(&mut self, stack: &mut Stack, function: &Value) {
        let filter = function.get("item_filter").unwrap_or(&Value::Null);
        // `ItemPredicate.test` sees the stack's real count, which is past the stack size here
        // only for counts no vanilla filter tests.
        let mut tested = stack.stack.clone();
        tested.item_count = u8::try_from(stack.count.max(0)).unwrap_or(u8::MAX);
        let branch = if item_matches(&tested, filter) {
            function.get("on_pass")
        } else {
            function.get("on_fail")
        };
        let Some(branch) = branch else {
            return;
        };
        let branch = match branch {
            Value::Array(functions) => functions.as_slice(),
            function => std::slice::from_ref(function),
        };
        for function in branch {
            if function.get("conditions").is_some() {
                tracing::warn!("Unsupported conditions on a nested loot function");
            }
            self.run_json(stack, function);
        }
    }
}

/// Vanilla `SetPotionFunction.run`: the potion, keeping any custom color and effects.
fn set_potion(stack: &mut Stack, function: &Value) {
    let Some(id) = function.get("id").and_then(Value::as_str) else {
        return;
    };
    let Some(potion) = PotionContentsImpl::read_data(&pumpkin_nbt::tag::NbtTag::String(id.into()))
    else {
        return;
    };
    let contents = match stack.stack.get_data_component::<PotionContentsImpl>() {
        Some(old) => PotionContentsImpl {
            potion_id: potion.potion_id,
            ..old.clone()
        },
        None => potion,
    };
    stack.stack.set_data_component(contents);
}

/// Vanilla `SetNameFunction.run`. No vanilla table resolves an entity into the name.
fn set_name(stack: &mut Stack, function: &Value) {
    let Some(name) = function.get("name") else {
        return;
    };
    match function
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or("custom_name")
    {
        "item_name" => {
            if let Some(name) = json_to_nbt(name).and_then(|name| ItemNameImpl::read_data(&name)) {
                stack.stack.set_data_component(name);
            }
        }
        _ => {
            if let Ok(name) = serde_json::from_value::<TextComponent>(name.clone()) {
                stack.stack.set_data_component(CustomNameImpl { name });
            }
        }
    }
}

/// Vanilla `SetComponentsFunction.run`: `ItemStack.applyComponentsAndValidate`, with `!id`
/// removing a component.
fn set_components(stack: &mut Stack, function: &Value) {
    let Some(components) = function.get("components").and_then(Value::as_object) else {
        return;
    };
    for (name, value) in components {
        let (removed, name) = name
            .strip_prefix('!')
            .map_or((false, name.as_str()), |name| (true, name));
        let Some(id) = DataComponent::try_from_name(name) else {
            tracing::warn!("Unknown component {name} in set_components");
            continue;
        };
        let component = if removed {
            None
        } else {
            let Some(component) = json_to_nbt(value).and_then(|value| read_data(id, &value)) else {
                tracing::warn!("Couldn't read component {name} in set_components");
                continue;
            };
            Some(component)
        };
        if let Some((_, old)) = stack
            .stack
            .patch
            .iter_mut()
            .find(|(patched, _)| *patched == id)
        {
            *old = component;
        } else {
            stack.stack.patch.push((id, component));
        }
    }
}
