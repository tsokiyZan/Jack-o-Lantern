use std::sync::Arc;

use pumpkin_data::block_properties::is_air;
use pumpkin_data::translation;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::PermissionLvl;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use pumpkin_world::world::BlockFlags;

use crate::block::entities::block_entity_from_nbt;
use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::block::{BlockArgumentType, BlockInput};
use crate::command::argument_types::coordinates::block_pos::BlockPosArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::world::World;

const DESCRIPTION: &str = "Place a block.";
const PERMISSION: &str = "minecraft:command.setblock";

const ERROR_FAILED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_SETBLOCK_FAILED,
    translation::java::COMMANDS_SETBLOCK_FAILED,
);

#[derive(Clone, Copy)]
enum Mode {
    /// with particles + item drops
    Destroy,

    /// only replaces air
    Keep,

    /// default; without particles
    Replace,

    /// places block without triggering updates around it
    Strict,
}

struct SetBlockExecutor(Mode);

impl CommandExecutor for SetBlockExecutor {
    /// Vanilla `SetBlockCommand.setBlock`.
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let input = BlockArgumentType::get(context, "block")?;
        let mode = self.0;
        let world = context.source.world();
        let pos = BlockPosArgumentType::get_loaded_block_pos(context, "pos")?;

        if matches!(mode, Mode::Keep) && !world.get_block_state(&pos).is_air() {
            return Err(ERROR_FAILED.create_without_context());
        }

        let place_needed = if matches!(mode, Mode::Destroy) {
            world.break_block(&pos, None, BlockFlags::NOTIFY_ALL | BlockFlags::FORCE_STATE);
            !is_air(input.state) || !world.get_block_state(&pos).is_air()
        } else {
            true
        };

        let (flags, strict) = match mode {
            Mode::Strict => (
                BlockFlags::NOTIFY_LISTENERS
                    | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK
                    | BlockFlags::FORCE_STATE,
                true,
            ),
            Mode::Destroy | Mode::Keep | Mode::Replace => {
                (BlockFlags::NOTIFY_ALL | BlockFlags::FORCE_STATE, false)
            }
        };
        if place_needed && !place_block_input(world, &pos, &input, flags, !strict) {
            return Err(ERROR_FAILED.create_without_context());
        }

        world.flush_block_updates();
        context.source.send_feedback(
            TextComponent::translate_cross(
                pumpkin_data::translation::java::COMMANDS_SETBLOCK_SUCCESS,
                pumpkin_data::translation::bedrock::COMMANDS_SETBLOCK_SUCCESS,
                [
                    TextComponent::text(pos.0.x.to_string()),
                    TextComponent::text(pos.0.y.to_string()),
                    TextComponent::text(pos.0.z.to_string()),
                ],
            ),
            true,
        );
        Ok(1)
    }
}

/// Vanilla `BlockInput.place`: the input's state, reshaped to fit its neighbours unless
/// the placement skips shape updates, with the input's NBT loaded into the block entity.
/// Returns whether anything changed.
pub fn place_block_input(
    world: &Arc<World>,
    pos: &BlockPos,
    input: &BlockInput,
    flags: BlockFlags,
    reshape: bool,
) -> bool {
    let mut state = if reshape {
        world.update_from_neighbor_shapes(input.state, pos)
    } else {
        input.state
    };
    if is_air(state) {
        state = input.state;
    }
    state = input.overwrite_with_defined_properties(state);
    let mut affected = world.set_block_state(pos, state, flags) != state;
    if let Some(tag) = &input.nbt {
        affected |= load_block_entity_nbt(world, pos, tag);
    }
    affected
}

/// Loads command NBT into the block entity at `pos`, keeping its id and position like
/// vanilla's `BlockEntity.loadWithComponents`. Returns whether the data changed.
pub fn load_block_entity_nbt(world: &Arc<World>, pos: &BlockPos, tag: &NbtCompound) -> bool {
    let Some(block_entity) = world.get_block_entity(pos) else {
        return false;
    };
    let mut before = NbtCompound::new();
    block_entity.write_internal(&mut before);
    let mut after = tag.clone();
    for key in ["id", "x", "y", "z"] {
        if let Some(value) = before.get(key) {
            after.put(key, value.clone());
        }
    }
    if after == before {
        return false;
    }
    block_entity_from_nbt(&after).is_some_and(|loaded| {
        world.add_block_entity(loaded);
        true
    })
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("setblock", DESCRIPTION).requires(PERMISSION).then(
            argument("pos", BlockPosArgumentType).then(
                argument("block", BlockArgumentType)
                    .executes(SetBlockExecutor(Mode::Replace))
                    .then(literal("destroy").executes(SetBlockExecutor(Mode::Destroy)))
                    .then(literal("keep").executes(SetBlockExecutor(Mode::Keep)))
                    .then(literal("replace").executes(SetBlockExecutor(Mode::Replace)))
                    .then(literal("strict").executes(SetBlockExecutor(Mode::Strict))),
            ),
        ),
    );
}
