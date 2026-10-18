use std::collections::BTreeMap;

use heck::ToShoutySnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::entity_type::{points_tokens, read_attachments};

/// Generates a constant for every static `EntityDimensions` in vanilla's entity classes (baby
/// sizes, poses), with the attachment points vanilla built for it. `Zombie.BABY_DIMENSIONS`
/// becomes `ZOMBIE_BABY_DIMENSIONS`, `Avatar.POSES.CROUCHING` `AVATAR_POSES_CROUCHING`.
pub fn build() -> TokenStream {
    let attachments = read_attachments();
    let mut names = BTreeMap::new();
    let mut consts = TokenStream::new();
    for (key, dims) in &attachments.statics {
        // `monster.zombie.Zombie.BABY_DIMENSIONS`: drop the package.
        let parts: Vec<&str> = key.split('.').collect();
        let class = parts
            .iter()
            .position(|p| p.starts_with(|c: char| c.is_ascii_uppercase()))
            .unwrap_or_else(|| panic!("no class in {key}"));
        let name = format!(
            "{}_{}",
            parts[class].to_shouty_snake_case(),
            parts[class + 1..].join("_")
        );
        if let Some(other) = names.insert(name.clone(), key) {
            panic!("{key} and {other} both map to {name}");
        }
        let ident = format_ident!("{}", name);
        let doc = format!("Vanilla `{}`.", parts[class..].join("."));
        let width = dims.width;
        let height = dims.height;
        let eye_height = dims.eye_height;
        let passenger = points_tokens(&dims.passenger);
        let vehicle = points_tokens(&dims.vehicle);
        consts.extend(quote! {
            #[doc = #doc]
            pub const #ident: EntityDimensions = EntityDimensions::new(#width, #height, #eye_height)
                .with_attachments(AttachmentPoints::Points(#passenger), AttachmentPoints::Points(#vehicle));
        });
    }
    quote! {
        use pumpkin_util::math::boundingbox::{AttachmentPoints, EntityDimensions};

        #consts
    }
}
