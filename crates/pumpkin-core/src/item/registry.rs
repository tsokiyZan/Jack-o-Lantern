use crate::block::registry::BlockActionResult;
use crate::entity::EntityBase;
use crate::entity::player::{Player, cooldown_group};
use crate::item::interaction_result::InteractionResult;
use crate::server::Server;
use pumpkin_data::Block;
use pumpkin_data::BlockDirection;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::Hand;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rustc_hash::FxHashMap;
use std::sync::Arc;

use super::{ItemBehaviour, ItemMetadata, default_use};
use pumpkin_data::data_component_impl::KineticWeaponImpl;
use pumpkin_inventory::screen_handler::ScreenHandler;

pub(crate) const fn should_try_block_placement(result: &BlockActionResult) -> bool {
    matches!(result, BlockActionResult::Pass)
}

#[derive(Default)]
pub struct ItemRegistry {
    items: FxHashMap<u16, Arc<dyn ItemBehaviour>>,
}

impl ItemRegistry {
    pub fn register<T: ItemBehaviour + ItemMetadata + 'static>(&mut self, item: T) {
        let val = Arc::new(item);
        self.items.reserve(T::ids().len());
        for i in T::ids() {
            self.items.insert(i, val.clone());
        }
    }

    /// Vanilla `ServerPlayerGameMode.useItem` once its spectator and cooldown checks passed:
    /// `ItemStack.use` on `stack`, the stack in `hand`, then, unless the use left the hand's
    /// stack as it was or failed to start a timed use, the whole inventory sent again when no
    /// item is in use.
    pub fn use_item(&self, stack: &ItemStack, player: &Player, hand: Hand) -> InteractionResult {
        let (old_count, old_damage) = (stack.item_count, stack.get_damage());
        let result = self.on_use(stack, player, hand);
        let result_stack = player.inventory.get_stack_in_hand(hand);
        let use_duration = self.stack_use_duration(&result_stack);
        // `resultStack == itemStack`: the same stack, which the item kind stands in for.
        if result_stack.item.id == stack.item.id
            && result_stack.item_count == old_count
            && use_duration <= 0
            && result_stack.get_damage() == old_damage
        {
            return result;
        }
        let using_item = player
            .living_entity
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some();
        if result == InteractionResult::FAIL && use_duration > 0 && !using_item {
            return result;
        }
        if !using_item {
            // `inventoryMenu.sendAllDataToRemote`.
            player
                .player_screen_handler
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .sync_state();
        }
        result
    }

    /// Vanilla `ItemStack.getUseDuration`: the item's own duration, else a consumable's, else
    /// 72000 for a shield or a kinetic weapon.
    #[must_use]
    pub fn stack_use_duration(&self, stack: &ItemStack) -> i32 {
        if let Some(duration) = self.get_use_duration(stack.item.id) {
            return duration;
        }
        if stack.get_data_component::<KineticWeaponImpl>().is_some() {
            return 72000;
        }
        stack.get_max_use_time()
    }

    /// Vanilla `ItemStack.use`: the item's `use`, then, for an item used at once (no use
    /// duration) that succeeded, its `use_cooldown`.
    pub fn on_use(&self, stack: &ItemStack, player: &Player, hand: Hand) -> InteractionResult {
        let item = stack.item;
        let cooldown = stack.get_use_cooldown();
        let cooldown_group = cooldown_group(stack);

        if player.is_on_cooldown(&cooldown_group) {
            return InteractionResult::PASS;
        }

        let pumpkin_item = self.get_pumpkin_item(item.id);
        let instantly_used = self.stack_use_duration(stack) <= 0;
        let result = pumpkin_item.map_or_else(
            || default_use(player, hand),
            |pumpkin_item| pumpkin_item.normal_use(item, player, hand),
        );

        // `applyAfterUseComponentSideEffects`.
        // TODO: `use_remainder`.
        if instantly_used
            && result.consumes_action()
            && let Some(cooldown) = cooldown
        {
            player.start_cooldown(cooldown_group, (cooldown.seconds * 20.0) as i32);
        }
        result
    }

    pub fn on_stopped_using(&self, stack: &ItemStack, player: &Player) {
        if let Some(behaviour) = self.get_pumpkin_item(stack.item.id) {
            behaviour.on_stopped_using(stack, player);
        }
    }

    pub fn on_spear_jab(&self, stack: &ItemStack, player: &Player) {
        if let Some(behaviour) = self.get_pumpkin_item(stack.item.id) {
            behaviour.on_spear_jab(stack, player);
        }
    }

    pub fn on_use_tick(&self, stack: &ItemStack, player: &Player, remaining_use_ticks: i32) {
        if let Some(behaviour) = self.get_pumpkin_item(stack.item.id) {
            behaviour.on_use_tick(stack, player, remaining_use_ticks);
        }
    }

    /// Returns the item's use duration in ticks, as defined by its registered behaviour.
    /// Returns `None` if the item has no registered behaviour or its duration is 0.
    #[must_use]
    pub fn get_use_duration(&self, item_id: u16) -> Option<i32> {
        self.get_pumpkin_item(item_id)
            .map(|b| b.get_use_duration())
            .filter(|&d| d > 0)
    }

    #[expect(clippy::too_many_arguments)]
    pub fn use_on_block(
        &self,
        stack: &mut ItemStack,
        player: &Player,
        location: BlockPos,
        face: BlockDirection,
        cursor_pos: Vector3<f32>,
        block: &Block,
        server: &Server,
    ) -> BlockActionResult {
        let cooldown = stack.get_use_cooldown().cloned();
        let cooldown_group = cooldown_group(stack);

        if player.is_on_cooldown(&cooldown_group) {
            return BlockActionResult::Pass;
        }

        let pumpkin_item = self.get_pumpkin_item(stack.item.id);
        let result = pumpkin_item.map_or(BlockActionResult::Pass, |pumpkin_item| {
            pumpkin_item.use_on_block(stack, player, location, face, cursor_pos, block, server)
        });

        if let Some(cooldown) = cooldown {
            player.start_cooldown(cooldown_group, (cooldown.seconds * 20.0) as i32);
        }

        result
    }

    /// Vanilla `ItemStack.interactLivingEntity`: uses `stack`, the stack in `hand`, on `entity`.
    /// Unlike a plain use, this starts no item cooldown and ignores a running one.
    pub fn use_on_entity(
        &self,
        stack: &mut ItemStack,
        player: &Player,
        hand: Hand,
        entity: Arc<dyn EntityBase>,
    ) -> InteractionResult {
        // `Equippable.equipOnInteract`: `equipOnTarget` first, for any stack.
        if entity
            .get_mob()
            .is_some_and(|mob| mob.equip_on_target(player, stack))
        {
            return InteractionResult::SUCCESS;
        }
        self.get_pumpkin_item(stack.item.id)
            .map_or(InteractionResult::PASS, |pumpkin_item| {
                pumpkin_item.use_on_entity(stack, player, hand, entity)
            })
    }

    pub fn can_mine(&self, item: &Item, player: &Player) -> bool {
        let pumpkin_block = self.get_pumpkin_item(item.id);
        if let Some(pumpkin_block) = pumpkin_block {
            return pumpkin_block.can_mine(player);
        }
        true
    }

    #[must_use]
    pub fn get_pumpkin_item(&self, item: u16) -> Option<&Arc<dyn ItemBehaviour>> {
        self.items.get(&item)
    }
}

#[cfg(test)]
mod tests {
    use super::should_try_block_placement;
    use crate::block::registry::BlockActionResult;

    #[test]
    fn block_placement_only_follows_pass() {
        assert!(should_try_block_placement(&BlockActionResult::Pass));

        for result in [
            BlockActionResult::Success,
            BlockActionResult::SuccessServer,
            BlockActionResult::Consume,
            BlockActionResult::Fail,
            BlockActionResult::PassToDefaultBlockAction,
        ] {
            assert!(!should_try_block_placement(&result));
        }
    }
}
