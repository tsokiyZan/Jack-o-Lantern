pub mod interaction_result;
pub mod items;
pub mod potion;
pub mod registry;

use std::any::Any;
use std::sync::Arc;

use crate::block::registry::BlockActionResult;
use crate::entity::EntityBase;
use crate::entity::mob::has_prevent_armor_change;
use crate::entity::player::Player;
use crate::entity::player::statistics::StatisticCategory;
use crate::item::interaction_result::InteractionResult;
use crate::item::items::mace::MaceItem;
use crate::server::Server;
use pumpkin_data::Block;
use pumpkin_data::BlockDirection;
use pumpkin_data::data_component_impl::{
    BlocksAttacksImpl, ConsumableImpl, EquipmentSlot, EquippableImpl, FoodImpl, IDSet,
    KineticWeaponImpl, WeaponImpl,
};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::tag::Taggable;
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_util::GameMode;
use pumpkin_util::Hand;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

/// Vanilla `ItemStack.hurtEnemy`: the item's own `hurtEnemy` (only the mace has one), then the
/// player's `ITEM_USED` stat for a weapon. Returns whether the stack is a weapon.
pub fn item_stack_hurt_enemy(
    stack: &ItemStack,
    victim: &dyn EntityBase,
    attacker: &dyn EntityBase,
) -> bool {
    if stack.item.id == Item::MACE.id {
        MaceItem::hurt_enemy(victim, attacker);
    }
    if stack.get_data_component::<WeaponImpl>().is_none() {
        return false;
    }
    if let Some(player) = attacker.get_player() {
        player.increment_stat(StatisticCategory::Used, stack.item.id as i32, 1);
    }
    true
}

/// Vanilla `ItemUtils.createFilledResult`: `stack` becomes what the hand holds once one of it
/// was filled into `new_stack`.
pub fn create_filled_result(
    stack: &mut ItemStack,
    player: &Player,
    mut new_stack: ItemStack,
    limit_creative_stack_size: bool,
) {
    if limit_creative_stack_size && player.has_infinite_materials() {
        if !player.inventory.contains_item(new_stack.item) {
            player.inventory.insert_stack_anywhere(&mut new_stack);
        }
        return;
    }
    stack.decrement_unless_creative(player.gamemode.load(), 1);
    if stack.is_empty() {
        *stack = new_stack;
    } else if !player.inventory.insert_stack_anywhere(&mut new_stack) {
        player.drop_from_hand(new_stack, false, false);
    }
}

/// Vanilla `itemStack.consume(1, player)` on the stack in `hand`: one fewer, unless the player
/// has infinite materials.
pub fn consume_in_hand(player: &Player, hand: Hand) {
    let mut stack = player.inventory.get_stack_in_hand(hand);
    stack.decrement_unless_creative(player.gamemode.load(), 1);
    player.inventory.set_stack_in_hand(hand, stack);
}

/// Vanilla's base `Item.use`, for items with no `use` of their own.
///
/// A consumable starts being consumed, a swappable equippable swaps into its slot, a shield or
/// a kinetic weapon starts being used; anything else passes.
pub fn default_use(player: &Player, hand: Hand) -> InteractionResult {
    let stack = player.inventory.get_stack_in_hand(hand);
    if stack.get_data_component::<ConsumableImpl>().is_some() {
        // `Consumable.startConsuming`; `canConsume` is the food check.
        if let Some(food) = stack.get_data_component::<FoodImpl>()
            && !player.can_eat(food.can_always_eat)
        {
            return InteractionResult::FAIL;
        }
        // TODO: an instant consumable (`consumeTicks` 0) is consumed here, not started.
        let duration = stack.get_max_use_time();
        start_using_item(player, hand, stack, duration);
        return InteractionResult::CONSUME;
    }
    if let Some(equippable) = stack
        .get_data_component::<EquippableImpl>()
        .filter(|equippable| equippable.swappable)
    {
        let (slot, allowed) = (equippable.slot, equippable.allowed_entities.clone());
        return swap_with_equipment_slot(player, hand, stack, slot, allowed.as_ref());
    }
    if stack.get_data_component::<BlocksAttacksImpl>().is_some() {
        start_using_item(player, hand, stack, 72000);
        return InteractionResult::CONSUME;
    }
    if let Some(kinetic) = stack.get_data_component::<KineticWeaponImpl>() {
        let sound = kinetic.sound.clone();
        start_using_item(player, hand, stack, 72000);
        // `KineticWeapon.makeSound`.
        if let Some(sound) = sound {
            player.world().play_sound_event_expect(
                player,
                &sound,
                SoundCategory::Players,
                &player.position(),
            );
        }
        return InteractionResult::CONSUME;
    }
    InteractionResult::PASS
}

/// Vanilla `LivingEntity.startUsingItem`: only a non-empty stack, and only when nothing is in
/// use yet.
pub fn start_using_item(player: &Player, hand: Hand, stack: ItemStack, duration: i32) {
    let using = player
        .living_entity
        .item_in_use
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some();
    if stack.is_empty() || using {
        return;
    }
    player.living_entity.set_active_hand(hand, stack, duration);
}

/// Vanilla `Equippable.swapWithEquipmentSlot` for the stack in `hand`.
fn swap_with_equipment_slot(
    player: &Player,
    hand: Hand,
    in_hand: ItemStack,
    slot: &EquipmentSlot,
    allowed: Option<&IDSet<EntityType>>,
) -> InteractionResult {
    // `player.canUseSlot` is always true for a player.
    let allowed = match allowed {
        None => true,
        Some(IDSet::Tag(tag)) => EntityType::PLAYER.is_tagged_with(tag).unwrap_or(false),
        Some(IDSet::IDs(types)) => types.contains(&&EntityType::PLAYER),
    };
    if !allowed {
        return InteractionResult::PASS;
    }
    let inventory = player.inventory();
    let in_slot = inventory
        .entity_equipment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(slot);
    let creative = player.gamemode.load() == GameMode::Creative;
    if (has_prevent_armor_change(&in_slot) && !creative)
        || in_hand.are_items_and_components_equal(&in_slot)
    {
        return InteractionResult::FAIL;
    }
    player.award_item_used(in_hand.item);
    // `setItemSlot`'s equip sound; tracking clients see the stack at the next equipment check.
    player.enqueue_equipment_change(slot, &in_hand);
    if in_hand.item_count <= 1 {
        let to_hand = if in_slot.is_empty() {
            if creative {
                in_hand.clone()
            } else {
                ItemStack::EMPTY.clone()
            }
        } else {
            in_slot
        };
        put_equipment(player, slot, in_hand);
        inventory.set_stack_in_hand(hand, to_hand);
        return InteractionResult::SUCCESS;
    }
    let mut rest = in_hand.clone();
    rest.decrement_unless_creative(player.gamemode.load(), 1);
    put_equipment(player, slot, in_hand.copy_with_count(1));
    inventory.set_stack_in_hand(hand, rest);
    let mut to_inventory = in_slot;
    if !to_inventory.is_empty() && !inventory.insert_stack_anywhere(&mut to_inventory) {
        player.drop_from_hand(to_inventory, false, false);
    }
    InteractionResult::SUCCESS
}

fn put_equipment(player: &Player, slot: &EquipmentSlot, stack: ItemStack) {
    player
        .inventory()
        .entity_equipment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .put(slot, stack);
}

pub trait ItemMetadata {
    fn ids() -> Box<[u16]>;
}

pub trait ItemBehaviour: Send + Sync {
    /// Vanilla `Item.use`: right-clicking with the stack in `hand` when the click reached no
    /// block or entity. The player's rotation is already the one the click was made with
    /// (`handleUseItem` snaps it). Defaults to vanilla's base `Item.use` ([`default_use`]).
    fn normal_use(&self, _item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        default_use(player, hand)
    }

    #[expect(clippy::too_many_arguments)]
    fn use_on_block(
        &self,
        _item: &mut ItemStack,
        _player: &Player,
        _hand: pumpkin_util::Hand,
        _location: BlockPos,
        _face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        _block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        BlockActionResult::Pass
    }

    /// Vanilla `Item.interactLivingEntity`: using the item in `hand` on `entity`.
    fn use_on_entity(
        &self,
        _item: &mut ItemStack,
        _player: &Player,
        _hand: Hand,
        _entity: Arc<dyn EntityBase>,
    ) -> InteractionResult {
        InteractionResult::PASS
    }

    fn on_stopped_using(&self, _stack: &ItemStack, _player: &Player) {}

    fn on_spear_jab(&self, _stack: &ItemStack, _player: &Player) {}

    fn on_use_tick(&self, _stack: &ItemStack, _player: &Player, _remaining_use_ticks: i32) {}

    /// Returns the maximum number of ticks this item can be used for.
    /// Return 0 if the item does not have a behaviour-driven use duration.
    fn get_use_duration(&self, _stack: &ItemStack) -> i32 {
        0
    }

    fn can_mine(&self, _player: &Player) -> bool {
        true
    }

    fn get_start_and_end_pos(&self, player: &Player) -> (Vector3<f64>, Vector3<f64>) {
        let start_pos = player.eye_position();
        let (yaw, pitch) = player.rotation();
        let (yaw_rad, pitch_rad) = (f64::from(yaw.to_radians()), f64::from(pitch.to_radians()));
        let block_interaction_range = 4.5; // This is not the same as the block_interaction_range in the
        // player entity.
        let direction = Vector3::new(
            -yaw_rad.sin() * pitch_rad.cos() * block_interaction_range,
            -pitch_rad.sin() * block_interaction_range,
            pitch_rad.cos() * yaw_rad.cos() * block_interaction_range,
        );

        let end_pos = start_pos.add(&direction);
        (start_pos, end_pos)
    }

    fn as_any(&self) -> &dyn Any;
}
