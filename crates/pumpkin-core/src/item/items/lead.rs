use crate::block::registry::BlockActionResult;
use crate::entity::decoration::leash_knot::LeashKnotEntity;
use crate::entity::player::Player;
use crate::entity::{EntityBase, leashable};
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::Block;
use pumpkin_data::BlockDirection;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::Taggable;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct LeadItem;

impl ItemMetadata for LeadItem {
    fn ids() -> Box<[u16]> {
        [Item::LEAD.id].into()
    }
}

impl ItemBehaviour for LeadItem {
    fn use_on_block(
        &self,
        _item: &mut ItemStack,
        player: &Player,
        _hand: pumpkin_util::Hand,
        location: BlockPos,
        _face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        if !block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_FENCES) {
            return BlockActionResult::Pass;
        }

        // `LeadItem.useOn`: on the server, with a player.
        bind_player_mobs(player, location)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Vanilla `LeadItem.bindPlayerMobs`: the player's leashed mobs around the fence go to its knot
/// (an existing one or a new one); the item isn't used up.
pub fn bind_player_mobs(player: &Player, pos: BlockPos) -> BlockActionResult {
    let world = player.world();
    let player_id = player.entity_id();
    let entities_to_leash =
        leashable::leashable_in_area(&world, pos.to_centered_f64(), |leashable| {
            leashable::leash_holder(leashable.get_entity())
                .is_some_and(|holder| holder.get_entity().entity_id == player_id)
        });
    if entities_to_leash.is_empty() {
        return BlockActionResult::Pass;
    }

    let existing_knot = LeashKnotEntity::get_knot(&world, pos);
    let created = existing_knot.is_none();
    // A plugin may cancel the new knot's spawn: then nothing is tied.
    let Some(active_knot) = existing_knot.or_else(|| LeashKnotEntity::create_knot(&world, pos))
    else {
        return BlockActionResult::Pass;
    };
    let mut any_leashed = false;

    for leashable in entities_to_leash {
        if leashable::can_have_a_leash_attached_to(leashable.as_ref(), active_knot.get_entity()) {
            leashable::set_leashed_to(leashable.as_ref(), active_knot.clone(), true);
            any_leashed = true;
        }
    }

    if any_leashed {
        if let Some(knot) = active_knot.cast_any().downcast_ref::<LeashKnotEntity>() {
            knot.play_placement_sound();
        }
        let player_base: &dyn EntityBase = player;
        world.game_event_from(
            Some(player_base),
            GameEvent::BlockAttach,
            pos.to_centered_f64(),
        );
        return BlockActionResult::SuccessServer;
    }

    if created {
        active_knot.get_entity().remove();
    }

    BlockActionResult::Pass
}
