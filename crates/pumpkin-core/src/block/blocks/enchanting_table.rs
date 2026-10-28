use std::sync::Arc;
use std::sync::Mutex;

use crate::block::entities::enchanting_table::EnchantingTableBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, GetScreenHandlerFactoryArgs, NormalUseArgs, PathComputationType, PlacedArgs,
};
use pumpkin_data::{
    BlockState,
    tag::{self, Taggable},
    translation,
};
use pumpkin_inventory::enchanting::enchanting_screen_handler::EnchantingTableScreenHandler;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_inventory::screen_handler::{
    InventoryPlayer, ScreenHandlerFactory, SharedScreenHandler,
};
use pumpkin_inventory::{Inventory, SimpleInventory};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;

#[pumpkin_block("minecraft:enchanting_table")]
pub struct EnchantingTableBlock;

impl BlockBehaviour for EnchantingTableBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.is_new_block() {
            return;
        }
        let entity = EnchantingTableBlockEntity::new(*args.position);
        args.world.add_block_entity(Arc::new(entity));
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        if let Some(factory) = self.get_screen_handler_factory(GetScreenHandlerFactoryArgs {
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

    fn get_screen_handler_factory(
        &self,
        args: GetScreenHandlerFactoryArgs<'_>,
    ) -> Option<Box<dyn ScreenHandlerFactory>> {
        // Vanilla `EnchantmentMenu.slotsChanged`: every valid `BOOKSHELF_OFFSETS` entry.
        let bookshelf_count = bookshelf_offsets()
            .filter(|offset| Self::is_valid_bookshelf(args.world, args.position, offset))
            .count();
        let bookshelf_count = bookshelf_count.min(15) as i32;

        let seed = args.player.enchantment_seed();
        Some(Box::new(EnchantingTableScreenFactory {
            bookshelf_count,
            seed,
        }))
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

/// Vanilla `EnchantingTableBlock.BOOKSHELF_OFFSETS`: the ring two blocks out, `y` 0 and 1, in
/// `BlockPos.betweenClosed` order.
fn bookshelf_offsets() -> impl Iterator<Item = Vector3<i32>> {
    (-2..=2).flat_map(|z: i32| {
        (0..=1).flat_map(move |y| {
            (-2..=2)
                .filter(move |x: &i32| x.abs() == 2 || z.abs() == 2)
                .map(move |x| Vector3::new(x, y, z))
        })
    })
}

impl EnchantingTableBlock {
    /// Vanilla `EnchantingTableBlock.isValidBookShelf`: a power provider at the offset with a
    /// power transmitter halfway to it.
    fn is_valid_bookshelf(
        world: &Arc<crate::world::World>,
        pos: &BlockPos,
        offset: &Vector3<i32>,
    ) -> bool {
        world
            .get_block(&pos.add(offset.x, offset.y, offset.z))
            .has_tag(&tag::Block::MINECRAFT_ENCHANTMENT_POWER_PROVIDER)
            && world
                .get_block(&pos.add(offset.x / 2, offset.y, offset.z / 2))
                .has_tag(&tag::Block::MINECRAFT_ENCHANTMENT_POWER_TRANSMITTER)
    }
}

struct EnchantingTableScreenFactory {
    bookshelf_count: i32,
    seed: i32,
}

impl ScreenHandlerFactory for EnchantingTableScreenFactory {
    fn create_screen_handler(
        &self,
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        _player: &dyn InventoryPlayer,
    ) -> Option<SharedScreenHandler> {
        let inventory: Arc<dyn Inventory> = Arc::new(SimpleInventory::new(2));
        let handler = EnchantingTableScreenHandler::new(
            sync_id,
            player_inventory,
            &inventory,
            self.seed,
            self.bookshelf_count,
        );
        let screen_handler_arc = Arc::new(Mutex::new(handler));
        Some(screen_handler_arc as SharedScreenHandler)
    }

    fn get_display_name(&self) -> TextComponent {
        pumpkin_macros::translate_cross!(
            translation::java::CONTAINER_ENCHANT,
            translation::bedrock::CONTAINER_ENCHANT
        )
    }
}
