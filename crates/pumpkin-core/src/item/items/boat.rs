use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::sync::Arc;

use crate::entity::Entity;
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::entity::vehicle::boat::BoatEntity;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::vector3::Vector3;

pub struct BoatItem;

impl BoatItem {
    /// Maps boat item to corresponding entity type
    pub(crate) fn item_to_entity(item: &Item) -> &'static EntityType {
        match item.id {
            val if val == Item::OAK_BOAT.id => &EntityType::OAK_BOAT,
            val if val == Item::OAK_CHEST_BOAT.id => &EntityType::OAK_CHEST_BOAT,
            val if val == Item::SPRUCE_BOAT.id => &EntityType::SPRUCE_BOAT,
            val if val == Item::SPRUCE_CHEST_BOAT.id => &EntityType::SPRUCE_CHEST_BOAT,
            val if val == Item::BIRCH_BOAT.id => &EntityType::BIRCH_BOAT,
            val if val == Item::BIRCH_CHEST_BOAT.id => &EntityType::BIRCH_CHEST_BOAT,
            val if val == Item::JUNGLE_BOAT.id => &EntityType::JUNGLE_BOAT,
            val if val == Item::JUNGLE_CHEST_BOAT.id => &EntityType::JUNGLE_CHEST_BOAT,
            val if val == Item::ACACIA_BOAT.id => &EntityType::ACACIA_BOAT,
            val if val == Item::ACACIA_CHEST_BOAT.id => &EntityType::ACACIA_CHEST_BOAT,
            val if val == Item::DARK_OAK_BOAT.id => &EntityType::DARK_OAK_BOAT,
            val if val == Item::DARK_OAK_CHEST_BOAT.id => &EntityType::DARK_OAK_CHEST_BOAT,
            val if val == Item::MANGROVE_BOAT.id => &EntityType::MANGROVE_BOAT,
            val if val == Item::MANGROVE_CHEST_BOAT.id => &EntityType::MANGROVE_CHEST_BOAT,
            val if val == Item::CHERRY_BOAT.id => &EntityType::CHERRY_BOAT,
            val if val == Item::CHERRY_CHEST_BOAT.id => &EntityType::CHERRY_CHEST_BOAT,
            val if val == Item::PALE_OAK_BOAT.id => &EntityType::PALE_OAK_BOAT,
            val if val == Item::PALE_OAK_CHEST_BOAT.id => &EntityType::PALE_OAK_CHEST_BOAT,
            val if val == Item::POPLAR_BOAT.id => &EntityType::POPLAR_BOAT,
            val if val == Item::POPLAR_CHEST_BOAT.id => &EntityType::POPLAR_CHEST_BOAT,
            val if val == Item::BAMBOO_RAFT.id => &EntityType::BAMBOO_RAFT,
            val if val == Item::BAMBOO_CHEST_RAFT.id => &EntityType::BAMBOO_CHEST_RAFT,
            _ => {
                tracing::error!("Unknown boat item ID: {}", item.id);
                &EntityType::OAK_BOAT
            }
        }
    }

    /// Gets entity dimensions for the boat type
    const fn get_entity_dimensions(entity_type: &EntityType) -> EntityDimensions {
        EntityDimensions::new(
            entity_type.dimension[0],
            entity_type.dimension[1],
            entity_type.eye_height,
        )
    }
}

impl ItemMetadata for BoatItem {
    fn ids() -> Box<[u16]> {
        [
            Item::OAK_BOAT.id,
            Item::OAK_CHEST_BOAT.id,
            Item::SPRUCE_BOAT.id,
            Item::SPRUCE_CHEST_BOAT.id,
            Item::BIRCH_BOAT.id,
            Item::BIRCH_CHEST_BOAT.id,
            Item::JUNGLE_BOAT.id,
            Item::JUNGLE_CHEST_BOAT.id,
            Item::ACACIA_BOAT.id,
            Item::ACACIA_CHEST_BOAT.id,
            Item::DARK_OAK_BOAT.id,
            Item::DARK_OAK_CHEST_BOAT.id,
            Item::MANGROVE_BOAT.id,
            Item::MANGROVE_CHEST_BOAT.id,
            Item::CHERRY_BOAT.id,
            Item::CHERRY_CHEST_BOAT.id,
            Item::PALE_OAK_BOAT.id,
            Item::PALE_OAK_CHEST_BOAT.id,
            Item::POPLAR_BOAT.id,
            Item::POPLAR_CHEST_BOAT.id,
            Item::BAMBOO_RAFT.id,
            Item::BAMBOO_CHEST_RAFT.id,
        ]
        .into()
    }
}

impl ItemBehaviour for BoatItem {
    /// Vanilla: `BoatItem.use()` - raycasts to find placement position
    fn normal_use(&self, item: &Item, player: &Player, hand: Hand) -> InteractionResult {
        let world = player.world();
        let (start_pos, end_pos) = self.get_start_and_end_pos(player);

        // Vanilla `getPlayerPOVHitResult(level, player, Fluid.ANY)`: outline shapes.
        let hit = world.clip(&pumpkin_world::clip::ClipContext::new(
            start_pos,
            end_pos,
            pumpkin_world::clip::ClipBlock::Outline,
            pumpkin_world::clip::ClipFluid::Any,
        ));
        if hit.is_miss {
            return InteractionResult::PASS;
        }

        let hit_vec = hit.location;

        // Vanilla: `level.getEntities(player, box, CAN_BE_PICKED)` along the view, other players
        // included; any whose box holds the player's eyes blocks the use.
        let (yaw, pitch) = player.rotation();
        let view = Entity::calculate_view_vector(pitch, yaw);
        let search_box = player
            .get_entity()
            .bounding_box
            .load()
            .stretch(Vector3::new(view.x * 5.0, view.y * 5.0, view.z * 5.0))
            .expand_all(1.0);
        let player_id = player.get_entity().entity_id;
        let player_eye_pos = player.eye_position();
        // Vanilla `getPickRadius()` is 0 for every entity that can be picked here.
        if world.get_all_at_box(&search_box).iter().any(|entity| {
            entity.get_entity().entity_id != player_id
                && !entity.is_spectator()
                && entity.can_hit()
                && entity
                    .get_entity()
                    .bounding_box
                    .load()
                    .contains(player_eye_pos)
        }) {
            return InteractionResult::PASS;
        }

        let entity_type = Self::item_to_entity(item);
        let entity = Entity::new(world.clone(), hit_vec, entity_type);
        entity.set_rotation(yaw, 0.0);
        let boat_entity = Arc::new(BoatEntity::new(entity));

        // Vanilla `level.noCollision(boat, boat.getBoundingBox())`: blocks, and the entities a
        // boat collides with (other vehicles, shulkers, and anything pushable).
        let dimensions = Self::get_entity_dimensions(entity_type);
        let boat_box = BoundingBox::new_from_pos(hit_vec.x, hit_vec.y, hit_vec.z, &dimensions);
        if !world.is_space_empty(boat_box)
            || !world
                .get_entity_collisions(boat_entity.as_ref(), boat_box)
                .is_empty()
        {
            return InteractionResult::FAIL;
        }

        world.spawn_entity(boat_entity);
        world.game_event_from(Some(player), GameEvent::EntityPlace, hit_vec);
        crate::item::consume_in_hand(player, hand);

        player.award_item_used(item);
        InteractionResult::SUCCESS
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
