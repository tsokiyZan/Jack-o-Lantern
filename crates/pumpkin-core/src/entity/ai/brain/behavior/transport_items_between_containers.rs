//! Vanilla `TransportItemsBetweenContainers`.

use std::sync::Arc;

use pumpkin_data::BlockDirection;
use pumpkin_data::BlockState;
use pumpkin_data::block_properties::{ChestLikeProperties, ChestType};
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::Inventory;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::clip::{ClipBlock, ClipContext, ClipFluid};
use rustc_hash::FxHashSet;

use super::super::BrainTick;
use super::super::memory::position_tracker::{BlockPosTracker, PositionTracker};
use super::super::memory::{GlobalPos, MemoryModuleId, MemoryStatus, types};
use super::timed::Behavior;
use super::utils;
use crate::block::blocks::chests;
use crate::block::entities::BlockEntity;
use crate::block::entities::chest::ChestBlockEntity;
use crate::block::entities::trapped_chest::TrappedChestBlockEntity;
use crate::entity::ai::pathfinder::path::Path;
use crate::world::World;

pub const TARGET_INTERACTION_TIME: i32 = 60;
const VISITED_POSITIONS_MEMORY_TIME: i64 = 6000;
const TRANSPORTED_ITEM_MAX_STACK_SIZE: u8 = 16;
const MAX_VISITED_POSITIONS: usize = 10;
const MAX_UNREACHABLE_POSITIONS: usize = 50;
const PASSENGER_MOB_TARGET_SEARCH_DISTANCE: i32 = 1;
const IDLE_COOLDOWN: i32 = 140;
const CLOSE_ENOUGH_TO_START_QUEUING_DISTANCE: f64 = 3.0;
const CLOSE_ENOUGH_TO_START_INTERACTING_WITH_TARGET_DISTANCE: f64 = 0.5;
const CLOSE_ENOUGH_TO_START_INTERACTING_WITH_TARGET_PATH_END_DISTANCE: f64 = 1.0;
const CLOSE_ENOUGH_TO_CONTINUE_INTERACTING_WITH_TARGET: f64 = 2.0;

const ENTRY_CONDITIONS: [(MemoryModuleId, MemoryStatus); 4] = [
    (
        types::VISITED_BLOCK_POSITIONS.id(),
        MemoryStatus::Registered,
    ),
    (
        types::UNREACHABLE_TRANSPORT_BLOCK_POSITIONS.id(),
        MemoryStatus::Registered,
    ),
    (
        types::TRANSPORT_ITEMS_COOLDOWN_TICKS.id(),
        MemoryStatus::ValueAbsent,
    ),
    (types::IS_PANICKING.id(), MemoryStatus::ValueAbsent),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ContainerInteractionState {
    PickupItem,
    PickupNoItem,
    PlaceItem,
    PlaceNoItem,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TransportItemState {
    Travelling,
    Queuing,
    Interacting,
}

/// Vanilla `TransportItemsBetweenContainers.TransportItemTarget`.
#[derive(Clone)]
pub struct TransportItemTarget {
    pub pos: BlockPos,
    pub container: Arc<dyn Inventory>,
    pub block_entity: Arc<dyn BlockEntity>,
    pub state: &'static BlockState,
}

impl TransportItemTarget {
    /// Vanilla `TransportItemTarget.tryCreatePossibleTarget(BlockEntity, Level)`.
    #[must_use]
    pub fn try_create_possible_target(
        block_entity: Arc<dyn BlockEntity>,
        world: &World,
    ) -> Option<Self> {
        let pos = block_entity.get_position();
        let state = world.get_block_state(&pos);
        let container = Self::get_block_entity_container(&block_entity, world, &pos)?;
        Some(Self {
            pos,
            container,
            block_entity,
            state,
        })
    }

    /// Vanilla `TransportItemTarget.tryCreatePossibleTarget(BlockPos, Level)`.
    #[must_use]
    pub fn try_create_possible_target_at(pos: &BlockPos, world: &World) -> Option<Self> {
        Self::try_create_possible_target(world.get_block_entity(pos)?, world)
    }

    fn get_block_entity_container(
        block_entity: &Arc<dyn BlockEntity>,
        world: &World,
        pos: &BlockPos,
    ) -> Option<Arc<dyn Inventory>> {
        if chests::is_chest_block(world.get_block(pos)) {
            chests::get_container(world, pos)
        } else {
            Arc::clone(block_entity).get_inventory()
        }
    }

    /// The chest half this target pairs with, when its block is part of a double chest.
    fn connected_pos(&self) -> Option<BlockPos> {
        if !chests::is_chest_block(pumpkin_data::Block::from_state_id(self.state.id)) {
            return None;
        }
        let props = ChestLikeProperties::from_state_id(self.state.id);
        (props.r#type != ChestType::Single)
            .then(|| chests::get_connected_block_pos(&self.pos, &props))
    }
}

pub type OnTargetReachedInteraction =
    Box<dyn Fn(&mut BrainTick<'_>, &TransportItemTarget, i32) + Send + Sync>;

/// Vanilla `TransportItemsBetweenContainers`: carries items from source containers to
/// destination containers holding the same item, or empty ones.
pub struct TransportItemsBetweenContainers {
    speed_modifier: f32,
    horizontal_search_distance: i32,
    vertical_search_distance: i32,
    source_block_type: fn(&BlockState) -> bool,
    destination_block_type: fn(&BlockState) -> bool,
    should_queue_for_target: fn(&World, &TransportItemTarget) -> bool,
    on_start_travelling: fn(&mut BrainTick<'_>),
    on_target_interaction_actions: [Option<OnTargetReachedInteraction>; 4],
    target: Option<TransportItemTarget>,
    state: TransportItemState,
    interaction_state: Option<ContainerInteractionState>,
    ticks_since_reaching_target: i32,
}

impl TransportItemsBetweenContainers {
    #[must_use]
    #[expect(clippy::too_many_arguments, reason = "mirrors the vanilla constructor")]
    pub fn new(
        speed_modifier: f32,
        source_block_type: fn(&BlockState) -> bool,
        destination_block_type: fn(&BlockState) -> bool,
        horizontal_search_distance: i32,
        vertical_search_distance: i32,
        on_target_interaction_actions: [Option<OnTargetReachedInteraction>; 4],
        on_start_travelling: fn(&mut BrainTick<'_>),
        should_queue_for_target: fn(&World, &TransportItemTarget) -> bool,
    ) -> Self {
        Self {
            speed_modifier,
            horizontal_search_distance,
            vertical_search_distance,
            source_block_type,
            destination_block_type,
            should_queue_for_target,
            on_start_travelling,
            on_target_interaction_actions,
            target: None,
            state: TransportItemState::Travelling,
            interaction_state: None,
            ticks_since_reaching_target: 0,
        }
    }

    fn update_invalid_target(&mut self, tick: &mut BrainTick<'_>) -> bool {
        if self.has_valid_target(tick) {
            return false;
        }
        self.stop_targeting_current_target(tick);
        if let Some(target) = self.get_transport_target(tick) {
            let pos = target.pos;
            self.target = Some(target);
            self.on_start_travelling(tick);
            self.set_visited_block_pos(tick, pos);
        } else {
            self.enter_cooldown_after_no_matching_target_found(tick);
        }
        true
    }

    fn on_queuing_for_target(&mut self, tick: &mut BrainTick<'_>, target: &TransportItemTarget) {
        if !self.is_another_mob_interacting_with_target(target, tick.world) {
            self.resume_travelling(tick);
        }
    }

    fn on_travel_to_target(&mut self, tick: &mut BrainTick<'_>, target: &TransportItemTarget) {
        let center = Self::get_center_pos(tick);
        if Self::is_within_target_distance(
            CLOSE_ENOUGH_TO_START_QUEUING_DISTANCE,
            target,
            tick,
            center,
        ) && self.is_another_mob_interacting_with_target(target, tick.world)
        {
            self.start_queuing(tick);
        } else if Self::is_within_target_distance(
            Self::get_interaction_range(tick),
            target,
            tick,
            center,
        ) {
            self.start_on_reached_target_interaction(tick, target);
        } else {
            self.walk_towards_target(tick);
        }
    }

    fn get_center_pos(tick: &BrainTick<'_>) -> Vector3<f64> {
        Self::set_middle_y_position(tick, tick.mob.get_entity().pos.load())
    }

    fn on_reached_target(&mut self, tick: &mut BrainTick<'_>, target: &TransportItemTarget) {
        if !Self::is_within_target_distance(
            CLOSE_ENOUGH_TO_CONTINUE_INTERACTING_WITH_TARGET,
            target,
            tick,
            Self::get_center_pos(tick),
        ) {
            self.on_start_travelling(tick);
            return;
        }
        self.ticks_since_reaching_target += 1;
        self.on_target_interaction(tick, target);
        if self.ticks_since_reaching_target >= TARGET_INTERACTION_TIME {
            match Self::reached_target_interaction(tick, target) {
                ContainerInteractionState::PickupItem => {
                    self.pick_up_items(tick, &target.container);
                }
                ContainerInteractionState::PlaceItem => {
                    self.put_down_item(tick, &target.container);
                }
                ContainerInteractionState::PickupNoItem
                | ContainerInteractionState::PlaceNoItem => {
                    self.stop_targeting_current_target(tick);
                }
            }
            self.on_start_travelling(tick);
        }
    }

    fn start_queuing(&mut self, tick: &BrainTick<'_>) {
        Self::stop_in_place(tick);
        self.state = TransportItemState::Queuing;
    }

    fn resume_travelling(&mut self, tick: &mut BrainTick<'_>) {
        self.state = TransportItemState::Travelling;
        self.walk_towards_target(tick);
    }

    fn walk_towards_target(&self, tick: &mut BrainTick<'_>) {
        if let Some(target) = &self.target {
            utils::set_walk_and_look_target_memories_to_block(
                tick.brain,
                target.pos,
                self.speed_modifier,
                0,
            );
        }
    }

    fn start_on_reached_target_interaction(
        &mut self,
        tick: &BrainTick<'_>,
        target: &TransportItemTarget,
    ) {
        self.interaction_state = Some(Self::reached_target_interaction(tick, target));
        self.state = TransportItemState::Interacting;
    }

    /// Vanilla's private `onStartTravelling(body)`.
    fn on_start_travelling(&mut self, tick: &mut BrainTick<'_>) {
        (self.on_start_travelling)(tick);
        self.state = TransportItemState::Travelling;
        self.interaction_state = None;
        self.ticks_since_reaching_target = 0;
    }

    fn on_target_interaction(&self, tick: &mut BrainTick<'_>, target: &TransportItemTarget) {
        tick.brain.set(
            types::LOOK_TARGET,
            Arc::new(BlockPosTracker::new(target.pos)) as Arc<dyn PositionTracker>,
        );
        Self::stop_in_place(tick);
        if let Some(state) = self.interaction_state
            && let Some(action) = &self.on_target_interaction_actions[state as usize]
        {
            action(tick, target, self.ticks_since_reaching_target);
        }
    }

    /// Vanilla `doReachedTargetInteraction`, which picks one of four callbacks; this returns
    /// which one.
    fn reached_target_interaction(
        tick: &BrainTick<'_>,
        target: &TransportItemTarget,
    ) -> ContainerInteractionState {
        // `RandomizableContainerBlockEntity.isEmpty` unpacks a pending loot table first.
        Self::unpack_loot_tables(tick.world, target);
        if Self::is_picking_up_items(tick) {
            if Self::matches_getting_items_requirement(&target.container) {
                ContainerInteractionState::PickupItem
            } else {
                ContainerInteractionState::PickupNoItem
            }
        } else if Self::matches_leaving_items_requirement(tick, &target.container) {
            ContainerInteractionState::PlaceItem
        } else {
            ContainerInteractionState::PlaceNoItem
        }
    }

    fn unpack_loot_tables(world: &World, target: &TransportItemTarget) {
        chests::unpack_loot_table(&target.block_entity);
        if let Some(connected) = target
            .connected_pos()
            .and_then(|pos| world.get_block_entity(&pos))
        {
            chests::unpack_loot_table(&connected);
        }
    }

    fn get_transport_target(&self, tick: &BrainTick<'_>) -> Option<TransportItemTarget> {
        let search_area = self.get_target_search_area(tick);
        let visited = Self::get_visited_positions(tick);
        let unreachable = Self::get_unreachable_positions(tick);
        let entity = tick.mob.get_entity();
        let block_pos = entity.block_pos.load();
        let position = entity.pos.load();
        let radius = self.get_horizontal_search_distance(tick).div_euclid(16) + 1;
        let center_x = block_pos.0.x >> 4;
        let center_z = block_pos.0.z >> 4;
        let mut target = None;
        let mut closest_distance = f64::from(f32::MAX);
        // `ChunkPos.rangeClosed` walks x fastest, then z.
        for chunk_z in center_z - radius..=center_z + radius {
            for chunk_x in center_x - radius..=center_x + radius {
                let block_entities: Vec<Arc<dyn BlockEntity>> = tick
                    .world
                    .block_entities
                    .get(&Vector2::new(chunk_x, chunk_z))
                    .map(|chunk| chunk.values().cloned().collect())
                    .unwrap_or_default();
                for potential_target in block_entities {
                    let any = potential_target.as_any();
                    if !any.is::<ChestBlockEntity>() && !any.is::<TrappedChestBlockEntity>() {
                        continue;
                    }
                    let distance = dist_to_center_sqr(&potential_target.get_position(), position);
                    if distance < closest_distance
                        && let Some(valid) = self.is_target_valid_to_pick(
                            tick,
                            potential_target,
                            &visited,
                            &unreachable,
                            &search_area,
                        )
                    {
                        target = Some(valid);
                        closest_distance = distance;
                    }
                }
            }
        }
        target
    }

    fn is_target_valid_to_pick(
        &self,
        tick: &BrainTick<'_>,
        block_entity: Arc<dyn BlockEntity>,
        visited: &FxHashSet<GlobalPos>,
        unreachable: &FxHashSet<GlobalPos>,
        search_area: &BoundingBox,
    ) -> Option<TransportItemTarget> {
        let pos = block_entity.get_position();
        if !search_area.contains(pos.0.to_f64()) {
            return None;
        }
        let target = TransportItemTarget::try_create_possible_target(block_entity, tick.world)?;
        // TODO: `isContainerLocked` needs `BaseContainerBlockEntity.lockKey`, which Pumpkin's
        // chests don't keep.
        (self.is_wanted_block(tick, target.state)
            && !Self::is_position_already_visited(visited, unreachable, &target, tick.world))
        .then_some(target)
    }

    fn has_valid_target(&mut self, tick: &mut BrainTick<'_>) -> bool {
        let Some(target) = &self.target else {
            return false;
        };
        let target_is_of_valid_type = self.is_wanted_block(tick, target.state)
            && Self::target_has_not_changed(tick.world, target);
        if target_is_of_valid_type && !chests::is_chest_blocked(tick.world, &target.pos) {
            if self.state != TransportItemState::Travelling {
                return true;
            }
            if Self::has_valid_travelling_path(tick, target) {
                return true;
            }
            let pos = target.pos;
            self.mark_visited_block_pos_as_unreachable(tick, pos);
        }
        false
    }

    fn has_valid_travelling_path(tick: &BrainTick<'_>, target: &TransportItemTarget) -> bool {
        let path = {
            let mob_entity = tick.mob.get_mob_entity();
            let mut navigator = mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let existing = navigator.get_path().cloned();
            existing.or_else(|| {
                navigator.create_path(&mob_entity.living_entity, target.pos.0.to_f64(), 0)
            })
        };
        let pos_from_which_to_reach_target =
            Self::get_position_to_reach_target_from(tick, path.as_ref());
        let can_reach_target = Self::is_within_target_distance(
            Self::get_interaction_range(tick),
            target,
            tick,
            pos_from_which_to_reach_target,
        );
        let has_not_yet_created_path_to_target = path.is_none() && !can_reach_target;
        has_not_yet_created_path_to_target
            || Self::target_is_reachable_from_position(
                tick,
                can_reach_target,
                pos_from_which_to_reach_target,
                target,
            )
    }

    fn get_position_to_reach_target_from(
        tick: &BrainTick<'_>,
        path: Option<&Path>,
    ) -> Vector3<f64> {
        let bottom_center = path.and_then(Path::get_end_node).map_or_else(
            || tick.mob.get_entity().pos.load(),
            |end| {
                Vector3::new(
                    f64::from(end.pos.0.x) + 0.5,
                    f64::from(end.pos.0.y),
                    f64::from(end.pos.0.z) + 0.5,
                )
            },
        );
        Self::set_middle_y_position(tick, bottom_center)
    }

    fn set_middle_y_position(tick: &BrainTick<'_>, pos: Vector3<f64>) -> Vector3<f64> {
        let bounding_box = tick.mob.get_entity().bounding_box.load();
        pos.add_raw(0.0, (bounding_box.max.y - bounding_box.min.y) / 2.0, 0.0)
    }

    fn target_has_not_changed(world: &World, target: &TransportItemTarget) -> bool {
        world.get_block_entity(&target.pos).is_some_and(|current| {
            std::ptr::addr_eq(Arc::as_ptr(&current), Arc::as_ptr(&target.block_entity))
        })
    }

    /// Vanilla `getConnectedTargets`.
    fn get_connected_targets(
        target: &TransportItemTarget,
        world: &World,
    ) -> Vec<TransportItemTarget> {
        let mut targets = vec![target.clone()];
        if let Some(connected) = target
            .connected_pos()
            .and_then(|pos| TransportItemTarget::try_create_possible_target_at(&pos, world))
        {
            targets.push(connected);
        }
        targets
    }

    fn get_target_search_area(&self, tick: &BrainTick<'_>) -> BoundingBox {
        let horizontal = f64::from(self.get_horizontal_search_distance(tick));
        let vertical = f64::from(self.get_vertical_search_distance(tick));
        BoundingBox::from_block(&tick.mob.get_entity().block_pos.load())
            .expand(horizontal, vertical, horizontal)
    }

    fn get_horizontal_search_distance(&self, tick: &BrainTick<'_>) -> i32 {
        if tick.mob.get_entity().has_vehicle() {
            PASSENGER_MOB_TARGET_SEARCH_DISTANCE
        } else {
            self.horizontal_search_distance
        }
    }

    fn get_vertical_search_distance(&self, tick: &BrainTick<'_>) -> i32 {
        if tick.mob.get_entity().has_vehicle() {
            PASSENGER_MOB_TARGET_SEARCH_DISTANCE
        } else {
            self.vertical_search_distance
        }
    }

    fn get_visited_positions(tick: &BrainTick<'_>) -> FxHashSet<GlobalPos> {
        tick.brain
            .get(types::VISITED_BLOCK_POSITIONS)
            .cloned()
            .unwrap_or_default()
    }

    fn get_unreachable_positions(tick: &BrainTick<'_>) -> FxHashSet<GlobalPos> {
        tick.brain
            .get(types::UNREACHABLE_TRANSPORT_BLOCK_POSITIONS)
            .cloned()
            .unwrap_or_default()
    }

    fn is_position_already_visited(
        visited: &FxHashSet<GlobalPos>,
        unreachable: &FxHashSet<GlobalPos>,
        target: &TransportItemTarget,
        world: &World,
    ) -> bool {
        Self::get_connected_targets(target, world)
            .iter()
            .filter_map(|connected| utils::global_pos_in(world, connected.pos))
            .any(|pos| visited.contains(&pos) || unreachable.contains(&pos))
    }

    fn has_finished_path(tick: &BrainTick<'_>) -> bool {
        tick.mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_path()
            .is_some_and(Path::is_done)
    }

    fn set_visited_block_pos(&mut self, tick: &mut BrainTick<'_>, target: BlockPos) {
        let mut visited = Self::get_visited_positions(tick);
        if let Some(pos) = utils::global_pos_in(tick.world, target) {
            visited.insert(pos);
        }
        if visited.len() > MAX_VISITED_POSITIONS {
            self.enter_cooldown_after_no_matching_target_found(tick);
        } else {
            tick.brain.set_with_expiry(
                types::VISITED_BLOCK_POSITIONS,
                visited,
                VISITED_POSITIONS_MEMORY_TIME,
            );
        }
    }

    fn mark_visited_block_pos_as_unreachable(
        &mut self,
        tick: &mut BrainTick<'_>,
        target: BlockPos,
    ) {
        let mut visited = Self::get_visited_positions(tick);
        let mut unreachable = Self::get_unreachable_positions(tick);
        if let Some(pos) = utils::global_pos_in(tick.world, target) {
            visited.remove(&pos);
            unreachable.insert(pos);
        }
        if unreachable.len() > MAX_UNREACHABLE_POSITIONS {
            self.enter_cooldown_after_no_matching_target_found(tick);
        } else {
            tick.brain.set_with_expiry(
                types::VISITED_BLOCK_POSITIONS,
                visited,
                VISITED_POSITIONS_MEMORY_TIME,
            );
            tick.brain.set_with_expiry(
                types::UNREACHABLE_TRANSPORT_BLOCK_POSITIONS,
                unreachable,
                VISITED_POSITIONS_MEMORY_TIME,
            );
        }
    }

    fn is_wanted_block(&self, tick: &BrainTick<'_>, state: &BlockState) -> bool {
        if Self::is_picking_up_items(tick) {
            (self.source_block_type)(state)
        } else {
            (self.destination_block_type)(state)
        }
    }

    fn get_interaction_range(tick: &BrainTick<'_>) -> f64 {
        if Self::has_finished_path(tick) {
            CLOSE_ENOUGH_TO_START_INTERACTING_WITH_TARGET_PATH_END_DISTANCE
        } else {
            CLOSE_ENOUGH_TO_START_INTERACTING_WITH_TARGET_DISTANCE
        }
    }

    fn is_within_target_distance(
        distance: f64,
        target: &TransportItemTarget,
        tick: &BrainTick<'_>,
        from_pos: Vector3<f64>,
    ) -> bool {
        let bounding_box = tick.mob.get_entity().bounding_box.load();
        let half_x = (bounding_box.max.x - bounding_box.min.x) / 2.0;
        let half_y = (bounding_box.max.y - bounding_box.min.y) / 2.0;
        let half_z = (bounding_box.max.z - bounding_box.min.z) / 2.0;
        // `AABB.ofSize`.
        let moved_bound_box = BoundingBox::new(
            Vector3::new(
                from_pos.x - half_x,
                from_pos.y - half_y,
                from_pos.z - half_z,
            ),
            Vector3::new(
                from_pos.x + half_x,
                from_pos.y + half_y,
                from_pos.z + half_z,
            ),
        );
        let Some(bounds) = collision_shape_bounds(target.state) else {
            return false;
        };
        bounds
            .expand(distance, 0.5, distance)
            .shift(target.pos.0.to_f64())
            .intersects(&moved_bound_box)
    }

    fn target_is_reachable_from_position(
        tick: &BrainTick<'_>,
        can_reach_target: bool,
        pos: Vector3<f64>,
        target: &TransportItemTarget,
    ) -> bool {
        can_reach_target && Self::can_see_any_target_side(target, tick.world, pos)
    }

    /// Vanilla `canSeeAnyTargetSide`. Pumpkin's clip has no entity collision context.
    fn can_see_any_target_side(
        target: &TransportItemTarget,
        world: &World,
        eye_position: Vector3<f64>,
    ) -> bool {
        let center = target.pos.to_centered_f64();
        BlockDirection::all().into_iter().any(|direction| {
            let step = direction.to_offset();
            let hit_target = center.add_raw(
                0.5 * f64::from(step.x),
                0.5 * f64::from(step.y),
                0.5 * f64::from(step.z),
            );
            let hit = world.clip(&ClipContext::new(
                eye_position,
                hit_target,
                ClipBlock::Collider,
                ClipFluid::None,
            ));
            !hit.is_miss && hit.block_pos == target.pos
        })
    }

    fn is_another_mob_interacting_with_target(
        &self,
        target: &TransportItemTarget,
        world: &World,
    ) -> bool {
        Self::get_connected_targets(target, world)
            .iter()
            .any(|connected| (self.should_queue_for_target)(world, connected))
    }

    fn is_picking_up_items(tick: &BrainTick<'_>) -> bool {
        tick.mob
            .get_mob_entity()
            .get_item_slot(&EquipmentSlot::MAIN_HAND)
            .is_empty()
    }

    fn matches_getting_items_requirement(container: &Arc<dyn Inventory>) -> bool {
        !container.is_empty()
    }

    fn matches_leaving_items_requirement(
        tick: &BrainTick<'_>,
        container: &Arc<dyn Inventory>,
    ) -> bool {
        container.is_empty() || Self::has_item_matching_hand_item(tick, container)
    }

    fn has_item_matching_hand_item(tick: &BrainTick<'_>, container: &Arc<dyn Inventory>) -> bool {
        let main_hand_item = tick
            .mob
            .get_mob_entity()
            .get_item_slot(&EquipmentSlot::MAIN_HAND);
        // `ItemStack.isSameItem`.
        (0..container.size()).any(|slot| container.get_stack(slot).item == main_hand_item.item)
    }

    fn pick_up_items(&mut self, tick: &mut BrainTick<'_>, container: &Arc<dyn Inventory>) {
        let mob_entity = tick.mob.get_mob_entity();
        mob_entity.set_item_slot(
            &EquipmentSlot::MAIN_HAND,
            Self::pickup_item_from_container(container),
        );
        mob_entity.set_guaranteed_drop(&EquipmentSlot::MAIN_HAND);
        container.mark_dirty();
        self.clear_memories_after_matching_target_found(tick);
    }

    fn put_down_item(&mut self, tick: &mut BrainTick<'_>, container: &Arc<dyn Inventory>) {
        let items_left_after_visiting_chest = Self::add_items_to_container(tick, container);
        container.mark_dirty();
        let empty = items_left_after_visiting_chest.is_empty();
        tick.mob
            .get_mob_entity()
            .set_item_slot(&EquipmentSlot::MAIN_HAND, items_left_after_visiting_chest);
        if empty {
            self.clear_memories_after_matching_target_found(tick);
        } else {
            self.stop_targeting_current_target(tick);
        }
    }

    fn pickup_item_from_container(container: &Arc<dyn Inventory>) -> ItemStack {
        for slot in 0..container.size() {
            let item_stack = container.get_stack(slot);
            if !item_stack.is_empty() {
                let item_count = item_stack.item_count.min(TRANSPORTED_ITEM_MAX_STACK_SIZE);
                return container.remove_stack_specific(slot, item_count);
            }
        }
        ItemStack::EMPTY.clone()
    }

    fn add_items_to_container(tick: &BrainTick<'_>, container: &Arc<dyn Inventory>) -> ItemStack {
        let mut item_stack = tick
            .mob
            .get_mob_entity()
            .get_item_slot(&EquipmentSlot::MAIN_HAND);
        for slot in 0..container.size() {
            let mut container_item_stack = container.get_stack(slot);
            if container_item_stack.is_empty() {
                container.set_stack(slot, item_stack);
                return ItemStack::EMPTY.clone();
            }
            let max_stack_size = container_item_stack.get_max_stack_size();
            if container_item_stack.are_items_and_components_equal(&item_stack)
                && container_item_stack.item_count < max_stack_size
            {
                let count_that_can_be_added = max_stack_size - container_item_stack.item_count;
                let count_to_add = count_that_can_be_added.min(item_stack.item_count);
                container_item_stack.item_count += count_to_add;
                // Vanilla takes off the room left, not what moved; a negative count is empty.
                item_stack.item_count = item_stack
                    .item_count
                    .saturating_sub(count_that_can_be_added);
                container.set_stack(slot, container_item_stack);
                if item_stack.is_empty() {
                    return ItemStack::EMPTY.clone();
                }
            }
        }
        item_stack
    }

    fn stop_targeting_current_target(&mut self, tick: &mut BrainTick<'_>) {
        self.ticks_since_reaching_target = 0;
        self.target = None;
        tick.mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
        tick.brain.erase(types::WALK_TARGET.id());
    }

    fn clear_memories_after_matching_target_found(&mut self, tick: &mut BrainTick<'_>) {
        self.stop_targeting_current_target(tick);
        tick.brain.erase(types::VISITED_BLOCK_POSITIONS.id());
        tick.brain
            .erase(types::UNREACHABLE_TRANSPORT_BLOCK_POSITIONS.id());
    }

    fn enter_cooldown_after_no_matching_target_found(&mut self, tick: &mut BrainTick<'_>) {
        self.stop_targeting_current_target(tick);
        tick.brain
            .set(types::TRANSPORT_ITEMS_COOLDOWN_TICKS, IDLE_COOLDOWN);
        tick.brain.erase(types::VISITED_BLOCK_POSITIONS.id());
        tick.brain
            .erase(types::UNREACHABLE_TRANSPORT_BLOCK_POSITIONS.id());
    }

    /// Vanilla's own `stopInPlace`, which unlike `Mob.stopInPlace` keeps the vertical motion.
    fn stop_in_place(tick: &BrainTick<'_>) {
        let mob_entity = tick.mob.get_mob_entity();
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
        let living = &mob_entity.living_entity;
        let input = living.movement_input.load();
        living.movement_input.store(Vector3::new(0.0, 0.0, input.z));
        mob_entity.set_speed(0.0);
        let velocity = living.entity.velocity.load();
        living
            .entity
            .velocity
            .store(Vector3::new(0.0, velocity.y, 0.0));
    }

    fn set_can_path_to_targets_below_surface(tick: &BrainTick<'_>, can_path: bool) {
        tick.mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_can_path_to_targets_below_surface(can_path);
    }
}

impl Behavior for TransportItemsBetweenContainers {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &ENTRY_CONDITIONS
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        !tick.mob.get_entity().is_leashed()
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        Self::set_can_path_to_targets_below_surface(tick, true);
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        !tick
            .brain
            .has_memory_value(types::TRANSPORT_ITEMS_COOLDOWN_TICKS.id())
            && !tick.brain.has_memory_value(types::IS_PANICKING.id())
            && !tick.mob.get_entity().is_leashed()
    }

    fn times_out(&self) -> bool {
        false
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let updated_invalid_target = self.update_invalid_target(tick);
        let Some(target) = self.target.clone() else {
            // Vanilla calls its own `stop` here and keeps running.
            self.stop(tick);
            return;
        };
        if updated_invalid_target {
            return;
        }
        if self.state == TransportItemState::Queuing {
            self.on_queuing_for_target(tick, &target);
        }
        if self.state == TransportItemState::Travelling {
            self.on_travel_to_target(tick, &target);
        }
        if self.state == TransportItemState::Interacting {
            self.on_reached_target(tick, &target);
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        self.on_start_travelling(tick);
        Self::set_can_path_to_targets_below_surface(tick, false);
    }

    fn debug_name(&self) -> &'static str {
        "TransportItemsBetweenContainers"
    }
}

/// Vanilla `BlockPos.distToCenterSqr(Position)`.
fn dist_to_center_sqr(pos: &BlockPos, position: Vector3<f64>) -> f64 {
    let center = pos.to_centered_f64();
    center.squared_distance_to_vec(&position)
}

/// Vanilla `getCollisionShape(level, pos).bounds()`, block-local.
fn collision_shape_bounds(state: &BlockState) -> Option<BoundingBox> {
    state.get_block_collision_shapes().reduce(|bounds, shape| {
        BoundingBox::new(
            Vector3::new(
                bounds.min.x.min(shape.min.x),
                bounds.min.y.min(shape.min.y),
                bounds.min.z.min(shape.min.z),
            ),
            Vector3::new(
                bounds.max.x.max(shape.max.x),
                bounds.max.y.max(shape.max.y),
                bounds.max.z.max(shape.max.z),
            ),
        )
    })
}
