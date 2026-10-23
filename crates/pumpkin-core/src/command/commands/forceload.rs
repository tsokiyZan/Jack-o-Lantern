use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::coordinates::Coordinates;
use crate::command::argument_types::coordinates::column_pos::ColumnPosArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

const DESCRIPTION: &str = "Constantly load chunks in the world.";
const PERMISSION: &str = "minecraft:command.forceload";

static ERROR_FAILED_ADD: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_FORCELOAD_ADDED_FAILURE,
    translation::java::COMMANDS_FORCELOAD_ADDED_FAILURE,
);
static ERROR_FAILED_REMOVE: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_FORCELOAD_REMOVED_FAILURE,
    translation::java::COMMANDS_FORCELOAD_REMOVED_FAILURE,
);
static ERROR_FAILED_QUERY: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_FORCELOAD_QUERY_FAILURE,
    translation::java::COMMANDS_FORCELOAD_QUERY_FAILURE,
);
static ERROR_TOO_MANY: CommandErrorType<2> = CommandErrorType::new(
    translation::java::COMMANDS_FORCELOAD_TOOBIG,
    translation::java::COMMANDS_FORCELOAD_TOOBIG,
);

static ERROR_OUT_OF_WORLD: CommandErrorType<0> = CommandErrorType::new(
    translation::java::ARGUMENT_POS_OUTOFWORLD,
    translation::java::ARGUMENT_POS_OUTOFWORLD,
);
static ERROR_NOT_TICKING: CommandErrorType<2> = CommandErrorType::new(
    translation::java::COMMANDS_FORCELOAD_QUERY_FAILURE,
    translation::java::COMMANDS_FORCELOAD_QUERY_FAILURE,
);

/// Vanilla `ChunkPos.toString`, which `Component.translationArg(ChunkPos)` prints.
fn chunk_pos_text(x: i32, z: i32) -> TextComponent {
    TextComponent::text(format!("[{x}, {z}]"))
}

/// Vanilla `ForceLoadCommand.changeForceLoad`: `setChunkForced` on every chunk of the area,
/// reported through a `CommandResponseTracker` counting only the chunks that changed (none
/// fails with `added.failure` / `removed.failure`; one names that chunk).
fn change_force_load(context: &CommandContext, add: bool) -> CommandExecutorResult {
    let from = ColumnPosArgumentType::get_column_pos(context, "from")?;
    let to = if context.get_argument::<Coordinates>("to").is_ok() {
        ColumnPosArgumentType::get_column_pos(context, "to")?
    } else {
        from
    };
    let min_x = from.0.x.min(to.0.x);
    let min_z = from.0.y.min(to.0.y);
    let max_x = from.0.x.max(to.0.x);
    let max_z = from.0.y.max(to.0.y);
    if min_x < -30_000_000 || min_z < -30_000_000 || max_x >= 30_000_000 || max_z >= 30_000_000 {
        return Err(ERROR_OUT_OF_WORLD.create_without_context());
    }
    let min_chunk_x = min_x >> 4;
    let min_chunk_z = min_z >> 4;
    let max_chunk_x = max_x >> 4;
    let max_chunk_z = max_z >> 4;
    let chunk_count =
        (i64::from(max_chunk_x - min_chunk_x) + 1) * (i64::from(max_chunk_z - min_chunk_z) + 1);
    if chunk_count > 256 {
        return Err(ERROR_TOO_MANY.create_without_context(
            TextComponent::text("256"),
            TextComponent::text(chunk_count.to_string()),
        ));
    }

    let failed = if add {
        &ERROR_FAILED_ADD
    } else {
        &ERROR_FAILED_REMOVE
    };
    let world = context
        .source
        .world
        .as_ref()
        .ok_or_else(|| failed.create_without_context())?;

    let mut changed = 0;
    let mut only_changed = None;
    for x in min_chunk_x..=max_chunk_x {
        for z in min_chunk_z..=max_chunk_z {
            if world.set_chunk_forced(x, z, add) {
                changed += 1;
                only_changed = (changed == 1).then_some((x, z));
            }
        }
    }
    if changed == 0 {
        return Err(failed.create_without_context());
    }

    let dimension = TextComponent::text(world.dimension.minecraft_name.to_string());
    let text = if let Some((x, z)) = only_changed {
        let key = if add {
            translation::java::COMMANDS_FORCELOAD_ADDED_SINGLE
        } else {
            translation::java::COMMANDS_FORCELOAD_REMOVED_SINGLE
        };
        TextComponent::translate_cross(key, key, [chunk_pos_text(x, z), dimension])
    } else {
        let key = if add {
            translation::java::COMMANDS_FORCELOAD_ADDED_MULTIPLE
        } else {
            translation::java::COMMANDS_FORCELOAD_REMOVED_MULTIPLE
        };
        TextComponent::translate_cross(
            key,
            key,
            [
                TextComponent::text(changed.to_string()),
                dimension,
                chunk_pos_text(min_chunk_x, min_chunk_z),
                chunk_pos_text(max_chunk_x, max_chunk_z),
            ],
        )
    };
    context.source.send_feedback(text, true);
    Ok(changed)
}

struct ForceloadAddExecutor;

impl CommandExecutor for ForceloadAddExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        change_force_load(context, true)
    }
}

struct ForceloadRemoveExecutor;

impl CommandExecutor for ForceloadRemoveExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        change_force_load(context, false)
    }
}

struct ForceloadRemoveAllExecutor;

impl CommandExecutor for ForceloadRemoveAllExecutor {
    /// Vanilla `ForceLoadCommand.removeAll`: `setChunkForced(false)` on each forced chunk;
    /// always succeeds and returns 0.
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let world = context
            .source
            .world
            .as_ref()
            .ok_or_else(|| ERROR_FAILED_REMOVE.create_without_context())?;

        let forced: Vec<Vector2<i32>> = world
            .forced_chunks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .copied()
            .collect();
        for pos in forced {
            world.set_chunk_forced(pos.x, pos.y, false);
        }

        let text = TextComponent::translate_cross(
            translation::java::COMMANDS_FORCELOAD_REMOVED_ALL,
            translation::java::COMMANDS_FORCELOAD_REMOVED_ALL,
            [TextComponent::text(
                world.dimension.minecraft_name.to_string(),
            )],
        );
        context.source.send_feedback(text, true);

        Ok(0)
    }
}

struct ForceloadQueryExecutor;

impl CommandExecutor for ForceloadQueryExecutor {
    /// Vanilla `ForceLoadCommand.queryForceLoad`.
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let world = context
            .source
            .world
            .as_ref()
            .ok_or_else(|| ERROR_FAILED_QUERY.create_without_context())?;
        let pos = ColumnPosArgumentType::get_column_pos(context, "pos")?;
        let chunk_pos = Vector2::new(pos.0.x >> 4, pos.0.y >> 4);
        let dimension = TextComponent::text(world.dimension.minecraft_name.to_string());
        let is_forced = world
            .forced_chunks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&chunk_pos);
        if !is_forced {
            return Err(ERROR_NOT_TICKING
                .create_without_context(chunk_pos_text(chunk_pos.x, chunk_pos.y), dimension));
        }
        let text = TextComponent::translate_cross(
            translation::java::COMMANDS_FORCELOAD_QUERY_SUCCESS,
            translation::java::COMMANDS_FORCELOAD_QUERY_SUCCESS,
            [chunk_pos_text(chunk_pos.x, chunk_pos.y), dimension],
        );
        context.source.send_feedback(text, false);
        Ok(1)
    }
}

struct ForceloadListExecutor;

impl CommandExecutor for ForceloadListExecutor {
    /// Vanilla `ForceLoadCommand.listForceLoad`: the forced chunks sorted by packed position.
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let world = context
            .source
            .world
            .as_ref()
            .ok_or_else(|| ERROR_FAILED_QUERY.create_without_context())?;
        let dimension = TextComponent::text(world.dimension.minecraft_name.to_string());
        let mut forced: Vec<Vector2<i32>> = world
            .forced_chunks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .copied()
            .collect();
        // `ChunkPos.pack`: x in the low 32 bits, z in the high, compared as a signed long.
        forced.sort_by_key(|pos| (i64::from(pos.y) << 32) | i64::from(pos.x as u32));
        let chunk_count = forced.len();
        if chunk_count == 0 {
            context.source.send_error(TextComponent::translate_cross(
                translation::java::COMMANDS_FORCELOAD_ADDED_NONE,
                translation::java::COMMANDS_FORCELOAD_ADDED_NONE,
                [dimension],
            ));
            return Ok(0);
        }
        let list = forced
            .iter()
            .map(|pos| format!("[{}, {}]", pos.x, pos.y))
            .collect::<Vec<_>>()
            .join(", ");
        let text = if chunk_count == 1 {
            TextComponent::translate_cross(
                translation::java::COMMANDS_FORCELOAD_LIST_SINGLE,
                translation::java::COMMANDS_FORCELOAD_LIST_SINGLE,
                [dimension, TextComponent::text(list)],
            )
        } else {
            TextComponent::translate_cross(
                translation::java::COMMANDS_FORCELOAD_LIST_MULTIPLE,
                translation::java::COMMANDS_FORCELOAD_LIST_MULTIPLE,
                [
                    TextComponent::text(chunk_count.to_string()),
                    dimension,
                    TextComponent::text(list),
                ],
            )
        };
        context.source.send_feedback(text, false);
        Ok(chunk_count as i32)
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    let builder = command("forceload", DESCRIPTION)
        .requires(PERMISSION)
        .then(
            literal("add").then(
                argument("from", ColumnPosArgumentType)
                    .executes(ForceloadAddExecutor)
                    .then(argument("to", ColumnPosArgumentType).executes(ForceloadAddExecutor)),
            ),
        )
        .then(
            literal("remove")
                .then(literal("all").executes(ForceloadRemoveAllExecutor))
                .then(
                    argument("from", ColumnPosArgumentType)
                        .executes(ForceloadRemoveExecutor)
                        .then(
                            argument("to", ColumnPosArgumentType).executes(ForceloadRemoveExecutor),
                        ),
                ),
        )
        .then(
            literal("query")
                .executes(ForceloadListExecutor)
                .then(argument("pos", ColumnPosArgumentType).executes(ForceloadQueryExecutor)),
        );

    dispatcher.register(builder);
}
