//! Vanilla `Leashable` and the leash half of `Entity.interact`, shared by every leashable
//! entity (mobs and boats) and every leash holder.

use std::sync::Arc;

use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::Hand;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::vector3::Vector3;

use super::{Entity, EntityBase};
use crate::entity::player::Player;
use crate::item::interaction_result::InteractionResult;
use crate::world::World;

/// The entity that holds `entity`'s leash, if any.
#[must_use]
pub fn leash_holder(entity: &Entity) -> Option<Arc<dyn EntityBase>> {
    entity
        .leashed_to
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Vanilla `Leashable.leashableInArea(level, pos, test)`: the leashable entities in the
/// 32-block box around `center` that pass `test`.
pub fn leashable_in_area(
    world: &World,
    center: Vector3<f64>,
    test: impl Fn(&dyn EntityBase) -> bool,
) -> Vec<Arc<dyn EntityBase>> {
    let area = BoundingBox::new(
        Vector3::new(center.x - 16.0, center.y - 16.0, center.z - 16.0),
        Vector3::new(center.x + 16.0, center.y + 16.0, center.z + 16.0),
    );
    world
        .get_entities_at_box(&area)
        .into_iter()
        .filter(|candidate| candidate.is_leashable() && test(candidate.as_ref()))
        .collect()
}

/// Vanilla `Leashable.leashableInArea(entity, l -> l.getLeashHolder() == holder)`: around the
/// centre of `entity`'s box, the leashables held by the entity with id `holder_id`.
pub fn leashable_held_by_in_area(
    world: &World,
    entity: &Entity,
    holder_id: i32,
) -> Vec<Arc<dyn EntityBase>> {
    leashable_in_area(
        world,
        entity.bounding_box.load().get_center(),
        |candidate| {
            leash_holder(candidate.get_entity())
                .is_some_and(|holder| holder.get_entity().entity_id == holder_id)
        },
    )
}

/// Vanilla `Leashable.leashableLeashedTo(entity)`.
pub fn leashable_leashed_to(world: &World, entity: &Entity) -> Vec<Arc<dyn EntityBase>> {
    leashable_held_by_in_area(world, entity, entity.entity_id)
}

/// Vanilla `Leashable.canBeLeashed`: `Mob`'s own answer, `true` for the other leashables.
#[must_use]
pub fn can_be_leashed(leashable: &dyn EntityBase) -> bool {
    leashable
        .get_mob()
        .is_none_or(crate::entity::mob::Mob::can_be_leashed)
}

/// Vanilla `Leashable.canHaveALeashAttachedTo`: not itself, box centres within the snap
/// distance, and `canBeLeashed`.
#[must_use]
pub fn can_have_a_leash_attached_to(leashable: &dyn EntityBase, holder: &Entity) -> bool {
    let entity = leashable.get_entity();
    if entity.entity_id == holder.entity_id {
        return false;
    }
    let leash_distance = holder
        .bounding_box
        .load()
        .get_center()
        .squared_distance_to_vec(&entity.bounding_box.load().get_center())
        .sqrt();
    leash_distance <= Entity::LEASH_SNAP_DISTANCE && can_be_leashed(leashable)
}

/// Vanilla `Leashable.setLeashedTo(holder, true)`: the link goes out and a leashed entity
/// gets off what it rides.
pub fn set_leashed_to(leashable: &dyn EntityBase, holder: Arc<dyn EntityBase>) {
    let entity = leashable.get_entity();
    if entity.entity_id == holder.get_entity().entity_id {
        return;
    }
    entity.leash_to(holder);
    if let Some(vehicle) = entity.get_vehicle() {
        vehicle.get_entity().remove_passenger(entity.entity_id);
    }
}

/// Vanilla `Leashable.dropLeash` (`drop_lead`) and `removeLeash`.
pub fn drop_leash(leashable: &dyn EntityBase, drop_lead: bool) {
    let entity = leashable.get_entity();
    if !entity.is_leashed() {
        return;
    }
    entity.unleash();
    if let Some(mob) = leashable.get_mob() {
        mob.on_leash_removed();
    }
    if drop_lead {
        entity.spawn_at_location(ItemStack::new(1, &Item::LEAD));
    }
}

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
    let leashables = leashable_leashed_to(&world, entity);
    let this_leashed = this.is_leashable() && entity.is_leashed();
    // Something to cut: the plugin event may still call the shearing off.
    if (this_leashed || !leashables.is_empty())
        && let Some((player, hand)) = player
        && crate::entity::mob::shear_event_cancelled(player, entity.entity_id, hand)
    {
        return None;
    }
    let dropped = if this_leashed {
        drop_leash(this, true);
        true
    } else {
        !leashables.is_empty()
    };
    for leashable in &leashables {
        drop_leash(leashable.as_ref(), true);
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
        for mob in leashable_held_by_in_area(&world, entity, player.entity_id()) {
            if can_have_a_leash_attached_to(mob.as_ref(), entity) {
                set_leashed_to(mob.as_ref(), this.clone());
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
        drop_leash(this.as_ref(), true);
    }
    set_leashed_to(this.as_ref(), player.clone());
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
