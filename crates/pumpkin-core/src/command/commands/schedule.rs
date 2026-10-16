use pumpkin_data::dimension::Dimension;
use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::function::FunctionArgumentType;
use crate::command::argument_types::identifier::IdentifierArgumentType;
use crate::command::argument_types::time::TimeArgumentType;
use crate::command::commands::function::{FunctionSuggestionProvider, get_functions};
use crate::command::context::command_context::CommandContext;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::command::suggestion::provider::{SuggestionProvider, SuggestionProviderResult};
use crate::command::suggestion::suggestions::SuggestionsBuilder;

const DESCRIPTION: &str = "Schedules an action or function to execute after a given duration.";
const PERMISSION: &str = "minecraft:command.schedule";

pub const ERROR_SAME_TICK: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_SCHEDULE_SAME_TICK,
    translation::java::COMMANDS_SCHEDULE_SAME_TICK,
);

pub const ERROR_CANT_REMOVE: CommandErrorType<1> = CommandErrorType::new(
    translation::java::COMMANDS_SCHEDULE_CLEARED_FAILURE,
    translation::java::COMMANDS_SCHEDULE_CLEARED_FAILURE,
);

#[expect(dead_code)]
pub const ERROR_MACRO: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_SCHEDULE_MACRO,
    translation::java::COMMANDS_SCHEDULE_MACRO,
);

struct ScheduleSuggestionProvider;

impl SuggestionProvider for ScheduleSuggestionProvider {
    fn suggest(
        &self,
        context: &CommandContext,
        mut builder: SuggestionsBuilder,
    ) -> SuggestionProviderResult {
        let server = context.server();
        for id in server.scheduled_functions.get_event_ids() {
            builder = builder.suggest(id);
        }
        builder.build()
    }
}

fn execute_schedule(context: &CommandContext, replace: bool) -> Result<i32, CommandSyntaxError> {
    let function_name = FunctionArgumentType::get(context, "function")?.to_string();
    // Vanilla FunctionArgument.getFunctionOrTag: a function must exist, a tag may be empty.
    if !function_name.starts_with('#') {
        get_functions(context, &function_name)?;
    }
    let time = TimeArgumentType::get(context, "time")?;

    if time == 0 {
        return Err(ERROR_SAME_TICK.create_without_context());
    }

    let server = context.server();
    // `source.getLevel().getGameTime()`: every dimension shares the overworld's game time.
    let game_time = server
        .get_world_from_dimension(&Dimension::OVERWORLD)
        .level
        .game_time();
    let tick_time = game_time + i64::from(time);
    let is_tag = function_name.starts_with('#');
    let callback_id = function_name.trim_start_matches('#').to_string();

    server.scheduled_functions.schedule(
        function_name.clone(),
        tick_time as u64,
        function_name,
        is_tag,
        replace,
    );

    if is_tag {
        context.source.send_feedback(
            TextComponent::translate_cross(
                translation::java::COMMANDS_SCHEDULE_CREATED_TAG,
                translation::java::COMMANDS_SCHEDULE_CREATED_TAG,
                [
                    TextComponent::text(callback_id),
                    TextComponent::text(time.to_string()),
                    TextComponent::text(tick_time.to_string()),
                ],
            ),
            true,
        );
    } else {
        context.source.send_feedback(
            TextComponent::translate_cross(
                translation::java::COMMANDS_SCHEDULE_CREATED_FUNCTION,
                translation::java::COMMANDS_SCHEDULE_CREATED_FUNCTION,
                [
                    TextComponent::text(callback_id),
                    TextComponent::text(time.to_string()),
                    TextComponent::text(tick_time.to_string()),
                ],
            ),
            true,
        );
    }

    Ok(tick_time.rem_euclid(i64::from(i32::MAX)) as i32)
}

struct ScheduleExecutor {
    replace: bool,
}

impl CommandExecutor for ScheduleExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        execute_schedule(context, self.replace)
    }
}

struct ClearExecutor;

impl CommandExecutor for ClearExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let function_name = IdentifierArgumentType::get(context, "function")?.to_string();
        let server = context.server();
        let count = server.scheduled_functions.remove(&function_name);

        if count == 0 {
            return Err(
                ERROR_CANT_REMOVE.create_without_context(TextComponent::text(function_name))
            );
        }

        context.source.send_feedback(
            TextComponent::translate_cross(
                translation::java::COMMANDS_SCHEDULE_CLEARED_SUCCESS,
                translation::java::COMMANDS_SCHEDULE_CLEARED_SUCCESS,
                [
                    TextComponent::text(count.to_string()),
                    TextComponent::text(function_name),
                ],
            ),
            true,
        );

        Ok(count as i32)
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("schedule", DESCRIPTION)
            .requires(PERMISSION)
            .then(
                literal("function").then(
                    argument("function", FunctionArgumentType)
                        .suggests(FunctionSuggestionProvider)
                        .then(
                            argument("time", TimeArgumentType::any())
                                .executes(ScheduleExecutor { replace: true })
                                .then(
                                    literal("append").executes(ScheduleExecutor { replace: false }),
                                )
                                .then(
                                    literal("replace").executes(ScheduleExecutor { replace: true }),
                                ),
                        ),
                ),
            )
            .then(
                literal("clear").then(
                    argument("function", IdentifierArgumentType)
                        .suggests(ScheduleSuggestionProvider)
                        .executes(ClearExecutor),
                ),
            ),
    );
}
