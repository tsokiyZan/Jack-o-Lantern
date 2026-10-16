#![allow(
    clippy::too_many_lines,
    clippy::collapsible_if,
    clippy::unnecessary_wraps,
    clippy::significant_drop_in_scrutinee,
    clippy::nonminimal_bool,
    clippy::if_not_else,
    clippy::explicit_auto_deref,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::match_same_arms,
    clippy::struct_excessive_bools
)]

use crate::command::CommandSource;
use crate::command::argument_builder::{
    ArgumentBuilder, LiteralArgumentBuilder, RequiredArgumentBuilder, argument, command, literal,
};
use crate::command::argument_types::block_predicate::BlockPredicateArgumentType;
use crate::command::argument_types::coordinates::block_pos::BlockPosArgumentType;
use crate::command::argument_types::coordinates::rotation::RotationArgumentType;
use crate::command::argument_types::coordinates::swizzle::SwizzleArgumentType;
use crate::command::argument_types::coordinates::vec3::Vec3ArgumentType;
use crate::command::argument_types::core::double::DoubleArgumentType;
use crate::command::argument_types::dimension::{DIMENSION_REGISTRY, DimensionArgumentType};
use crate::command::argument_types::entity::EntityArgumentType;
use crate::command::argument_types::entity_anchor::{EntityAnchorArgumentType, EntityAnchorExt};
use crate::command::argument_types::function::FunctionArgumentType;
use crate::command::argument_types::heightmap::HeightmapTypeArgumentType;
use crate::command::argument_types::identifier::IdentifierArgumentType;
use crate::command::argument_types::nbt_path::{NbtPath, NbtPathArgumentType};
use crate::command::argument_types::objective::ObjectiveArgumentType;
use crate::command::argument_types::range::{FloatRangeArgumentType, IntRangeArgumentType};
use crate::command::argument_types::resource::{ENTITY_TYPE_ARGUMENT, ResourceArgument};
use crate::command::argument_types::resource_key::{BIOME_REGISTRY, ResourceKeyArgument};
use crate::command::argument_types::resource_or_tag::{ResourceOrTag, ResourceOrTagArgument};
use crate::command::argument_types::score_holder::ScoreHolderArgumentType;
use crate::command::argument_types::score_holder::ScoreHolderSuggestionProvider;
use crate::command::commands::bossbar::{BossbarIdArgumentType, handle_bossbar_error};
use crate::command::commands::data::{
    BlockDataAccessor, DataAccessor, EntityDataAccessor, StorageDataAccessor, TargetKind,
};
use crate::command::commands::fill::matches_filter;
use crate::command::commands::function::{
    FunctionSuggestionProvider, get_functions, modify_sender_for_execution,
};
use crate::command::context::command_context::CommandContext;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::attached::{CommandNodeId, NodeId};
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::tree::Tree;
use crate::command::node::{CommandExecutor, CommandExecutorResult, RedirectModifier, Redirection};
use crate::data::datapack::DatapackManager;
use crate::data::datapack::function_loader::FunctionInstantiationError;
use crate::entity::EntityBase;
use crate::entity::r#type::from_type;
use crate::world::World;
use crate::world::custom_bossbar::BossbarUpdateError;
use crate::world::scoreboard::Scoreboard;
use crate::world::stopwatches::Stopwatches;
use pumpkin_command::argument_types::coordinates::block_pos::NOT_LOADED_ERROR_TYPE;
use pumpkin_command::execution;
use pumpkin_command::source::ResultValueTaker;
use pumpkin_command::source::ReturnValue;
use pumpkin_data::biome::Biome;
use pumpkin_data::tag::{self, RegistryKey};
use pumpkin_data::{BlockId, translation};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::PermissionLvl;
use pumpkin_util::identifier::Identifier;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

const DESCRIPTION: &str = "Execute a command with a modified context.";
const PERMISSION: &str = "minecraft:command.execute";

static ERROR_INVALID_DIMENSION: CommandErrorType<1> =
    CommandErrorType::new("argument.dimension.invalid", "argument.dimension.invalid");

const MAX_TEST_AREA: i64 = 32768;

const ERROR_FUNCTION_CONDITION_INSTANTATION_FAILURE: CommandErrorType<2> = CommandErrorType::new(
    translation::java::COMMANDS_EXECUTE_FUNCTION_INSTANTIATIONFAILURE,
    translation::java::COMMANDS_EXECUTE_FUNCTION_INSTANTIATIONFAILURE,
);
const ERROR_AREA_TOO_LARGE: CommandErrorType<2> = CommandErrorType::new(
    translation::java::COMMANDS_EXECUTE_BLOCKS_TOOBIG,
    translation::java::COMMANDS_EXECUTE_BLOCKS_TOOBIG,
);

const ERROR_CONDITIONAL_FAILED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_EXECUTE_CONDITIONAL_FAIL,
    translation::java::COMMANDS_EXECUTE_CONDITIONAL_FAIL,
);

const ERROR_CONDITIONAL_FAILED_COUNT: CommandErrorType<1> = CommandErrorType::new(
    translation::java::COMMANDS_EXECUTE_CONDITIONAL_FAIL_COUNT,
    translation::java::COMMANDS_EXECUTE_CONDITIONAL_FAIL_COUNT,
);

fn execute_as_modifier(context: &CommandContext) -> crate::command::node::RedirectModifierResult {
    let targets = EntityArgumentType::get_optional_entities(context, "targets")?;
    let mut sources = Vec::new();
    for target in targets {
        let mut source = context.source.as_ref().clone();
        let display_name = target.get_display_name();
        let name = target.get_name().get_text();
        source.entity = Some(target.clone());
        source.name = name;
        source.display_name = display_name;
        sources.push(Arc::new(source));
    }
    Ok(sources)
}

fn execute_at_modifier(context: &CommandContext) -> crate::command::node::RedirectModifierResult {
    let targets = EntityArgumentType::get_optional_entities(context, "targets")?;
    let mut sources = Vec::new();
    for target in targets {
        let entity = target.get_entity();
        let mut source = context.source.as_ref().clone();
        source.position = entity.pos.load();
        source.rotation = Vector2::new(entity.yaw.load(), entity.pitch.load());
        source.world = Some(entity.world.load().clone());
        sources.push(Arc::new(source));
    }
    Ok(sources)
}

fn execute_in_modifier(context: &CommandContext) -> crate::command::node::RedirectModifierResult {
    let target_world = get_dimension(context, "dimension")?;
    let mut source = context.source.as_ref().clone();
    source.world = Some(target_world);
    Ok(vec![Arc::new(source)])
}

fn execute_positioned_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let pos = Vec3ArgumentType::get_vector3(context, "pos")?;
    let mut source = context.source.as_ref().clone();
    source.position = pos;
    Ok(vec![Arc::new(source)])
}

fn execute_positioned_as_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let targets = EntityArgumentType::get_optional_entities(context, "targets")?;
    let mut sources = Vec::new();
    for target in targets {
        let mut source = context.source.as_ref().clone();
        source.position = target.get_entity().pos.load();
        sources.push(Arc::new(source));
    }
    Ok(sources)
}

fn execute_positioned_over_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let heightmap = HeightmapTypeArgumentType::get(context, "heightmap")?;
    let mut source = context.source.as_ref().clone();
    let pos = source.position;
    let world = context.world();
    let (block_x, block_z) = (pos.x.floor() as i32, pos.z.floor() as i32);
    if !world
        .level
        .is_chunk_loaded(&Vector2::new(block_x >> 4, block_z >> 4))
    {
        return Err(NOT_LOADED_ERROR_TYPE.create_without_context());
    }
    source.position.y = f64::from(world.get_height(heightmap, block_x, block_z));
    Ok(vec![Arc::new(source)])
}

fn execute_rotated_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let rot_coords = RotationArgumentType::get(context, "rotation")?;
    let rot = rot_coords.rotation(&context.source);
    let mut source = context.source.as_ref().clone();
    source.rotation = rot;
    Ok(vec![Arc::new(source)])
}

fn execute_rotated_as_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let targets = EntityArgumentType::get_optional_entities(context, "targets")?;
    let mut sources = Vec::new();
    for target in targets {
        let entity = target.get_entity();
        let mut source = context.source.as_ref().clone();
        source.rotation = Vector2::new(entity.yaw.load(), entity.pitch.load());
        sources.push(Arc::new(source));
    }
    Ok(sources)
}

fn execute_align_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let swizzle = SwizzleArgumentType::get(context, "axes")?;
    let mut source = context.source.as_ref().clone();

    source.position = swizzle.align(source.position);

    Ok(vec![Arc::new(source)])
}

fn execute_anchored_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let anchor = EntityAnchorArgumentType::get(context, "anchor")?;
    let mut source = context.source.as_ref().clone();
    source.entity_anchor = anchor;
    Ok(vec![Arc::new(source)])
}

fn execute_facing_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let pos = Vec3ArgumentType::get_vector3(context, "pos")?;
    let mut source = context.source.as_ref().clone();

    let dx = pos.x - source.position.x;
    let dy = pos.y - source.position.y;
    let dz = pos.z - source.position.z;

    let xz_dist = dx.hypot(dz);
    let yaw = (dz.atan2(dx).to_degrees() as f32) - 90.0;
    let pitch = -(dy.atan2(xz_dist).to_degrees() as f32);

    source.rotation = Vector2::new(yaw, pitch);
    Ok(vec![Arc::new(source)])
}

fn execute_facing_entity_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let targets = EntityArgumentType::get_optional_entities(context, "targets")?;
    let anchor = EntityAnchorArgumentType::get(context, "anchor")?;
    let mut sources = Vec::new();

    for target in targets {
        let target_pos = anchor.position_at_entity(target.get_entity());
        let mut source = context.source.as_ref().clone();

        let dx = target_pos.x - source.position.x;
        let dy = target_pos.y - source.position.y;
        let dz = target_pos.z - source.position.z;

        let xz_dist = dx.hypot(dz);
        let yaw = (dz.atan2(dx).to_degrees() as f32) - 90.0;
        let pitch = -(dy.atan2(xz_dist).to_degrees() as f32);

        source.rotation = Vector2::new(yaw, pitch);
        sources.push(Arc::new(source));
    }
    Ok(sources)
}

fn execute_summon_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let entity_type = ResourceArgument::get_summonable_entity_type(context, "entity_type")?;
    let entity = from_type(
        entity_type,
        context.source.position,
        context.source.world(),
        Uuid::new_v4(),
    );
    context.source.world().spawn_entity(entity.clone());
    let mut source = context.source.as_ref().clone();
    source.entity = Some(entity);
    Ok(vec![Arc::new(source)])
}

fn execute_on_owner_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let mut sources = Vec::new();
    if let Some(ref entity) = context.source.entity {
        let world = context.source.world();
        if let Some(mob) = entity.get_mob() {
            if let Some(owner_uuid) = mob.get_owner_uuid() {
                if let Some(owner) = world.get_player_by_uuid(owner_uuid) {
                    let mut source = context.source.as_ref().clone();
                    let name = owner.get_name().get_text();
                    let display_name = owner.get_display_name();
                    source.entity = Some(owner);
                    source.name = name;
                    source.display_name = display_name;
                    sources.push(Arc::new(source));
                }
            }
        }
    }
    Ok(sources)
}

fn execute_on_leasher_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let mut sources = Vec::new();
    if let Some(ref entity) = context.source.entity {
        if let Some(leasher) = entity
            .get_entity()
            .leashed_to
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            let mut source = context.source.as_ref().clone();
            let name = leasher.get_name().get_text();
            let display_name = leasher.get_display_name();
            source.entity = Some(leasher);
            source.name = name;
            source.display_name = display_name;
            sources.push(Arc::new(source));
        }
    }
    Ok(sources)
}

fn execute_on_target_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let mut sources = Vec::new();
    if let Some(ref entity) = context.source.entity {
        if let Some(mob) = entity.get_mob() {
            if let Some(target) = mob.get_mob_entity().get_target() {
                let mut source = context.source.as_ref().clone();
                let name = target.get_name().get_text();
                let display_name = target.get_display_name();
                source.entity = Some(target);
                source.name = name;
                source.display_name = display_name;
                sources.push(Arc::new(source));
            }
        }
    }
    Ok(sources)
}

fn execute_on_attacker_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let mut sources = Vec::new();
    if let Some(ref entity) = context.source.entity {
        if let Some(living) = entity.get_living_entity() {
            let attacker_id = living
                .last_attacker_id
                .load(std::sync::atomic::Ordering::Relaxed);
            if attacker_id != 0 {
                let world = context.source.world();
                if let Some(attacker) = world.get_entity_by_id(attacker_id) {
                    let mut source = context.source.as_ref().clone();
                    let name = attacker.get_name().get_text();
                    let display_name = attacker.get_display_name();
                    source.entity = Some(attacker);
                    source.name = name;
                    source.display_name = display_name;
                    sources.push(Arc::new(source));
                }
            }
        }
    }
    Ok(sources)
}

fn execute_on_vehicle_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let mut sources = Vec::new();
    if let Some(ref entity) = context.source.entity {
        if let Some(vehicle) = entity
            .get_entity()
            .vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            let mut source = context.source.as_ref().clone();
            let name = vehicle.get_name().get_text();
            let display_name = vehicle.get_display_name();
            source.entity = Some(vehicle);
            source.name = name;
            source.display_name = display_name;
            sources.push(Arc::new(source));
        }
    }
    Ok(sources)
}

fn execute_on_controller_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let mut sources = Vec::new();
    if let Some(ref entity) = context.source.entity {
        if let Some(controller) = entity
            .get_entity()
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .first()
            .cloned()
        {
            let mut source = context.source.as_ref().clone();
            let name = controller.get_name().get_text();
            let display_name = controller.get_display_name();
            source.entity = Some(controller);
            source.name = name;
            source.display_name = display_name;
            sources.push(Arc::new(source));
        }
    }
    Ok(sources)
}

fn execute_on_origin_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    execute_on_owner_modifier(context)
}

fn execute_on_passengers_modifier(
    context: &CommandContext,
) -> crate::command::node::RedirectModifierResult {
    let mut sources = Vec::new();
    if let Some(ref entity) = context.source.entity {
        let passengers = entity
            .get_entity()
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for passenger in passengers {
            let mut source = context.source.as_ref().clone();
            let name = passenger.get_name().get_text();
            let display_name = passenger.get_display_name();
            source.entity = Some(passenger);
            source.name = name;
            source.display_name = display_name;
            sources.push(Arc::new(source));
        }
    }
    Ok(sources)
}

/// Vanilla `DimensionArgument.getDimension`.
fn get_dimension(context: &CommandContext, name: &str) -> Result<Arc<World>, CommandSyntaxError> {
    let dimension_key = ResourceKeyArgument::get_registry_key(
        context,
        name,
        DIMENSION_REGISTRY,
        &ERROR_INVALID_DIMENSION,
    )?;
    let dimension_name = dimension_key.identifier.to_string();
    context
        .server()
        .worlds
        .load()
        .iter()
        .find(|world| world.dimension.minecraft_name == dimension_name)
        .cloned()
        .ok_or_else(|| {
            ERROR_INVALID_DIMENSION.create_without_context(TextComponent::text(dimension_name))
        })
}

/// Vanilla `ExecuteCommand.isChunkLoaded`: a full, entity-ticking chunk. Pumpkin ticks the
/// loaded chunks in its active set and loads their entities with them.
fn is_chunk_loaded(world: &World, pos: &BlockPos) -> bool {
    let chunk_pos = pos.chunk_position();
    world.level.is_chunk_loaded(&chunk_pos)
        && world
            .active_chunks
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&chunk_pos)
}

/// Vanilla `ExecuteCommand.CommandPredicate`.
type CommandPredicate = fn(&CommandContext) -> Result<bool, CommandSyntaxError>;

/// Vanilla `ExecuteCommand.CommandNumericPredicate`.
type CommandNumericPredicate = fn(&CommandContext) -> Result<i32, CommandSyntaxError>;

fn expect(context: &CommandContext, expected: bool, result: bool) -> Vec<Arc<CommandSource>> {
    if result == expected {
        vec![context.source.clone()]
    } else {
        Vec::new()
    }
}

fn send_conditional_pass(context: &CommandContext) {
    context.source.send_feedback(
        TextComponent::translate_cross(
            translation::java::COMMANDS_EXECUTE_CONDITIONAL_PASS,
            translation::java::COMMANDS_EXECUTE_CONDITIONAL_PASS,
            [],
        ),
        false,
    );
}

fn send_conditional_pass_count(context: &CommandContext, count: i32) {
    context.source.send_feedback(
        TextComponent::translate_cross(
            translation::java::COMMANDS_EXECUTE_CONDITIONAL_PASS_COUNT,
            translation::java::COMMANDS_EXECUTE_CONDITIONAL_PASS_COUNT,
            [TextComponent::text(count.to_string())],
        ),
        false,
    );
}

struct ConditionalExecutor {
    expected: bool,
    predicate: CommandPredicate,
}

impl CommandExecutor for ConditionalExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        if self.expected == (self.predicate)(context)? {
            send_conditional_pass(context);
            Ok(1)
        } else {
            Err(ERROR_CONDITIONAL_FAILED.create_without_context())
        }
    }
}

/// Vanilla `ExecuteCommand.addConditional`.
fn add_conditional<B: ArgumentBuilder>(
    argument: B,
    expected: bool,
    predicate: CommandPredicate,
) -> B {
    argument
        .fork(
            Redirection::Root,
            RedirectModifier::Custom(Arc::new(move |context: &CommandContext| {
                Ok(expect(context, expected, predicate(context)?))
            })),
        )
        .executes(ConditionalExecutor {
            expected,
            predicate,
        })
}

/// Vanilla `ExecuteCommand.createNumericConditionalHandler`.
struct NumericConditionalExecutor {
    expected: bool,
    condition: CommandNumericPredicate,
}

impl CommandExecutor for NumericConditionalExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let count = (self.condition)(context)?;
        if self.expected {
            if count > 0 {
                send_conditional_pass_count(context, count);
                Ok(count)
            } else {
                Err(ERROR_CONDITIONAL_FAILED.create_without_context())
            }
        } else if count == 0 {
            send_conditional_pass(context);
            Ok(1)
        } else {
            Err(ERROR_CONDITIONAL_FAILED_COUNT
                .create_without_context(TextComponent::text(count.to_string())))
        }
    }
}

fn add_numeric_conditional(
    argument: RequiredArgumentBuilder,
    expected: bool,
    condition: CommandNumericPredicate,
) -> RequiredArgumentBuilder {
    argument
        .fork(
            Redirection::Root,
            RedirectModifier::Custom(Arc::new(move |context: &CommandContext| {
                Ok(expect(context, expected, condition(context)? > 0))
            })),
        )
        .executes(NumericConditionalExecutor {
            expected,
            condition,
        })
}

/// Builds the tag `execute store` writes from the value and the scale.
type StoredTag = fn(i32, f64) -> NbtTag;

/// The value vanilla `execute store` writes: the result, or 1/0 for success.
const fn stored_value(store_result: bool, result: ReturnValue) -> i32 {
    if store_result {
        result.result_value()
    } else {
        result.success_value() as i32
    }
}

/// Vanilla `CommandSourceStack.withCallback(callback, CommandResultCallback::chain)`.
fn with_store_callback(
    context: &CommandContext,
    callback: impl Fn(ReturnValue) + Send + Sync + 'static,
) -> Vec<Arc<CommandSource>> {
    vec![Arc::new(
        context
            .source
            .as_ref()
            .clone()
            .merge_command_result_taker(&ResultValueTaker::from_fn(callback)),
    )]
}

/// Vanilla `ExecuteCommand.storeValue` for scores.
fn store_score(
    context: &CommandContext,
    store_result: bool,
) -> Result<Vec<Arc<CommandSource>>, CommandSyntaxError> {
    let holders = ScoreHolderArgumentType::get_score_holders(context, "targets")?;
    let objective = ObjectiveArgumentType::get(context, "objective")?.to_string();
    let world = context.world().clone();
    ObjectiveArgumentType::objective_or_error(
        &world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        &objective,
    )?;
    Ok(with_store_callback(context, move |result| {
        let value = stored_value(store_result, result);
        let mut scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for holder in &holders {
            scoreboard.set_score_value(
                world.as_ref(),
                holder.name.clone(),
                objective.clone(),
                value,
            );
        }
    }))
}

/// Vanilla `ExecuteCommand.storeValue` for boss bars.
fn store_bossbar(
    context: &CommandContext,
    store_into_value: bool,
    store_result: bool,
) -> Result<Vec<Arc<CommandSource>>, CommandSyntaxError> {
    let id = BossbarIdArgumentType::get(context, "id")?;
    let server = context.server().clone();
    if !server
        .bossbars
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .has_bossbar(&id)
    {
        return Err(handle_bossbar_error(
            BossbarUpdateError::InvalidResourceLocation(id),
        ));
    }
    Ok(with_store_callback(context, move |result| {
        let value = stored_value(store_result, result);
        let mut bossbars = server
            .bossbars
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // An unchanged value is not an error here; the boss bar just keeps it.
        let _ = if store_into_value {
            bossbars.update_value(&server, id.clone(), value)
        } else {
            bossbars.update_max(&server, id.clone(), value)
        };
    }))
}

/// Vanilla `ExecuteCommand.storeData`.
fn store_data(
    context: &CommandContext,
    target: TargetKind,
    constructor: StoredTag,
    store_result: bool,
) -> Result<Vec<Arc<CommandSource>>, CommandSyntaxError> {
    let accessor: Arc<dyn DataAccessor> = match target {
        TargetKind::Entity => Arc::new(EntityDataAccessor::new(EntityArgumentType::get_entity(
            context, "target",
        )?)),
        TargetKind::Block => Arc::new(BlockDataAccessor::new(
            BlockPosArgumentType::get_loaded_block_pos(context, "target")?,
            context.world().clone(),
        )?),
        TargetKind::Storage => Arc::new(StorageDataAccessor::new(
            context.get_argument::<Identifier>("target")?.to_string(),
            context.server().clone(),
        )),
    };
    let path = NbtPathArgumentType::get(context, "path")?.clone();
    let scale = DoubleArgumentType::get(context, "scale")?;
    Ok(with_store_callback(context, move |result| {
        let Ok(data) = accessor.get_data() else {
            return;
        };
        let mut data = NbtTag::Compound(data);
        let value = stored_value(store_result, result);
        if path.set(&mut data, constructor(value, scale)).is_ok()
            && let NbtTag::Compound(data) = data
        {
            let _ = accessor.set_data(&data);
        }
    }))
}

/// Vanilla `ExecuteCommand.wrapStores`.
fn wrap_stores(parent: LiteralArgumentBuilder, store_result: bool) -> LiteralArgumentBuilder {
    let bossbar = |store_into_value: bool| {
        RedirectModifier::Custom(Arc::new(move |context: &CommandContext| {
            store_bossbar(context, store_into_value, store_result)
        }))
    };
    let data_types: [(&str, StoredTag); 6] = [
        ("int", |v, scale| NbtTag::Int((f64::from(v) * scale) as i32)),
        ("float", |v, scale| {
            NbtTag::Float((f64::from(v) * scale) as f32)
        }),
        // Java narrows double to int first, then int to short or byte.
        ("short", |v, scale| {
            NbtTag::Short((f64::from(v) * scale) as i32 as i16)
        }),
        ("long", |v, scale| {
            NbtTag::Long((f64::from(v) * scale) as i64)
        }),
        ("double", |v, scale| NbtTag::Double(f64::from(v) * scale)),
        ("byte", |v, scale| {
            NbtTag::Byte((f64::from(v) * scale) as i32 as i8)
        }),
    ];
    let data_path = |target: TargetKind| {
        let mut path = argument("path", NbtPathArgumentType);
        for (name, constructor) in data_types {
            path = path.then(literal(name).then(
                argument("scale", DoubleArgumentType::any()).redirect_with_modifier(
                    Redirection::Root,
                    RedirectModifier::Custom(Arc::new(move |context: &CommandContext| {
                        store_data(context, target, constructor, store_result)
                    })),
                ),
            ));
        }
        path
    };

    parent
        .then(
            literal("score").then(
                argument("targets", ScoreHolderArgumentType::Multiple)
                    .suggests(ScoreHolderSuggestionProvider)
                    .then(
                        argument("objective", ObjectiveArgumentType).redirect_with_modifier(
                            Redirection::Root,
                            RedirectModifier::Custom(Arc::new(move |context: &CommandContext| {
                                store_score(context, store_result)
                            })),
                        ),
                    ),
            ),
        )
        .then(
            literal("bossbar").then(
                argument("id", BossbarIdArgumentType)
                    .then(literal("value").redirect_with_modifier(Redirection::Root, bossbar(true)))
                    .then(literal("max").redirect_with_modifier(Redirection::Root, bossbar(false))),
            ),
        )
        .then(literal("entity").then(
            argument("target", EntityArgumentType::Entity).then(data_path(TargetKind::Entity)),
        ))
        .then(
            literal("block")
                .then(argument("target", BlockPosArgumentType).then(data_path(TargetKind::Block))),
        )
        .then(
            literal("storage").then(
                argument("target", IdentifierArgumentType).then(data_path(TargetKind::Storage)),
            ),
        )
}

/// Vanilla `ExecuteCommand.scheduleFunctionConditionsAndTest` for one source: the functions
/// run in an isolated frame, and the source passes on when their return value matches.
fn execute_if_function(
    context: &CommandContext,
    expected: bool,
) -> Result<Vec<Arc<CommandSource>>, CommandSyntaxError> {
    let functions = get_functions(context, FunctionArgumentType::get(context, "name")?)?;
    if functions.is_empty() {
        return Ok(Vec::new());
    }
    let server = context.server();
    let function_source = modify_sender_for_execution(
        &context
            .source
            .as_ref()
            .clone()
            .with_command_result_taker(ResultValueTaker::new()),
    );
    // Instantiation stops at the first function that fails; the ones before it still run.
    let mut instantiated = Vec::with_capacity(functions.len());
    for (id, function) in &functions {
        match DatapackManager::instantiate(server, function, None, &function_source) {
            Ok(lines) => instantiated.push(lines),
            Err(FunctionInstantiationError(reason)) => {
                if !context.chain_modifiers.is_forked() {
                    let error = ERROR_FUNCTION_CONDITION_INSTANTATION_FAILURE
                        .create_without_context(TextComponent::text(id.clone()), reason);
                    context.source.send_error(error.message);
                }
                break;
            }
        }
    }
    let passed = Arc::new(AtomicBool::new(false));
    let callback = {
        let passed = passed.clone();
        ResultValueTaker::from_fn(move |result| {
            if (result.result_value() != 0) == expected {
                passed.store(true, Ordering::Relaxed);
            }
        })
    };
    // Vanilla IsolatedCall, then FallthroughTask if no function returned.
    execution::run_in_new_frame(callback, false, |frame| {
        let generation = frame.generation();
        for lines in &instantiated {
            if frame.generation() != generation {
                return;
            }
            DatapackManager::call_function(
                server,
                lines,
                &function_source,
                frame.return_value_consumer().clone(),
                true,
            );
        }
        if frame.generation() == generation {
            frame.return_failure();
            frame.discard();
        }
    });
    Ok(if passed.load(Ordering::Relaxed) {
        vec![context.source.clone()]
    } else {
        Vec::new()
    })
}

fn check_block(context: &CommandContext) -> Result<bool, CommandSyntaxError> {
    let predicate = BlockPredicateArgumentType::get(context, "block")?;
    let world = context.source.world();
    let pos = BlockPosArgumentType::get_loaded_block_pos(context, "pos")?;
    Ok(matches_filter(
        world,
        &predicate,
        &pos,
        world.get_block_state_id(&pos),
    ))
}

fn check_biome(context: &CommandContext) -> Result<bool, CommandSyntaxError> {
    let biome_arg = context.get_argument::<ResourceOrTag>("biome")?;
    let world = context.source.world();
    let biome =
        world.get_zoomed_biome(&BlockPosArgumentType::get_loaded_block_pos(context, "pos")?);
    Ok(biome_matches(biome_arg, biome))
}

/// Whether `biome` is the biome or in the tag a biome `resource_or_tag` argument names.
#[must_use]
pub fn biome_matches(biome_arg: &ResourceOrTag, biome: &Biome) -> bool {
    match biome_arg {
        ResourceOrTag::Resource(id) => {
            Biome::from_name(id.path()).is_some_and(|b| b.id == biome.id)
        }
        ResourceOrTag::Tag(id) => tag::get_tag_values(RegistryKey::WorldgenBiome, &id.to_string())
            .is_some_and(|names| {
                names
                    .iter()
                    .filter_map(|name| Biome::from_name(name))
                    .any(|b| b.id == biome.id)
            }),
    }
}

fn check_loaded(context: &CommandContext) -> Result<bool, CommandSyntaxError> {
    Ok(is_chunk_loaded(
        context.source.world(),
        &BlockPosArgumentType::get_block_pos(context, "pos")?,
    ))
}

fn check_dimension(context: &CommandContext) -> Result<bool, CommandSyntaxError> {
    Ok(Arc::ptr_eq(
        &get_dimension(context, "dimension")?,
        context.source.world(),
    ))
}

fn get_score_value(
    scoreboard: &Scoreboard,
    target: &str,
    objective: &str,
) -> Result<Option<i32>, CommandSyntaxError> {
    ObjectiveArgumentType::objective_or_error(scoreboard, objective)?;
    Ok(scoreboard
        .get_score(target, objective)
        .map(|score| score.value.0))
}

fn get_score(
    context: &CommandContext,
    target: &str,
    objective: &str,
) -> Result<Option<i32>, CommandSyntaxError> {
    let scoreboard = context
        .source
        .world()
        .scoreboard
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    get_score_value(&scoreboard, target, objective)
}

/// Vanilla `ExecuteCommand.checkScore` with an operation.
fn check_score(
    context: &CommandContext,
    operation: fn(i32, i32) -> bool,
) -> Result<bool, CommandSyntaxError> {
    let target = ScoreHolderArgumentType::get_score_holder(context, "target")?.name;
    let target_objective = ObjectiveArgumentType::get(context, "targetObjective")?;
    let target_score = get_score(context, &target, target_objective)?;
    let source = ScoreHolderArgumentType::get_score_holder(context, "source")?.name;
    let source_objective = ObjectiveArgumentType::get(context, "sourceObjective")?;
    let source_score = get_score(context, &source, source_objective)?;
    Ok(match (target_score, source_score) {
        (Some(a), Some(b)) => operation(a, b),
        _ => false,
    })
}

/// Vanilla `ExecuteCommand.checkScore` with a range.
fn check_score_range(context: &CommandContext) -> Result<bool, CommandSyntaxError> {
    let target = ScoreHolderArgumentType::get_score_holder(context, "target")?.name;
    let target_objective = ObjectiveArgumentType::get(context, "targetObjective")?;
    let range = IntRangeArgumentType::get(context, "range")?;
    Ok(get_score(context, &target, target_objective)?.is_some_and(|score| range.matches(score)))
}

/// Vanilla `ExecuteCommand.checkStopwatch`.
fn check_stopwatch(context: &CommandContext) -> Result<bool, CommandSyntaxError> {
    let id = context.get_argument::<Identifier>("id")?;
    let range = FloatRangeArgumentType::get(context, "range")?;
    let stopwatches = context
        .server()
        .stopwatches
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let id_str = id.to_string();
    let Some(stopwatch) = stopwatches.get(&id_str) else {
        return Err(crate::command::commands::stopwatch::ERROR_DOES_NOT_EXIST
            .create_without_context(TextComponent::text(id_str)));
    };
    let now = Stopwatches::current_time();
    Ok(range.matches(stopwatch.elapsed_seconds(now)))
}

fn count_entities(context: &CommandContext) -> Result<i32, CommandSyntaxError> {
    Ok(EntityArgumentType::get_optional_entities(context, "entities")?.len() as i32)
}

/// Vanilla `ExecuteCommand.checkMatchingData`.
fn check_matching_data(
    context: &CommandContext,
    accessor: &dyn DataAccessor,
) -> Result<i32, CommandSyntaxError> {
    let path = context.get_argument::<NbtPath>("path")?;
    Ok(path.count_matching(&NbtTag::Compound(accessor.get_data()?)) as i32)
}

fn check_entity_data(context: &CommandContext) -> Result<i32, CommandSyntaxError> {
    let accessor = EntityDataAccessor::new(EntityArgumentType::get_entity(context, "source")?);
    check_matching_data(context, &accessor)
}

fn check_block_data(context: &CommandContext) -> Result<i32, CommandSyntaxError> {
    let pos = BlockPosArgumentType::get_loaded_block_pos(context, "source")?;
    let accessor = BlockDataAccessor::new(pos, context.source.world().clone())?;
    check_matching_data(context, &accessor)
}

fn check_storage_data(context: &CommandContext) -> Result<i32, CommandSyntaxError> {
    let id = context.get_argument::<Identifier>("source")?;
    let accessor = StorageDataAccessor::new(id.to_string(), context.server().clone());
    check_matching_data(context, &accessor)
}

/// Vanilla `ExecuteCommand.addIfBlocksConditional`.
fn add_if_blocks_conditional(
    argument: LiteralArgumentBuilder,
    expected: bool,
    skip_air: bool,
) -> LiteralArgumentBuilder {
    argument
        .fork(
            Redirection::Root,
            RedirectModifier::Custom(Arc::new(move |context: &CommandContext| {
                Ok(expect(
                    context,
                    expected,
                    check_regions(context, skip_air)?.is_some(),
                ))
            })),
        )
        .executes(IfBlocksExecutor { expected, skip_air })
}

struct IfBlocksExecutor {
    expected: bool,
    skip_air: bool,
}

impl CommandExecutor for IfBlocksExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        if self.expected {
            check_if_regions(context, self.skip_air)
        } else {
            check_unless_regions(context, self.skip_air)
        }
    }
}

fn check_if_regions(context: &CommandContext, skip_air: bool) -> CommandExecutorResult {
    let Some(count) = check_regions(context, skip_air)? else {
        return Err(ERROR_CONDITIONAL_FAILED.create_without_context());
    };
    send_conditional_pass_count(context, count);
    Ok(count)
}

fn check_unless_regions(context: &CommandContext, skip_air: bool) -> CommandExecutorResult {
    if let Some(count) = check_regions(context, skip_air)? {
        return Err(ERROR_CONDITIONAL_FAILED_COUNT
            .create_without_context(TextComponent::text(count.to_string())));
    }
    send_conditional_pass(context);
    Ok(1)
}

fn check_regions(
    context: &CommandContext,
    skip_air: bool,
) -> Result<Option<i32>, CommandSyntaxError> {
    check_regions_in(
        context.source.world(),
        BlockPosArgumentType::get_loaded_block_pos(context, "start")?,
        BlockPosArgumentType::get_loaded_block_pos(context, "end")?,
        BlockPosArgumentType::get_loaded_block_pos(context, "destination")?,
        skip_air,
    )
}

fn check_regions_in(
    world: &World,
    start: BlockPos,
    end: BlockPos,
    destination: BlockPos,
    skip_air: bool,
) -> Result<Option<i32>, CommandSyntaxError> {
    let (min_x, max_x) = (start.0.x.min(end.0.x), start.0.x.max(end.0.x));
    let (min_y, max_y) = (start.0.y.min(end.0.y), start.0.y.max(end.0.y));
    let (min_z, max_z) = (start.0.z.min(end.0.z), start.0.z.max(end.0.z));
    let offset = BlockPos::new(
        destination.0.x - min_x,
        destination.0.y - min_y,
        destination.0.z - min_z,
    );
    // Java's long product, wrapping like vanilla's.
    let area = i64::from(max_x - min_x + 1)
        .wrapping_mul(i64::from(max_y - min_y + 1))
        .wrapping_mul(i64::from(max_z - min_z + 1));
    if area > MAX_TEST_AREA {
        return Err(ERROR_AREA_TOO_LARGE.create_without_context(
            TextComponent::text(MAX_TEST_AREA.to_string()),
            TextComponent::text(area.to_string()),
        ));
    }

    let mut count = 0;
    for z in min_z..=max_z {
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let source_pos = BlockPos::new(x, y, z);
                let destination_pos = BlockPos::new(x + offset.0.x, y + offset.0.y, z + offset.0.z);
                let source_state = world.get_block_state_id(&source_pos);
                if skip_air && source_state.to_block_id() == BlockId::AIR {
                    continue;
                }
                if source_state != world.get_block_state_id(&destination_pos) {
                    return Ok(None);
                }
                if let Some(source_entity) = world.get_block_entity(&source_pos) {
                    let Some(destination_entity) = world.get_block_entity(&destination_pos) else {
                        return Ok(None);
                    };
                    if destination_entity.resource_location() != source_entity.resource_location() {
                        return Ok(None);
                    }
                    // Vanilla `saveCustomOnly`; Pumpkin keeps block entity components in it.
                    let mut source_tag = NbtCompound::new();
                    source_entity.write_nbt(&mut source_tag);
                    let mut destination_tag = NbtCompound::new();
                    destination_entity.write_nbt(&mut destination_tag);
                    if source_tag != destination_tag {
                        return Ok(None);
                    }
                }
                count += 1;
            }
        }
    }
    Ok(Some(count))
}

/// Vanilla `ExecuteCommand.addConditionals`.
fn add_conditionals(parent: LiteralArgumentBuilder, expected: bool) -> LiteralArgumentBuilder {
    let score_comparison = |operator: &'static str, predicate: CommandPredicate| {
        literal(operator).then(
            argument("source", ScoreHolderArgumentType::Single)
                .suggests(ScoreHolderSuggestionProvider)
                .then(add_conditional(
                    argument("sourceObjective", ObjectiveArgumentType),
                    expected,
                    predicate,
                )),
        )
    };
    let data_source = |source: RequiredArgumentBuilder, condition: CommandNumericPredicate| {
        source.then(add_numeric_conditional(
            argument("path", NbtPathArgumentType),
            expected,
            condition,
        ))
    };

    parent
        .then(
            literal("block").then(argument("pos", BlockPosArgumentType).then(add_conditional(
                argument("block", BlockPredicateArgumentType),
                expected,
                check_block,
            ))),
        )
        .then(
            literal("biome").then(argument("pos", BlockPosArgumentType).then(add_conditional(
                argument("biome", ResourceOrTagArgument(BIOME_REGISTRY.clone())),
                expected,
                check_biome,
            ))),
        )
        .then(literal("loaded").then(add_conditional(
            argument("pos", BlockPosArgumentType),
            expected,
            check_loaded,
        )))
        .then(literal("dimension").then(add_conditional(
            argument("dimension", DimensionArgumentType),
            expected,
            check_dimension,
        )))
        .then(
            literal("score").then(
                argument("target", ScoreHolderArgumentType::Single)
                    .suggests(ScoreHolderSuggestionProvider)
                    .then(
                        argument("targetObjective", ObjectiveArgumentType)
                            .then(score_comparison("=", |c| check_score(c, |a, b| a == b)))
                            .then(score_comparison("<", |c| check_score(c, |a, b| a < b)))
                            .then(score_comparison("<=", |c| check_score(c, |a, b| a <= b)))
                            .then(score_comparison(">", |c| check_score(c, |a, b| a > b)))
                            .then(score_comparison(">=", |c| check_score(c, |a, b| a >= b)))
                            .then(literal("matches").then(add_conditional(
                                argument("range", IntRangeArgumentType),
                                expected,
                                check_score_range,
                            ))),
                    ),
            ),
        )
        .then(
            literal("blocks").then(
                argument("start", BlockPosArgumentType).then(
                    argument("end", BlockPosArgumentType).then(
                        argument("destination", BlockPosArgumentType)
                            .then(add_if_blocks_conditional(literal("all"), expected, false))
                            .then(add_if_blocks_conditional(literal("masked"), expected, true)),
                    ),
                ),
            ),
        )
        .then(literal("entity").then(add_numeric_conditional(
            argument("entities", EntityArgumentType::Entities),
            expected,
            count_entities,
        )))
        .then(
            literal("function").then(
                argument("name", FunctionArgumentType)
                    .suggests(FunctionSuggestionProvider)
                    .fork(
                        Redirection::Root,
                        RedirectModifier::Custom(Arc::new(move |context: &CommandContext| {
                            execute_if_function(context, expected)
                        })),
                    ),
            ),
        )
        .then(
            literal("stopwatch").then(argument("id", IdentifierArgumentType).then(
                add_conditional(
                    argument("range", FloatRangeArgumentType),
                    expected,
                    check_stopwatch,
                ),
            )),
        )
        .then(
            literal("data")
                .then(literal("entity").then(data_source(
                    argument("source", EntityArgumentType::Entity),
                    check_entity_data,
                )))
                .then(literal("block").then(data_source(
                    argument("source", BlockPosArgumentType),
                    check_block_data,
                )))
                .then(literal("storage").then(data_source(
                    argument("source", IdentifierArgumentType),
                    check_storage_data,
                ))),
        )
}

#[allow(clippy::too_many_lines)]
pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    let builder = command("execute", DESCRIPTION)
        .requires(PERMISSION)
        .then(literal("run").redirect(Redirection::Root))
        .then(
            literal("store")
                .then(wrap_stores(literal("result"), true))
                .then(wrap_stores(literal("success"), false)),
        )
        .then(
            literal("as").then(argument("targets", EntityArgumentType::Entities).fork(
                Redirection::Root,
                RedirectModifier::Custom(Arc::new(execute_as_modifier)),
            )),
        )
        .then(
            literal("at").then(argument("targets", EntityArgumentType::Entities).fork(
                Redirection::Root,
                RedirectModifier::Custom(Arc::new(execute_at_modifier)),
            )),
        )
        .then(literal("in").then(
            argument("dimension", DimensionArgumentType).redirect_with_modifier(
                Redirection::Root,
                RedirectModifier::Custom(Arc::new(execute_in_modifier)),
            ),
        ))
        .then(
            literal("positioned")
                .then(
                    literal("as").then(argument("targets", EntityArgumentType::Entities).fork(
                        Redirection::Root,
                        RedirectModifier::Custom(Arc::new(execute_positioned_as_modifier)),
                    )),
                )
                .then(literal("over").then(
                    argument("heightmap", HeightmapTypeArgumentType).redirect_with_modifier(
                        Redirection::Root,
                        RedirectModifier::Custom(Arc::new(execute_positioned_over_modifier)),
                    ),
                ))
                .then(
                    argument("pos", Vec3ArgumentType::Default).redirect_with_modifier(
                        Redirection::Root,
                        RedirectModifier::Custom(Arc::new(execute_positioned_modifier)),
                    ),
                ),
        )
        .then(
            literal("rotated")
                .then(
                    literal("as").then(argument("targets", EntityArgumentType::Entities).fork(
                        Redirection::Root,
                        RedirectModifier::Custom(Arc::new(execute_rotated_as_modifier)),
                    )),
                )
                .then(
                    argument("rotation", RotationArgumentType).redirect_with_modifier(
                        Redirection::Root,
                        RedirectModifier::Custom(Arc::new(execute_rotated_modifier)),
                    ),
                ),
        )
        .then(literal("align").then(
            argument("axes", SwizzleArgumentType).redirect_with_modifier(
                Redirection::Root,
                RedirectModifier::Custom(Arc::new(execute_align_modifier)),
            ),
        ))
        .then(literal("anchored").then(
            argument("anchor", EntityAnchorArgumentType).redirect_with_modifier(
                Redirection::Root,
                RedirectModifier::Custom(Arc::new(execute_anchored_modifier)),
            ),
        ))
        .then(
            literal("facing")
                .then(literal("entity").then(
                    argument("targets", EntityArgumentType::Entities).then(
                        argument("anchor", EntityAnchorArgumentType).fork(
                            Redirection::Root,
                            RedirectModifier::Custom(Arc::new(execute_facing_entity_modifier)),
                        ),
                    ),
                ))
                .then(
                    argument("pos", Vec3ArgumentType::Default).redirect_with_modifier(
                        Redirection::Root,
                        RedirectModifier::Custom(Arc::new(execute_facing_modifier)),
                    ),
                ),
        )
        .then(literal("summon").then(
            argument("entity_type", ENTITY_TYPE_ARGUMENT.clone()).redirect_with_modifier(
                Redirection::Root,
                RedirectModifier::Custom(Arc::new(execute_summon_modifier)),
            ),
        ))
        .then(
            literal("on")
                .then(literal("owner").fork(
                    Redirection::Root,
                    RedirectModifier::Custom(Arc::new(execute_on_owner_modifier)),
                ))
                .then(literal("leasher").fork(
                    Redirection::Root,
                    RedirectModifier::Custom(Arc::new(execute_on_leasher_modifier)),
                ))
                .then(literal("target").fork(
                    Redirection::Root,
                    RedirectModifier::Custom(Arc::new(execute_on_target_modifier)),
                ))
                .then(literal("attacker").fork(
                    Redirection::Root,
                    RedirectModifier::Custom(Arc::new(execute_on_attacker_modifier)),
                ))
                .then(literal("vehicle").fork(
                    Redirection::Root,
                    RedirectModifier::Custom(Arc::new(execute_on_vehicle_modifier)),
                ))
                .then(literal("controller").fork(
                    Redirection::Root,
                    RedirectModifier::Custom(Arc::new(execute_on_controller_modifier)),
                ))
                .then(literal("origin").fork(
                    Redirection::Root,
                    RedirectModifier::Custom(Arc::new(execute_on_origin_modifier)),
                ))
                .then(literal("passengers").fork(
                    Redirection::Root,
                    RedirectModifier::Custom(Arc::new(execute_on_passengers_modifier)),
                )),
        )
        .then(add_conditionals(literal("if"), true))
        .then(add_conditionals(literal("unless"), false));

    let execute_node_id = dispatcher.register(builder);

    set_redirects_to_execute(
        &mut dispatcher.tree,
        NodeId::from(execute_node_id),
        execute_node_id,
    );
}

fn set_redirects_to_execute(tree: &mut Tree, parent: NodeId, execute_id: CommandNodeId) {
    for child_id in tree.get_children(parent) {
        if let Some(redirect) = tree[child_id].redirect()
            && matches!(redirect, Redirection::Root)
            && tree[child_id].name() != "run"
        {
            tree[child_id].set_redirect(Some(Redirection::Local(NodeId::from(execute_id))));
        }
        set_redirects_to_execute(tree, child_id, execute_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandSource;
    use crate::world::scoreboard::{NoTarget, ScoreboardObjective};
    use pumpkin_protocol::java::client::play::RenderType;

    #[test]
    fn score_lookup_distinguishes_missing_objectives_and_scores() {
        let mut scoreboard = Scoreboard::new();
        let error = get_score_value(&scoreboard, "#target", "missing").unwrap_err();
        let message = serde_json::to_value(error.message).unwrap();
        assert_eq!(
            message["translate"],
            translation::java::ARGUMENTS_OBJECTIVE_NOTFOUND
        );
        assert_eq!(message["with"][0]["text"], "missing");

        for criterion in ["dummy", "health"] {
            scoreboard.add_objective(
                &NoTarget,
                ScoreboardObjective::new(
                    criterion,
                    TextComponent::text(criterion),
                    RenderType::Integer,
                    None,
                    criterion,
                ),
            );
            assert_eq!(
                get_score_value(&scoreboard, "#target", criterion).unwrap(),
                None
            );
            assert!(scoreboard.get_score("#target", criterion).is_none());
            for value in [0, -7, i32::MAX] {
                scoreboard.set_score_value(&NoTarget, "#target", criterion, value);
                assert_eq!(
                    get_score_value(&scoreboard, "#target", criterion).unwrap(),
                    Some(value)
                );
            }
        }
    }

    #[test]
    fn add_conditionals_accept_single_score_holders() {
        let mut dispatcher = CommandDispatcher::new();
        register(&mut dispatcher, &PermissionRegistry::default());
        let source = Arc::new(CommandSource::dummy());
        for condition in ["if", "unless"] {
            for holder in ["Fake", "#temp", "$x", "@s", "@e[limit=1]"] {
                for comparison in [
                    "matches 1..",
                    "= @s other",
                    "< @e[limit=1] other",
                    "<= $reference other",
                    "> #reference other",
                    ">= Fake other",
                ] {
                    let input = format!("execute {condition} score {holder} test {comparison}");
                    let result = dispatcher.parse_input(&input, &source);
                    assert!(result.errors.is_empty(), "{input}: {:?}", result.errors);
                    assert_eq!(result.reader.remaining_length(), 0, "{input}");
                    assert!(result.context.build(&input).command.is_some(), "{input}");
                }
            }
        }
    }

    #[test]
    fn add_conditionals_reject_multiple_holder_selectors() {
        let mut dispatcher = CommandDispatcher::new();
        register(&mut dispatcher, &PermissionRegistry::default());
        let source = Arc::new(CommandSource::dummy());
        for condition in ["if", "unless"] {
            for arguments in [
                "@a test matches 1",
                "@e test matches 1",
                "Fake test = @a other",
                "@s test >= @e other",
            ] {
                let input = format!("execute {condition} score {arguments}");
                let result = dispatcher.parse_input(&input, &source);
                assert!(!result.errors.is_empty(), "{input}");
            }
        }
    }
}
