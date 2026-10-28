use crate::block::blocks::chests::get_connected_direction;
use crate::block::registry::BlockActionResult;
use crate::enchantment::HurtAndBreak;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::block_transformer::AXE;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockDirection, tag};
use pumpkin_util::GameMode;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;

pub struct AxeItem;

impl ItemMetadata for AxeItem {
    fn ids() -> Box<[u16]> {
        tag::Item::MINECRAFT_AXES.1.into()
    }
}

impl ItemBehaviour for AxeItem {
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        _hand: pumpkin_util::Hand,
        location: BlockPos,
        face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        let world = player.world();
        let get_block = |dx: i8, dy: i8, dz: i8| {
            let check_pos = BlockPos(location.0 + Vector3::new(dx as i32, dy as i32, dz as i32));
            world.get_block(&check_pos)
        };

        let current_state_id = world.get_block_state_id(&location);

        if let Some(result) = AXE.transform(block, current_state_id, face, &get_block) {
            // Vanilla `BlockTransformer`: the player predicts its own sound and particles.
            if let Some(sound) = result.entry.sound {
                world.play_block_sound_expect(player, sound, SoundCategory::Blocks, location);
            }
            if let Some(particle) = result.entry.particle {
                world.sync_world_event_from(player, particle, location, 0);
                if block.has_tag(&tag::Block::MINECRAFT_COPPER_CHESTS)
                    && let Some(direction) = get_connected_direction(current_state_id)
                {
                    world.sync_world_event_from(
                        player,
                        particle,
                        location.offset(direction.to_offset()),
                        0,
                    );
                }
            }

            // The other half of a door or double copper chest follows through its updateShape.
            world.set_block_state(&location, result.new_state_id, BlockFlags::NOTIFY_ALL);

            if player.gamemode.load() != GameMode::Creative {
                let _ = item.hurt_and_break(&world, i32::from(result.entry.item_damage_per_use));
            }
            return BlockActionResult::Success;
        }
        BlockActionResult::Pass
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
