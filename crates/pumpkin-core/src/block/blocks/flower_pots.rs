use crate::block::blocks::plant::eyeblossom::{
    CLOSED_PARTICLE_COLOR, OPEN_PARTICLE_COLOR, spawn_transform_particle,
};
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, NormalUseArgs, PathComputationType, RandomTickArgs, UseWithItemArgs,
};
use crate::entity::EntityBase;
use pumpkin_data::flower_pot_transformations::{get_potted_content, get_potted_item};
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, BlockId, BlockState};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_world::world::BlockFlags;

#[pumpkin_block_from_tag("minecraft:flower_pots")]
pub struct FlowerPotBlock;

impl BlockBehaviour for FlowerPotBlock {
    /// Vanilla `FlowerPotBlock.useItemOn`: a pottable plant goes into an empty pot; anything
    /// else tries the empty hand.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        let item = args.item_stack.item;
        let potted_block_id = get_potted_item(item.id);
        if potted_block_id == BlockId::AIR {
            return BlockActionResult::PassToDefaultBlockAction;
        }
        if args.block != &Block::FLOWER_POT {
            return BlockActionResult::Consume;
        }

        args.world.set_block_state(
            args.position,
            Block::from_id(potted_block_id).default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        let source: &dyn EntityBase = args.player.as_ref();
        args.world.game_event_from(
            Some(source),
            GameEvent::BlockChange,
            args.position.to_centered_f64(),
        );
        args.player.increment_stat(
            pumpkin_data::statistic::StatisticCategory::Custom,
            pumpkin_data::statistic::CustomStatistic::PotFlower as i32,
            1,
        );
        if !args.player.has_infinite_materials() {
            args.item_stack.decrement(1);
        }
        BlockActionResult::Success
    }

    /// Vanilla `FlowerPotBlock.useWithoutItem`: the plant goes back to the player, or drops,
    /// and the pot is left empty.
    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        let Some(plant) = get_potted_content(args.block.id.as_u16()).and_then(Item::from_id) else {
            return BlockActionResult::Consume;
        };

        // `Player.addItem`: a player with infinite materials keeps nothing back to drop.
        let mut stack = ItemStack::new(1, plant);
        if !args.player.inventory().insert_stack_anywhere(&mut stack)
            && !args.player.has_infinite_materials()
        {
            args.player.drop_from_hand(stack, false, false);
        }

        args.world.set_block_state(
            args.position,
            Block::FLOWER_POT.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        let source: &dyn EntityBase = args.player.as_ref();
        args.world.game_event_from(
            Some(source),
            GameEvent::BlockChange,
            args.position.to_centered_f64(),
        );
        BlockActionResult::Success
    }

    /// Vanilla `FlowerPotBlock.randomTick`: a potted eyeblossom follows `EYEBLOSSOM_OPEN`.
    fn random_tick(&self, args: RandomTickArgs<'_>) {
        let is_open_potted = args.block.eq(&Block::POTTED_OPEN_EYEBLOSSOM);
        let is_closed_potted = args.block.eq(&Block::POTTED_CLOSED_EYEBLOSSOM);
        if !is_open_potted && !is_closed_potted {
            return;
        }

        let is_open = is_open_potted;
        let should_be_open = args.world.eyeblossom_open(args.position).unwrap_or(is_open);

        if is_open != should_be_open {
            // `Type.fromBoolean(isOpen).transform()`: the type it turns into.
            let (next_block, particle_color, sound) = if is_open {
                (
                    &Block::POTTED_CLOSED_EYEBLOSSOM,
                    CLOSED_PARTICLE_COLOR,
                    Sound::BlockEyeblossomCloseLong,
                )
            } else {
                (
                    &Block::POTTED_OPEN_EYEBLOSSOM,
                    OPEN_PARTICLE_COLOR,
                    Sound::BlockEyeblossomOpenLong,
                )
            };
            args.world.set_block_state(
                args.position,
                next_block.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
            spawn_transform_particle(args.world, args.position, particle_color);
            args.world.play_sound(
                sound,
                SoundCategory::Blocks,
                &args.position.to_centered_f64(),
            );
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
