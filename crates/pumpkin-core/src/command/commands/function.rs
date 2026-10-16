use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

use crate::command::CommandSource;
use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::function::FunctionArgumentType;
use crate::command::argument_types::nbt::NbtCompoundArgumentType;
use crate::command::argument_types::nbt_path::{NbtPath, NbtPathArgumentType};
use crate::command::commands::data::{DataAccessor, TargetKind, get_single_tag, make_target_arg};
use crate::command::context::command_context::CommandContext;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::command::suggestion::provider::{SuggestionProvider, SuggestionProviderResult};
use crate::command::suggestion::suggestions::SuggestionsBuilder;
use crate::data::datapack::function_loader::FunctionInstantiationError;
use crate::data::datapack::{DatapackManager, FunctionLookupError, ResolvedFunction};
use crate::server::Server;
use pumpkin_command::execution::{self, ChainModifiers};
use pumpkin_command::source::{ResultValueTaker, ReturnValue};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use std::sync::{Arc, Mutex};

const DESCRIPTION: &str = "Runs commands found in the corresponding function files.";
const PERMISSION: &str = "minecraft:command.function";

const ARG_NAME: &str = "name";
const ARG_ARGUMENTS: &str = "arguments";
const ARG_SOURCE: &str = "source";
const ARG_PATH: &str = "path";

static ERROR_UNKNOWN_FUNCTION: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENTS_FUNCTION_UNKNOWN,
    translation::java::ARGUMENTS_FUNCTION_UNKNOWN,
);
static ERROR_NO_FUNCTIONS: CommandErrorType<1> = CommandErrorType::new(
    translation::java::COMMANDS_FUNCTION_SCHEDULED_NO_FUNCTIONS,
    translation::java::COMMANDS_FUNCTION_SCHEDULED_NO_FUNCTIONS,
);
static ERROR_ARGUMENT_NOT_COMPOUND: CommandErrorType<1> = CommandErrorType::new(
    translation::java::COMMANDS_FUNCTION_ERROR_ARGUMENT_NOT_COMPOUND,
    translation::java::COMMANDS_FUNCTION_ERROR_ARGUMENT_NOT_COMPOUND,
);
static ERROR_FUNCTION_INSTANTATION_FAILURE: CommandErrorType<2> = CommandErrorType::new(
    translation::java::COMMANDS_FUNCTION_INSTANTIATIONFAILURE,
    translation::java::COMMANDS_FUNCTION_INSTANTIATIONFAILURE,
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

/// A function's id and the commands vanilla's `InstantiatedFunction` holds.
type InstantiatedFunction = (String, Arc<[String]>);

/// Vanilla `FunctionCommand.instantiateAndQueueFunctions`' instantiation for every function, done
/// before any of them runs. Stops at the first function that fails, with the ones before it.
fn instantiate_functions(
    server: &Server,
    functions: Vec<ResolvedFunction>,
    arguments: Option<&NbtCompound>,
    function_source: &CommandSource,
) -> (Vec<InstantiatedFunction>, Option<CommandSyntaxError>) {
    let mut instantiated = Vec::with_capacity(functions.len());
    for (id, function) in functions {
        match DatapackManager::instantiate(server, &function, arguments, function_source) {
            Ok(lines) => instantiated.push((id, lines)),
            Err(FunctionInstantiationError(reason)) => {
                let error = ERROR_FUNCTION_INSTANTATION_FAILURE
                    .create_without_context(TextComponent::text(id), reason);
                return (instantiated, Some(error));
            }
        }
    }
    (instantiated, None)
}

/// Vanilla `FunctionCommand.queueFunctions`.
fn queue_functions(
    server: &Arc<Server>,
    functions: &[InstantiatedFunction],
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

/// Where a `/function` call takes its macro arguments from.
#[derive(Clone, Copy)]
enum ArgumentsSource {
    None,
    /// The `arguments` compound.
    Compound,
    /// `with <block|entity|storage> <source> [<path>]`.
    With {
        kind: TargetKind,
        has_path: bool,
    },
}

/// Vanilla `FunctionCommand.getArgumentTag`.
fn get_argument_tag(
    path: &NbtPath,
    accessor: &dyn DataAccessor,
) -> Result<NbtCompound, CommandSyntaxError> {
    match get_single_tag(path, accessor)? {
        NbtTag::Compound(compound) => Ok(compound),
        tag => Err(ERROR_ARGUMENT_NOT_COMPOUND
            .create_without_context(TextComponent::text(tag.get_type_name()))),
    }
}

/// Vanilla `FunctionCommand.FunctionCustomExecutor.runGuarded`.
struct FunctionExecutor(ArgumentsSource);

impl FunctionExecutor {
    fn arguments(
        &self,
        context: &CommandContext,
    ) -> Result<Option<NbtCompound>, CommandSyntaxError> {
        Ok(match self.0 {
            ArgumentsSource::None => None,
            ArgumentsSource::Compound => {
                Some(NbtCompoundArgumentType::get(context, ARG_ARGUMENTS)?.clone())
            }
            ArgumentsSource::With { kind, has_path } => {
                let accessor = kind.access(context, ARG_SOURCE)?;
                Some(if has_path {
                    get_argument_tag(
                        NbtPathArgumentType::get(context, ARG_PATH)?,
                        accessor.as_ref(),
                    )?
                } else {
                    accessor.get_data()?
                })
            }
        })
    }
}

impl CommandExecutor for FunctionExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let name = FunctionArgumentType::get(context, ARG_NAME)?;
        let functions = get_functions(context, name)?;
        if functions.is_empty() {
            return Err(
                ERROR_NO_FUNCTIONS.create_without_context(TextComponent::text(
                    name.strip_prefix('#').unwrap_or(name).to_string(),
                )),
            );
        }

        let arguments = self.arguments(context)?;
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
        let function_count = functions.len();
        let (functions, failure) = instantiate_functions(
            context.server(),
            functions,
            arguments.as_ref(),
            &function_source,
        );
        if let Some(error) = &failure {
            // Vanilla throws out of runGuarded once a function fails to instantiate; the
            // functions queued before it still run, after the error is reported.
            if !context.chain_modifiers.is_forked() {
                context.source.send_error(error.message.clone());
            }
            context
                .source
                .command_result_taker
                .call(ReturnValue::Failure);
        }
        queue_functions(
            context.server(),
            &functions,
            &context.source,
            &function_source,
            context.chain_modifiers,
        );
        Ok(if failure.is_some() {
            0
        } else {
            function_count as i32
        })
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

    let mut with = literal("with");
    for (name, kind) in [
        ("block", TargetKind::Block),
        ("entity", TargetKind::Entity),
        ("storage", TargetKind::Storage),
    ] {
        with = with.then(
            literal(name).then(
                make_target_arg(kind, ARG_SOURCE)
                    .executes(FunctionExecutor(ArgumentsSource::With {
                        kind,
                        has_path: false,
                    }))
                    .then(
                        argument(ARG_PATH, NbtPathArgumentType).executes(FunctionExecutor(
                            ArgumentsSource::With {
                                kind,
                                has_path: true,
                            },
                        )),
                    ),
            ),
        );
    }

    dispatcher.register(
        command("function", DESCRIPTION).requires(PERMISSION).then(
            argument(ARG_NAME, FunctionArgumentType)
                .suggests(FunctionSuggestionProvider)
                .executes(FunctionExecutor(ArgumentsSource::None))
                .then(
                    argument(ARG_ARGUMENTS, NbtCompoundArgumentType)
                        .executes(FunctionExecutor(ArgumentsSource::Compound)),
                )
                .then(with),
        ),
    );
}
