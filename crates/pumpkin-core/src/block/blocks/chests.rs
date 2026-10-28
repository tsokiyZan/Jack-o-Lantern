use std::sync::Arc;

use crate::block::entities::BlockEntity;
use crate::block::entities::chest::ChestBlockEntity;
use crate::block::entities::trapped_chest::TrappedChestBlockEntity;
use pumpkin_data::BlockStateId;
use pumpkin_data::block_properties::{ChestLikeProperties, ChestType, HorizontalFacing};
use pumpkin_data::entity::{EntityPose, EntityType};
use pumpkin_data::fluid::Fluid;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockDirection, BlockId, translation};
use pumpkin_inventory::Inventory;
use pumpkin_inventory::double::DoubleInventory;
use pumpkin_inventory::generic_container_screen_handler::{create_generic_9x3, create_generic_9x6};
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_inventory::screen_handler::{
    InventoryPlayer, ScreenHandlerFactory, SharedScreenHandler,
};
use pumpkin_macros::{pumpkin_block, pumpkin_block_from_tag};
use pumpkin_util::GameMode;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::text::TextComponent;
use pumpkin_world::tick::TickPriority;
use std::sync::Mutex;

use crate::block::{
    BlockBehaviour, EmitsRedstonePowerArgs, GetComparatorOutputArgs, GetRedstonePowerArgs,
    GetScreenHandlerFactoryArgs, GetStateForNeighborUpdateArgs, NormalUseArgs, OnPlaceArgs,
    OnScheduledTickArgs, OnSyncedBlockEventArgs, PathComputationType, PlacedArgs, RandomTickArgs,
    registry::BlockActionResult,
};
use crate::entity::EntityBase;
use crate::entity::ai::util::goal_utils;
use crate::entity::mob::Mob;
use crate::entity::passive::copper_golem::CopperGolemEntity;
use crate::entity::player::Player;
use crate::world::World;
use crate::world::loot::{container_unpack_params, fill_chest_inventory_handle};
use pumpkin_data::BlockState;

struct ChestScreenFactory {
    inventory: Arc<dyn Inventory>,
    loot_pending: bool,
}

impl ScreenHandlerFactory for ChestScreenFactory {
    fn create_screen_handler(
        &self,
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        player: &dyn InventoryPlayer,
    ) -> Option<SharedScreenHandler> {
        if self.loot_pending && player.is_spectator() {
            return None;
        }

        let concrete_handler = if self.inventory.size() > 27 {
            create_generic_9x6(sync_id, player_inventory, self.inventory.clone(), player)
        } else {
            create_generic_9x3(sync_id, player_inventory, self.inventory.clone(), player)
        };

        let concrete_arc = Arc::new(Mutex::new(concrete_handler));

        Some(concrete_arc as SharedScreenHandler)
    }

    fn get_display_name(&self) -> TextComponent {
        if self.inventory.size() > 27 {
            pumpkin_macros::translate_cross!(
                translation::java::CONTAINER_CHESTDOUBLE,
                translation::bedrock::CONTAINER_CHESTDOUBLE
            )
        } else {
            pumpkin_macros::translate_cross!(
                translation::java::CONTAINER_CHEST,
                translation::bedrock::CONTAINER_CHEST
            )
        }
    }
}

// Shared chest behavior implementations
const LID_ANIMATION_EVENT_TYPE: u8 = 1;

fn on_place_chest_impl(args: &OnPlaceArgs<'_>) -> BlockStateId {
    let mut chest_props = ChestLikeProperties::default(args.block);
    chest_props.waterlogged = args.replacing.water_source();

    let (r#type, facing) = compute_chest_props(
        args.world,
        args.player,
        args.block,
        args.position,
        args.direction,
    );
    chest_props.facing = facing;
    chest_props.r#type = r#type;

    chest_props.to_state_id(args.block)
}

fn placed_chest_impl<E: BlockEntity + 'static>(
    args: &PlacedArgs<'_>,
    create_entity: impl FnOnce(BlockPos) -> E,
) {
    if !args.is_new_block() {
        return;
    }
    let chest = create_entity(*args.position);
    args.world.add_block_entity(Arc::new(chest));
}

/// Vanilla `ChestBlock.getConnectedDirection`, or `None` for a single chest.
#[must_use]
pub fn get_connected_direction(state_id: BlockStateId) -> Option<HorizontalFacing> {
    let chest_props = ChestLikeProperties::from_state_id(state_id);
    match chest_props.r#type {
        ChestType::Single => None,
        ChestType::Left => Some(chest_props.facing.rotate_clockwise()),
        ChestType::Right => Some(chest_props.facing.rotate_counter_clockwise()),
    }
}

/// Vanilla `ChestBlock.getAnalogOutputSignal`: `getContainer(..., false)`, so a blocked chest
/// reads zero.
fn get_chest_comparator_output(args: &GetComparatorOutputArgs<'_>) -> u8 {
    get_container(args.world, args.position).map_or(0, |inventory| {
        crate::block::calculate_comparator_output(inventory.as_ref())
    })
}

/// Vanilla `RandomizableContainer.unpackLootTable(player)`: fills the chest from its pending
/// loot table.
pub fn unpack_loot_table(world: &World, entity: &Arc<dyn BlockEntity>, player: Option<&Player>) {
    if let Some((loot_key, seed)) = entity.take_loot_table()
        && let Some(table) = world.get_loot_table(&loot_key)
        && let Some(inv) = entity.clone().get_inventory()
    {
        fill_chest_inventory_handle(
            &inv,
            &table,
            world,
            seed,
            &container_unpack_params(entity.get_position().to_centered_f64(), player),
        );
        inv.mark_dirty();
    }
}

/// Vanilla `instanceof ChestBlock`: chests, trapped chests and every copper chest.
#[must_use]
pub fn is_chest_block(block: &Block) -> bool {
    block == &Block::CHEST
        || block == &Block::TRAPPED_CHEST
        || block.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS)
}

/// Vanilla `ChestBlock.getConnectedDirection` for a double chest half.
fn get_double_connected_direction(props: ChestLikeProperties) -> HorizontalFacing {
    if props.r#type == ChestType::Left {
        props.facing.rotate_clockwise()
    } else {
        props.facing.rotate_counter_clockwise()
    }
}

/// Vanilla `ChestBlock.getConnectedBlockPos`.
#[must_use]
pub fn get_connected_block_pos(pos: &BlockPos, props: &ChestLikeProperties) -> BlockPos {
    pos.offset(get_double_connected_direction(*props).to_offset())
}

/// The chest block entity at `pos` when it is of the type `block` makes, vanilla's
/// `blockEntityType.getBlockEntity(level, pos)`.
fn get_chest_block_entity(
    world: &World,
    pos: &BlockPos,
    block: &Block,
) -> Option<Arc<dyn BlockEntity>> {
    let block_entity = world.get_block_entity(pos)?;
    let any = block_entity.as_any();
    let matches = if block == &Block::TRAPPED_CHEST {
        any.is::<TrappedChestBlockEntity>()
    } else {
        any.is::<ChestBlockEntity>()
    };
    matches.then_some(block_entity)
}

/// Vanilla `ChestBlock.getContainer(block, state, level, pos, false)`, which pairs the halves
/// of a double chest through `DoubleBlockCombiner.combineWithNeigbour`.
#[must_use]
pub fn get_container(world: &World, pos: &BlockPos) -> Option<Arc<dyn Inventory>> {
    let (block, state_id) = world.get_block_and_state_id(pos);
    if !is_chest_block(block) {
        return None;
    }
    let block_entity = get_chest_block_entity(world, pos, block)?;
    if is_chest_blocked(world, pos) {
        return None;
    }
    let inventory = block_entity.get_inventory()?;
    let props = ChestLikeProperties::from_state_id(state_id);
    if props.r#type == ChestType::Single {
        return Some(inventory);
    }
    let neighbor_pos = get_connected_block_pos(pos, &props);
    let (neighbor_block, neighbor_state_id) = world.get_block_and_state_id(&neighbor_pos);
    if neighbor_block == block {
        let neighbor_props = ChestLikeProperties::from_state_id(neighbor_state_id);
        if neighbor_props.r#type != ChestType::Single
            && neighbor_props.r#type != props.r#type
            && neighbor_props.facing == props.facing
        {
            if is_chest_blocked(world, &neighbor_pos) {
                return None;
            }
            if let Some(neighbor) = get_chest_block_entity(world, &neighbor_pos, block)
                .and_then(BlockEntity::get_inventory)
            {
                // `ChestBlock.getBlockType`: the right half comes first.
                return Some(if props.r#type == ChestType::Right {
                    DoubleInventory::new(inventory, neighbor)
                } else {
                    DoubleInventory::new(neighbor, inventory)
                });
            }
        }
    }
    Some(inventory)
}

/// Vanilla `ChestBlockEntity.getEntitiesWithContainerOpen`: the players with this chest's menu
/// open and the copper golems interacting with it.
#[must_use]
pub fn get_entities_with_container_open(world: &World, pos: &BlockPos) -> Vec<Arc<dyn EntityBase>> {
    let props = ChestLikeProperties::from_state_id(world.get_block_state_id(pos));
    let connected_pos = (is_chest_block(world.get_block(pos)) && props.r#type != ChestType::Single)
        .then(|| get_connected_block_pos(pos, &props));
    // `ChestBlockEntity.isOwnContainer`: the menu is this chest, or the double chest holding it.
    let mut users: Vec<Arc<dyn EntityBase>> = world
        .players
        .load()
        .iter()
        .filter(|player| {
            !player.is_spectator()
                && player
                    .open_container_pos
                    .load()
                    .is_some_and(|open| open == *pos || Some(open) == connected_pos)
        })
        .map(|player| Arc::clone(player) as Arc<dyn EntityBase>)
        .collect();
    // Vanilla searches `maxInteractionRange + 4` around the chest; a golem only keeps a chest
    // open while it stands within reach of it, so its own range bounds the search.
    let search_box = BoundingBox::from_block(pos)
        .expand_all(CopperGolemEntity::CONTAINER_INTERACTION_RANGE + 4.0);
    users.extend(
        goal_utils::get_entities_of_class(world, Some(&EntityType::COPPER_GOLEM), &search_box)
            .into_iter()
            .filter(|entity| {
                entity
                    .cast_any()
                    .downcast_ref::<CopperGolemEntity>()
                    .is_some_and(|golem| golem.has_container_open(world, pos))
            }),
    );
    users
}

/// Vanilla `ContainerOpenersCounter.recheckOpeners` for a chest: recounts who has it open.
fn recheck_chest_openers(world: &Arc<World>, pos: &BlockPos) {
    let Some(block_entity) = world.get_block_entity(pos) else {
        return;
    };
    let any = block_entity.as_any();
    if let Some(chest) = any.downcast_ref::<ChestBlockEntity>() {
        chest.recheck_open(world);
    } else if let Some(chest) = any.downcast_ref::<TrappedChestBlockEntity>() {
        chest.recheck_open(world);
    }
}

/// Vanilla `ContainerOpenersCounter.scheduleRecheck`.
pub fn schedule_chest_recheck(world: &World, pos: &BlockPos) {
    world.schedule_block_tick(
        world.get_block(pos),
        *pos,
        CHECK_TICK_DELAY,
        TickPriority::Normal,
    );
}

/// Vanilla `ContainerOpenersCounter.CHECK_TICK_DELAY`.
const CHECK_TICK_DELAY: i32 = 5;

/// The chest's open and close sounds: vanilla `ChestBlock.getOpenChestSound` and
/// `getCloseChestSound`, which copper chests set from `CopperChestBlock.getHingeSound`.
#[must_use]
pub const fn get_chest_sounds(block: &Block) -> (Sound, Sound) {
    match block.id {
        BlockId::COPPER_CHEST
        | BlockId::EXPOSED_COPPER_CHEST
        | BlockId::WAXED_COPPER_CHEST
        | BlockId::WAXED_EXPOSED_COPPER_CHEST => {
            (Sound::BlockCopperChestOpen, Sound::BlockCopperChestClose)
        }
        BlockId::WEATHERED_COPPER_CHEST | BlockId::WAXED_WEATHERED_COPPER_CHEST => (
            Sound::BlockCopperChestWeatheredOpen,
            Sound::BlockCopperChestWeatheredClose,
        ),
        BlockId::OXIDIZED_COPPER_CHEST | BlockId::WAXED_OXIDIZED_COPPER_CHEST => (
            Sound::BlockCopperChestOxidizedOpen,
            Sound::BlockCopperChestOxidizedClose,
        ),
        _ => (Sound::BlockChestOpen, Sound::BlockChestClose),
    }
}

/// Returns a screen handler factory for opening a chest or double chest inventory.
///
/// Unpacks deferred loot tables on first open (for non-spectator players) and combines
/// connected inventories for double chests if neither chest half is obstructed.
fn get_chest_screen_handler_factory(
    args: GetScreenHandlerFactoryArgs<'_>,
) -> Option<Box<dyn ScreenHandlerFactory>> {
    let state = args.world.get_block_state_id(args.position);
    let first_chest = args.world.get_block_entity(args.position);

    let player_is_spectator = args.player.gamemode.load() == GameMode::Spectator;

    let chest_props = ChestLikeProperties::from_state_id(state);
    let connected_towards = match chest_props.r#type {
        ChestType::Single => None,
        ChestType::Left => Some(chest_props.facing.rotate_clockwise()),
        ChestType::Right => Some(chest_props.facing.rotate_counter_clockwise()),
    };

    // Unpack deferred loot table on first open (non-spectator only).
    if !player_is_spectator && let Some(ref entity) = first_chest {
        unpack_loot_table(args.world, entity, Some(args.player));
    }

    let first_loot_pending = first_chest
        .as_ref()
        .is_some_and(|entity| entity.has_loot_table());
    let first_inventory = first_chest.and_then(BlockEntity::get_inventory)?;

    if is_chest_blocked(args.world, args.position) {
        return None;
    }

    if let Some(direction) = connected_towards {
        let neighbor_pos = args.position.offset(direction.to_offset());
        if is_chest_blocked(args.world, &neighbor_pos) {
            return None;
        }
    }

    // Both halves of a double chest are unpacked at once, like vanilla's CompoundContainer.
    if !player_is_spectator
        && let Some(direction) = connected_towards
        && let Some(second) = args
            .world
            .get_block_entity(&args.position.offset(direction.to_offset()))
    {
        unpack_loot_table(args.world, &second, Some(args.player));
    }

    let loot_pending = first_loot_pending
        || connected_towards.is_some_and(|direction| {
            args.world
                .get_block_entity(&args.position.offset(direction.to_offset()))
                .is_some_and(|second| second.has_loot_table())
        });

    let inventory = if let Some(direction) = connected_towards
        && let Some(second_inventory) = args
            .world
            .get_block_entity(&args.position.offset(direction.to_offset()))
            .and_then(BlockEntity::get_inventory)
    {
        // Vanilla: chestType == ChestType.RIGHT ? DoubleBlockProperties.Type.FIRST : DoubleBlockProperties.Type.SECOND;
        if matches!(chest_props.r#type, ChestType::Right) {
            DoubleInventory::new(first_inventory, second_inventory)
        } else {
            DoubleInventory::new(second_inventory, first_inventory)
        }
    } else {
        first_inventory
    };

    Some(Box::new(ChestScreenFactory {
        inventory,
        loot_pending,
    }))
}

fn normal_use_chest_impl(args: &NormalUseArgs<'_>) -> BlockActionResult {
    let stat = if args.block.id == Block::TRAPPED_CHEST.id {
        pumpkin_data::statistic::CustomStatistic::TriggerTrappedChest
    } else {
        pumpkin_data::statistic::CustomStatistic::OpenChest
    };
    args.player.increment_stat(
        pumpkin_data::statistic::StatisticCategory::Custom,
        stat as i32,
        1,
    );

    if let Some(factory) = get_chest_screen_handler_factory(GetScreenHandlerFactoryArgs {
        server: args.server,
        world: args.world,
        block: args.block,
        position: args.position,
        player: args.player,
    }) {
        args.player
            .open_handled_screen(factory.as_ref(), Some(*args.position));
    }

    BlockActionResult::Success
}

#[pumpkin_block_from_tag("c:chests/wooden")]
pub struct ChestBlock;

impl BlockBehaviour for ChestBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        on_place_chest_impl(&args)
    }

    fn on_synced_block_event(&self, args: OnSyncedBlockEventArgs<'_>) -> bool {
        args.r#type == LID_ANIMATION_EVENT_TYPE
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        placed_chest_impl(&args, ChestBlockEntity::new);
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        normal_use_chest_impl(&args)
    }

    fn get_screen_handler_factory(
        &self,
        args: GetScreenHandlerFactoryArgs<'_>,
    ) -> Option<Box<dyn ScreenHandlerFactory>> {
        get_chest_screen_handler_factory(args)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        chest_update_shape(&args)
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        Some(get_chest_comparator_output(&args))
    }

    /// Vanilla `ChestBlock.tick`: `ChestBlockEntity.recheckOpen`.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        recheck_chest_openers(args.world, args.position);
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

/// Copper chests have the same behavior as wooden chests but also oxidize over time.
#[pumpkin_block_from_tag("minecraft:copper_chests")]
pub struct CopperChestBlock;

impl
    crate::block::blocks::weathering_copper::ChangeOverTimeBlock<
        crate::block::blocks::weathering_copper::WeatherState,
    > for CopperChestBlock
{
    fn get_age(
        &self,
        block: &Block,
    ) -> Option<crate::block::blocks::weathering_copper::WeatherState> {
        crate::block::blocks::weathering_copper::get_weather_state(block)
    }

    fn get_chance_modifier(
        &self,
        age: crate::block::blocks::weathering_copper::WeatherState,
    ) -> f32 {
        crate::block::blocks::weathering_copper::get_chance_modifier(age)
    }

    fn get_next(&self, block: &Block) -> Option<&'static Block> {
        crate::block::blocks::weathering_copper::get_next(block)
    }

    fn get_previous(&self, block: &Block) -> Option<&'static Block> {
        crate::block::blocks::weathering_copper::get_previous(block)
    }

    fn get_first(&self, block: &Block) -> Option<&'static Block> {
        crate::block::blocks::weathering_copper::get_first(block)
    }
}

impl crate::block::blocks::weathering_copper::WeatheringCopper for CopperChestBlock {}

impl BlockBehaviour for CopperChestBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        on_place_chest_impl(&args)
    }

    fn on_synced_block_event(&self, args: OnSyncedBlockEventArgs<'_>) -> bool {
        args.r#type == LID_ANIMATION_EVENT_TYPE
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        placed_chest_impl(&args, ChestBlockEntity::new);
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        normal_use_chest_impl(&args)
    }

    fn get_screen_handler_factory(
        &self,
        args: GetScreenHandlerFactoryArgs<'_>,
    ) -> Option<Box<dyn ScreenHandlerFactory>> {
        get_chest_screen_handler_factory(args)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        chest_update_shape(&args)
    }

    fn random_tick(&self, args: RandomTickArgs<'_>) {
        let current_state_id = args.world.get_block_state_id(args.position);
        let chest_props = ChestLikeProperties::from_state_id(current_state_id);

        // Only oxidize LEFT or SINGLE chests (not RIGHT) to prevent double oxidation
        if chest_props.r#type == ChestType::Right {
            return;
        }

        // Only oxidize if no players are viewing the chest
        if let Some(block_entity) = args.world.get_block_entity(args.position)
            && let Some(chest_entity) = block_entity.as_any().downcast_ref::<ChestBlockEntity>()
            && chest_entity.get_viewer_count() > 0
        {
            return;
        }

        crate::block::blocks::weathering_copper::change_over_time(
            args.world,
            args.position,
            args.block,
        );
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        Some(get_chest_comparator_output(&args))
    }

    /// Vanilla `ChestBlock.tick`: `ChestBlockEntity.recheckOpen`.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        recheck_chest_openers(args.world, args.position);
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

/// Trapped chests have the same behavior as wooden chests but also emit redstone power based on viewer count.
#[pumpkin_block("minecraft:trapped_chest")]
pub struct TrappedChestBlock;

impl BlockBehaviour for TrappedChestBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        on_place_chest_impl(&args)
    }

    fn on_synced_block_event(&self, args: OnSyncedBlockEventArgs<'_>) -> bool {
        args.r#type == LID_ANIMATION_EVENT_TYPE
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        use crate::block::entities::trapped_chest::TrappedChestBlockEntity;
        placed_chest_impl(&args, TrappedChestBlockEntity::new);
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        normal_use_chest_impl(&args)
    }

    fn get_screen_handler_factory(
        &self,
        args: GetScreenHandlerFactoryArgs<'_>,
    ) -> Option<Box<dyn ScreenHandlerFactory>> {
        get_chest_screen_handler_factory(args)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        chest_update_shape(&args)
    }

    fn emits_redstone_power(&self, _args: EmitsRedstonePowerArgs<'_>) -> bool {
        true
    }

    fn get_weak_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        use crate::block::entities::trapped_chest::TrappedChestBlockEntity;

        // Get viewer count from this chest
        let viewer_count = if let Some(block_entity) = args.world.get_block_entity(args.position)
            && let Some(trapped_chest) = block_entity
                .as_any()
                .downcast_ref::<TrappedChestBlockEntity>()
        {
            trapped_chest.get_viewer_count()
        } else {
            0
        };

        viewer_count.min(15) as u8
    }

    fn get_strong_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        // Strong power emitted to the block beneath the trapped chest
        // The block below queries with direction Up (from below looking up at the chest)
        if args.direction == BlockDirection::Up {
            self.get_weak_redstone_power(args)
        } else {
            0
        }
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        Some(get_chest_comparator_output(&args))
    }

    /// Vanilla `ChestBlock.tick`: `ChestBlockEntity.recheckOpen`.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        recheck_chest_openers(args.world, args.position);
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

fn compute_chest_props(
    world: &World,
    player: &Player,
    block: &Block,
    block_pos: &BlockPos,
    face: BlockDirection,
) -> (ChestType, HorizontalFacing) {
    let player_facing = player.get_entity().get_horizontal_facing();
    let chest_facing = player_facing.opposite();

    if player.get_entity().pose.load() == EntityPose::Crouching {
        let Some(face) = face.to_horizontal_facing() else {
            return (ChestType::Single, chest_facing);
        };

        let (clicked_block, clicked_block_state) =
            world.get_block_and_state_id(&block_pos.offset(face.to_offset()));

        if clicked_block == block {
            let clicked_props = ChestLikeProperties::from_state_id(clicked_block_state);

            if clicked_props.r#type != ChestType::Single {
                return (ChestType::Single, chest_facing);
            }

            if clicked_props.facing.rotate_clockwise() == face {
                return (ChestType::Left, clicked_props.facing);
            } else if clicked_props.facing.rotate_counter_clockwise() == face {
                return (ChestType::Right, clicked_props.facing);
            }
        }

        return (ChestType::Single, chest_facing);
    }

    if get_chest_properties_if_can_connect(
        world,
        block,
        block_pos,
        chest_facing,
        chest_facing.rotate_clockwise(),
        ChestType::Single,
    )
    .is_some()
    {
        (ChestType::Left, chest_facing)
    } else if get_chest_properties_if_can_connect(
        world,
        block,
        block_pos,
        chest_facing,
        chest_facing.rotate_counter_clockwise(),
        ChestType::Single,
    )
    .is_some()
    {
        (ChestType::Right, chest_facing)
    } else {
        (ChestType::Single, chest_facing)
    }
}

/// Vanilla `ChestBlock.chestCanConnectTo`: the same block, or for `CopperChestBlock` any
/// copper chest.
fn chest_can_connect_to(block: &Block, neighbor: &Block) -> bool {
    if block.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS) {
        neighbor.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS)
    } else {
        neighbor == block
    }
}

/// Vanilla `ChestBlock.updateShape` and `CopperChestBlock.updateShape`: a single chest pairs
/// with a neighbouring half facing the same way and turned towards it, a half whose other half
/// goes is single again, and a copper half takes the block of the copper half it is joined to.
fn chest_update_shape(args: &GetStateForNeighborUpdateArgs<'_>) -> BlockStateId {
    let mut props = ChestLikeProperties::from_state_id(args.state_id);
    if props.waterlogged {
        args.world.schedule_fluid_tick(
            &Fluid::WATER,
            *args.position,
            Fluid::WATER.flow_speed as i32,
            TickPriority::Normal,
        );
    }
    let neighbor_block = Block::from_state_id(args.neighbor_state_id);
    let can_connect = chest_can_connect_to(args.block, neighbor_block);
    let direction = args.direction.to_horizontal_facing();
    let mut state_id = args.state_id;
    if can_connect && let Some(direction) = direction {
        let neighbor = ChestLikeProperties::from_state_id(args.neighbor_state_id);
        if props.r#type == ChestType::Single
            && neighbor.r#type != ChestType::Single
            && props.facing == neighbor.facing
            && get_connected_direction(args.neighbor_state_id) == Some(direction.opposite())
        {
            props.r#type = neighbor.r#type.opposite();
            state_id = props.to_state_id(args.block);
        }
    } else if direction.is_some() && get_connected_direction(args.state_id) == direction {
        props.r#type = ChestType::Single;
        state_id = props.to_state_id(args.block);
    }
    // `CopperChestBlock.updateShape`: `neighbourState.getBlock().withPropertiesOf(state)`.
    if args.block.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS)
        && can_connect
        && direction.is_some()
        && get_connected_direction(state_id) == direction
    {
        return ChestLikeProperties::from_state_id(state_id).to_state_id(neighbor_block);
    }
    state_id
}

fn get_chest_properties_if_can_connect(
    world: &World,
    block: &Block,
    block_pos: &BlockPos,
    facing: HorizontalFacing,
    direction: HorizontalFacing,
    wanted_type: ChestType,
) -> Option<ChestLikeProperties> {
    let (neighbor_block, neighbor_block_state) =
        world.get_block_and_state_id(&block_pos.offset(direction.to_offset()));

    if !chest_can_connect_to(block, neighbor_block) {
        return None;
    }

    let neighbor_props = ChestLikeProperties::from_state_id(neighbor_block_state);
    if neighbor_props.facing == facing && neighbor_props.r#type == wanted_type {
        return Some(neighbor_props);
    }

    None
}

/// Vanilla `ChestBlock.isChestBlockedAt`.
pub fn is_chest_blocked(world: &World, block_pos: &BlockPos) -> bool {
    has_block_on_top(world, block_pos) || is_cat_sitting_on_chest(world, block_pos)
}

/// Vanilla `ChestBlock.isCatSittingOnChest`.
fn is_cat_sitting_on_chest(world: &World, block_pos: &BlockPos) -> bool {
    let area = BoundingBox::from_block(&block_pos.up());
    goal_utils::get_entities_of_class(world, Some(&EntityType::CAT), &area)
        .iter()
        .any(|cat| cat.get_mob().is_some_and(Mob::is_sitting))
}
fn has_block_on_top(world: &World, block_pos: &BlockPos) -> bool {
    let above_pos = block_pos.up();
    let above_state = world.get_block_state(&above_pos);
    above_state.is_solid_block()
}

trait ChestTypeExt {
    fn opposite(&self) -> ChestType;
}

impl ChestTypeExt for ChestType {
    fn opposite(&self) -> Self {
        match self {
            Self::Single => Self::Single,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}
