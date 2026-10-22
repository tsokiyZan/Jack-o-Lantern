use pumpkin_data::{
    Block, BlockState,
    block_properties::CampfireLikeProperties,
    fluid::Fluid,
    tag::{self, Taggable},
};
use rustc_hash::FxHashMap;

use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

use crate::entity::ai::pathfinder::{
    node::{Node, PATH_TYPE_COUNT, PathType, Target},
    pathfinding_context::PathfindingContext,
};

/// Vanilla `NodeEvaluator.isBurningBlock`.
#[must_use]
pub fn is_burning_block(block: &Block, state: &BlockState) -> bool {
    block.has_tag(&tag::Block::MINECRAFT_FIRE)
        || block == &Block::LAVA
        || block == &Block::MAGMA_BLOCK
        || (block.has_tag(&tag::Block::MINECRAFT_CAMPFIRES)
            && CampfireLikeProperties::from_state_id(state.id).lit)
        || block == &Block::LAVA_CAULDRON
}

pub trait NodeEvaluator {
    fn prepare(&mut self, context: PathfindingContext, mob_data: MobData);
    fn done(&mut self);
    fn get_start(&mut self) -> Option<Node>;
    fn get_target(&mut self, pos: BlockPos) -> Target;
    fn get_neighbors(&mut self, current: &Node, out: &mut Vec<Node>);
    fn get_path_type_of_mob(
        &mut self,
        context: &mut PathfindingContext,
        pos: Vector3<i32>,
        mob_data: &MobData,
    ) -> PathType;
    fn get_path_type(&mut self, context: &mut PathfindingContext, pos: Vector3<i32>) -> PathType;
    fn set_can_pass_doors(&mut self, can_pass: bool);
    fn set_can_open_doors(&mut self, can_open: bool);
    fn set_can_float(&mut self, can_float: bool);
    fn set_can_walk_over_fences(&mut self, can_walk: bool);
    fn can_pass_doors(&self) -> bool;
    fn can_open_doors(&self) -> bool;
    fn can_float(&self) -> bool;
    fn can_walk_over_fences(&self) -> bool;
}

/// What the node evaluators read from the pathing mob.
#[derive(Debug, Clone, Copy)]
pub struct MobData {
    pub position: Vector3<f64>,
    pub width: f32,
    pub height: f32,
    /// Vanilla `LivingEntity.maxUpStep`.
    pub max_up_step: f32,
    /// Vanilla `Mob.getMaxFallDistance`.
    pub max_fall_distance: i32,
    /// Vanilla `LivingEntity.canStandOnFluid` holds for lava (striders).
    pub can_stand_on_lava: bool,
    pub on_ground: bool,
    /// Vanilla `isInWater`, which is also `isInFloatableFluid` while
    /// `#entity_floatable` holds only water.
    pub is_in_water: bool,
    /// The mob's own `pathfindingMalus` map, which `setPathfindingMalus` writes.
    pub path_type_malus: [Option<f32>; PATH_TYPE_COUNT],
    /// The controlled vehicle's map while it lends it (`shouldPassengersInheritMalus`), which
    /// `getPathfindingMalus` then reads instead of the own map.
    pub inherited_path_type_malus: Option<[Option<f32>; PATH_TYPE_COUNT]>,
    /// Vanilla `Creaking.getHomePos`, which its `HomeNodeEvaluator` keeps paths near.
    pub home_pos: Option<BlockPos>,
    /// The pathing entity, for draws from its random (vanilla `mob.getRandom()`).
    pub entity_id: Option<i32>,
}

impl MobData {
    #[must_use]
    pub const fn new(position: Vector3<f64>, width: f32, height: f32) -> Self {
        Self {
            position,
            width,
            height,
            max_up_step: 0.0,
            max_fall_distance: 3,
            can_stand_on_lava: false,
            on_ground: true,
            is_in_water: false,
            path_type_malus: [None; PATH_TYPE_COUNT],
            inherited_path_type_malus: None,
            home_pos: None,
            entity_id: None,
        }
    }

    /// Vanilla `LivingEntity.canStandOnFluid`.
    #[must_use]
    pub fn can_stand_on_fluid(&self, fluid: &Fluid) -> bool {
        self.can_stand_on_lava && fluid.has_tag(&tag::Fluid::MINECRAFT_LAVA)
    }

    /// Vanilla `Mob.getPathfindingMalus`: the lending vehicle's map, else the own one.
    #[must_use]
    pub fn get_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.inherited_path_type_malus
            .as_ref()
            .unwrap_or(&self.path_type_malus)[path_type as usize]
            .unwrap_or_else(|| path_type.get_malus())
    }

    /// Vanilla `Mob.setPathfindingMalus`: always the mob's own map, which a lending vehicle's
    /// map shadows for reads.
    pub const fn set_pathfinding_malus(&mut self, path_type: PathType, malus: f32) {
        self.path_type_malus[path_type as usize] = Some(malus);
    }

    /// The mob's own map, whatever a vehicle lends.
    #[must_use]
    pub fn own_pathfinding_malus(&self, path_type: PathType) -> f32 {
        self.path_type_malus[path_type as usize].unwrap_or_else(|| path_type.get_malus())
    }

    /// Reads come from `malus` (the controlled vehicle's map) from now on.
    pub const fn inherit_pathfinding_malus(&mut self, malus: [Option<f32>; PATH_TYPE_COUNT]) {
        self.inherited_path_type_malus = Some(malus);
    }

    #[must_use]
    pub const fn block_position(&self) -> (i32, i32, i32) {
        (
            self.position.x.floor() as i32,
            self.position.y.floor() as i32,
            self.position.z.floor() as i32,
        )
    }

    #[must_use]
    pub fn get_bb_width(&self) -> i32 {
        (self.width + 1.0).floor() as i32
    }

    #[must_use]
    pub fn get_bb_height(&self) -> i32 {
        (self.height + 1.0).floor() as i32
    }
}

pub struct BaseNodeEvaluator {
    pub context: Option<PathfindingContext>,
    pub mob_data: Option<MobData>,
    pub nodes: FxHashMap<Vector3<i32>, Node>,
    pub entity_width: i32,
    pub entity_height: i32,
    pub entity_depth: i32,
    pub can_pass_doors: bool,
    pub can_open_doors: bool,
    pub can_float: bool,
    pub can_walk_over_fences: bool,
}

impl Default for BaseNodeEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl BaseNodeEvaluator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            context: None,
            mob_data: None,
            nodes: FxHashMap::default(),
            entity_width: 1,
            entity_height: 2,
            entity_depth: 1,
            can_pass_doors: true,
            can_open_doors: false,
            can_float: false,
            can_walk_over_fences: false,
        }
    }

    /// Vanilla `NodeEvaluator.getNode`, as a copy of the cached node.
    pub fn get_node(&mut self, pos: BlockPos) -> Node {
        *self.node_mut(pos)
    }

    /// The cached node at `pos`. Vanilla hands out one shared `Node` per position
    /// and mutates it in place, so every change has to land here to persist.
    pub fn node_mut(&mut self, pos: BlockPos) -> &mut Node {
        self.nodes.entry(pos.0).or_insert_with(|| Node::new(pos))
    }

    pub fn reset(&mut self) {
        self.nodes.clear();
        self.context = None;
        self.mob_data = None;
    }

    #[must_use]
    pub fn is_position_in_bounds(&self, x: i32, y: i32, z: i32) -> bool {
        self.mob_data.as_ref().is_none_or(|mob_data| {
            let mob_pos = mob_data.block_position();
            let dx = (x - mob_pos.0).abs();
            let dy = (y - mob_pos.1).abs();
            let dz = (z - mob_pos.2).abs();

            dx <= self.entity_width / 2 && dy <= self.entity_height && dz <= self.entity_depth / 2
        })
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_util::math::vector3::Vector3;

    use super::MobData;
    use crate::entity::ai::pathfinder::node::{PATH_TYPE_COUNT, PathType};

    /// Vanilla `Mob.setPathfindingMalus` writes the mob's own map, while
    /// `getPathfindingMalus` reads the vehicle's map when it lends one: a write made
    /// while inheriting (`AmphibiousNodeEvaluator.prepare`) does not show in reads.
    #[test]
    fn writes_go_to_own_map_while_reads_come_from_the_inherited_one() {
        let mut mob_data = MobData::new(Vector3::new(0.0, 0.0, 0.0), 0.6, 1.8);
        let mut inherited = [None; PATH_TYPE_COUNT];
        inherited[PathType::Water as usize] = Some(8.0);
        mob_data.inherit_pathfinding_malus(inherited);

        mob_data.set_pathfinding_malus(PathType::Water, 0.0);

        assert_eq!(mob_data.get_pathfinding_malus(PathType::Water), 8.0);
        assert_eq!(mob_data.own_pathfinding_malus(PathType::Water), 0.0);
        // Untouched types fall back to the default, not to the own map.
        assert_eq!(
            mob_data.get_pathfinding_malus(PathType::Lava),
            PathType::Lava.get_malus()
        );
    }

    #[test]
    fn without_a_lending_vehicle_writes_show_in_reads() {
        let mut mob_data = MobData::new(Vector3::new(0.0, 0.0, 0.0), 0.6, 1.8);
        mob_data.set_pathfinding_malus(PathType::Water, 0.0);
        assert_eq!(mob_data.get_pathfinding_malus(PathType::Water), 0.0);
    }
}
