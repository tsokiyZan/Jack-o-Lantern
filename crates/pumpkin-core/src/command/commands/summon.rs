use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::difficulty::Difficulty;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use uuid::Uuid;

use crate::command::argument_builder::{ArgumentBuilder, argument, command};
use crate::command::argument_types::coordinates::vec3::Vec3ArgumentType;
use crate::command::argument_types::nbt::NbtCompoundArgumentType;
use crate::command::argument_types::resource::{ENTITY_TYPE_ARGUMENT, ResourceArgument};
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::entity::EntityBase;
use crate::entity::mob::spawn::finalize_spawn;
use crate::entity::r#type::from_type;
use crate::world::World;
use pumpkin_data::entity::EntityType;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;

const DESCRIPTION: &str = "Spawns an entity at position.";
const PERMISSION: &str = "minecraft:command.summon";

const ERROR_FAILED_PEACEFUL: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_SUMMON_FAILED_PEACEFUL,
    translation::bedrock::COMMANDS_SUMMON_FAILED,
);
const ERROR_DUPLICATE_UUID: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_SUMMON_FAILED_UUID,
    translation::bedrock::COMMANDS_SUMMON_FAILED,
);
const INVALID_POSITION: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_SUMMON_INVALIDPOSITION,
    translation::bedrock::COMMANDS_SUMMON_OUTOFWORLD,
);

struct SummonExecutor {
    has_pos: bool,
    has_nbt: bool,
}

impl CommandExecutor for SummonExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let entity_type = ResourceArgument::get_summonable_entity_type(context, "entity")?;
        let pos = if self.has_pos {
            Vec3ArgumentType::get_coordinates(context, "pos")?.resolve(&context.source)
        } else {
            context.source.position
        };

        let block_pos = BlockPos::floored_v(pos);
        if !World::is_valid(block_pos) {
            return Err(INVALID_POSITION.create_without_context());
        }

        let world = context.source.world();
        let difficulty = world.level_info.load().difficulty;
        if difficulty == Difficulty::Peaceful && !entity_type.allowed_in_peaceful {
            return Err(ERROR_FAILED_PEACEFUL.create_without_context());
        }

        let nbt = if self.has_nbt {
            Some(NbtCompoundArgumentType::get(context, "nbt")?)
        } else {
            None
        };

        let empty = NbtCompound::new();
        // Vanilla `SummonCommand.createEntity` always loads the entity from a tag (an empty one
        // without the NBT argument), so unset fields take their load defaults, e.g.
        // `CanPickUpLoot` false.
        let mut loaded = Vec::new();
        let entity =
            load_entity_recursive(entity_type, nbt.unwrap_or(&empty), pos, world, &mut loaded);

        // Vanilla `tryAddFreshEntityWithPassengers`: no entity in the stack may reuse a loaded UUID.
        let entities = world.entities.load();
        if loaded.iter().any(|(e, _)| {
            entities
                .iter()
                .any(|w| w.get_entity().entity_uuid == e.get_entity().entity_uuid)
        }) {
            return Err(ERROR_DUPLICATE_UUID.create_without_context());
        }

        let name = entity.get_display_name();
        // Vanilla only finalizes the spawn when no NBT argument was given.
        if !self.has_nbt {
            finalize_spawn(
                &entity,
                world,
                crate::entity::mob::spawn::EntitySpawnReason::Command,
                None,
            );
        }
        // Vanilla `addFreshEntityWithPassengers`: the entity, then its passengers, each one riding
        // the vehicle it was loaded under.
        for (e, vehicle) in loaded {
            world.spawn_entity(e.clone());
            if let Some(vehicle) = vehicle {
                vehicle
                    .get_entity()
                    .add_passenger_without_event(vehicle.clone(), e);
            }
        }

        context.source.send_feedback(
            TextComponent::translate_cross(
                translation::java::COMMANDS_SUMMON_SUCCESS,
                translation::bedrock::COMMANDS_SUMMON_SUCCESS,
                [name],
            ),
            true,
        );

        Ok(1)
    }
}

/// A loaded entity and the vehicle it rides.
type Loaded = (Arc<dyn EntityBase>, Option<Arc<dyn EntityBase>>);

/// Vanilla `EntityType.loadEntityRecursive` with `SummonCommand`'s post-load step (every entity is
/// snapped to the summon position): the entity and, depth first, the `Passengers` it carries.
/// `loaded` gets each entity with the vehicle it rides, vehicles first.
///
/// Not `World::load_entity_recursive`: that one is `loadEntityRecursive` with `addWithUUID` as
/// the post-load step (the entity enters the level as it loads). Summon's post-load step only
/// snaps, and the whole stack enters the level afterwards, after its duplicate-UUID check
/// (`tryAddFreshEntityWithPassengers`); Pumpkin mounts only entities already in the level, so
/// the mounts are collected here and made after the spawn.
fn load_entity_recursive(
    entity_type: &'static EntityType,
    nbt: &NbtCompound,
    pos: Vector3<f64>,
    world: &Arc<World>,
    loaded: &mut Vec<Loaded>,
) -> Arc<dyn EntityBase> {
    let uuid = nbt.get_uuid("UUID").unwrap_or_else(Uuid::new_v4);
    let entity = from_type(entity_type, pos, world, uuid);
    entity.read_nbt_non_mut(nbt);
    // `snapTo` → `reapplyPosition` → the virtual `setPos`.
    entity.set_position(pos);
    let index = loaded.len();
    loaded.push((entity.clone(), None));
    for passenger in nbt.get_list("Passengers").unwrap_or_default() {
        // `loadStaticEntity`: a passenger without a known `id` is skipped.
        let NbtTag::Compound(passenger) = passenger else {
            continue;
        };
        let Some(passenger_type) = passenger.get_string("id").and_then(EntityType::from_name)
        else {
            continue;
        };
        let start = loaded.len();
        load_entity_recursive(passenger_type, passenger, pos, world, loaded);
        // `startRiding(entity, true, false)`.
        loaded[start].1 = Some(loaded[index].0.clone());
    }
    entity
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("summon", DESCRIPTION).requires(PERMISSION).then(
            argument("entity", ENTITY_TYPE_ARGUMENT.clone())
                .executes(SummonExecutor {
                    has_pos: false,
                    has_nbt: false,
                })
                .then(
                    argument("pos", Vec3ArgumentType::Default)
                        .executes(SummonExecutor {
                            has_pos: true,
                            has_nbt: false,
                        })
                        .then(
                            argument("nbt", NbtCompoundArgumentType).executes(SummonExecutor {
                                has_pos: true,
                                has_nbt: true,
                            }),
                        ),
                ),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::commands::fetchprofile;
    use crate::command::context::command_source::CommandSource;
    use std::sync::Arc;

    #[test]
    fn parse_summon_cat() {
        let mut dispatcher = CommandDispatcher::new();
        let registry = PermissionRegistry::default();

        // Register fetchprofile first to ensure its "entity" argument doesn't collide
        fetchprofile::register(&mut dispatcher, &registry);
        register(&mut dispatcher, &registry);

        let source = Arc::new(CommandSource::dummy());
        let result = dispatcher.parse_input("summon cat", &source);
        assert!(result.errors.is_empty());
        let context = result.context.build("summon cat");
        let entity_type = ResourceArgument::get_summonable_entity_type(&context, "entity")
            .expect("Should resolve summonable cat entity type");
        assert_eq!(entity_type.resource_name, "cat");
    }

    #[test]
    fn parse_summon_cat_with_pos_and_nbt() {
        let mut dispatcher = CommandDispatcher::new();
        let registry = PermissionRegistry::default();

        fetchprofile::register(&mut dispatcher, &registry);
        register(&mut dispatcher, &registry);

        let source = Arc::new(CommandSource::dummy());
        let result = dispatcher.parse_input("summon cat ~ ~ ~ {NoAI:1b}", &source);
        assert!(result.errors.is_empty());
        let context = result.context.build("summon cat ~ ~ ~ {NoAI:1b}");
        let entity_type = ResourceArgument::get_summonable_entity_type(&context, "entity")
            .expect("Should resolve summonable cat entity type");
        assert_eq!(entity_type.resource_name, "cat");
        let nbt =
            NbtCompoundArgumentType::get(&context, "nbt").expect("Should resolve NBT argument");
        assert_eq!(nbt.get_byte("NoAI"), Some(1));
    }
}
