use std::sync::Arc;

use pumpkin_data::BlockStateId;
use pumpkin_data::block_properties::PoweredRailLikeProperties;
use pumpkin_data::block_properties::RailLikeProperties;
use pumpkin_data::block_properties::RailShape;
use pumpkin_data::block_properties::RailShapeStraight;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, tag};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use crate::world::World;

mod common;

pub mod activator_rail;
pub mod detector_rail;
pub mod powered_rail;
pub mod rail;

/// Vanilla `RailState`: a rail and the positions its shape connects to, used to pick and
/// propagate rail shapes.
struct RailState {
    position: BlockPos,
    block: &'static Block,
    properties: RailProperties,
    connections: Vec<BlockPos>,
}

/// Vanilla `BaseRailBlock.isRail(BlockState)`.
fn is_rail(block: &Block) -> bool {
    block.has_tag(&tag::Block::MINECRAFT_RAILS)
}

/// Vanilla `BaseRailBlock.isRail(Level, BlockPos)`.
fn is_rail_at(world: &World, position: &BlockPos) -> bool {
    is_rail(world.get_block(position))
}

impl RailState {
    fn new(position: BlockPos, state_id: BlockStateId) -> Self {
        let block = Block::from_state_id(state_id);
        let properties = RailProperties::new(state_id, block);
        let mut rail = Self {
            position,
            block,
            properties,
            connections: Vec::with_capacity(3),
        };
        rail.update_connections(rail.properties.shape());
        rail
    }

    fn connections(&self) -> &[BlockPos] {
        &self.connections
    }

    fn state_id(&self) -> BlockStateId {
        self.properties.to_state_id(self.block)
    }

    /// Vanilla `RailState.updateConnections`.
    fn update_connections(&mut self, shape: RailShape) {
        let pos = self.position;
        self.connections.clear();
        let [a, b] = match shape {
            RailShape::NorthSouth => [pos.north(), pos.south()],
            RailShape::EastWest => [pos.west(), pos.east()],
            RailShape::AscendingEast => [pos.west(), pos.east().up()],
            RailShape::AscendingWest => [pos.west().up(), pos.east()],
            RailShape::AscendingNorth => [pos.north().up(), pos.south()],
            RailShape::AscendingSouth => [pos.north(), pos.south().up()],
            RailShape::SouthEast => [pos.east(), pos.south()],
            RailShape::SouthWest => [pos.west(), pos.south()],
            RailShape::NorthWest => [pos.west(), pos.north()],
            RailShape::NorthEast => [pos.east(), pos.north()],
        };
        self.connections.push(a);
        self.connections.push(b);
    }

    /// Vanilla `RailState.removeSoftConnections`.
    fn remove_soft_connections(&mut self, world: &World) {
        let mut i = 0;
        while i < self.connections.len() {
            match Self::get_rail(world, self.connections[i]) {
                Some(rail) if rail.connects_to(self) => {
                    self.connections[i] = rail.position;
                    i += 1;
                }
                _ => {
                    self.connections.remove(i);
                }
            }
        }
    }

    /// Vanilla `RailState.hasRail`.
    fn has_rail(world: &World, position: BlockPos) -> bool {
        is_rail_at(world, &position)
            || is_rail_at(world, &position.up())
            || is_rail_at(world, &position.down())
    }

    /// Vanilla `RailState.getRail`: a rail at the position, above it or below it.
    fn get_rail(world: &World, position: BlockPos) -> Option<Self> {
        [position, position.up(), position.down()]
            .into_iter()
            .find_map(|test_pos| {
                let (block, state_id) = world.get_block_and_state_id(&test_pos);
                is_rail(block).then(|| Self::new(test_pos, state_id))
            })
    }

    /// Vanilla `RailState.connectsTo`.
    fn connects_to(&self, rail: &Self) -> bool {
        self.has_connection(rail.position)
    }

    /// Vanilla `RailState.hasConnection`: compares only X and Z.
    fn has_connection(&self, rail_pos: BlockPos) -> bool {
        self.connections
            .iter()
            .any(|pos| pos.0.x == rail_pos.0.x && pos.0.z == rail_pos.0.z)
    }

    /// Vanilla `RailState.countPotentialConnections`.
    fn count_potential_connections(&self, world: &World) -> usize {
        [
            self.position.north(),
            self.position.east(),
            self.position.south(),
            self.position.west(),
        ]
        .into_iter()
        .filter(|pos| Self::has_rail(world, *pos))
        .count()
    }

    /// Vanilla `RailState.canConnectTo`.
    fn can_connect_to(&self, rail: &Self) -> bool {
        self.connects_to(rail) || self.connections.len() != 2
    }

    /// Vanilla `RailState.connectTo`.
    fn connect_to(&mut self, world: &Arc<World>, rail: &Self) {
        self.connections.push(rail.position);
        let pos = self.position;
        let north = pos.north();
        let south = pos.south();
        let west = pos.west();
        let east = pos.east();
        let n = self.has_connection(north);
        let s = self.has_connection(south);
        let w = self.has_connection(west);
        let e = self.has_connection(east);
        // East-west wins over north-south.
        let mut shape = if w || e {
            Some(RailShape::EastWest)
        } else {
            (n || s).then_some(RailShape::NorthSouth)
        };

        if self.properties.can_curve() {
            if s && e && !n && !w {
                shape = Some(RailShape::SouthEast);
            }
            if s && w && !n && !e {
                shape = Some(RailShape::SouthWest);
            }
            if n && w && !s && !e {
                shape = Some(RailShape::NorthWest);
            }
            if n && e && !s && !w {
                shape = Some(RailShape::NorthEast);
            }
        }

        let shape = ascending_shape(world, pos, shape).unwrap_or(RailShape::NorthSouth);
        self.properties.set_shape(shape);
        world.set_block_state(&pos, self.state_id(), BlockFlags::NOTIFY_ALL);
    }

    /// Vanilla `RailState.hasNeighborRail`.
    fn has_neighbor_rail(&self, world: &World, rail_pos: BlockPos) -> bool {
        let Some(mut neighbor) = Self::get_rail(world, rail_pos) else {
            return false;
        };
        neighbor.remove_soft_connections(world);
        neighbor.can_connect_to(self)
    }

    /// Vanilla `RailState.place`.
    fn place(
        mut self,
        world: &Arc<World>,
        has_signal: bool,
        first: bool,
        default_shape: RailShape,
    ) -> Self {
        let pos = self.position;
        let n = self.has_neighbor_rail(world, pos.north());
        let s = self.has_neighbor_rail(world, pos.south());
        let w = self.has_neighbor_rail(world, pos.west());
        let e = self.has_neighbor_rail(world, pos.east());
        let mut shape = None;
        let north_or_south = n || s;
        let west_or_east = w || e;
        if north_or_south && !west_or_east {
            shape = Some(RailShape::NorthSouth);
        }
        if west_or_east && !north_or_south {
            shape = Some(RailShape::EastWest);
        }

        let south_and_east = s && e;
        let south_and_west = s && w;
        let north_and_east = n && e;
        let north_and_west = n && w;
        let can_curve = self.properties.can_curve();
        if can_curve {
            if south_and_east && !n && !w {
                shape = Some(RailShape::SouthEast);
            }
            if south_and_west && !n && !e {
                shape = Some(RailShape::SouthWest);
            }
            if north_and_west && !s && !e {
                shape = Some(RailShape::NorthWest);
            }
            if north_and_east && !s && !w {
                shape = Some(RailShape::NorthEast);
            }
        }

        if shape.is_none() {
            if north_or_south && west_or_east {
                shape = Some(default_shape);
            } else if north_or_south {
                shape = Some(RailShape::NorthSouth);
            } else if west_or_east {
                shape = Some(RailShape::EastWest);
            }

            if can_curve {
                if has_signal {
                    if south_and_east {
                        shape = Some(RailShape::SouthEast);
                    }
                    if south_and_west {
                        shape = Some(RailShape::SouthWest);
                    }
                    if north_and_east {
                        shape = Some(RailShape::NorthEast);
                    }
                    if north_and_west {
                        shape = Some(RailShape::NorthWest);
                    }
                } else {
                    if north_and_west {
                        shape = Some(RailShape::NorthWest);
                    }
                    if north_and_east {
                        shape = Some(RailShape::NorthEast);
                    }
                    if south_and_west {
                        shape = Some(RailShape::SouthWest);
                    }
                    if south_and_east {
                        shape = Some(RailShape::SouthEast);
                    }
                }
            }
        }

        let shape = ascending_shape(world, pos, shape).unwrap_or(default_shape);
        self.update_connections(shape);
        self.properties.set_shape(shape);
        let state_id = self.state_id();
        if first || world.get_block_state_id(&pos) != state_id {
            world.set_block_state(&pos, state_id, BlockFlags::NOTIFY_ALL);

            for i in 0..self.connections.len() {
                if let Some(mut neighbor) = Self::get_rail(world, self.connections[i]) {
                    neighbor.remove_soft_connections(world);
                    if neighbor.can_connect_to(&self) {
                        neighbor.connect_to(world, &self);
                    }
                }
            }
        }

        self
    }
}

/// The slope check shared by `RailState.connectTo` and `RailState.place`: a flat shape
/// ascends towards a rail one block up.
fn ascending_shape(world: &World, pos: BlockPos, shape: Option<RailShape>) -> Option<RailShape> {
    let mut shape = shape?;
    if shape == RailShape::NorthSouth {
        if is_rail_at(world, &pos.north().up()) {
            shape = RailShape::AscendingNorth;
        }
        if is_rail_at(world, &pos.south().up()) {
            shape = RailShape::AscendingSouth;
        }
    }
    if shape == RailShape::EastWest {
        if is_rail_at(world, &pos.east().up()) {
            shape = RailShape::AscendingEast;
        }
        if is_rail_at(world, &pos.west().up()) {
            shape = RailShape::AscendingWest;
        }
    }
    Some(shape)
}

enum RailProperties {
    Rail(RailLikeProperties),
    StraightRail(PoweredRailLikeProperties),
}

impl RailProperties {
    pub fn default(block: &Block) -> Self {
        if block == &Block::RAIL {
            Self::Rail(RailLikeProperties::default(block))
        } else {
            Self::StraightRail(PoweredRailLikeProperties::default(block))
        }
    }

    pub fn new(state_id: BlockStateId, block: &Block) -> Self {
        if block == &Block::RAIL {
            Self::Rail(RailLikeProperties::from_state_id(state_id))
        } else {
            Self::StraightRail(PoweredRailLikeProperties::from_state_id(state_id))
        }
    }

    const fn can_curve(&self) -> bool {
        match self {
            Self::Rail(_) => true,
            Self::StraightRail(_) => false,
        }
    }

    const fn shape(&self) -> RailShape {
        match self {
            Self::Rail(props) => props.shape,
            Self::StraightRail(props) => match props.shape {
                RailShapeStraight::NorthSouth => RailShape::NorthSouth,
                RailShapeStraight::EastWest => RailShape::EastWest,
                RailShapeStraight::AscendingEast => RailShape::AscendingEast,
                RailShapeStraight::AscendingWest => RailShape::AscendingWest,
                RailShapeStraight::AscendingNorth => RailShape::AscendingNorth,
                RailShapeStraight::AscendingSouth => RailShape::AscendingSouth,
            },
        }
    }

    fn to_state_id(&self, block: &Block) -> BlockStateId {
        match self {
            Self::Rail(props) => props.to_state_id(block),
            Self::StraightRail(props) => props.to_state_id(block),
        }
    }

    const fn is_waterlogged(&self) -> bool {
        match self {
            Self::Rail(props) => props.waterlogged,
            Self::StraightRail(props) => props.waterlogged,
        }
    }

    const fn set_waterlogged(&mut self, waterlogged: bool) {
        match self {
            Self::Rail(props) => props.waterlogged = waterlogged,
            Self::StraightRail(props) => props.waterlogged = waterlogged,
        }
    }

    fn set_shape(&mut self, shape: RailShape) {
        match self {
            Self::Rail(props) => props.shape = shape,
            Self::StraightRail(props) => {
                props.shape = match shape {
                    RailShape::NorthSouth => RailShapeStraight::NorthSouth,
                    RailShape::EastWest => RailShapeStraight::EastWest,
                    RailShape::AscendingEast => RailShapeStraight::AscendingEast,
                    RailShape::AscendingWest => RailShapeStraight::AscendingWest,
                    RailShape::AscendingNorth => RailShapeStraight::AscendingNorth,
                    RailShape::AscendingSouth => RailShapeStraight::AscendingSouth,
                    _ => {
                        tracing::error!("Trying to make a straight rail curved: {:?}", shape);
                        return;
                    }
                }
            }
        }
    }

    const fn is_powered(&self) -> bool {
        match self {
            Self::Rail(_) => false,
            Self::StraightRail(props) => props.powered,
        }
    }

    const fn set_powered(&mut self, powered: bool) {
        match self {
            Self::Rail(_) => {}
            Self::StraightRail(props) => props.powered = powered,
        }
    }
}
