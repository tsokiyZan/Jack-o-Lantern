use pumpkin_data::world::SAY_COMMAND;

use crate::command::argument_builder::{ArgumentBuilder, argument, command};
use crate::command::argument_types::message::MessageArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};

const NAME: &str = "say";

const DESCRIPTION: &str = "Broadcast a message to all Players.";
const PERMISSION: &str = "minecraft:command.say";
const ARG_MESSAGE: &str = "message";

struct Executor;

impl CommandExecutor for Executor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let msg = MessageArgumentType::get_message(context, ARG_MESSAGE)?;

        context
            .server()
            .broadcast_message(&msg, &context.source.display_name, SAY_COMMAND, None);

        Ok(1)
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));
    dispatcher.register(
        command(NAME, DESCRIPTION)
            .requires(PERMISSION)
            .then(argument(ARG_MESSAGE, MessageArgumentType).executes(Executor)),
    );
}
