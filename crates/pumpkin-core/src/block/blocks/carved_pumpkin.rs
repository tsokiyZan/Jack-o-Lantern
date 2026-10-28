use std::sync::Arc;

use pumpkin_data::block_properties::{
    ChestLikeProperties, ChestType, HorizontalFacing, WallTorchLikeProperties,
};
use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockDirection, BlockId, BlockState, BlockStateId, world::WorldEvent};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;

use crate::{
    block::{
        BlockBehaviour, BlockMetadata, OnPlaceArgs, PlacedArgs, blocks::chests,
        blocks::weathering_copper::WeatherState,
    },
    entity::{
        Entity, EntityBase,
        passive::{
            copper_golem::CopperGolemEntity, iron_golem::IronGolemEntity,
            snow_golem::SnowGolemEntity,
        },
    },
    world::World,
};

/// One cell of a vanilla `BlockPatternBuilder` aisle.
#[derive(Clone, Copy)]
enum Cell {
    /// `' '`: anything.
    Any,
    /// `'~'`: `BlockStateBase::isAir`.
    Air,
    /// `'^'`: vanilla `PUMPKINS_PREDICATE`.
    Pumpkin,
    /// `'#'` in the snow golem patterns.
    SnowBlock,
    /// `'#'` in the iron golem patterns.
    IronBlock,
    /// `'#'` in the copper golem patterns: `BlockTags.COPPER`.
    Copper,
}

impl Cell {
    fn test(self, state: &BlockState) -> bool {
        let block = Block::from_state_id(state.id);
        match self {
            Self::Any => true,
            Self::Air => state.is_air(),
            Self::Pumpkin => block == &Block::CARVED_PUMPKIN || block == &Block::JACK_O_LANTERN,
            Self::SnowBlock => block == &Block::SNOW_BLOCK,
            Self::IronBlock => block == &Block::IRON_BLOCK,
            Self::Copper => block.has_tag(&tag::Block::MINECRAFT_COPPER),
        }
    }
}

/// Vanilla `BlockPattern` with a single aisle, which is all the golem patterns use. Rows are
/// top to bottom, cells left to right.
struct BlockPattern(&'static [&'static [Cell]]);

const SNOW_GOLEM_BASE: BlockPattern =
    BlockPattern(&[&[Cell::Any], &[Cell::SnowBlock], &[Cell::SnowBlock]]);
const SNOW_GOLEM_FULL: BlockPattern =
    BlockPattern(&[&[Cell::Pumpkin], &[Cell::SnowBlock], &[Cell::SnowBlock]]);
const IRON_GOLEM_BASE: BlockPattern = BlockPattern(&[
    &[Cell::Air, Cell::Any, Cell::Air],
    &[Cell::IronBlock, Cell::IronBlock, Cell::IronBlock],
    &[Cell::Air, Cell::IronBlock, Cell::Air],
]);
const IRON_GOLEM_FULL: BlockPattern = BlockPattern(&[
    &[Cell::Air, Cell::Pumpkin, Cell::Air],
    &[Cell::IronBlock, Cell::IronBlock, Cell::IronBlock],
    &[Cell::Air, Cell::IronBlock, Cell::Air],
]);
const COPPER_GOLEM_BASE: BlockPattern = BlockPattern(&[&[Cell::Any], &[Cell::Copper]]);
const COPPER_GOLEM_FULL: BlockPattern = BlockPattern(&[&[Cell::Pumpkin], &[Cell::Copper]]);

/// Vanilla `BlockPattern.BlockPatternMatch`, with the states it read cached like vanilla's
/// `LoadingCache`.
struct BlockPatternMatch {
    front_top_left: BlockPos,
    forwards: BlockDirection,
    up: BlockDirection,
    width: i32,
    height: i32,
    states: Vec<&'static BlockState>,
}

impl BlockPatternMatch {
    /// Vanilla `getBlock(x, y, 0)`: the position and the state the match saw there.
    fn get_block(&self, x: i32, y: i32) -> (BlockPos, &'static BlockState) {
        let pos = translate_and_rotate(self.front_top_left, self.forwards, self.up, x, y);
        (pos, self.states[(y * self.width + x) as usize])
    }
}

/// Vanilla `BlockPattern.translateAndRotate` with `forwards` 0.
fn translate_and_rotate(
    origin: BlockPos,
    forwards: BlockDirection,
    up: BlockDirection,
    right: i32,
    down: i32,
) -> BlockPos {
    let forwards_vector = forwards.to_offset();
    let up_vector = up.to_offset();
    let right_vector = forwards_vector.cross(&up_vector);
    origin.offset(Vector3::new(
        up_vector.x * -down + right_vector.x * right,
        up_vector.y * -down + right_vector.y * right,
        up_vector.z * -down + right_vector.z * right,
    ))
}

impl BlockPattern {
    const fn width(&self) -> i32 {
        self.0[0].len() as i32
    }

    const fn height(&self) -> i32 {
        self.0.len() as i32
    }

    /// Vanilla `BlockPattern.matches(origin, forwards, up, cache)`.
    fn matches(
        &self,
        world: &World,
        origin: BlockPos,
        forwards: BlockDirection,
        up: BlockDirection,
    ) -> Option<BlockPatternMatch> {
        let mut states = Vec::with_capacity((self.width() * self.height()) as usize);
        for (y, row) in self.0.iter().enumerate() {
            for (x, cell) in row.iter().enumerate() {
                let pos = translate_and_rotate(origin, forwards, up, x as i32, y as i32);
                let state = world.get_block_state(&pos);
                if !cell.test(state) {
                    return None;
                }
                states.push(state);
            }
        }
        Some(BlockPatternMatch {
            front_top_left: origin,
            forwards,
            up,
            width: self.width(),
            height: self.height(),
            states,
        })
    }

    /// Vanilla `BlockPattern.find`: every corner within the pattern's size of `origin`, in
    /// every orientation.
    fn find(&self, world: &World, origin: &BlockPos) -> Option<BlockPatternMatch> {
        let dist = self.width().max(self.height());
        // `BlockPos.betweenClosed` walks x fastest, then y, then z.
        for dz in 0..dist {
            for dy in 0..dist {
                for dx in 0..dist {
                    let test_pos = origin.offset(Vector3::new(dx, dy, dz));
                    for forwards in BlockDirection::all() {
                        for up in BlockDirection::all() {
                            if up != forwards
                                && up != forwards.opposite()
                                && let Some(found) = self.matches(world, test_pos, forwards, up)
                            {
                                return Some(found);
                            }
                        }
                    }
                }
            }
        }
        None
    }
}

/// Vanilla `CarvedPumpkinBlock.canSpawnGolem`.
#[must_use]
pub fn can_spawn_golem(world: &World, top_pos: &BlockPos) -> bool {
    SNOW_GOLEM_BASE.find(world, top_pos).is_some()
        || IRON_GOLEM_BASE.find(world, top_pos).is_some()
        || COPPER_GOLEM_BASE.find(world, top_pos).is_some()
}

/// Vanilla `CarvedPumpkinBlock.trySpawnGolem`.
fn try_spawn_golem(world: &Arc<World>, top_pos: &BlockPos) {
    if let Some(found) = SNOW_GOLEM_FULL.find(world, top_pos) {
        let (spawn_pos, _) = found.get_block(0, 2);
        let golem = SnowGolemEntity::new(new_golem(world, &EntityType::SNOW_GOLEM, spawn_pos));
        spawn_golem_in_world(world, &found, golem);
        return;
    }

    if let Some(found) = IRON_GOLEM_FULL.find(world, top_pos) {
        let (spawn_pos, _) = found.get_block(1, 2);
        let golem = IronGolemEntity::new(new_golem(world, &EntityType::IRON_GOLEM, spawn_pos));
        golem.set_player_created(true);
        spawn_golem_in_world(world, &found, golem);
        return;
    }

    if let Some(found) = COPPER_GOLEM_FULL.find(world, top_pos) {
        let (spawn_pos, _) = found.get_block(0, 0);
        let golem = CopperGolemEntity::new(new_golem(world, &EntityType::COPPER_GOLEM, spawn_pos));
        spawn_golem_in_world(world, &found, golem.clone());
        replace_copper_block_with_chest(world, &found);
        golem.spawn(get_weather_state_from_pattern(&found));
    }
}

/// The golem `spawnGolemInWorld` snaps to `spawn_pos`, with no rotation.
fn new_golem(world: &Arc<World>, entity_type: &'static EntityType, spawn_pos: BlockPos) -> Entity {
    Entity::new(
        world.clone(),
        Vector3::new(
            f64::from(spawn_pos.0.x) + 0.5,
            f64::from(spawn_pos.0.y) + 0.05,
            f64::from(spawn_pos.0.z) + 0.5,
        ),
        entity_type,
    )
}

/// Vanilla `CarvedPumpkinBlock.spawnGolemInWorld`.
// TODO: `CriteriaTriggers.SUMMONED_ENTITY` for players within 5 blocks.
fn spawn_golem_in_world(world: &Arc<World>, found: &BlockPatternMatch, golem: Arc<dyn EntityBase>) {
    clear_pattern_blocks(world, found);
    world.spawn_entity(golem);
    update_pattern_blocks(world, found);
}

/// Vanilla `CarvedPumpkinBlock.clearPatternBlocks`.
fn clear_pattern_blocks(world: &Arc<World>, found: &BlockPatternMatch) {
    for x in 0..found.width {
        for y in 0..found.height {
            let (pos, state) = found.get_block(x, y);
            world.set_block_state(
                &pos,
                Block::AIR.default_state.id,
                BlockFlags::NOTIFY_LISTENERS,
            );
            world.sync_world_event(
                WorldEvent::ParticlesDestroyBlock,
                pos,
                i32::from(state.id.as_u16()),
            );
        }
    }
}

/// Vanilla `CarvedPumpkinBlock.updatePatternBlocks`.
fn update_pattern_blocks(world: &Arc<World>, found: &BlockPatternMatch) {
    for x in 0..found.width {
        for y in 0..found.height {
            let (pos, _) = found.get_block(x, y);
            world.update_neighbors_at(&pos, &Block::AIR, None);
        }
    }
}

/// The weathering copper chest for each copper block, with the block's age and whether it is
/// waxed: vanilla `CopperChestBlock.COPPER_TO_COPPER_CHEST_MAPPING`, `getAge` and `isWaxed`.
const fn copper_chest_for(block: BlockId) -> Option<(&'static Block, WeatherState, bool)> {
    Some(match block {
        BlockId::COPPER_BLOCK => (&Block::COPPER_CHEST, WeatherState::Unaffected, false),
        BlockId::EXPOSED_COPPER => (&Block::EXPOSED_COPPER_CHEST, WeatherState::Exposed, false),
        BlockId::WEATHERED_COPPER => (
            &Block::WEATHERED_COPPER_CHEST,
            WeatherState::Weathered,
            false,
        ),
        BlockId::OXIDIZED_COPPER => (&Block::OXIDIZED_COPPER_CHEST, WeatherState::Oxidized, false),
        BlockId::WAXED_COPPER_BLOCK => (&Block::WAXED_COPPER_CHEST, WeatherState::Unaffected, true),
        BlockId::WAXED_EXPOSED_COPPER => (
            &Block::WAXED_EXPOSED_COPPER_CHEST,
            WeatherState::Exposed,
            true,
        ),
        BlockId::WAXED_WEATHERED_COPPER => (
            &Block::WAXED_WEATHERED_COPPER_CHEST,
            WeatherState::Weathered,
            true,
        ),
        BlockId::WAXED_OXIDIZED_COPPER => (
            &Block::WAXED_OXIDIZED_COPPER_CHEST,
            WeatherState::Oxidized,
            true,
        ),
        _ => return None,
    })
}

/// A copper chest's age, whether it is waxed, and its unwaxed block.
const fn copper_chest_state(block: BlockId) -> Option<(WeatherState, bool, &'static Block)> {
    Some(match block {
        BlockId::COPPER_CHEST => (WeatherState::Unaffected, false, &Block::COPPER_CHEST),
        BlockId::EXPOSED_COPPER_CHEST => {
            (WeatherState::Exposed, false, &Block::EXPOSED_COPPER_CHEST)
        }
        BlockId::WEATHERED_COPPER_CHEST => (
            WeatherState::Weathered,
            false,
            &Block::WEATHERED_COPPER_CHEST,
        ),
        BlockId::OXIDIZED_COPPER_CHEST => {
            (WeatherState::Oxidized, false, &Block::OXIDIZED_COPPER_CHEST)
        }
        BlockId::WAXED_COPPER_CHEST => (WeatherState::Unaffected, true, &Block::COPPER_CHEST),
        BlockId::WAXED_EXPOSED_COPPER_CHEST => {
            (WeatherState::Exposed, true, &Block::EXPOSED_COPPER_CHEST)
        }
        BlockId::WAXED_WEATHERED_COPPER_CHEST => (
            WeatherState::Weathered,
            true,
            &Block::WEATHERED_COPPER_CHEST,
        ),
        BlockId::WAXED_OXIDIZED_COPPER_CHEST => {
            (WeatherState::Oxidized, true, &Block::OXIDIZED_COPPER_CHEST)
        }
        _ => return None,
    })
}

/// Vanilla `CarvedPumpkinBlock.getWeatherStateFromPattern`.
fn get_weather_state_from_pattern(found: &BlockPatternMatch) -> WeatherState {
    let (_, state) = found.get_block(0, 1);
    copper_chest_for(Block::from_state_id(state.id).id)
        .map_or(WeatherState::Unaffected, |(_, age, _)| age)
}

/// Vanilla `CarvedPumpkinBlock.replaceCopperBlockWithChest`.
fn replace_copper_block_with_chest(world: &Arc<World>, found: &BlockPatternMatch) {
    let (copper_pos, copper_state) = found.get_block(0, 1);
    let (_, pumpkin_state) = found.get_block(0, 0);
    let facing = WallTorchLikeProperties::from_state_id(pumpkin_state.id).facing;
    let state_id = get_from_copper_block(
        Block::from_state_id(copper_state.id),
        facing,
        world,
        &copper_pos,
    );
    world.set_block_state(&copper_pos, state_id, BlockFlags::NOTIFY_LISTENERS);
}

/// Vanilla `CopperChestBlock.getFromCopperBlock`.
fn get_from_copper_block(
    copper_block: &Block,
    facing: HorizontalFacing,
    world: &World,
    pos: &BlockPos,
) -> BlockStateId {
    let block =
        copper_chest_for(copper_block.id).map_or(&Block::COPPER_CHEST, |(chest, _, _)| chest);
    let mut props = ChestLikeProperties::default(block);
    props.facing = facing;
    props.r#type = get_chest_type(world, pos, facing);
    get_least_oxidized_chest_of_connected_blocks(block, props, world, pos)
}

/// Vanilla `ChestBlock.getChestType` with `CopperChestBlock.chestCanConnectTo`.
fn get_chest_type(world: &World, pos: &BlockPos, facing: HorizontalFacing) -> ChestType {
    if Some(facing) == candidate_partner_facing(world, pos, facing.rotate_clockwise()) {
        ChestType::Left
    } else if Some(facing)
        == candidate_partner_facing(world, pos, facing.rotate_counter_clockwise())
    {
        ChestType::Right
    } else {
        ChestType::Single
    }
}

/// Vanilla `ChestBlock.candidatePartnerFacing`.
fn candidate_partner_facing(
    world: &World,
    pos: &BlockPos,
    neighbour_direction: HorizontalFacing,
) -> Option<HorizontalFacing> {
    let (block, state_id) =
        world.get_block_and_state_id(&pos.offset(neighbour_direction.to_offset()));
    if !block.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS) {
        return None;
    }
    let props = ChestLikeProperties::from_state_id(state_id);
    (props.r#type == ChestType::Single).then_some(props.facing)
}

/// Vanilla `CopperChestBlock.getLeastOxidizedChestOfConnectedBlocks`.
fn get_least_oxidized_chest_of_connected_blocks(
    block: &'static Block,
    props: ChestLikeProperties,
    world: &World,
    pos: &BlockPos,
) -> BlockStateId {
    let connected_block = world.get_block(&chests::get_connected_block_pos(pos, &props));
    let (Some((age, waxed, unwaxed)), Some((connected_age, connected_waxed, connected_unwaxed))) = (
        copper_chest_state(block.id),
        copper_chest_state(connected_block.id),
    ) else {
        return props.to_state_id(block);
    };
    if props.r#type == ChestType::Single {
        return props.to_state_id(block);
    }
    let (updated, connected_predicted) = if waxed == connected_waxed {
        (block, connected_block)
    } else {
        (unwaxed, connected_unwaxed)
    };
    let least_oxidized = if age.ordinal() <= connected_age.ordinal() {
        updated
    } else {
        connected_predicted
    };
    props.to_state_id(least_oxidized)
}

pub struct CarvedPumpkinBlock;

impl BlockMetadata for CarvedPumpkinBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::JACK_O_LANTERN, BlockId::CARVED_PUMPKIN].into()
    }
}

impl BlockBehaviour for CarvedPumpkinBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = WallTorchLikeProperties::default(args.block);
        props.facing = args
            .player
            .living_entity
            .entity
            .get_horizontal_facing()
            .opposite();
        props.to_state_id(args.block)
    }

    /// Vanilla `CarvedPumpkinBlock.onPlace`.
    fn placed(&self, args: PlacedArgs<'_>) {
        if args.is_new_block() {
            try_spawn_golem(args.world, args.position);
        }
    }
}
