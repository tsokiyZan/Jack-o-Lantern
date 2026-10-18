use crate::{
    argument_types::argument_type::{ArgumentType, JavaClientArgumentType},
    argument_types::nbt::NbtCompoundArgumentType,
    context::command_context::CommandContext,
    errors::command_syntax_error::CommandSyntaxError,
    errors::error_types::CommandErrorType,
    string_reader::StringReader,
    suggestion::suggestions::{Suggestions, SuggestionsBuilder},
};
use pumpkin_data::{Block, BlockStateId, Property, PropertyKey, translation};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::text::TextComponent;

pub const INVALID_BLOCK_ERROR_TYPE: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENT_BLOCK_ID_INVALID,
    translation::java::ARGUMENT_BLOCK_ID_INVALID,
);

pub const ERROR_UNKNOWN_PROPERTY: CommandErrorType<2> = CommandErrorType::new(
    translation::java::ARGUMENT_BLOCK_PROPERTY_UNKNOWN,
    translation::java::ARGUMENT_BLOCK_PROPERTY_UNKNOWN,
);

pub const ERROR_DUPLICATE_PROPERTY: CommandErrorType<2> = CommandErrorType::new(
    translation::java::ARGUMENT_BLOCK_PROPERTY_DUPLICATE,
    translation::java::ARGUMENT_BLOCK_PROPERTY_DUPLICATE,
);

pub const ERROR_INVALID_VALUE: CommandErrorType<3> = CommandErrorType::new(
    translation::java::ARGUMENT_BLOCK_PROPERTY_INVALID,
    translation::java::ARGUMENT_BLOCK_PROPERTY_INVALID,
);

pub const ERROR_EXPECTED_VALUE: CommandErrorType<2> = CommandErrorType::new(
    translation::java::ARGUMENT_BLOCK_PROPERTY_NOVALUE,
    translation::java::ARGUMENT_BLOCK_PROPERTY_NOVALUE,
);

pub const ERROR_EXPECTED_END_OF_PROPERTIES: CommandErrorType<0> = CommandErrorType::new(
    translation::java::ARGUMENT_BLOCK_PROPERTY_UNCLOSED,
    translation::java::ARGUMENT_BLOCK_PROPERTY_UNCLOSED,
);

/// Vanilla `BlockInput`: a block state with the properties the command named, and
/// optional block entity data.
#[derive(Clone, Debug)]
pub struct BlockInput {
    pub state: BlockStateId,
    /// The properties written out in the input.
    pub properties: Vec<&'static str>,
    pub nbt: Option<NbtCompound>,
}

impl BlockInput {
    #[must_use]
    pub const fn block(&self) -> &'static Block {
        Block::from_state_id(self.state)
    }

    /// Vanilla `BlockInput.overwriteWithDefinedProperties`: sets each property the input
    /// named on `state` to the input's value.
    #[must_use]
    pub fn overwrite_with_defined_properties(&self, state: BlockStateId) -> BlockStateId {
        if state == self.state || self.properties.is_empty() {
            return state;
        }
        let block = Block::from_state_id(state);
        // Vanilla's `copyProperty` skips properties the state lacks; a different block
        // shares none of the input's.
        if block.id != self.block().id {
            return state;
        }
        let (Some(input), Some(current)) =
            (self.block().properties(self.state), block.properties(state))
        else {
            return state;
        };
        let input = property_pairs(input.to_props());
        let merged: Vec<(&str, &str)> = property_pairs(current.to_props())
            .into_iter()
            .map(|(key, value)| {
                if self.properties.contains(&key)
                    && let Some(&(_, wanted)) = input.iter().find(|(name, _)| *name == key)
                {
                    (key, wanted)
                } else {
                    (key, value)
                }
            })
            .collect();
        state_from_pairs(block, &merged)
    }
}

/// A state's properties as vanilla's `name=value` strings, in the block's order.
pub(crate) fn property_pairs(properties: Vec<Property>) -> Vec<(&'static str, &'static str)> {
    properties
        .into_iter()
        .map(|property| (property.key().to_str(), property.value_str()))
        .collect()
}

/// The block's state with these `name=value` properties.
fn state_from_pairs(block: &'static Block, pairs: &[(&str, &str)]) -> BlockStateId {
    let properties: Vec<Property> = pairs
        .iter()
        .filter_map(|(key, value)| Property::from_key_value_str(PropertyKey::from_str(key)?, value))
        .collect();
    block.from_properties(&properties).to_state_id(block)
}

/// Whether `state` has the same value as `expected` for each of `properties`.
pub(crate) fn properties_match(
    block: &'static Block,
    expected: BlockStateId,
    state: BlockStateId,
    properties: &[&'static str],
) -> bool {
    if properties.is_empty() {
        return true;
    }
    let (Some(expected), Some(actual)) = (block.properties(expected), block.properties(state))
    else {
        return false;
    };
    let expected = property_pairs(expected.to_props());
    let actual = property_pairs(actual.to_props());
    properties.iter().all(|name| {
        expected.iter().find(|(key, _)| key == name) == actual.iter().find(|(key, _)| key == name)
    })
}

/// Reads a namespaced block id, like vanilla `BlockStateParser.readBlock`.
pub(crate) fn read_block(reader: &mut StringReader) -> Result<&'static Block, CommandSyntaxError> {
    let start = reader.cursor();
    while let Some(c) = reader.peek() {
        if c.is_alphanumeric() || c == '_' || c == ':' || c == '/' || c == '.' || c == '-' {
            reader.skip();
        } else {
            break;
        }
    }
    let block_name = &reader.string()[start..reader.cursor()];
    let normalized = if block_name.contains(':') {
        block_name.to_string()
    } else {
        format!("minecraft:{block_name}")
    };

    Block::from_name(&normalized).ok_or_else(|| {
        reader.set_cursor(start);
        INVALID_BLOCK_ERROR_TYPE.create(reader, TextComponent::text(normalized))
    })
}

/// Vanilla `BlockStateParser.readProperties`: applies `[key=value,...]` to the block's
/// default state, rejecting properties the block lacks, repeats and invalid values.
pub(crate) fn read_properties(
    reader: &mut StringReader,
    block: &'static Block,
) -> Result<(BlockStateId, Vec<&'static str>), CommandSyntaxError> {
    let mut state = block.default_state.id;
    let mut named: Vec<&'static str> = Vec::new();
    if reader.peek() != Some('[') {
        return Ok((state, named));
    }
    let block_id = || TextComponent::text(format!("minecraft:{}", block.name));
    reader.skip();
    reader.skip_whitespace();

    while reader.can_read_char() && reader.peek() != Some(']') {
        reader.skip_whitespace();
        let key_start = reader.cursor();
        let key = reader.read_string()?;
        let current = block
            .properties(state)
            .map(|properties| property_pairs(properties.to_props()))
            .unwrap_or_default();
        let Some(&(name, _)) = current.iter().find(|(name, _)| *name == key) else {
            reader.set_cursor(key_start);
            return Err(ERROR_UNKNOWN_PROPERTY.create(
                reader,
                block_id(),
                TextComponent::text(key),
            ));
        };
        if named.contains(&name) {
            reader.set_cursor(key_start);
            return Err(ERROR_DUPLICATE_PROPERTY.create(
                reader,
                TextComponent::text(key),
                block_id(),
            ));
        }

        reader.skip_whitespace();
        if reader.peek() != Some('=') {
            return Err(ERROR_EXPECTED_VALUE.create(reader, TextComponent::text(key), block_id()));
        }
        reader.skip();
        reader.skip_whitespace();
        let value_start = reader.cursor();
        let value = reader.read_string()?;
        // The state with this value and the other properties unchanged. The generated
        // property setters panic on unknown values, so look it up among the block's states.
        let requested: Vec<(&str, &str)> = current
            .iter()
            .map(|&(key, current)| {
                if key == name {
                    (key, value.as_str())
                } else {
                    (key, current)
                }
            })
            .collect();
        let candidate = block.states.iter().find(|candidate| {
            block
                .properties(candidate.id)
                .is_some_and(|properties| property_pairs(properties.to_props()) == requested)
        });
        let Some(candidate) = candidate.map(|candidate| candidate.id) else {
            reader.set_cursor(value_start);
            return Err(ERROR_INVALID_VALUE.create(
                reader,
                block_id(),
                TextComponent::text(value),
                TextComponent::text(name),
            ));
        };
        state = candidate;
        named.push(name);

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
    Ok((state, named))
}

/// Vanilla `BlockStateParser.readNbt`.
pub(crate) fn read_nbt(
    reader: &mut StringReader,
) -> Result<Option<NbtCompound>, CommandSyntaxError> {
    if reader.peek() == Some('{') {
        let tag =
            ArgumentType::<crate::source::DummySource>::parse(&NbtCompoundArgumentType, reader)?;
        Ok(Some(tag))
    } else {
        Ok(None)
    }
}

/// Vanilla `BlockStateArgument`: `block[properties]{nbt}`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BlockArgumentType;

impl<S: crate::source::CommandSource> ArgumentType<S> for BlockArgumentType {
    type Item = BlockInput;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let block = read_block(reader)?;
        let (state, properties) = read_properties(reader, block)?;
        let nbt = read_nbt(reader)?;
        Ok(BlockInput {
            state,
            properties,
            nbt,
        })
    }

    fn client_side_parser(&'_ self) -> JavaClientArgumentType {
        JavaClientArgumentType::BlockState
    }

    fn examples(&self) -> Vec<String> {
        vec![
            "stone".to_string(),
            "minecraft:stone".to_string(),
            "stone[foo=bar]".to_string(),
            "foo{bar=baz}".to_string(),
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

impl BlockArgumentType {
    pub fn get<S: crate::source::CommandSource>(
        context: &CommandContext<S>,
        name: &str,
    ) -> Result<BlockInput, CommandSyntaxError> {
        Ok(context.get_argument::<BlockInput>(name)?.clone())
    }
}
