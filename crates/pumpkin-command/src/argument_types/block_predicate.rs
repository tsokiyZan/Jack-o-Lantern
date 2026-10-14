use pumpkin_data::tag::{RegistryKey, get_tag_ids};
use pumpkin_data::{Block, BlockStateId, translation};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;

use crate::argument_types::argument_type::{ArgumentType, JavaClientArgumentType};
use crate::argument_types::block::{
    ERROR_DUPLICATE_PROPERTY, ERROR_EXPECTED_END_OF_PROPERTIES, ERROR_EXPECTED_VALUE,
    properties_match, read_block, read_nbt, read_properties,
};
use crate::argument_types::nbt_path::compare_nbt;
use crate::context::command_context::CommandContext;
use crate::errors::command_syntax_error::CommandSyntaxError;
use crate::errors::error_types::CommandErrorType;
use crate::string_reader::StringReader;
use crate::suggestion::suggestions::{Suggestions, SuggestionsBuilder};

pub const ERROR_UNKNOWN_TAG: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENTS_BLOCK_TAG_UNKNOWN,
    translation::java::ARGUMENTS_BLOCK_TAG_UNKNOWN,
);

/// Vanilla `BlockPredicateArgument`'s result: a block with the properties it names, or a
/// block tag with property strings checked against each block.
#[derive(Clone, Debug)]
pub enum BlockPredicate {
    Block {
        state: BlockStateId,
        properties: Vec<&'static str>,
        nbt: Option<NbtCompound>,
    },
    Tag {
        tag_name: String,
        block_ids: Vec<u16>,
        properties: Vec<(String, String)>,
        nbt: Option<NbtCompound>,
    },
}

impl BlockPredicate {
    /// The block and property part of the vanilla predicate's `test`.
    #[must_use]
    pub fn test(&self, state: BlockStateId) -> bool {
        let block = Block::from_state_id(state);
        match self {
            Self::Block {
                state: expected,
                properties,
                ..
            } => {
                let expected_block = Block::from_state_id(*expected);
                block.id == expected_block.id
                    && properties_match(expected_block, *expected, state, properties)
            }
            // Vanilla `TagPredicate.test`: a property the block lacks, or a value it
            // cannot take, fails the test.
            Self::Tag {
                block_ids,
                properties,
                ..
            } => {
                if !block_ids.contains(&block.id.as_u16()) {
                    return false;
                }
                if properties.is_empty() {
                    return true;
                }
                let Some(actual) = block.properties(state) else {
                    return false;
                };
                let actual = actual.to_props();
                properties.iter().all(|(name, value)| {
                    actual.iter().any(|(key, set)| key == name && set == value)
                })
            }
        }
    }

    /// The block entity part of the vanilla predicate's `test`: a partial match of the
    /// predicate's NBT against the block entity's saved data.
    #[must_use]
    pub fn test_nbt(&self, block_entity: Option<&NbtCompound>) -> bool {
        self.nbt().is_none_or(|expected| {
            block_entity.is_some_and(|saved| {
                compare_nbt(
                    &NbtTag::Compound(expected.clone()),
                    &NbtTag::Compound(saved.clone()),
                )
            })
        })
    }

    #[must_use]
    pub const fn nbt(&self) -> Option<&NbtCompound> {
        match self {
            Self::Block { nbt, .. } | Self::Tag { nbt, .. } => nbt.as_ref(),
        }
    }

    #[must_use]
    pub const fn requires_nbt(&self) -> bool {
        self.nbt().is_some()
    }
}

/// Vanilla `BlockStateParser.readVagueProperties`: property strings for a tag, checked
/// only when a block is tested.
fn read_vague_properties(
    reader: &mut StringReader,
    tag_name: &str,
) -> Result<Vec<(String, String)>, CommandSyntaxError> {
    let mut properties: Vec<(String, String)> = Vec::new();
    if reader.peek() != Some('[') {
        return Ok(properties);
    }
    reader.skip();
    reader.skip_whitespace();
    let tag_text = || TextComponent::text(tag_name.to_string());
    while reader.can_read_char() && reader.peek() != Some(']') {
        reader.skip_whitespace();
        let key_start = reader.cursor();
        let key = reader.read_string()?;
        if properties.iter().any(|(name, _)| *name == key) {
            reader.set_cursor(key_start);
            return Err(ERROR_DUPLICATE_PROPERTY.create(
                reader,
                TextComponent::text(key),
                tag_text(),
            ));
        }
        reader.skip_whitespace();
        if reader.peek() != Some('=') {
            return Err(ERROR_EXPECTED_VALUE.create(reader, TextComponent::text(key), tag_text()));
        }
        reader.skip();
        reader.skip_whitespace();
        let value = reader.read_string()?;
        properties.push((key, value));
        reader.skip_whitespace();
        if reader.can_read_char() {
            if reader.peek() != Some(',') {
                if reader.peek() != Some(']') {
                    return Err(ERROR_EXPECTED_END_OF_PROPERTIES.create(reader));
                }
                break;
            }
            reader.skip();
        }
    }
    if reader.can_read_char() {
        reader.skip();
    } else {
        return Err(ERROR_EXPECTED_END_OF_PROPERTIES.create(reader));
    }
    Ok(properties)
}

pub struct BlockPredicateArgumentType;

impl<S: crate::source::CommandSource> ArgumentType<S> for BlockPredicateArgumentType {
    type Item = BlockPredicate;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        if reader.peek() == Some('#') {
            reader.skip();
            let start = reader.cursor();
            while let Some(c) = reader.peek() {
                if c.is_alphanumeric() || c == '_' || c == ':' || c == '/' || c == '.' || c == '-' {
                    reader.skip();
                } else {
                    break;
                }
            }
            let tag_str = &reader.string()[start..reader.cursor()];
            let normalized = if tag_str.contains(':') {
                tag_str.to_string()
            } else {
                format!("minecraft:{tag_str}")
            };
            let stripped = normalized.strip_prefix("minecraft:").unwrap_or(&normalized);

            let tag_ids = get_tag_ids(RegistryKey::Block, &normalized)
                .or_else(|| get_tag_ids(RegistryKey::Block, stripped));

            let block_ids = match tag_ids {
                Some(ids) => ids.to_vec(),
                None => {
                    return Err(ERROR_UNKNOWN_TAG.create(reader, TextComponent::text(normalized)));
                }
            };

            let properties = read_vague_properties(reader, &normalized)?;
            let nbt = read_nbt(reader)?;

            Ok(BlockPredicate::Tag {
                tag_name: normalized,
                block_ids,
                properties,
                nbt,
            })
        } else {
            let block = read_block(reader)?;
            let (state, properties) = read_properties(reader, block)?;
            let nbt = read_nbt(reader)?;
            Ok(BlockPredicate::Block {
                state,
                properties,
                nbt,
            })
        }
    }

    fn client_side_parser(&self) -> JavaClientArgumentType {
        JavaClientArgumentType::BlockPredicate
    }

    fn examples(&self) -> Vec<String> {
        vec![
            "stone".to_string(),
            "minecraft:stone".to_string(),
            "stone[foo=bar]".to_string(),
            "#stone".to_string(),
            "#stone[foo=bar]{baz=nbt}".to_string(),
        ]
    }

    fn list_suggestions(
        &self,
        _context: &CommandContext<S>,
        builder: SuggestionsBuilder,
    ) -> Suggestions {
        builder.build()
    }
}

impl BlockPredicateArgumentType {
    pub fn get<S: crate::source::CommandSource>(
        context: &CommandContext<S>,
        name: &str,
    ) -> Result<BlockPredicate, CommandSyntaxError> {
        Ok(context.get_argument::<BlockPredicate>(name)?.clone())
    }
}
