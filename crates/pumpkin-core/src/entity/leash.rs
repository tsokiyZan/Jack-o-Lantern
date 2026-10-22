//! The leash half of vanilla `Entity.interact`, and `Entity.shearOffAllLeashConnections`.
//!
//! Shared by every leashable entity (mobs and boats) and every leash holder. Attaching,
//! dropping and finding leashes is `leashable`'s.

use std::sync::Arc;

use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::Hand;

use super::leashable::{
    can_be_leashed, can_have_a_leash_attached_to, drop_leash, leash_holder,
    leashable_held_by_in_area, leashable_leashed_to, set_leashed_to,
};
use super::{Entity, EntityBase};
use crate::entity::player::Player;
use crate::item::interaction_result::InteractionResult;

/// Vanilla `Entity.shearOffAllLeashConnections(player)` through `dropAllLeashConnections`.
///
/// Every leash this entity holds and its own drop, with a `shear` game event and the snip
/// sound. `None` when a plugin cancelled the player's shearing.
pub fn shear_off_all_leash_connections(
    this: &dyn EntityBase,
    player: Option<(&Arc<Player>, Hand)>,
) -> Option<bool> {
    let entity = this.get_entity();
    let world = entity.world.load_full();
    let leashables = leashable_leashed_to(entity);
    let this_leashed = this.is_leashable() && entity.is_leashed();
    // Something to cut: the plugin event may still call the shearing off.
    if (this_leashed || !leashables.is_empty())
        && let Some((player, hand)) = player
        && crate::entity::mob::shear_event_cancelled(player, entity.entity_id, hand)
    {
        return None;
    }
    let dropped = if this_leashed {
        drop_leash(this, true, true);
        true
    } else {
        !leashables.is_empty()
    };
    for leashable in &leashables {
        drop_leash(leashable.as_ref(), true, true);
    }
    if !dropped {
        return Some(false);
    }
    let source = player.map(|(player, _)| player.as_ref() as &dyn EntityBase);
    world.game_event_from(source, GameEvent::Shear, entity.pos.load());
    // `level.playSound(null, blockPosition(), SHEARS_SNIP, player != null ?
    // player.getSoundSource() : this.getSoundSource())`.
    let category = if player.is_some() {
        SoundCategory::Players
    } else {
        this.entity_sound_source()
    };
    world.play_sound(
        Sound::ItemShearsSnip,
        category,
        &entity.block_pos.load().to_centered_f64(),
    );
    Some(true)
}

/// Vanilla `Entity.interact`.
///
/// A sneaking player hands the leashables it leads over to this leashable, shears cut the
/// leashes on and of this entity or shear off a mob's equipment, the holder takes its lead
/// back, and a lead ties up a leashable that isn't held by a player.
pub fn entity_interact(
    this: &Arc<dyn EntityBase>,
    player: &Arc<Player>,
    hand: Hand,
    item_stack: &mut ItemStack,
) -> InteractionResult {
    let entity = this.get_entity();
    let world = entity.world.load_full();
    let player_base: &dyn EntityBase = player.as_ref();
    let sneaking = player.get_entity().is_sneaking();
    let leashable = this.is_leashable();
    if sneaking
        && leashable
        && can_be_leashed(this.as_ref())
        && entity.is_alive()
        && !entity.is_baby()
    {
        let mut any_leashed = false;
        for mob in leashable_held_by_in_area(entity, player.entity_id()) {
            if can_have_a_leash_attached_to(mob.as_ref(), entity) {
                set_leashed_to(mob.as_ref(), this.clone(), true);
                any_leashed = true;
            }
        }
        if any_leashed {
            world.game_event_from(
                Some(player_base),
                GameEvent::EntityAction,
                entity.block_pos.load().to_centered_f64(),
            );
            this.play_sound_at_self(Sound::ItemLeadTied, 1.0, 1.0);
            // `SUCCESS_SERVER.withoutItem()`.
            return InteractionResult::SUCCESS_SERVER;
        }
    }

    if item_stack.item == &Item::SHEARS {
        // A plugin cancelled the cut: the click does nothing more.
        let Some(leash_cut) = shear_off_all_leash_connections(this.as_ref(), Some((player, hand)))
        else {
            return InteractionResult::PASS;
        };
        if leash_cut {
            player.damage_item_in_hand(hand, 1);
            return InteractionResult::SUCCESS;
        }
    }
    if let Some(mob) = this.get_mob()
        && item_stack.item == &Item::SHEARS
        && mob.can_shear_equipment(player)
        && !sneaking
        && crate::entity::mob::attempt_to_shear_equipment(mob, player, hand)
    {
        return InteractionResult::SUCCESS;
    }

    if !(entity.is_alive() && leashable) {
        return InteractionResult::PASS;
    }
    let holder = leash_holder(entity);
    if holder
        .as_ref()
        .is_some_and(|holder| holder.get_entity().entity_id == player.entity_id())
    {
        // `removeLeash` in creative, else `dropLeash`.
        drop_leash(
            this.as_ref(),
            true,
            player.gamemode.load() != pumpkin_util::GameMode::Creative,
        );
        world.game_event_from(
            Some(player_base),
            GameEvent::EntityInteract,
            entity.pos.load(),
        );
        this.play_sound_at_self(Sound::ItemLeadUntied, 1.0, 1.0);
        // `SUCCESS.withoutItem()`.
        return InteractionResult::SUCCESS;
    }
    if item_stack.item != &Item::LEAD
        || holder
            .as_ref()
            .is_some_and(|holder| holder.get_player().is_some())
    {
        return InteractionResult::PASS;
    }
    if !can_have_a_leash_attached_to(this.as_ref(), player.get_entity()) {
        return InteractionResult::PASS;
    }
    if holder.is_some() {
        drop_leash(this.as_ref(), true, true);
    }
    set_leashed_to(this.as_ref(), player.clone(), true);
    this.play_sound_at_self(Sound::ItemLeadTied, 1.0, 1.0);
    item_stack.decrement_unless_creative(player.gamemode.load(), 1);
    InteractionResult::SUCCESS_SERVER
}

/// `entity_interact` for the entity `entity`, looked up as the `Arc` the leashes hold.
pub fn entity_interact_by_id(
    entity: &Entity,
    player: &Arc<Player>,
    hand: Hand,
    item_stack: &mut ItemStack,
) -> InteractionResult {
    let Some(this) = entity.world.load().get_entity_by_id(entity.entity_id) else {
        return InteractionResult::PASS;
    };
    entity_interact(&this, player, hand, item_stack)
}
