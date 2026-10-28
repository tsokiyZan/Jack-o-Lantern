use std::sync::Arc;

use crate::{
    block::{
        BlockBehaviour, GetComparatorOutputArgs, NormalUseArgs, OnScheduledTickArgs,
        PathComputationType, PlacedArgs, UseWithItemArgs, registry::BlockActionResult,
    },
    entity::{Entity, EntityBase, item::ItemEntity},
    world::{World, game_event::GameEventContext},
};
use pumpkin_data::{
    Block, BlockState, BlockStateId,
    block_properties::ComposterLikeProperties,
    data_component_impl::CompostableImpl,
    entity::EntityType,
    game_event::GameEvent,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
    world::WorldEvent,
};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::{tick::TickPriority, world::BlockFlags};

#[pumpkin_block("minecraft:composter")]
pub struct ComposterBlock;

impl BlockBehaviour for ComposterBlock {
    /// Vanilla `ComposterBlock.useWithoutItem`: a ready composter gives its bone meal.
    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        if ComposterLikeProperties::from_state_id(state_id).level == 8 {
            Self::extract_produce(
                Some(args.player.as_ref()),
                args.world,
                args.position,
                state_id,
            );
            return BlockActionResult::Success;
        }
        BlockActionResult::Pass
    }

    /// Vanilla `ComposterBlock.useItemOn`: a compostable item goes in below level 7; at level 7
    /// it is kept but still succeeds; a full composter or anything else tries the empty hand.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        let level = ComposterLikeProperties::from_state_id(state_id).level;
        let Some(chance) = args
            .item_stack
            .get_data_component::<CompostableImpl>()
            .map(CompostableImpl::chance)
        else {
            return BlockActionResult::PassToDefaultBlockAction;
        };
        if level >= 8 {
            return BlockActionResult::PassToDefaultBlockAction;
        }

        if level < 7 {
            let new_state_id = Self::add_layer(
                Some(args.player.as_ref()),
                args.world,
                args.position,
                state_id,
                chance,
            );
            args.world.sync_world_event(
                WorldEvent::ComposterFill,
                *args.position,
                i32::from(new_state_id != state_id),
            );
            args.player.award_item_used(args.item_stack.item);
            if !args.player.has_infinite_materials() {
                args.item_stack.decrement(1);
            }
        }
        BlockActionResult::Success
    }

    /// Vanilla `ComposterBlock.onPlace`: reaching level 7 by any route schedules the tick that
    /// makes bone meal ready.
    fn placed(&self, args: PlacedArgs<'_>) {
        if ComposterLikeProperties::from_state_id(args.state_id).level == 7 {
            args.world
                .schedule_block_tick(args.block, *args.position, 20, TickPriority::Normal);
        }
    }

    /// Vanilla `ComposterBlock.tick`: level 7 becomes ready.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        let mut props = ComposterLikeProperties::from_state_id(state_id);
        if props.level == 7 {
            props.level = 8;
            args.world.set_block_state(
                args.position,
                props.to_state_id(args.block),
                BlockFlags::NOTIFY_ALL,
            );
            args.world.play_sound(
                Sound::BlockComposterReady,
                SoundCategory::Blocks,
                &args.position.to_centered_f64(),
            );
        }
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        let props = ComposterLikeProperties::from_state_id(args.state.id);
        Some(props.level)
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

impl ComposterBlock {
    /// The layers vanilla's `compostable/*` provider adds: one at level 0 (its `match_block`
    /// case), else a draw from its 100-weight `weighted_list` on the level random.
    fn layers_to_add(world: &World, level: u8, chance: f32) -> u8 {
        if chance <= 0.0 {
            return 0;
        }
        if level == 0 || chance >= 1.0 {
            return 1;
        }
        let roll = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_bounded_i32(100);
        u8::from(roll < (chance * 100.0).round() as i32)
    }

    /// Vanilla `ComposterBlock.addLayer`. Returns the composter's state afterwards.
    fn add_layer(
        source: Option<&dyn EntityBase>,
        world: &Arc<World>,
        position: &BlockPos,
        state_id: BlockStateId,
        chance: f32,
    ) -> BlockStateId {
        let mut props = ComposterLikeProperties::from_state_id(state_id);
        let layers = Self::layers_to_add(world, props.level, chance);
        if layers == 0 {
            return state_id;
        }
        props.level = (props.level + layers).min(7);
        let new_state_id = props.to_state_id(&Block::COMPOSTER);
        world.set_block_state(position, new_state_id, BlockFlags::NOTIFY_ALL);
        world.game_event(
            GameEvent::BlockChange,
            position.to_centered_f64(),
            GameEventContext::of(source, Some(BlockState::from_id(new_state_id))),
        );
        // `scheduleTick` at level 7 is left to `placed` (vanilla `onPlace` schedules it too).
        new_state_id
    }

    /// Vanilla `ComposterBlock.extractProduce`: one bone meal above the composter, then
    /// `empty` and the empty sound.
    pub fn extract_produce(
        source: Option<&dyn EntityBase>,
        world: &Arc<World>,
        position: &BlockPos,
        state_id: BlockStateId,
    ) {
        // `Vec3.atLowerCornerWithOffset(pos, 0.5, 1.01, 0.5).offsetRandomXZ(random, 0.7F)`.
        let item_position = {
            let mut random = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let dx = f64::from((random.next_f32() - 0.5) * 0.7);
            let dz = f64::from((random.next_f32() - 0.5) * 0.7);
            position.to_f64().add_raw(0.5 + dx, 1.01, 0.5 + dz)
        };
        let item_entity = ItemEntity::new(
            Entity::new(world.clone(), item_position, &EntityType::ITEM),
            ItemStack::new(1, &Item::BONE_MEAL),
        );
        world.spawn_entity(Arc::new(item_entity));

        // `ComposterBlock.empty`.
        let mut props = ComposterLikeProperties::from_state_id(state_id);
        props.level = 0;
        let empty_state_id = props.to_state_id(&Block::COMPOSTER);
        world.set_block_state(position, empty_state_id, BlockFlags::NOTIFY_ALL);
        world.game_event(
            GameEvent::BlockChange,
            position.to_centered_f64(),
            GameEventContext::of(source, Some(BlockState::from_id(empty_state_id))),
        );
        world.play_sound(
            Sound::BlockComposterEmpty,
            SoundCategory::Blocks,
            &position.to_centered_f64(),
        );
    }
}
