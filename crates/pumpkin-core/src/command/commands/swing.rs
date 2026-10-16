use std::sync::Arc;

use crate::command::argument_builder::{
    ArgumentBuilder, LiteralArgumentBuilder, argument, command, literal,
};
use crate::command::argument_types::entity::EntityArgumentType;
use crate::command::argument_types::swing_animation::SwingAnimationArgumentType;
use crate::command::argument_types::time::TimeArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::context::command_source::CommandSource;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::command::response_tracker::CommandResponseTracker;
use crate::entity::EntityBase;
use pumpkin_data::data_component_impl::SwingAnimationImpl;
use pumpkin_data::translation;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use pumpkin_util::{Hand, PermissionLvl};

const DESCRIPTION: &str = "Causes entities to swing their arm.";

const PERMISSION: &str = "minecraft:command.swing";

const ARG_TARGETS: &str = "targets";
const ARG_ANIMATION: &str = "animation";
const ARG_DURATION: &str = "duration";

const ERROR_NO_LIVING_ENTITY: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_SWING_FAILED_NOTLIVING,
    translation::java::COMMANDS_SWING_FAILED_NOTLIVING,
);

fn swing(
    source: &CommandSource,
    targets: &[Arc<dyn EntityBase>],
    hand: Hand,
    animation: SwingAnimationImpl,
) -> CommandExecutorResult {
    let mut tracker = CommandResponseTracker::default();
    for entity in targets {
        if let Some(living) = entity.get_living_entity() {
            living.swing(hand, animation, true);
            tracker.track(entity, true);
        }
    }
    tracker.throw_if_zero(&ERROR_NO_LIVING_ENTITY)?;
    Ok(tracker.send_feedback(
        source,
        true,
        |entity, _| {
            TextComponent::translate_cross(
                translation::java::COMMANDS_SWING_SUCCESS_SINGLE,
                translation::java::COMMANDS_SWING_SUCCESS_SINGLE,
                [entity.get_display_name()],
            )
        },
        |entity_count, _| {
            TextComponent::translate_cross(
                translation::java::COMMANDS_SWING_SUCCESS_MULTIPLE,
                translation::java::COMMANDS_SWING_SUCCESS_MULTIPLE,
                [TextComponent::text(entity_count.to_string())],
            )
        },
    ))
}

struct SelfExecutor;

impl CommandExecutor for SelfExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let target = context.source.entity_or_err()?;
        swing(
            &context.source,
            &[target],
            Hand::Right,
            SwingAnimationImpl::DEFAULT,
        )
    }
}

/// Swings the targets' `hand`, with the parsed animation and duration where given.
struct TargetsExecutor {
    hand: Hand,
    has_animation: bool,
    has_duration: bool,
}

impl CommandExecutor for TargetsExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let targets = EntityArgumentType::get_entities(context, ARG_TARGETS)?;
        let mut animation = SwingAnimationImpl::DEFAULT;
        if self.has_animation {
            animation.animation_type = SwingAnimationArgumentType::get(context, ARG_ANIMATION)?;
        }
        if self.has_duration {
            animation.duration = TimeArgumentType::get(context, ARG_DURATION)?;
        }
        swing(&context.source, &targets, self.hand, animation)
    }
}

fn hand_swing(name: &'static str, hand: Hand) -> LiteralArgumentBuilder {
    literal(name)
        .executes(TargetsExecutor {
            hand,
            has_animation: false,
            has_duration: false,
        })
        .then(
            argument(ARG_ANIMATION, SwingAnimationArgumentType)
                .executes(TargetsExecutor {
                    hand,
                    has_animation: true,
                    has_duration: false,
                })
                .then(
                    argument(ARG_DURATION, TimeArgumentType::new(1)).executes(TargetsExecutor {
                        hand,
                        has_animation: true,
                        has_duration: true,
                    }),
                ),
        )
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("swing", DESCRIPTION)
            .requires(PERMISSION)
            .executes(SelfExecutor)
            .then(
                argument(ARG_TARGETS, EntityArgumentType::Entities)
                    .executes(TargetsExecutor {
                        hand: Hand::Right,
                        has_animation: false,
                        has_duration: false,
                    })
                    .then(hand_swing("mainhand", Hand::Right))
                    .then(hand_swing("offhand", Hand::Left)),
            ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn parse_swing() {
        let mut dispatcher = CommandDispatcher::new();
        let registry = PermissionRegistry::default();
        register(&mut dispatcher, &registry);

        let source = Arc::new(CommandSource::dummy());

        for input in [
            "swing",
            "swing @e",
            "swing @e mainhand",
            "swing @e offhand",
            "swing @e offhand stab",
            "swing @e mainhand none 2s",
        ] {
            assert!(
                dispatcher.parse_input(input, &source).errors.is_empty(),
                "{input}"
            );
        }
        assert!(
            !dispatcher
                .parse_input("swing @e mainhand whack 0", &source)
                .errors
                .is_empty()
        );
    }
}
