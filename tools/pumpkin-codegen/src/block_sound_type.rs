use std::collections::BTreeMap;
use std::fs;

use heck::{ToPascalCase, ToShoutySnakeCase};
use proc_macro2::{Literal, TokenStream};
use quote::{format_ident, quote};
use serde::Deserialize;

#[derive(Deserialize)]
struct SoundTypeEntry {
    volume: f32,
    pitch: f32,
    #[serde(rename = "break")]
    break_sound: String,
    step: String,
    place: String,
    hit: String,
    fall: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BlockSoundTypes {
    Single(String),
    PerState(Vec<String>),
}

#[derive(Deserialize)]
struct SoundTypesFile {
    sound_types: BTreeMap<String, SoundTypeEntry>,
    blocks: BTreeMap<String, BlockSoundTypes>,
}

#[derive(Deserialize)]
struct BlockState {
    id: u16,
}

#[derive(Deserialize)]
struct Block {
    name: String,
    states: Vec<BlockState>,
}

#[derive(Deserialize)]
struct BlocksFile {
    blocks: Vec<Block>,
}

/// Generates vanilla's `SoundType` constants and a per-block-state lookup
/// (`BlockBehaviour.getSoundType(state)`), from `block_sound_types.json`
/// (dumped from a vanilla server by `tools/vanilla-dump/BlockSoundTypes.java`).
pub fn build() -> TokenStream {
    let file: SoundTypesFile =
        serde_json::from_str(&fs::read_to_string("../../assets/block_sound_types.json").unwrap())
            .expect("Failed to parse block_sound_types.json");
    let blocks: BlocksFile =
        serde_json::from_str(&fs::read_to_string("../../assets/blocks.json").unwrap())
            .expect("Failed to parse blocks.json");

    let names: Vec<&String> = file.sound_types.keys().collect();
    assert!(names.len() <= usize::from(u8::MAX));
    let index_of = |name: &str| -> u8 {
        names
            .iter()
            .position(|n| *n == name)
            .unwrap_or_else(|| panic!("unknown sound type {name}")) as u8
    };

    let sound = |path: &str| {
        let variant = format_ident!("{}", path.to_pascal_case());
        quote! { Sound::#variant }
    };

    let mut constants = TokenStream::new();
    let mut all = TokenStream::new();
    for (name, t) in &file.sound_types {
        let ident = format_ident!("{}", name.to_shouty_snake_case());
        let (volume, pitch) = (t.volume, t.pitch);
        let (b, s, p, h, f) = (
            sound(&t.break_sound),
            sound(&t.step),
            sound(&t.place),
            sound(&t.hit),
            sound(&t.fall),
        );
        constants.extend(quote! {
            pub const #ident: SoundType = SoundType {
                volume: #volume,
                pitch: #pitch,
                break_sound: #b,
                step_sound: #s,
                place_sound: #p,
                hit_sound: #h,
                fall_sound: #f,
            };
        });
        all.extend(quote! { &SoundType::#ident, });
    }

    let mut by_state: Vec<(u16, u8)> = Vec::new();
    for block in &blocks.blocks {
        let entry = file
            .blocks
            .get(&block.name)
            .unwrap_or_else(|| panic!("no sound type for block {}", block.name));
        match entry {
            BlockSoundTypes::Single(name) => {
                let idx = index_of(name);
                by_state.extend(block.states.iter().map(|s| (s.id, idx)));
            }
            BlockSoundTypes::PerState(per_state) => {
                assert_eq!(per_state.len(), block.states.len(), "{}", block.name);
                by_state.extend(
                    block
                        .states
                        .iter()
                        .zip(per_state)
                        .map(|(s, n)| (s.id, index_of(n))),
                );
            }
        }
    }
    by_state.sort_by_key(|(id, _)| *id);
    for (i, (id, _)) in by_state.iter().enumerate() {
        assert_eq!(usize::from(*id), i, "block state ids are not contiguous");
    }
    let table = by_state.iter().map(|(_, idx)| Literal::u8_unsuffixed(*idx));
    let state_count = by_state.len();

    quote! {
        use crate::sound::Sound;

        /// Vanilla `SoundType`: the sounds a block makes and their volume and pitch.
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub struct SoundType {
            pub volume: f32,
            pub pitch: f32,
            pub break_sound: Sound,
            pub step_sound: Sound,
            pub place_sound: Sound,
            pub hit_sound: Sound,
            pub fall_sound: Sound,
        }

        impl SoundType {
            #constants

            const ALL: &[&Self] = &[ #all ];

            #[allow(clippy::large_stack_arrays)]
            const BY_STATE: [u8; #state_count] = [ #(#table),* ];

            /// Vanilla `BlockState.getSoundType()`.
            #[must_use]
            pub fn from_state_id(state_id: u16) -> &'static Self {
                Self::ALL[Self::BY_STATE[state_id as usize] as usize]
            }
        }
    }
}
