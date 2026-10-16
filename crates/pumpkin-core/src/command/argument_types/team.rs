use crate::command::{
    CommandSource,
    argument_types::argument_type::{ArgumentType, JavaClientArgumentType},
    context::command_context::CommandContext,
    errors::command_syntax_error::CommandSyntaxError,
    errors::error_types::CommandErrorType,
    string_reader::StringReader,
    suggestion::suggestions::{Suggestions, SuggestionsBuilder},
};
use crate::world::scoreboard::{Scoreboard, Team};
use pumpkin_data::translation;
use pumpkin_util::text::TextComponent;

const ERROR_TEAM_NOT_FOUND: CommandErrorType<1> = CommandErrorType::new(
    translation::java::TEAM_NOTFOUND,
    translation::java::TEAM_NOTFOUND,
);

/// Represents an argument type parsing a team name.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct TeamArgumentType;

impl ArgumentType<CommandSource> for TeamArgumentType {
    type Item = String;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let name = reader.read_unquoted_string();
        Ok(name)
    }

    fn client_side_parser(&'_ self) -> JavaClientArgumentType {
        JavaClientArgumentType::Team
    }

    fn list_suggestions(
        &self,
        context: &CommandContext,
        mut builder: SuggestionsBuilder,
    ) -> Suggestions {
        let scoreboard = context
            .world()
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for team_name in scoreboard.get_teams().keys() {
            builder = builder.filter_and_suggest_one(team_name.as_str());
        }
        builder.build()
    }

    fn examples(&self) -> Vec<String> {
        vec!["foo".to_string(), "foo_bar".to_string()]
    }
}

impl TeamArgumentType {
    /// Returns a [`CommandContext`]'s parsed `String` argument as a string slice.
    pub fn get<'a>(context: &'a CommandContext, name: &str) -> Result<&'a str, CommandSyntaxError> {
        Ok(context.get_argument::<String>(name)?.as_str())
    }

    /// Vanilla `TeamArgument.getTeam`.
    pub fn get_team<'a>(
        context: &CommandContext,
        name: &str,
        scoreboard: &'a Scoreboard,
    ) -> Result<&'a Team, CommandSyntaxError> {
        let team_name = Self::get(context, name)?;
        scoreboard.get_team(team_name).ok_or_else(|| {
            ERROR_TEAM_NOT_FOUND.create_without_context(TextComponent::text(team_name.to_string()))
        })
    }
}
