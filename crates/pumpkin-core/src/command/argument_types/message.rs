use pumpkin_data::translation;
use pumpkin_util::text::TextComponent;

use crate::command::CommandSource;
use crate::command::argument_types::argument_type::{ArgumentType, JavaClientArgumentType};
use crate::command::argument_types::entity::ENTITY_SELECTOR_PERMISSION;
use crate::command::argument_types::entity_selector::EntitySelector;
use crate::command::argument_types::entity_selector::parser::{
    EntitySelectorParser, MISSING_SELECTOR_TYPE_ERROR_TYPE, UNKNOWN_SELECTOR_TYPE_ERROR_TYPE,
};
use crate::command::context::command_context::CommandContext;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::string_reader::StringReader;

const TOO_LONG_ERROR_TYPE: CommandErrorType<2> = CommandErrorType::new(
    translation::java::ARGUMENT_MESSAGE_TOO_LONG,
    translation::java::ARGUMENT_MESSAGE_TOO_LONG,
);

/// `MessageArgument`'s length limit, in UTF-16 code units.
const MAX_LENGTH: usize = 256;

/// A selector inside a message, at byte offsets into [`Message::text`].
pub struct Part {
    start: usize,
    end: usize,
    selector: EntitySelector,
}

impl Part {
    /// Vanilla `Part.toComponent`: `EntitySelector.joinNames` of what the selector finds.
    fn to_component(&self, source: &CommandSource) -> Result<TextComponent, CommandSyntaxError> {
        let names = self
            .selector
            .find_entities(source)?
            .iter()
            .map(|entity| entity.get_display_name())
            .collect();
        Ok(TextComponent::join_with_comma(names))
    }
}

/// Vanilla `MessageArgument.Message`: the rest of the command, with the selectors in it.
pub struct Message {
    pub text: String,
    parts: Vec<Part>,
}

impl Message {
    /// Vanilla `Message.resolveComponent`.
    pub fn resolve_component(
        &self,
        source: &CommandSource,
    ) -> Result<TextComponent, CommandSyntaxError> {
        self.to_component(source, source.has_permission(ENTITY_SELECTOR_PERMISSION))
    }

    /// Vanilla `Message.toComponent`.
    pub fn to_component(
        &self,
        source: &CommandSource,
        allow_selectors: bool,
    ) -> Result<TextComponent, CommandSyntaxError> {
        let Some(first) = self.parts.first().filter(|_| allow_selectors) else {
            return Ok(TextComponent::text(self.text.clone()));
        };
        let mut result = TextComponent::text(self.text[..first.start].to_string());
        let mut read_to = first.start;
        for part in &self.parts {
            let component = part.to_component(source)?;
            if read_to < part.start {
                result = result.add_child(TextComponent::text(
                    self.text[read_to..part.start].to_string(),
                ));
            }
            result = result.add_child(component);
            read_to = part.end;
        }
        if read_to < self.text.len() {
            result = result.add_child(TextComponent::text(self.text[read_to..].to_string()));
        }
        Ok(result)
    }

    /// Vanilla `Message.parseText`.
    fn parse_text(
        reader: &mut StringReader,
        allow_selectors: bool,
    ) -> Result<Self, CommandSyntaxError> {
        let length = reader.remaining_part().encode_utf16().count();
        if length > MAX_LENGTH {
            return Err(TOO_LONG_ERROR_TYPE.create_without_context(
                TextComponent::text(length.to_string()),
                TextComponent::text(MAX_LENGTH.to_string()),
            ));
        }
        let text = reader.remaining_part().to_string();
        if !allow_selectors {
            reader.set_cursor(reader.total_length());
            return Ok(Self {
                text,
                parts: Vec::new(),
            });
        }
        let mut parts = Vec::new();
        let offset = reader.cursor();
        while let Some(c) = reader.peek() {
            if c != '@' {
                reader.skip();
                continue;
            }
            let start = reader.cursor();
            match EntitySelectorParser::new(reader, true).parse_and_consume() {
                Ok(selector) => parts.push(Part {
                    start: start - offset,
                    end: reader.cursor() - offset,
                    selector,
                }),
                Err(error)
                    if error.is(&MISSING_SELECTOR_TYPE_ERROR_TYPE)
                        || error.is(&UNKNOWN_SELECTOR_TYPE_ERROR_TYPE) =>
                {
                    reader.set_cursor(start + 1);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(Self { text, parts })
    }
}

/// Vanilla `MessageArgument`.
pub struct MessageArgumentType;

impl ArgumentType<CommandSource> for MessageArgumentType {
    type Item = Message;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        Message::parse_text(reader, true)
    }

    fn parse_with_source(
        &self,
        reader: &mut StringReader,
        source: &CommandSource,
    ) -> Result<Self::Item, CommandSyntaxError> {
        Message::parse_text(reader, source.has_permission(ENTITY_SELECTOR_PERMISSION))
    }

    fn client_side_parser(&'_ self) -> JavaClientArgumentType {
        JavaClientArgumentType::Message
    }

    fn examples(&self) -> Vec<String> {
        examples!("Hello world!", "foo", "@e", "Hello @p :)")
    }
}

impl MessageArgumentType {
    /// Vanilla `MessageArgument.getMessage`.
    pub fn get_message(
        context: &CommandContext,
        name: &str,
    ) -> Result<TextComponent, CommandSyntaxError> {
        context
            .get_argument::<Message>(name)?
            .resolve_component(context.source.as_ref())
    }
}
