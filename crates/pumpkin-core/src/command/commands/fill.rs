use pumpkin_data::BlockStateId;
use pumpkin_data::translation;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::PermissionLvl;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use pumpkin_world::world::BlockFlags;

use crate::command::argument_builder::{
    ArgumentBuilder, RequiredArgumentBuilder, argument, command, literal,
};
use crate::command::argument_types::block::{BlockArgumentType, BlockInput};
use crate::command::argument_types::block_predicate::{BlockPredicate, BlockPredicateArgumentType};
use crate::command::argument_types::coordinates::block_pos::BlockPosArgumentType;
use crate::command::commands::setblock::place_block_input;
use crate::command::context::command_context::CommandContext;
use crate::command::context::command_source::CommandSource;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::world::World;
use pumpkin_data::block_properties::is_air;
use std::sync::Arc;

const DESCRIPTION: &str = "Fills all or parts of a region with a specific block.";
const PERMISSION: &str = "minecraft:command.fill";

const ERROR_AREA_TOO_LARGE: CommandErrorType<2> = CommandErrorType::new(
    translation::java::COMMANDS_FILL_TOOBIG,
    translation::java::COMMANDS_FILL_TOOBIG,
);

const ERROR_FAILED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_FILL_FAILED,
    translation::java::COMMANDS_FILL_FAILED,
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum FillMode {
    Replace,
    Outline,
    Hollow,
    Destroy,
    Keep,
}

#[derive(Clone, Copy)]
enum FilterMode {
    None,
    WithFilter,
    KeepAir,
}

fn fill_blocks(
    source: &CommandSource,
    from: BlockPos,
    to: BlockPos,
    input: &BlockInput,
    mode: FillMode,
    filter: Option<&BlockPredicate>,
    strict: bool,
) -> Result<i32, CommandSyntaxError> {
    let min_x = from.0.x.min(to.0.x);
    let min_y = from.0.y.min(to.0.y);
    let min_z = from.0.z.min(to.0.z);
    let max_x = from.0.x.max(to.0.x);
    let max_y = from.0.y.max(to.0.y);
    let max_z = from.0.z.max(to.0.z);

    let x_span = i64::from(max_x - min_x + 1);
    let y_span = i64::from(max_y - min_y + 1);
    let z_span = i64::from(max_z - min_z + 1);
    let area = x_span * y_span * z_span;

    let world = source.world().clone();
    let max_block_modifications = {
        let level_info = world.level_info.load();
        level_info.game_rules.max_block_modifications
    };

    if area > max_block_modifications {
        return Err(ERROR_AREA_TOO_LARGE.create_without_context(
            TextComponent::text(max_block_modifications.to_string()),
            TextComponent::text(area.to_string()),
        ));
    }

    // Vanilla `FillCommand.fillBlocks`: places with `2 | (strict ? 816 : 256)`, and a non-strict
    // fill sends the neighbour updates afterwards.
    let flags = if strict {
        BlockFlags::NOTIFY_LISTENERS
            | BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK
            | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK
            | BlockFlags::SKIP_DROPS
            | BlockFlags::UPDATE_KNOWN_SHAPE
    } else {
        BlockFlags::NOTIFY_LISTENERS | BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK
    };
    // `FillCommand.HOLLOW_CORE`.
    let hollow_core = BlockInput {
        state: BlockStateId::AIR,
        properties: Vec::new(),
        nbt: None,
    };

    let mut count = 0;
    let mut update_positions = Vec::new();
    // `BlockPos.betweenClosed` walks x fastest, then y, then z.
    for z in min_z..=max_z {
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let pos = BlockPos::new(x, y, z);
                let old_state = world.get_block_state_id(&pos);
                if let Some(filter) = filter
                    && !matches_filter(&world, filter, &pos, old_state)
                {
                    continue;
                }

                // `Mode.affector`: destroy breaks the old block, dropping items.
                let affected = mode == FillMode::Destroy && !is_air(old_state) && {
                    world.break_block(&pos, None, BlockFlags::NOTIFY_ALL);
                    true
                };

                // `Mode.filter`: the block to place here, if any.
                let is_edge = x == min_x
                    || x == max_x
                    || y == min_y
                    || y == max_y
                    || z == min_z
                    || z == max_z;
                let block = match mode {
                    FillMode::Replace | FillMode::Destroy => Some(input),
                    FillMode::Outline => is_edge.then_some(input),
                    FillMode::Hollow => Some(if is_edge { input } else { &hollow_core }),
                    FillMode::Keep => is_air(world.get_block_state_id(&pos)).then_some(input),
                };

                if block.is_some_and(|block| place_block_input(&world, &pos, block, flags, !strict))
                {
                    if !strict {
                        update_positions.push((pos, old_state));
                    }
                    count += 1;
                } else if affected {
                    count += 1;
                }
            }
        }
    }

    for (pos, old_state) in update_positions {
        world.update_neighbours_on_block_set(&pos, old_state);
    }
    world.flush_block_updates();

    if count == 0 {
        return Err(ERROR_FAILED.create_without_context());
    }

    source.send_feedback(
        TextComponent::translate_cross(
            translation::java::COMMANDS_FILL_SUCCESS,
            translation::java::COMMANDS_FILL_SUCCESS,
            [TextComponent::text(count.to_string())],
        ),
        true,
    );

    Ok(count)
}

struct FillExecutor {
    mode: FillMode,
    filter_mode: FilterMode,
    strict: bool,
}

impl CommandExecutor for FillExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let from = BlockPosArgumentType::get_loaded_block_pos(context, "from")?;
        let to = BlockPosArgumentType::get_loaded_block_pos(context, "to")?;
        let input = BlockArgumentType::get(context, "block")?;

        let filter = if matches!(self.filter_mode, FilterMode::WithFilter) {
            Some(BlockPredicateArgumentType::get(context, "filter")?)
        } else {
            None
        };

        fill_blocks(
            &context.source,
            from,
            to,
            &input,
            self.mode,
            filter.as_ref(),
            self.strict,
        )
    }
}

fn wrap_with_mode(builder: RequiredArgumentBuilder, has_filter: bool) -> RequiredArgumentBuilder {
    let filter_mode = if has_filter {
        FilterMode::WithFilter
    } else {
        FilterMode::None
    };

    builder
        .executes(FillExecutor {
            mode: FillMode::Replace,
            filter_mode,
            strict: false,
        })
        .then(literal("outline").executes(FillExecutor {
            mode: FillMode::Outline,
            filter_mode,
            strict: false,
        }))
        .then(literal("hollow").executes(FillExecutor {
            mode: FillMode::Hollow,
            filter_mode,
            strict: false,
        }))
        .then(literal("destroy").executes(FillExecutor {
            mode: FillMode::Destroy,
            filter_mode,
            strict: false,
        }))
        .then(literal("strict").executes(FillExecutor {
            mode: FillMode::Replace,
            filter_mode,
            strict: true,
        }))
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    let filter_arg = wrap_with_mode(argument("filter", BlockPredicateArgumentType), true);

    let replace_literal = literal("replace")
        .executes(FillExecutor {
            mode: FillMode::Replace,
            filter_mode: FilterMode::None,
            strict: false,
        })
        .then(filter_arg);

    let keep_literal = literal("keep").executes(FillExecutor {
        mode: FillMode::Keep,
        filter_mode: FilterMode::KeepAir,
        strict: false,
    });

    let block_arg = wrap_with_mode(argument("block", BlockArgumentType), false)
        .then(replace_literal)
        .then(keep_literal);

    dispatcher.register(
        command("fill", DESCRIPTION).requires(PERMISSION).then(
            argument("from", BlockPosArgumentType)
                .then(argument("to", BlockPosArgumentType).then(block_arg)),
        ),
    );
}

/// Vanilla `BlockPredicateArgument`'s test on a `BlockInWorld`: the state, then the block
/// entity's saved data when the predicate names any.
fn matches_filter(
    world: &Arc<World>,
    filter: &BlockPredicate,
    pos: &BlockPos,
    state: BlockStateId,
) -> bool {
    if !filter.test(state) {
        return false;
    }
    if !filter.requires_nbt() {
        return true;
    }
    let saved = world.get_block_entity(pos).map(|block_entity| {
        let mut nbt = NbtCompound::new();
        block_entity.write_internal(&mut nbt);
        nbt
    });
    filter.test_nbt(saved.as_ref())
}
