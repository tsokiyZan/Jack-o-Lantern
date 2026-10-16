use crate::command::CommandSource;
use crate::command::argument_types::argument_type::{ArgumentType, JavaClientArgumentType};
use crate::command::context::command_context::CommandContext;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::string_reader::StringReader;
use crate::command::suggestion::suggestions::{Suggestions, SuggestionsBuilder};
use pumpkin_data::translation;
use pumpkin_util::text::TextComponent;

const ERROR_INVALID_VALUE: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENT_ENUM_INVALID,
    translation::java::ARGUMENT_ENUM_INVALID,
);

/// The heightmaps vanilla keeps after world generation (`Heightmap.Types.keepAfterWorldgen`).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HeightmapType {
    WorldSurface,
    OceanFloor,
    MotionBlocking,
    MotionBlockingNoLeaves,
}

impl HeightmapType {
    const ALL: [Self; 4] = [
        Self::WorldSurface,
        Self::OceanFloor,
        Self::MotionBlocking,
        Self::MotionBlockingNoLeaves,
    ];

    #[must_use]
    pub const fn serialized_name(self) -> &'static str {
        match self {
            Self::WorldSurface => "world_surface",
            Self::OceanFloor => "ocean_floor",
            Self::MotionBlocking => "motion_blocking",
            Self::MotionBlockingNoLeaves => "motion_blocking_no_leaves",
        }
    }
}

/// Vanilla `HeightmapTypeArgument`.
pub struct HeightmapTypeArgumentType;

impl ArgumentType<CommandSource> for HeightmapTypeArgumentType {
    type Item = HeightmapType;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let id = reader.read_unquoted_string();
        HeightmapType::ALL
            .into_iter()
            .find(|heightmap| heightmap.serialized_name() == id)
            .ok_or_else(|| ERROR_INVALID_VALUE.create(reader, TextComponent::text(id)))
    }

    fn client_side_parser(&self) -> JavaClientArgumentType {
        JavaClientArgumentType::Heightmap
    }

    fn list_suggestions(
        &self,
        _context: &CommandContext,
        builder: SuggestionsBuilder,
    ) -> Suggestions {
        builder
            .filter_and_suggest_iter(
                HeightmapType::ALL
                    .into_iter()
                    .map(HeightmapType::serialized_name),
            )
            .build()
    }

    fn examples(&self) -> Vec<String> {
        examples!("world_surface", "motion_blocking")
    }
}

impl HeightmapTypeArgumentType {
    pub fn get(context: &CommandContext, name: &str) -> Result<HeightmapType, CommandSyntaxError> {
        context.get_argument::<HeightmapType>(name).copied()
    }
}
