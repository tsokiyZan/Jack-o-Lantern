use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use heck::ToPascalCase;
use proc_macro2::TokenStream;
use pumpkin_util::math::float_provider::FloatProvider;
use quote::{format_ident, quote};
use serde::Deserialize;

#[derive(Deserialize)]
struct ArchetypeJson {
    items: String,
    attribute_modifiers: Vec<AttributeEntryJson>,
    #[serde(default)]
    buoyant: bool,
    explosion: Option<ExplosionJson>,
    contact_damage: Option<ContactDamageJson>,
    knockback_modifiers: KnockbackModifiersJson,
    sound_settings: SoundSettingsJson,
}

#[derive(Deserialize)]
struct AttributeEntryJson {
    attribute: String,
    id: String,
    amount: f64,
    operation: String,
}

#[derive(Deserialize)]
struct ExplosionJson {
    power: i32,
    causes_fire: bool,
    fuse: i32,
}

#[derive(Deserialize)]
struct ContactDamageJson {
    damage_type: String,
    amount: FloatProvider,
    attribute_to_source: bool,
}

#[derive(Deserialize)]
struct KnockbackModifiersJson {
    horizontal_power: f32,
    vertical_power: f32,
}

#[derive(Deserialize)]
struct SoundSettingsJson {
    hit_sound: String,
    push_sound: String,
    push_sound_impulse_threshold: f32,
    push_sound_cooldown: f32,
}

fn strip_namespace(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}

fn sound_ident(id: &str) -> proc_macro2::Ident {
    format_ident!("{}", strip_namespace(id).replace('.', "_").to_pascal_case())
}

/// Generates the `sulfur_cube_archetype` registry: what a sulfur cube becomes with a block
/// in its body.
pub fn build() -> TokenStream {
    let dir = Path::new("../../assets/datapack/data/minecraft/sulfur_cube_archetype");

    // Vanilla loads registry entries sorted by id, and `SulfurCube.matchingArchetypes` keeps
    // that order.
    let mut archetypes: BTreeMap<String, ArchetypeJson> = BTreeMap::new();
    for entry in fs::read_dir(dir)
        .expect("read sulfur_cube_archetype dir")
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path.extension().is_some_and(|ext| ext == "json") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let content = fs::read_to_string(&path).expect("read sulfur_cube_archetype file");
        let json: ArchetypeJson =
            serde_json::from_str(&content).expect("parse sulfur_cube_archetype JSON");
        archetypes.insert(stem, json);
    }

    let mut consts = Vec::new();
    let mut all = Vec::new();

    for (name, json) in &archetypes {
        let const_ident = format_ident!("{}", name.to_uppercase());
        let namespaced = format!("minecraft:{name}");

        let tag_name = json
            .items
            .strip_prefix('#')
            .expect("sulfur cube archetype items must be an item tag");
        let tag_ident = format_ident!(
            "{}",
            tag_name.replace([':', '/', '.', '-'], "_").to_uppercase()
        );

        let modifiers = json.attribute_modifiers.iter().map(|m| {
            let attribute = format_ident!("{}", strip_namespace(&m.attribute).to_uppercase());
            let id = &m.id;
            let amount = m.amount;
            let operation = match m.operation.as_str() {
                "add_value" => quote! { Operation::AddValue },
                "add_multiplied_base" => quote! { Operation::AddMultipliedBase },
                "add_multiplied_total" => quote! { Operation::AddMultipliedTotal },
                other => panic!("unknown attribute modifier operation {other}"),
            };
            quote! {
                AttributeEntry {
                    attribute: &Attributes::#attribute,
                    id: #id,
                    amount: #amount,
                    operation: #operation,
                }
            }
        });

        let buoyant = json.buoyant;

        let explosion = json.explosion.as_ref().map_or_else(
            || quote! { None },
            |e| {
                let power = e.power;
                let causes_fire = e.causes_fire;
                let fuse = e.fuse;
                quote! {
                    Some(ExplosionData { power: #power, causes_fire: #causes_fire, fuse: #fuse })
                }
            },
        );

        let contact_damage = json.contact_damage.as_ref().map_or_else(
            || quote! { None },
            |c| {
                let damage_type =
                    format_ident!("{}", strip_namespace(&c.damage_type).to_uppercase());
                let amount = &c.amount;
                let attribute_to_source = c.attribute_to_source;
                quote! {
                    Some(ContactDamage {
                        damage_type: DamageType::#damage_type,
                        amount: #amount,
                        attribute_to_source: #attribute_to_source,
                    })
                }
            },
        );

        let horizontal_power = json.knockback_modifiers.horizontal_power;
        let vertical_power = json.knockback_modifiers.vertical_power;

        let hit_sound = sound_ident(&json.sound_settings.hit_sound);
        let push_sound = sound_ident(&json.sound_settings.push_sound);
        let threshold = json.sound_settings.push_sound_impulse_threshold;
        let cooldown = json.sound_settings.push_sound_cooldown;

        consts.push(quote! {
            pub const #const_ident: Self = Self {
                name: #namespaced,
                items: &tag::Item::#tag_ident,
                attribute_modifiers: &[#(#modifiers),*],
                buoyant: #buoyant,
                explosion: #explosion,
                contact_damage: #contact_damage,
                knockback_modifiers: KnockbackModifiers {
                    horizontal_power: #horizontal_power,
                    vertical_power: #vertical_power,
                },
                sound_settings: SoundSettings {
                    hit_sound: Sound::#hit_sound,
                    push_sound: Sound::#push_sound,
                    push_sound_impulse_threshold: #threshold,
                    push_sound_cooldown: #cooldown,
                },
            };
        });
        all.push(quote! { &Self::#const_ident });
    }

    quote! {
        use crate::attributes::Attributes;
        use crate::damage::DamageType;
        use crate::data_component_impl::Operation;
        use crate::sound::Sound;
        use crate::tag::{self, Tag};
        use pumpkin_util::math::float_provider::FloatProvider;

        /// Vanilla `SulfurCubeArchetype.AttributeEntry`.
        #[derive(Debug)]
        pub struct AttributeEntry {
            pub attribute: &'static Attributes,
            pub id: &'static str,
            pub amount: f64,
            pub operation: Operation,
        }

        /// Vanilla `SulfurCubeArchetype.ExplosionData`.
        #[derive(Clone, Copy, Debug)]
        pub struct ExplosionData {
            pub power: i32,
            pub causes_fire: bool,
            pub fuse: i32,
        }

        /// Vanilla `SulfurCubeArchetype.ContactDamage`.
        #[derive(Clone, Debug)]
        pub struct ContactDamage {
            pub damage_type: DamageType,
            pub amount: FloatProvider,
            pub attribute_to_source: bool,
        }

        /// Vanilla `SulfurCubeArchetype.KnockbackModifiers`.
        #[derive(Clone, Copy, Debug)]
        pub struct KnockbackModifiers {
            pub horizontal_power: f32,
            pub vertical_power: f32,
        }

        /// Vanilla `SulfurCubeArchetype.SoundSettings`.
        #[derive(Clone, Copy, Debug)]
        pub struct SoundSettings {
            pub hit_sound: Sound,
            pub push_sound: Sound,
            pub push_sound_impulse_threshold: f32,
            pub push_sound_cooldown: f32,
        }

        /// An entry of the `minecraft:sulfur_cube_archetype` registry.
        #[derive(Debug)]
        pub struct SulfurCubeArchetype {
            pub name: &'static str,
            pub items: &'static Tag,
            pub attribute_modifiers: &'static [AttributeEntry],
            pub buoyant: bool,
            pub explosion: Option<ExplosionData>,
            pub contact_damage: Option<ContactDamage>,
            pub knockback_modifiers: KnockbackModifiers,
            pub sound_settings: SoundSettings,
        }

        impl SulfurCubeArchetype {
            #(#consts)*

            /// Every archetype, in registry order.
            pub const ALL: &'static [&'static Self] = &[#(#all),*];
        }
    }
}
