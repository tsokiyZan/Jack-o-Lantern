use crate::block::registry::BlockActionResult;
use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::sync::Arc;

use crate::block::entities::mob_spawner::MobSpawnerBlockEntity;
use crate::entity::EntityBase;
use crate::entity::mob::spawn::finalize_spawn;
use crate::entity::player::Player;
use crate::entity::r#type::from_type;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::plugin::api::events::entity::creature_spawn::CreatureSpawnReason;
use crate::server::Server;
use crate::world::World;
use pumpkin_data::data_component_impl::{
    AxolotlVariantImpl, CatVariantImpl, ChickenVariantImpl, CowVariantImpl, EntityDataImpl,
    FoxVariantImpl, FrogVariantImpl, HorseVariantImpl, LlamaVariantImpl, MooshroomVariantImpl,
    PigVariantImpl, RabbitVariantImpl, SheepColorImpl, ShulkerColorImpl, VillagerVariantImpl,
    WolfVariantImpl, ZombieNautilusVariantImpl,
};
use pumpkin_data::entity::EntityType;
use pumpkin_data::entity::entity_from_egg;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::translation;
use pumpkin_data::{Block, BlockDirection};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::Difficulty;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::wrap_degrees;
use pumpkin_util::permission::PermissionLvl;
use pumpkin_util::text::TextComponent;
use uuid::Uuid;

pub struct SpawnEggItem;

impl ItemMetadata for SpawnEggItem {
    fn ids() -> Box<[u16]> {
        pumpkin_data::entity::spawn_egg_ids()
    }
}

/// Vanilla's `EntityTypes.OP_ONLY_CUSTOM_DATA`: types whose `entity_data` only a server
/// operator may set. None of them has a spawn egg today, but a datapack-authored loot table
/// can still hand out an item with this component, so the gate is checked here too.
fn only_op_can_set_nbt(entity_type: &EntityType) -> bool {
    std::ptr::eq(entity_type, &EntityType::FALLING_BLOCK)
        || std::ptr::eq(entity_type, &EntityType::COMMAND_BLOCK_MINECART)
        || std::ptr::eq(entity_type, &EntityType::SPAWNER_MINECART)
}

/// Permission node for op-only `entity_data`, so permission plugins can grant or deny it.
pub const NBT_PLACE_PERMISSION: &str = "minecraft:nbt.place";

/// Vanilla `PlayerList.isOp`: any ops list entry, whatever its level, passes the node's
/// `Op(One)` default, so without plugins or attachments the result matches vanilla.
fn can_place_op_nbt(player: &Player) -> bool {
    let world = player.world();
    let Some(server) = world.server.upgrade() else {
        return false;
    };
    let Some(player) = world.get_player_by_uuid(player.gameprofile.id) else {
        return false;
    };
    let is_op = server
        .data
        .operator_config
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_entry(&player.gameprofile.id)
        .is_some();
    let level = if is_op {
        PermissionLvl::Four
    } else {
        PermissionLvl::Zero
    };
    player.has_permission_at_level(&server, NBT_PLACE_PERMISSION, level)
}

/// Loads the stack's `entity_data` NBT into the mob. Identity and placement stay as spawned.
///
/// `user` is the player who caused the spawn, matching vanilla's `updateCustomEntityTag` user
/// argument; it is `None` for non-player sources such as a dispenser.
fn apply_entity_data(item: &ItemStack, mob: &dyn EntityBase, user: Option<&Player>) {
    let Some(nbt) = item
        .get_data_component::<EntityDataImpl>()
        .and_then(|comp| comp.nbt.as_ref())
    else {
        return;
    };
    let entity_type = mob.get_entity().entity_type;
    // Vanilla EntityType.updateCustomEntityTag loads the data only into the type it names.
    if let Some(id) = nbt.get_string("id")
        && id.strip_prefix("minecraft:").unwrap_or(id) != entity_type.resource_name
    {
        return;
    }
    if only_op_can_set_nbt(entity_type) && !user.is_some_and(can_place_op_nbt) {
        return;
    }
    let mut nbt = nbt.clone();
    for key in ["id", "UUID", "Pos"] {
        nbt.child_tags.remove(key);
    }
    if !nbt.is_empty() {
        // Vanilla TypedEntityData.loadInto merges into the mob's own save so unset keys keep their state.
        let mut merged = NbtCompound::new();
        mob.write_nbt(&mut merged);
        merged.merge(&nbt);
        mob.read_nbt_non_mut(&merged);
    }
}

/// Vanilla `EntityType.appendDefaultStackConfig`: item components first, `entity_data` last.
pub(crate) fn apply_entity_variant(item: &ItemStack, mob: &dyn EntityBase, user: Option<&Player>) {
    apply_variant(item, mob);
    apply_entity_data(item, mob, user);
}

fn apply_variant(item: &ItemStack, mob: &dyn EntityBase) {
    if mob
        .get_mob()
        .is_some_and(|mob| mob.mob_apply_item_components(item))
    {
        return;
    }
    macro_rules! apply_variant {
        ($($ty:ty),+ $(,)?) => {
            $(
                if let Some(comp) = item.get_data_component::<$ty>() {
                    mob.set_variant_name(&comp.value);
                    return;
                }
            )+
        };
    }
    apply_variant!(
        ChickenVariantImpl,
        FrogVariantImpl,
        WolfVariantImpl,
        CatVariantImpl,
        VillagerVariantImpl,
        FoxVariantImpl,
        MooshroomVariantImpl,
        RabbitVariantImpl,
        PigVariantImpl,
        CowVariantImpl,
        HorseVariantImpl,
        LlamaVariantImpl,
        AxolotlVariantImpl,
        SheepColorImpl,
        ShulkerColorImpl,
        ZombieNautilusVariantImpl,
    );
}

/// Finalizes a mob spawned by a spawn egg, then applies the egg's components (vanilla order).
pub(crate) fn prepare_egg_mob(
    item: &ItemStack,
    mob: &Arc<dyn EntityBase>,
    world: &Arc<World>,
    user: Option<&Player>,
    reason: crate::entity::mob::spawn::EntitySpawnReason,
) {
    finalize_spawn(mob, world, reason, None);
    apply_entity_variant(item, mob.as_ref(), user);
}

impl ItemBehaviour for SpawnEggItem {
    /// Vanilla `SpawnEggItem.use`: only a click on a liquid spawns.
    fn normal_use(&self, item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let world = player.world();
        let (start_pos, end_pos) = self.get_start_and_end_pos(player);
        let hit = world.clip(&pumpkin_world::clip::ClipContext::new(
            start_pos,
            end_pos,
            pumpkin_world::clip::ClipBlock::Outline,
            pumpkin_world::clip::ClipFluid::SourceOnly,
        ));

        if hit.is_miss {
            return InteractionResult::PASS;
        }
        let Some(entity_type) = entity_from_egg(item.id) else {
            return InteractionResult::FAIL;
        };

        if !can_spawn(entity_type, &world) {
            return InteractionResult::FAIL;
        }

        let hit_pos = hit.block_pos;
        // `getBlockState(pos).getBlock() instanceof LiquidBlock`.
        let hit_block = world.get_block(&hit_pos);
        if hit_block != &Block::WATER && hit_block != &Block::LAVA {
            return InteractionResult::PASS;
        }

        let mut stack = player.inventory.get_stack_in_hand(hand);
        if !spawn_mob(
            entity_type,
            player,
            &mut stack,
            &world,
            hit_pos,
            false,
            false,
        ) {
            return InteractionResult::FAIL;
        }
        player.inventory.set_stack_in_hand(hand, stack);
        player.award_item_used(item);
        InteractionResult::SUCCESS
    }

    /// Vanilla `SpawnEggItem.useOn`.
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        _hand: pumpkin_util::Hand,
        location: BlockPos,
        face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        _block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        let world = player.world();
        let Some(entity_type) = entity_from_egg(item.item.id) else {
            return BlockActionResult::Pass;
        };
        if !can_spawn(entity_type, &world) {
            return BlockActionResult::Fail;
        }

        if let Some(block_entity) = world.get_block_entity(&location) {
            let spawner = block_entity
                .as_any()
                .downcast_ref::<MobSpawnerBlockEntity>();
            let trial_spawner = block_entity
                .as_any()
                .downcast_ref::<crate::block::entities::trial_spawner::TrialSpawnerBlockEntity>(
            );
            if spawner.is_some() || trial_spawner.is_some() {
                if !world.level_info.load().game_rules.spawner_blocks_work {
                    player.send_system_message(&TextComponent::translate(
                        translation::java::ADVMODE_NOTENABLED_SPAWNER,
                        [],
                    ));
                    return BlockActionResult::Fail;
                }
                if let Some(spawner) = spawner {
                    spawner.set_entity_type(entity_type);
                } else if let Some(trial_spawner) = trial_spawner {
                    trial_spawner.set_entity_type(entity_type, &world);
                }
                world.update_block_entity(&block_entity);
                world.game_event_from(
                    Some(player),
                    GameEvent::BlockChange,
                    location.to_centered_f64(),
                );
                item.decrement_unless_creative(player.gamemode.load(), 1);
                return BlockActionResult::Success;
            }
        }

        // `blockState.getCollisionShape(level, pos).isEmpty()`.
        let spawn_block_pos = if world
            .get_block_state(&location)
            .get_block_collision_shapes()
            .next()
            .is_none()
        {
            location
        } else {
            location.offset(face.to_offset())
        };
        let moved_up = spawn_block_pos != location && face == BlockDirection::Up;
        if spawn_mob(
            entity_type,
            player,
            item,
            &world,
            spawn_block_pos,
            true,
            moved_up,
        ) {
            BlockActionResult::Success
        } else {
            BlockActionResult::Fail
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Vanilla `EntityType.canSpawn`: monsters and other types not allowed in peaceful need
/// another difficulty.
fn can_spawn(entity_type: &EntityType, world: &World) -> bool {
    entity_type.allowed_in_peaceful || world.level_info.load().difficulty != Difficulty::Peaceful
}

/// Vanilla `SpawnEggItem.spawnMob`: `EntityType.spawn` with `SPAWN_ITEM_USE`, then the stack
/// is consumed and `ENTITY_PLACE` emitted. Returns whether the mob spawned.
fn spawn_mob(
    entity_type: &'static EntityType,
    player: &Player,
    item: &mut ItemStack,
    world: &Arc<World>,
    spawn_pos: BlockPos,
    try_move_down: bool,
    moved_up: bool,
) -> bool {
    // `EntityType.create`: with `tryMoveDown` the mob drops onto the collision below.
    let y_offset = if try_move_down {
        let half_width = f64::from(entity_type.dimension[0]) / 2.0;
        let height = f64::from(entity_type.dimension[1]);
        let (x, y, z) = (
            f64::from(spawn_pos.0.x) + 0.5,
            f64::from(spawn_pos.0.y) + 1.0,
            f64::from(spawn_pos.0.z) + 0.5,
        );
        let entity_box = BoundingBox::new(
            Vector3::new(x - half_width, y, z - half_width),
            Vector3::new(x + half_width, y + height, z + half_width),
        );
        crate::item::spawn_y_offset(world, spawn_pos, moved_up, &entity_box)
    } else {
        0.0
    };
    let pos = Vector3::new(
        f64::from(spawn_pos.0.x) + 0.5,
        f64::from(spawn_pos.0.y) + y_offset,
        f64::from(spawn_pos.0.z) + 0.5,
    );
    // `EntityType.create`: `wrapDegrees(level.getRandom().nextFloat() * 360)`.
    let yaw = wrap_degrees(world.random_f32() * 360.0);
    let mob = from_type(entity_type, pos, world, Uuid::new_v4());
    mob.get_entity().set_rotation(yaw, 0.0);

    prepare_egg_mob(
        item,
        &mob,
        world,
        Some(player),
        crate::entity::mob::spawn::EntitySpawnReason::SpawnItemUse,
    );
    if !world.spawn_creature(
        mob,
        CreatureSpawnReason::SpawnerEgg,
        world.get_player_by_uuid(player.gameprofile.id),
    ) {
        return false;
    }
    item.decrement_unless_creative(player.gamemode.load(), 1);
    world.game_event_from(
        Some(player),
        GameEvent::EntityPlace,
        spawn_pos.to_centered_f64(),
    );
    true
}

/// Vanilla `SpawnEggItem.spawnOffspringFromSpawnEgg`: returns whether a baby spawned.
///
/// It is the spawn egg step of `Mob.checkAndHandleImportantInteractions`: an egg of the mob's
/// own type spawns a baby of it. `SpawnEggItem` has no `interactLivingEntity`, so the egg does
/// nothing as an item used on an entity.
pub fn spawn_offspring_from_spawn_egg(
    item: &mut ItemStack,
    player: &Player,
    entity: &Arc<dyn EntityBase>,
) -> bool {
    if let Some(entity_type) = entity_from_egg(item.item.id)
        && entity.get_entity().entity_type.id == entity_type.id
    {
        let world = player.world();
        let pos = entity.get_entity().pos.load();
        let mob = from_type(entity_type, pos, &world, Uuid::new_v4());
        // no baby form, no offspring.
        if !mob
            .get_mob()
            .is_some_and(crate::entity::mob::Mob::spawn_as_baby)
        {
            return false;
        }
        // `offspring.snapTo(pos, 0, 0)`.
        mob.get_entity().set_rotation(0.0, 0.0);
        apply_entity_variant(item, mob.as_ref(), Some(player));
        if world.spawn_creature(
            mob,
            CreatureSpawnReason::SpawnerEgg,
            world.get_player_by_uuid(player.gameprofile.id),
        ) {
            item.decrement_unless_creative(player.gamemode.load(), 1);
            return true;
        }
    }
    false
}
