use crate::argument_types::argument_type::{ArgumentType, JavaClientArgumentType};
use crate::context::command_context::CommandContext;
use crate::errors::command_syntax_error::CommandSyntaxError;
use crate::errors::error_types::CommandErrorType;
use crate::string_reader::StringReader;
use crate::suggestion::suggestions::{Suggestions, SuggestionsBuilder};
use pumpkin_data::data_component_impl::SwingAnimationType;
use pumpkin_data::translation::java::ARGUMENT_SWING_ANIMATION_INVALID;
use pumpkin_util::text::TextComponent;

pub const ERROR_INVALID: CommandErrorType<1> = CommandErrorType::new(
    ARGUMENT_SWING_ANIMATION_INVALID,
    ARGUMENT_SWING_ANIMATION_INVALID,
);

/// Vanilla `SwingAnimationArgument`.
pub struct SwingAnimationArgumentType;

impl<S: crate::source::CommandSource> ArgumentType<S> for SwingAnimationArgumentType {
    type Item = SwingAnimationType;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let name = reader.read_unquoted_string();
        SwingAnimationType::from_name(&name)
            .ok_or_else(|| ERROR_INVALID.create(reader, TextComponent::text(name)))
    }

    fn client_side_parser(&'_ self) -> JavaClientArgumentType {
        JavaClientArgumentType::SwingAnimation
    }

    fn list_suggestions(
        &self,
        _context: &CommandContext<S>,
        builder: SuggestionsBuilder,
    ) -> Suggestions {
        builder
            .filter_and_suggest(&SwingAnimationType::VALUES.map(|value| value.to_name()))
            .build()
    }

    fn examples(&self) -> Vec<String> {
        examples!("whack", "stab")
    }
}

impl_copy_get!(SwingAnimationArgumentType, SwingAnimationType);
