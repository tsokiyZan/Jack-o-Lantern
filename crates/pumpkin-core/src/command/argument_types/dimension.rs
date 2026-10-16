use crate::command::CommandSource;
use crate::command::argument_types::FromStringReader;
use crate::command::argument_types::argument_type::{ArgumentType, JavaClientArgumentType};
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::string_reader::StringReader;
use pumpkin_util::identifier::Identifier;
use pumpkin_util::resource::ResourceKey;

pub static DIMENSION_REGISTRY: &Identifier = &Identifier::vanilla_static("dimension");

/// Vanilla `DimensionArgument`: an identifier, checked against the loaded dimensions when used.
pub struct DimensionArgumentType;

impl ArgumentType<CommandSource> for DimensionArgumentType {
    type Item = ResourceKey;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        Ok(ResourceKey::new(
            DIMENSION_REGISTRY.clone(),
            Identifier::from_reader(reader)?,
        ))
    }

    fn client_side_parser(&self) -> JavaClientArgumentType {
        JavaClientArgumentType::Dimension
    }

    fn examples(&self) -> Vec<String> {
        examples!("world", "nether")
    }
}
