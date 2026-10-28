use std::sync::Arc;

use pumpkin_data::BlockStateId;
use pumpkin_data::block_properties::{LadderLikeProperties, VaultLikeProperties, VaultState};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_macros::pumpkin_block;
use pumpkin_world::world::BlockFlags;

use crate::block::entities::vault::VaultBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::block::{BlockBehaviour, OnPlaceArgs, PlacedArgs, UseWithItemArgs};

#[pumpkin_block("minecraft:vault")]
pub struct VaultBlock;

impl BlockBehaviour for VaultBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = LadderLikeProperties::default(args.block);
        props.facing = args
            .player
            .living_entity
            .entity
            .get_horizontal_facing()
            .opposite();
        props.to_state_id(args.block)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        if !args.is_new_block() {
            return;
        }
        {
            let entity = VaultBlockEntity::new(*args.position);
            args.world.add_block_entity(Arc::new(entity));
        }
    }

    /// Vanilla `VaultBlock.useItemOn`: an item on an active vault tries the key
    /// (`VaultBlockEntity.Server.tryInsertKey`); anything else tries the empty hand.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        let mut props = VaultLikeProperties::from_state_id(state_id);
        if args.item_stack.is_empty() || props.vault_state != VaultState::Active {
            return BlockActionResult::PassToDefaultBlockAction;
        }
        let Some(block_entity) = args.world.get_block_entity(args.position) else {
            return BlockActionResult::PassToDefaultBlockAction;
        };
        let Some(vault_entity) = block_entity.as_any().downcast_ref::<VaultBlockEntity>() else {
            return BlockActionResult::PassToDefaultBlockAction;
        };

        // TODO: the vault's `VaultConfig` key and loot table; an ominous vault's default config
        // takes the ominous key.
        let key = if props.ominous {
            &Item::OMINOUS_TRIAL_KEY
        } else {
            &Item::TRIAL_KEY
        };
        let center = args.position.to_centered_f64();
        if args.item_stack.item != key {
            args.world.play_sound(
                Sound::BlockVaultInsertItemFail,
                SoundCategory::Blocks,
                &center,
            );
            return BlockActionResult::SuccessServer;
        }

        let player_uuid = args.player.gameprofile.id;
        if vault_entity.has_rewarded(&player_uuid) {
            args.world.play_sound(
                Sound::BlockVaultRejectRewardedPlayer,
                SoundCategory::Blocks,
                &center,
            );
            return BlockActionResult::SuccessServer;
        }

        args.player.award_item_used(args.item_stack.item);
        if !args.player.has_infinite_materials() {
            args.item_stack.decrement(1);
        }
        vault_entity.mark_rewarded(player_uuid);

        args.world
            .play_sound(Sound::BlockVaultInsertItem, SoundCategory::Blocks, &center);
        args.world
            .play_sound(Sound::BlockVaultOpenShutter, SoundCategory::Blocks, &center);

        props.vault_state = VaultState::Ejecting;
        args.world.set_block_state(
            args.position,
            props.to_state_id(args.block),
            BlockFlags::NOTIFY_ALL,
        );

        args.world
            .play_sound(Sound::BlockVaultEjectItem, SoundCategory::Blocks, &center);

        // TODO: the config's loot table; a fixed stand-in for the trial vault's loot.
        let loot_stacks = [
            ItemStack::new(4, &Item::EMERALD),
            ItemStack::new(1, &Item::DIAMOND),
            ItemStack::new(2, &Item::IRON_INGOT),
        ];
        for stack in loot_stacks {
            args.world.drop_stack(args.position, stack);
        }

        props.vault_state = VaultState::Active;
        args.world.set_block_state(
            args.position,
            props.to_state_id(args.block),
            BlockFlags::NOTIFY_ALL,
        );

        BlockActionResult::SuccessServer
    }
}
