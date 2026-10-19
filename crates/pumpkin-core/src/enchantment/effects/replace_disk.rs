use std::sync::Arc;

use pumpkin_data::BlockState;
use pumpkin_data::enchantment::{LevelBasedValue, ReplaceDiskPredicate};
use pumpkin_data::game_event::GameEvent;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;

use crate::world::World;

/// Enchantment entity effect that replaces a disk of blocks around an offset position.
pub struct ReplaceDisk {
    pub radius: LevelBasedValue,
    pub height: LevelBasedValue,
    pub offset: Vector3<i32>,
    pub predicate: Option<ReplaceDiskPredicate>,
    pub block_state: &'static BlockState,
    /// The properties `block_state` is set to; its block's default state when empty.
    pub block_state_properties: &'static [(&'static str, &'static str)],
    pub trigger_game_event: Option<GameEvent>,
}

pub trait ReplaceDiskPredicateExt {
    fn test_world(&self, world: &World, pos: &BlockPos) -> bool;
}

impl ReplaceDiskPredicateExt for ReplaceDiskPredicate {
    fn test_world(&self, world: &World, pos: &BlockPos) -> bool {
        match self {
            Self::MatchingBlockTag { offset, tag } => {
                let test_pos = pos.offset(*offset);
                let state_id = world.get_block_state_id(&test_pos);
                state_id.to_block_id().has_tag(**tag)
            }
            Self::MatchingBlocks { offset, blocks } => {
                let test_pos = pos.offset(*offset);
                let block = world.get_block(&test_pos);
                blocks.iter().any(|b| {
                    let stripped = b.strip_prefix("minecraft:").unwrap_or(b);
                    stripped == block.name
                })
            }
            // `FluidState.is(fluid)`: the source fluid for a source block, not its flowing family.
            Self::MatchingFluids { offset, fluids } => {
                let test_pos = pos.offset(*offset);
                let fluid = world.get_fluid_type(&test_pos);
                fluids.iter().any(|f| {
                    let stripped = f.strip_prefix("minecraft:").unwrap_or(f);
                    stripped == fluid.name
                })
            }
            // `UnobstructedPredicate`: no entity that blocks building is in the block.
            Self::Unobstructed => {
                let min = pos.to_f64();
                let block_box = pumpkin_util::math::boundingbox::BoundingBox::new(
                    min,
                    Vector3::new(min.x + 1.0, min.y + 1.0, min.z + 1.0),
                );
                let blocked = |entity: &dyn crate::entity::EntityBase| {
                    let base = entity.get_entity();
                    !base.is_removed()
                        && blocks_building(entity)
                        && base.bounding_box.load().intersects(&block_box)
                };
                !world
                    .get_entities_at_box(&block_box)
                    .iter()
                    .any(|entity| blocked(entity.as_ref()))
                    && !world
                        .get_players_at_box(&block_box)
                        .iter()
                        .any(|player| blocked(player.as_ref()))
            }
            Self::AllOf(predicates) => predicates.iter().all(|p| p.test_world(world, pos)),
        }
    }
}

/// Vanilla `Entity.blocksBuilding`: living entities, boats, minecarts, falling blocks, primed
/// TNT and end crystals.
fn blocks_building(entity: &dyn crate::entity::EntityBase) -> bool {
    let entity_type = entity.get_entity().entity_type;
    let name = entity_type.resource_name;
    entity.get_living_entity().is_some()
        || name.ends_with("_boat")
        || name.ends_with("_raft")
        || name.contains("minecart")
        || [
            &pumpkin_data::entity::EntityType::FALLING_BLOCK,
            &pumpkin_data::entity::EntityType::TNT,
            &pumpkin_data::entity::EntityType::END_CRYSTAL,
        ]
        .contains(&entity_type)
}

impl ReplaceDisk {
    #[must_use]
    pub const fn new(
        radius: LevelBasedValue,
        height: LevelBasedValue,
        offset: Vector3<i32>,
        predicate: Option<ReplaceDiskPredicate>,
        block_state: &'static BlockState,
        trigger_game_event: Option<GameEvent>,
    ) -> Self {
        Self {
            radius,
            height,
            offset,
            predicate,
            block_state,
            block_state_properties: &[],
            trigger_game_event,
        }
    }

    #[must_use]
    pub const fn simple(
        radius: LevelBasedValue,
        height: LevelBasedValue,
        offset: Vector3<i32>,
        block_state: &'static BlockState,
        trigger_game_event: Option<GameEvent>,
    ) -> Self {
        Self {
            radius,
            height,
            offset,
            predicate: None,
            block_state,
            block_state_properties: &[],
            trigger_game_event,
        }
    }

    /// The state placed: `block_state`'s block with `block_state_properties`.
    #[must_use]
    pub fn resolved_state_id(&self) -> pumpkin_data::BlockStateId {
        if self.block_state_properties.is_empty() {
            return self.block_state.id;
        }
        let block = pumpkin_data::Block::from_state_id(self.block_state.id);
        let properties: Vec<pumpkin_data::Property> = self
            .block_state_properties
            .iter()
            .filter_map(|(key, value)| {
                pumpkin_data::Property::from_key_value_str(
                    pumpkin_data::PropertyKey::from_str(key)?,
                    value,
                )
            })
            .collect();
        block.from_properties(&properties).to_state_id(block)
    }

    #[must_use]
    pub fn calculate_radius(&self, level: i32) -> f32 {
        self.radius.calculate(level)
    }

    #[must_use]
    pub fn calculate_height(&self, level: i32) -> f32 {
        self.height.calculate(level)
    }

    #[must_use]
    pub fn center_position(&self, origin: Vector3<f64>) -> BlockPos {
        BlockPos::containing_vec(origin).offset(self.offset)
    }

    #[must_use]
    pub fn is_in_radius(pos: &BlockPos, position: Vector3<f64>, dist: i32) -> bool {
        pos.dist_to_center_sqr(position.x, f64::from(pos.0.y) + 0.5, position.z)
            < f64::from(dist * dist)
    }

    /// Iterates through all candidate block positions within the disk region.
    pub fn iterate_blocks(
        &self,
        level: i32,
        position: Vector3<f64>,
    ) -> impl Iterator<Item = BlockPos> {
        let center_block = self.center_position(position);
        let dist = self.radius.calculate(level) as i32;
        let height = self.height.calculate(level) as i32;
        let start = center_block.offset(Vector3::new(-dist, 0, -dist));
        let end = center_block.offset(Vector3::new(dist, (height - 1).min(0), dist));
        BlockPos::between_closed(start, end)
            .filter(move |pos| Self::is_in_radius(pos, position, dist))
    }

    /// Applies the replace disk effect on the given world and position.
    pub fn apply(
        &self,
        world: &Arc<World>,
        enchantment_level: i32,
        _entity: Option<&crate::entity::Entity>,
        position: Vector3<f64>,
    ) {
        let mut changed = false;
        for pos in self.iterate_blocks(enchantment_level, position) {
            let passes_predicate = self
                .predicate
                .as_ref()
                .is_none_or(|p| p.test_world(world.as_ref(), &pos));
            if !passes_predicate {
                continue;
            }

            let new_state_id = self.resolved_state_id();
            let old_state_id = world.set_block_state(&pos, new_state_id, BlockFlags::NOTIFY_ALL);
            if old_state_id != new_state_id {
                changed = true;
                if let Some(event) = self.trigger_game_event {
                    world.game_event_from(None, event, pos.to_centered_f64());
                }
            }
        }
        if changed {
            world.flush_block_updates();
        }
    }
}

impl super::EnchantmentEntityEffectExt for ReplaceDisk {
    fn apply(
        &self,
        world: &Arc<World>,
        enchantment_level: i32,
        _item: &crate::enchantment::EnchantedItemInUse<'_>,
        entity: &dyn crate::entity::EntityBase,
        position: Vector3<f64>,
    ) {
        let entity = Some(entity.get_entity());
        self.apply(world, enchantment_level, entity, position);
    }
}
