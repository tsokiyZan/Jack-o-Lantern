//! Vanilla `WaypointCommand`.

use std::sync::Arc;

use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use pumpkin_util::text::click::ClickEvent;
use pumpkin_util::text::color::RGBColor;
use pumpkin_util::text::hover::HoverEvent;

use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::entity::EntityArgumentType;
use crate::command::argument_types::hex_color::HexColorArgumentType;
use crate::command::argument_types::identifier::IdentifierArgumentType;
use crate::command::argument_types::team_color::TeamColorArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::entity::EntityBase;
use crate::entity::waypoint::WaypointIconState;

const DESCRIPTION: &str = "List or modify waypoints.";
const PERMISSION: &str = "minecraft:command.waypoint";

/// Vanilla `WaypointArgument.ERROR_NOT_A_WAYPOINT`.
const ERROR_NOT_A_WAYPOINT: CommandErrorType<0> = CommandErrorType::new(
    translation::java::ARGUMENT_WAYPOINT_INVALID,
    translation::java::ARGUMENT_WAYPOINT_INVALID,
);

/// An `RGB` colour as vanilla's `TextColor.fromRgb` (the low 24 bits).
const fn rgb_of(color: i32) -> RGBColor {
    RGBColor::new((color >> 16) as u8, (color >> 8) as u8, color as u8)
}

/// Vanilla `WaypointArgument.getWaypoint`: the single entity, which must be a transmitter
/// (a living entity).
fn get_waypoint(
    context: &CommandContext,
) -> Result<Arc<dyn EntityBase>, crate::command::errors::command_syntax_error::CommandSyntaxError> {
    let entity = EntityArgumentType::get_entity(context, "waypoint")?;
    if entity.get_living_entity().is_none() {
        return Err(ERROR_NOT_A_WAYPOINT.create_without_context());
    }
    Ok(entity)
}

/// Vanilla `mutateIcon`: untrack in the source's level, change the icon, track again.
fn mutate_icon(
    context: &CommandContext,
    waypoint: &Arc<dyn EntityBase>,
    change: impl FnOnce(&mut WaypointIconState),
) {
    let manager = &context.source.world().waypoint_manager;
    let id = waypoint.get_entity().entity_id;
    manager.untrack_waypoint(id);
    if let Some(living) = waypoint.get_living_entity() {
        change(
            &mut living
                .locator_bar_icon
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }
    manager.track_waypoint(waypoint);
}

struct ListExecutor;

impl CommandExecutor for ListExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let world = context.source.world();
        let waypoints = world.waypoint_manager.transmitters();
        let dimension = world.dimension.minecraft_name.to_string();
        if waypoints.is_empty() {
            context.source.send_feedback(
                pumpkin_macros::translate_cross!(
                    translation::java::COMMANDS_WAYPOINT_LIST_EMPTY,
                    translation::java::COMMANDS_WAYPOINT_LIST_EMPTY,
                    TextComponent::text(dimension)
                ),
                false,
            );
            return Ok(0);
        }
        let count = waypoints.len();
        let names: Vec<TextComponent> = waypoints
            .iter()
            .map(|transmitter| {
                let entity = transmitter.get_entity();
                let pos = entity.block_pos.load().0;
                let color = transmitter
                    .get_living_entity()
                    .and_then(|living| {
                        living
                            .locator_bar_icon
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .color
                    })
                    .unwrap_or(-1);
                transmitter
                    .get_display_name()
                    .click_event(ClickEvent::SuggestCommand {
                        command: format!(
                            "/execute in {dimension} run tp @s {} {} {}",
                            pos.x, pos.y, pos.z
                        )
                        .into(),
                    })
                    .hover_event(HoverEvent::show_text(TextComponent::translate(
                        translation::java::CHAT_COORDINATES_TOOLTIP,
                        [],
                    )))
                    .color_rgb(rgb_of(color))
            })
            .collect();
        // `ComponentUtils.formatList`: one name as it is, more joined by a grey ", ".
        let list = if names.len() == 1 {
            names
                .into_iter()
                .next()
                .unwrap_or_else(TextComponent::empty)
        } else {
            let mut list = TextComponent::empty();
            for (i, name) in names.into_iter().enumerate() {
                if i > 0 {
                    list = list.add_child(
                        TextComponent::text(", ")
                            .color_named(pumpkin_util::text::color::NamedColor::Gray),
                    );
                }
                list = list.add_child(name);
            }
            list
        };
        context.source.send_feedback(
            pumpkin_macros::translate_cross!(
                translation::java::COMMANDS_WAYPOINT_LIST_SUCCESS,
                translation::java::COMMANDS_WAYPOINT_LIST_SUCCESS,
                TextComponent::text(count.to_string()),
                TextComponent::text(dimension),
                list
            ),
            false,
        );
        Ok(count as i32)
    }
}

enum ColorAction {
    Named,
    Hex,
    Reset,
}

struct ColorExecutor(ColorAction);

impl CommandExecutor for ColorExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let waypoint = get_waypoint(context)?;
        let message = match self.0 {
            ColorAction::Named => {
                let color = TeamColorArgumentType::get(context, "color")?;
                let rgb = crate::entity::waypoint::named_color_rgb(color);
                mutate_icon(context, &waypoint, |icon| icon.color = Some(rgb));
                pumpkin_macros::translate_cross!(
                    translation::java::COMMANDS_WAYPOINT_MODIFY_COLOR,
                    translation::java::COMMANDS_WAYPOINT_MODIFY_COLOR,
                    TextComponent::text(color.name()).color_named(color)
                )
            }
            ColorAction::Hex => {
                let rgb = HexColorArgumentType::get(context, "color")?;
                // `HexColorArgument`: `ARGB.color(r, g, b)`, alpha 255.
                let color = i32::from_be_bytes([0xFF, rgb.red, rgb.green, rgb.blue]);
                mutate_icon(context, &waypoint, |icon| icon.color = Some(color));
                pumpkin_macros::translate_cross!(
                    translation::java::COMMANDS_WAYPOINT_MODIFY_COLOR,
                    translation::java::COMMANDS_WAYPOINT_MODIFY_COLOR,
                    TextComponent::text(format!(
                        "{:02X}{:02X}{:02X}",
                        rgb.red, rgb.green, rgb.blue
                    ))
                    .color_rgb(rgb)
                )
            }
            ColorAction::Reset => {
                mutate_icon(context, &waypoint, |icon| icon.color = None);
                pumpkin_macros::translate_cross!(
                    translation::java::COMMANDS_WAYPOINT_MODIFY_COLOR_RESET,
                    translation::java::COMMANDS_WAYPOINT_MODIFY_COLOR_RESET
                )
            }
        };
        context.source.send_feedback(message, false);
        Ok(0)
    }
}

enum StyleAction {
    Set,
    Reset,
}

struct StyleExecutor(StyleAction);

impl CommandExecutor for StyleExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let waypoint = get_waypoint(context)?;
        let style = match self.0 {
            StyleAction::Set => IdentifierArgumentType::get(context, "style")?.to_string(),
            StyleAction::Reset => {
                pumpkin_protocol::java::client::play::WaypointIcon::DEFAULT_STYLE.to_string()
            }
        };
        mutate_icon(context, &waypoint, |icon| icon.style = style);
        context.source.send_feedback(
            pumpkin_macros::translate_cross!(
                translation::java::COMMANDS_WAYPOINT_MODIFY_STYLE,
                translation::java::COMMANDS_WAYPOINT_MODIFY_STYLE
            ),
            false,
        );
        Ok(0)
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    let color_node = literal("color")
        .then(argument("color", TeamColorArgumentType).executes(ColorExecutor(ColorAction::Named)))
        .then(literal("hex").then(
            argument("color", HexColorArgumentType).executes(ColorExecutor(ColorAction::Hex)),
        ))
        .then(literal("reset").executes(ColorExecutor(ColorAction::Reset)));

    let style_node = literal("style")
        .then(literal("reset").executes(StyleExecutor(StyleAction::Reset)))
        .then(literal("set").then(
            argument("style", IdentifierArgumentType).executes(StyleExecutor(StyleAction::Set)),
        ));

    let modify_node = literal("modify").then(
        argument("waypoint", EntityArgumentType::Entity)
            .then(color_node)
            .then(style_node),
    );

    dispatcher.register(
        command("waypoint", DESCRIPTION)
            .requires(PERMISSION)
            .then(literal("list").executes(ListExecutor))
            .then(modify_node),
    );
}
