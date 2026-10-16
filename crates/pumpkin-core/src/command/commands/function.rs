use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

use crate::command::CommandSource;
use crate::command::argument_builder::{ArgumentBuilder, argument, command};
use crate::command::argument_types::function::FunctionArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::command::suggestion::provider::{SuggestionProvider, SuggestionProviderResult};
use crate::command::suggestion::suggestions::SuggestionsBuilder;
use crate::data::datapack::{DatapackManager, FunctionLookupError, ResolvedFunction};
use crate::server::Server;
use pumpkin_command::execution::{self, ChainModifiers};
use pumpkin_command::source::{ResultValueTaker, ReturnValue};
use std::sync::{Arc, Mutex};

const DESCRIPTION: &str = "Runs commands found in the corresponding function files.";
const PERMISSION: &str = "minecraft:command.function";

static ERROR_UNKNOWN_FUNCTION: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENTS_FUNCTION_UNKNOWN,
    translation::java::ARGUMENTS_FUNCTION_UNKNOWN,
);
static ERROR_NO_FUNCTIONS: CommandErrorType<1> = CommandErrorType::new(
    translation::java::COMMANDS_FUNCTION_SCHEDULED_NO_FUNCTIONS,
    translation::java::COMMANDS_FUNCTION_SCHEDULED_NO_FUNCTIONS,
);

pub struct FunctionSuggestionProvider;

impl SuggestionProvider for FunctionSuggestionProvider {
    fn suggest(
        &self,
        context: &CommandContext,
        mut builder: SuggestionsBuilder,
    ) -> SuggestionProviderResult {
        let server = context.server();
        let function_names = server.datapack_manager.get_function_names();
        for name in function_names {
            builder = builder.suggest(name);
        }
        builder.build()
    }
}

/// Vanilla `FunctionArgument.getFunctions`, with its unknown function error.
pub fn get_functions(
    context: &CommandContext,
    name: &str,
) -> Result<Vec<ResolvedFunction>, CommandSyntaxError> {
    context
        .server()
        .datapack_manager
        .resolve_functions(name)
        .map_err(|FunctionLookupError(id)| {
            ERROR_UNKNOWN_FUNCTION.create_without_context(TextComponent::text(id))
        })
}

/// Vanilla `FunctionCommand.modifySenderForExecution`.
#[must_use]
pub fn modify_sender_for_execution(sender: &CommandSource) -> CommandSource {
    sender.clone().with_silent()
}

/// Vanilla `FunctionCommand.decorateOutputIfNeeded` with `FULL_CONTEXT_CALLBACKS`.
fn decorate_output_if_needed(
    original_source: &Arc<CommandSource>,
    id: &str,
    callback: ResultValueTaker,
) -> ResultValueTaker {
    if original_source.silent {
        return callback;
    }
    let original_source = original_source.clone();
    let id = id.to_string();
    ResultValueTaker::from_fn(move |result| {
        original_source.send_feedback(
            TextComponent::translate_cross(
                translation::java::COMMANDS_FUNCTION_RESULT,
                translation::java::COMMANDS_FUNCTION_RESULT,
                [
                    TextComponent::text(id.clone()),
                    TextComponent::text(result.result_value().to_string()),
                ],
            ),
            true,
        );
        callback.call(result);
    })
}

/// Vanilla `FunctionCommand.queueFunctions`.
fn queue_functions(
    server: &Arc<Server>,
    functions: &[ResolvedFunction],
    original_source: &Arc<CommandSource>,
    function_source: &CommandSource,
    modifiers: ChainModifiers,
) {
    let no_callback_source = function_source
        .clone()
        .with_command_result_taker(ResultValueTaker::new());
    let original_callback = original_source.command_result_taker.clone();

    if modifiers.is_return() {
        // Vanilla queueFunctionsAsReturn: the functions return from the current frame, and
        // the frame falls through if none of them does.
        let Some(frame) = execution::current_frame() else {
            return;
        };
        let output_callback =
            ResultValueTaker::merge(&original_callback, frame.return_value_consumer());
        let generation = frame.generation();
        for (id, lines) in functions {
            if frame.generation() != generation {
                return;
            }
            let collector = decorate_output_if_needed(original_source, id, output_callback.clone());
            DatapackManager::call_function(server, lines, &no_callback_source, collector, true);
        }
        if frame.generation() == generation {
            frame.return_failure();
            frame.discard();
        }
        return;
    }

    if functions.len() == 1 || original_callback.is_empty() {
        for (id, lines) in functions {
            let collector =
                decorate_output_if_needed(original_source, id, original_callback.clone());
            DatapackManager::call_function(server, lines, &no_callback_source, collector, false);
        }
        return;
    }

    let accumulator = Arc::new(Mutex::new(None::<i32>));
    let partial = {
        let accumulator = accumulator.clone();
        ResultValueTaker::from_fn(move |result| {
            let mut sum = accumulator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *sum = Some(sum.unwrap_or(0).wrapping_add(result.result_value()));
        })
    };
    for (id, lines) in functions {
        let collector = decorate_output_if_needed(original_source, id, partial.clone());
        DatapackManager::call_function(server, lines, &no_callback_source, collector, false);
    }
    let sum = *accumulator
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(sum) = sum {
        original_callback.call(ReturnValue::Success(sum));
    }
}

/// Vanilla `FunctionCommand.FunctionCustomExecutor.runGuarded`.
struct FunctionExecutor;

impl CommandExecutor for FunctionExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let name = FunctionArgumentType::get(context, "name")?;
        let functions = get_functions(context, name)?;
        if functions.is_empty() {
            return Err(
                ERROR_NO_FUNCTIONS.create_without_context(TextComponent::text(
                    name.strip_prefix('#').unwrap_or(name).to_string(),
                )),
            );
        }

        let function_source = modify_sender_for_execution(&context.source);
        let feedback = if let [(id, _)] = functions.as_slice() {
            TextComponent::translate_cross(
                translation::java::COMMANDS_FUNCTION_SCHEDULED_SINGLE,
                translation::java::COMMANDS_FUNCTION_SCHEDULED_SINGLE,
                [TextComponent::text(id.clone())],
            )
        } else {
            TextComponent::translate_cross(
                translation::java::COMMANDS_FUNCTION_SCHEDULED_MULTIPLE,
                translation::java::COMMANDS_FUNCTION_SCHEDULED_MULTIPLE,
                [TextComponent::join_with_comma(
                    functions
                        .iter()
                        .map(|(id, _)| TextComponent::text(id.clone()))
                        .collect(),
                )],
            )
        };
        context.source.send_feedback(feedback, true);

        if let (Some(frame), Some(generation)) =
            (execution::current_frame(), context.frame_generation)
            && frame.generation() != generation
        {
            // What this source queued was discarded by an earlier source's function.
            return Ok(0);
        }
        queue_functions(
            context.server(),
            &functions,
            &context.source,
            &function_source,
            context.chain_modifiers,
        );
        Ok(functions.len() as i32)
    }

    fn reports_own_result(&self) -> bool {
        true
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("function", DESCRIPTION).requires(PERMISSION).then(
            argument("name", FunctionArgumentType)
                .suggests(FunctionSuggestionProvider)
                .executes(FunctionExecutor),
        ),
    );
}
