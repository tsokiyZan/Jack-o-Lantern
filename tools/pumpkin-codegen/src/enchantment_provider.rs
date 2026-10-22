use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use heck::ToShoutySnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde::Deserialize;

const DEFAULT_NAMESPACE: &str = "minecraft";

#[derive(Deserialize, Debug)]
struct RawProvider {
    #[serde(rename = "type")]
    provider_type: String,
    #[serde(default)]
    enchantment: Option<String>,
    #[serde(default)]
    level: Option<serde_json::Value>,
    #[serde(default)]
    enchantments: Option<serde_json::Value>,
    #[serde(default)]
    min_cost: Option<i32>,
    #[serde(default)]
    max_cost_span: Option<i32>,
    #[serde(default)]
    cost: Option<serde_json::Value>,
}

struct ProviderEntry {
    qualified: String,
    bare: String,
    const_name: String,
    rel_path: String,
    kind_tokens: TokenStream,
}

pub fn build() -> TokenStream {
    let provider_dir = Path::new("../../assets/datapack/data/minecraft/enchantment_provider");
    let tags_dir = Path::new("../../assets/datapack/data/minecraft/tags/enchantment");

    let mut raw_files: BTreeMap<String, (String, String, String)> = BTreeMap::new();
    if provider_dir.is_dir() {
        scan_dir(provider_dir, "", &mut raw_files);
    }

    let mut entries: Vec<ProviderEntry> = Vec::new();

    for (qualified, (bare, const_name, rel_path)) in raw_files {
        // Read and parse JSON
        let full_path = Path::new("../../assets/datapack/data/minecraft/enchantment_provider")
            .join(format!("{bare}.json"));
        let content = fs::read_to_string(&full_path).unwrap_or_else(|e| {
            panic!(
                "failed to read enchantment provider file {}: {}",
                full_path.display(),
                e
            )
        });
        let raw: RawProvider = serde_json::from_str(&content).unwrap_or_else(|e| {
            panic!(
                "failed to parse enchantment provider JSON {}: {}",
                full_path.display(),
                e
            )
        });

        let kind_tokens = match raw.provider_type.as_str() {
            "minecraft:single" | "single" => {
                let enc_str = raw
                    .enchantment
                    .expect("single provider requires enchantment");
                let enc_bare = enc_str.strip_prefix("minecraft:").unwrap_or(&enc_str);
                let enc_ident = format_ident!("{}", enc_bare.to_shouty_snake_case());
                let level = constant_int_provider(raw.level.as_ref(), &qualified);
                quote! {
                    EnchantmentProviderKind::Single {
                        enchantment: &Enchantment::#enc_ident,
                        level: #level,
                    }
                }
            }
            "minecraft:by_cost_with_difficulty" | "by_cost_with_difficulty" => {
                let enc_names = extract_enchantments(raw.enchantments.as_ref(), tags_dir);
                let enc_idents: Vec<_> = enc_names
                    .iter()
                    .map(|name| {
                        let bare = name.strip_prefix("minecraft:").unwrap_or(name);
                        let ident = format_ident!("{}", bare.to_shouty_snake_case());
                        quote! { &Enchantment::#ident }
                    })
                    .collect();
                let min_cost = raw.min_cost.unwrap_or(5);
                let max_cost_span = raw.max_cost_span.unwrap_or(17);
                quote! {
                    EnchantmentProviderKind::ByCostWithDifficulty {
                        enchantments: &[ #( #enc_idents ),* ],
                        min_cost: #min_cost,
                        max_cost_span: #max_cost_span,
                    }
                }
            }
            "minecraft:by_cost" | "by_cost" => {
                let enc_names = extract_enchantments(raw.enchantments.as_ref(), tags_dir);
                let enc_idents: Vec<_> = enc_names
                    .iter()
                    .map(|name| {
                        let bare = name.strip_prefix("minecraft:").unwrap_or(name);
                        let ident = format_ident!("{}", bare.to_shouty_snake_case());
                        quote! { &Enchantment::#ident }
                    })
                    .collect();
                let cost = constant_int_provider(raw.cost.as_ref(), &qualified);
                quote! {
                    EnchantmentProviderKind::ByCost {
                        enchantments: &[ #( #enc_idents ),* ],
                        cost: #cost,
                    }
                }
            }
            other => panic!("unknown enchantment provider type: {other}"),
        };

        entries.push(ProviderEntry {
            qualified,
            bare,
            const_name,
            rel_path,
            kind_tokens,
        });
    }

    let mut const_decls = Vec::new();
    let mut from_name_arms = Vec::new();
    let mut raw_json_arms = Vec::new();
    let mut all_const_idents = Vec::new();
    let mut all_qualified_names = Vec::new();

    for entry in &entries {
        let const_ident = format_ident!("{}", entry.const_name);
        let qualified = &entry.qualified;
        let bare = &entry.bare;
        let rel_path = &entry.rel_path;
        let kind_tokens = &entry.kind_tokens;

        const_decls.push(quote! {
            pub const #const_ident: Self = Self {
                name: #qualified,
                kind: #kind_tokens,
            };
        });

        from_name_arms.push(quote! {
            #qualified | #bare => Some(&Self::#const_ident),
        });

        raw_json_arms.push(quote! {
            #qualified | #bare => Some(include_str!(#rel_path)),
        });

        all_const_idents.push(quote! { Self::#const_ident });
        all_qualified_names.push(quote! { #qualified });
    }

    quote! {
        use crate::enchantment::Enchantment;

        #[derive(Clone, Copy, PartialEq)]
        pub enum EnchantmentProviderKind {
            Single {
                enchantment: &'static Enchantment,
                level: i32,
            },
            ByCostWithDifficulty {
                enchantments: &'static [&'static Enchantment],
                min_cost: i32,
                max_cost_span: i32,
            },
            ByCost {
                enchantments: &'static [&'static Enchantment],
                cost: i32,
            },
        }

        impl core::fmt::Debug for EnchantmentProviderKind {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self {
                    Self::Single { enchantment, level } => f
                        .debug_struct("Single")
                        .field("enchantment", &enchantment.registry_key)
                        .field("level", level)
                        .finish(),
                    Self::ByCostWithDifficulty {
                        enchantments,
                        min_cost,
                        max_cost_span,
                    } => {
                        let keys: Vec<&str> = enchantments.iter().map(|e| e.registry_key).collect();
                        f.debug_struct("ByCostWithDifficulty")
                            .field("enchantments", &keys)
                            .field("min_cost", min_cost)
                            .field("max_cost_span", max_cost_span)
                            .finish()
                    }
                    Self::ByCost {
                        enchantments,
                        cost,
                    } => {
                        let keys: Vec<&str> = enchantments.iter().map(|e| e.registry_key).collect();
                        f.debug_struct("ByCost")
                            .field("enchantments", &keys)
                            .field("cost", cost)
                            .finish()
                    }
                }
            }
        }

        #[derive(Clone, Copy, PartialEq)]
        pub struct EnchantmentProvider {
            pub name: &'static str,
            pub kind: EnchantmentProviderKind,
        }

        impl core::fmt::Debug for EnchantmentProvider {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_struct("EnchantmentProvider")
                    .field("name", &self.name)
                    .field("kind", &self.kind)
                    .finish()
            }
        }

        impl EnchantmentProvider {
            #( #const_decls )*

            pub const ALL: &'static [Self] = &[
                #( #all_const_idents ),*
            ];

            #[must_use]
            pub const fn name(&self) -> &'static str {
                self.name
            }

            #[must_use]
            pub const fn kind(&self) -> &EnchantmentProviderKind {
                &self.kind
            }

            #[allow(clippy::too_many_lines)]
            #[allow(clippy::match_same_arms)]
            #[must_use]
            pub fn from_name(name: &str) -> Option<&'static Self> {
                match name {
                    #( #from_name_arms )*
                    _ => None,
                }
            }

            #[allow(clippy::too_many_lines)]
            #[allow(clippy::match_same_arms)]
            #[must_use]
            pub fn get_raw_json(name: &str) -> Option<&'static str> {
                match name {
                    #( #raw_json_arms )*
                    _ => None,
                }
            }

            #[must_use]
            pub const fn all() -> &'static [Self] {
                Self::ALL
            }

            #[must_use]
            pub const fn all_provider_names() -> &'static [&'static str] {
                &[
                    #( #all_qualified_names ),*
                ]
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn all_providers_count() {
                assert!(EnchantmentProvider::ALL.len() >= 7);
            }

            #[test]
            fn lookup_by_name() {
                assert_eq!(
                    EnchantmentProvider::from_name("minecraft:mob_spawn_equipment"),
                    Some(&EnchantmentProvider::MOB_SPAWN_EQUIPMENT)
                );
                assert_eq!(
                    EnchantmentProvider::from_name("mob_spawn_equipment"),
                    Some(&EnchantmentProvider::MOB_SPAWN_EQUIPMENT)
                );
                assert_eq!(
                    EnchantmentProvider::from_name("enderman_loot_drop"),
                    Some(&EnchantmentProvider::ENDERMAN_LOOT_DROP)
                );
                assert_eq!(
                    EnchantmentProvider::from_name("raid/vindicator"),
                    Some(&EnchantmentProvider::RAID_VINDICATOR)
                );
                assert_eq!(
                    EnchantmentProvider::from_name("minecraft:raid/pillager_post_wave_5"),
                    Some(&EnchantmentProvider::RAID_PILLAGER_POST_WAVE_5)
                );
            }

            #[test]
            fn raw_json_retrieval() {
                assert!(EnchantmentProvider::get_raw_json("mob_spawn_equipment").is_some());
                assert!(EnchantmentProvider::get_raw_json("minecraft:enderman_loot_drop").is_some());
                assert!(EnchantmentProvider::get_raw_json("nonexistent").is_none());
            }
        }
    }
}

fn scan_dir(dir: &Path, prefix: &str, registry: &mut BTreeMap<String, (String, String, String)>) {
    let mut entries = fs::read_dir(dir)
        .expect("read dir")
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            let new_prefix = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            scan_dir(&path, &new_prefix, registry);
            continue;
        }

        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };

        let id = if prefix.is_empty() {
            stem.to_string()
        } else {
            format!("{prefix}/{stem}")
        };

        let qualified = format!("{DEFAULT_NAMESPACE}:{id}");
        let bare = id.clone();
        let const_name = id.replace(['/', '-', '.'], "_").to_uppercase();

        let rel_to_repo = path.strip_prefix("../../").expect("strip ../../");
        let rel_path = format!(
            "../../../../{}",
            rel_to_repo.to_string_lossy().replace('\\', "/")
        );

        registry.insert(qualified, (bare, const_name, rel_path));
    }
}

fn extract_enchantments(val: Option<&serde_json::Value>, tags_dir: &Path) -> Vec<String> {
    let Some(val) = val else {
        return Vec::new();
    };

    let mut result = Vec::new();
    match val {
        serde_json::Value::String(s) => {
            if s.starts_with('#') {
                let mut visited = HashSet::new();
                resolve_tag_recursive(s, tags_dir, &mut visited, &mut result);
            } else {
                let bare = s.strip_prefix("minecraft:").unwrap_or(s);
                result.push(bare.to_string());
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(s) = item.as_str() {
                    if s.starts_with('#') {
                        let mut visited = HashSet::new();
                        resolve_tag_recursive(s, tags_dir, &mut visited, &mut result);
                    } else {
                        let bare = s.strip_prefix("minecraft:").unwrap_or(s);
                        result.push(bare.to_string());
                    }
                }
            }
        }
        _ => {}
    }

    // Vanilla's `TagLoader` builds into a `LinkedHashSet`: first occurrence wins, in order. The
    // weighted pick in `selectEnchantment` depends on this order.
    let mut seen = HashSet::new();
    result.retain(|name| seen.insert(name.clone()));
    result
}

/// The value of a constant `IntProvider` (a bare number or `{"type": "constant"}`). Vanilla's
/// providers only use constants, which sample without drawing from the random.
fn constant_int_provider(val: Option<&serde_json::Value>, provider: &str) -> i32 {
    let value = match val {
        Some(serde_json::Value::Number(n)) => n.as_i64(),
        Some(serde_json::Value::Object(obj))
            if obj
                .get("type")
                .and_then(|t| t.as_str())
                .map(|t| t.trim_start_matches("minecraft:"))
                == Some("constant") =>
        {
            obj.get("value").and_then(serde_json::Value::as_i64)
        }
        _ => None,
    };
    let Some(value) = value else {
        panic!(
            "enchantment provider {provider}: only constant int providers are supported, got {val:?}"
        );
    };
    i32::try_from(value).expect("int provider value fits i32")
}

fn resolve_tag_recursive(
    tag: &str,
    tags_dir: &Path,
    visited: &mut HashSet<String>,
    output: &mut Vec<String>,
) {
    let tag_name = tag.strip_prefix('#').unwrap_or(tag);
    if !visited.insert(tag_name.to_string()) {
        return;
    }

    let bare = tag_name.strip_prefix("minecraft:").unwrap_or(tag_name);
    let tag_file = tags_dir.join(format!("{bare}.json"));
    if !tag_file.exists() {
        return;
    }

    let content = fs::read_to_string(&tag_file).unwrap_or_default();
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };

    if let Some(values) = json.get("values").and_then(|v| v.as_array()) {
        for v in values {
            if let Some(s) = v.as_str() {
                if s.starts_with('#') {
                    resolve_tag_recursive(s, tags_dir, visited, output);
                } else {
                    let enc = s.strip_prefix("minecraft:").unwrap_or(s);
                    output.push(enc.to_string());
                }
            }
        }
    }
}
