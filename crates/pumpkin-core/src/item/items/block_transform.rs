use crate::block::blocks::chests::get_connected_direction;
use crate::block::registry::BlockActionResult;
use crate::enchantment::HurtAndBreak;
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::world::game_event::GameEventContext;
use pumpkin_data::block_transformer::{BlockTransformer, DropStrategy, TransformType};
use pumpkin_data::data_component_impl::BlocksAttacksImpl;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockDirection, BlockState, tag};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_util::Hand;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;

/// Vanilla `BlockTransformer.transformBlock`, shared by axes, hoes and shovels.
pub fn transform_block(
    transformer: &BlockTransformer,
    item: &mut ItemStack,
    player: &Player,
    hand: Hand,
    location: BlockPos,
    face: BlockDirection,
    block: &Block,
) -> BlockActionResult {
    if player_has_blocking_item_use_intent(player, hand) {
        return BlockActionResult::Pass;
    }

    let world = player.world();
    let get_block = |dx: i8, dy: i8, dz: i8| {
        let check_pos = BlockPos(location.0 + Vector3::new(dx as i32, dy as i32, dz as i32));
        world.get_block(&check_pos)
    };

    let old_state_id = world.get_block_state_id(&location);
    let Some(result) = transformer.transform(block, old_state_id, face, &get_block) else {
        return BlockActionResult::Pass;
    };

    if let Some(loot_key) = result.entry.loot
        && let Some(loot_table) = world.get_loot_table(loot_key)
    {
        // Vanilla `Block.dropFromBlockInteractLootTable`: `getRandomItems(params)`.
        let drops = crate::world::loot::generate_loot_from_handle(
            &loot_table,
            &world,
            0,
            &crate::world::loot::LootContextParameters::default(),
        );
        for drop_stack in drops {
            if result.entry.drop_strategy == Some(DropStrategy::ClickedFace) {
                world.drop_stack_from_face(&location, face, drop_stack);
            } else {
                world.drop_stack(&location, drop_stack);
            }
        }
    }

    if !player.has_infinite_materials() {
        let _ = item.hurt_and_break(&world, i32::from(result.entry.item_damage_per_use));
    }

    let new_state_id = if result.entry.update_from_neighbors {
        world.update_from_neighbor_shapes(result.new_state_id, &location)
    } else {
        result.new_state_id
    };
    // The other half of a door or double copper chest follows through its updateShape.
    world.set_block_state(&location, new_state_id, BlockFlags::NOTIFY_ALL);
    // The player predicts its own sound and particles.
    if let Some(sound) = result.entry.sound {
        world.play_block_sound_expect(player, sound, SoundCategory::Blocks, location);
    }
    if let Some(particle) = result.entry.particle {
        world.sync_world_event_from(player, particle, location, 0);
    }
    world.game_event(
        GameEvent::BlockChange,
        location.to_centered_f64(),
        GameEventContext::of(Some(player), Some(BlockState::from_id(new_state_id))),
    );

    if result.entry.transform_type == Some(TransformType::CopperChest)
        && block.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS)
        && let Some(direction) = get_connected_direction(old_state_id)
    {
        let neighbor_pos = location.offset(direction.to_offset());
        world.game_event(
            GameEvent::BlockChange,
            neighbor_pos.to_centered_f64(),
            GameEventContext::of(Some(player), Some(world.get_block_state(&neighbor_pos))),
        );
        if let Some(particle) = result.entry.particle {
            world.sync_world_event_from(player, particle, neighbor_pos, 0);
        }
    }

    BlockActionResult::Success
}

/// Vanilla `BlockTransformer.playerHasBlockingItemUseIntent`.
fn player_has_blocking_item_use_intent(player: &Player, hand: Hand) -> bool {
    hand == Hand::Right
        && player
            .inventory
            .off_hand_item()
            .get_data_component::<BlocksAttacksImpl>()
            .is_some()
        && !player.get_entity().is_sneaking()
}
