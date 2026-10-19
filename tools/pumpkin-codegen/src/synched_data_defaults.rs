use std::{collections::BTreeMap, fs};

use proc_macro2::{Literal, TokenStream};
use quote::quote;
use serde::Deserialize;

#[derive(Deserialize)]
struct EntityTypeId {
    id: u16,
}

fn decode_hex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or_else(|_| panic!("bad hex {hex}")))
        .collect()
}

/// Generates `synched_data_default` from `synched_data_defaults.json`
/// (`tools/vanilla-dump/SynchedDataDefaults.java`): the bytes vanilla's
/// `SynchedEntityData.DataValue.write` sends for each synced data item's initial value.
pub fn build() -> TokenStream {
    let defaults: BTreeMap<String, BTreeMap<u8, String>> = serde_json::from_str(
        &fs::read_to_string("../../assets/synched_data_defaults.json").unwrap(),
    )
    .expect("Failed to parse synched_data_defaults.json");
    let entities: BTreeMap<String, EntityTypeId> =
        serde_json::from_str(&fs::read_to_string("../../assets/entities.json").unwrap())
            .expect("Failed to parse entities.json");

    let mut by_id: Vec<(u16, &BTreeMap<u8, String>)> = defaults
        .iter()
        .map(|(name, items)| {
            let id = entities
                .get(name)
                .unwrap_or_else(|| panic!("synched_data_defaults.json has unknown entity {name}"))
                .id;
            (id, items)
        })
        .collect();
    by_id.sort_by_key(|(id, _)| *id);

    let arms = by_id.iter().map(|(id, items)| {
        let entries = items.iter().map(|(index, hex)| {
            let bytes = Literal::byte_string(&decode_hex(hex));
            quote! { (#index, #bytes) }
        });
        quote! { #id => &[#(#entries),*], }
    });

    quote! {
        /// The bytes vanilla writes for synced data item `index` of the entity type at its
        /// initial value (index byte, serializer id, value), or `None` if the type has no such
        /// item or its default isn't fixed (drawn from the entity's random).
        #[must_use]
        pub fn synched_data_default(entity_type_id: u16, index: u8) -> Option<&'static [u8]> {
            let items: &[(u8, &[u8])] = match entity_type_id {
                #(#arms)*
                _ => return None,
            };
            items
                .binary_search_by_key(&index, |(i, _)| *i)
                .ok()
                .map(|i| items[i].1)
        }
    }
}
