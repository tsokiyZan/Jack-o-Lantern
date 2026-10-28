use proc_macro2::{Span, TokenStream};
use quote::quote;
use std::{collections::BTreeMap, fs};
use syn::LitInt;
/// Generates the `TokenStream` for the `get_potted_item` function mapping item IDs to potted block IDs,
/// and its inverse `get_potted_content`.
pub fn build() -> TokenStream {
    let flower_pot_transformation: BTreeMap<u16, u16> = serde_json::from_str(
        &fs::read_to_string("../../assets/flower_pot_transformations.json").unwrap(),
    )
    .expect("Failed to parse flower_pot_transformations.json");
    let content_arms: Vec<TokenStream> = flower_pot_transformation
        .iter()
        .map(|(item_id, potted_block_id)| {
            let item_id_lit = LitInt::new(&item_id.to_string(), Span::call_site());
            let potted_id_lit = LitInt::new(&potted_block_id.to_string(), Span::call_site());
            quote! {
                #potted_id_lit => Some(#item_id_lit),
            }
        })
        .collect();
    let match_arms: Vec<TokenStream> = flower_pot_transformation
        .into_iter()
        .map(|(item_id, potted_block_id)| {
            let item_id_lit = LitInt::new(&item_id.to_string(), Span::call_site());
            let potted_id_lit = LitInt::new(&potted_block_id.to_string(), Span::call_site());
            quote! {
                #item_id_lit => #potted_id_lit,
            }
        })
        .collect();
    quote! {
        use crate::BlockId;

        #[must_use]
        pub const fn get_potted_item(item_id: u16) -> BlockId {
            let raw_id = match item_id {
                #(#match_arms)*
                _ => 0,
            };
            BlockId::new_or_air(raw_id)
        }

        /// The item id of the plant in a potted block (vanilla `FlowerPotBlock.potted` as an
        /// item), or `None` for the empty pot or any other block.
        #[must_use]
        pub const fn get_potted_content(potted_block_id: u16) -> Option<u16> {
            match potted_block_id {
                #(#content_arms)*
                _ => None,
            }
        }
    }
}
