use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};

use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::integer::IntegerArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult, RedirectModifier, Redirection};
use pumpkin_command::execution;
use pumpkin_command::source::{CommandSource as _, ReturnValue};

const DESCRIPTION: &str = "Controls execution flow in functions and sets return values.";
const PERMISSION: &str = "minecraft:command.return";

/// Vanilla `ReturnCommand.ReturnValueCustomExecutor`.
struct ReturnValueExecutor;

impl CommandExecutor for ReturnValueExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let value = IntegerArgumentType::get(context, "value")?;
        context.source.call_result(ReturnValue::Success(value));
        if let Some(frame) = execution::current_frame() {
            frame.return_success(value);
            frame.discard();
        }
        Ok(value)
    }

    fn reports_own_result(&self) -> bool {
        true
    }
}

/// Vanilla `ReturnCommand.ReturnFailCustomExecutor`.
struct ReturnFailExecutor;

impl CommandExecutor for ReturnFailExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        context.source.call_result(ReturnValue::Failure);
        if let Some(frame) = execution::current_frame() {
            frame.return_failure();
            frame.discard();
        }
        Ok(0)
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
        command("return", DESCRIPTION)
            .requires(PERMISSION)
            .then(argument("value", IntegerArgumentType::any()).executes(ReturnValueExecutor))
            .then(literal("fail").executes(ReturnFailExecutor))
            .then(
                literal("run")
                    .redirect_with_modifier(Redirection::Root, RedirectModifier::ReturnRun),
            ),
    );
}
