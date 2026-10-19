use std::sync::Arc;

use pumpkin_data::game_event::GameEvent;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::EnchantmentEntityEffectExt;
use crate::entity::Entity;
use crate::entity::player::Player;
use crate::world::World;

/// Enchantment entity effect that sets block properties at an offset position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetBlockProperties {
    pub properties: Vec<(String, String)>,
    pub offset: Vector3<i32>,
    pub trigger_game_event: Option<GameEvent>,
}

impl SetBlockProperties {
    #[must_use]
    pub const fn new(
        properties: Vec<(String, String)>,
        offset: Vector3<i32>,
        trigger_game_event: Option<GameEvent>,
    ) -> Self {
        Self {
            properties,
            offset,
            trigger_game_event,
        }
    }

    #[must_use]
    pub const fn target_position(&self, origin: Vector3<f64>) -> BlockPos {
        let base_x = origin.x.floor() as i32;
        let base_y = origin.y.floor() as i32;
        let base_z = origin.z.floor() as i32;
        BlockPos::new(
            base_x + self.offset.x,
            base_y + self.offset.y,
            base_z + self.offset.z,
        )
    }

    pub fn apply(
        &self,
        world: &Arc<World>,
        _enchantment_level: i32,
        _owner: Option<&Arc<Player>>,
        _entity: Option<&Entity>,
        position: Vector3<f64>,
    ) {
        let target = self.target_position(position);
        if let Some(event) = self.trigger_game_event {
            // TODO: vanilla passes the enchanted entity as the source; this hook only has its
            // base `Entity`.
            world.game_event_from(None, event, target.to_centered_f64());
        }
    }
}

impl EnchantmentEntityEffectExt for SetBlockProperties {
    fn apply(
        &self,
        world: &Arc<World>,
        enchantment_level: i32,
        item: &crate::enchantment::EnchantedItemInUse<'_>,
        entity: &dyn crate::entity::EntityBase,
        position: Vector3<f64>,
    ) {
        let owner = item
            .owner
            .and_then(|owner| world.get_player_by_id(owner.get_entity().entity_id));
        let entity = Some(entity.get_entity());
        self.apply(world, enchantment_level, owner.as_ref(), entity, position);
    }
}
