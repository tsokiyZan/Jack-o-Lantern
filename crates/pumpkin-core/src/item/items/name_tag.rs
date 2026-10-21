use std::sync::Arc;

use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component_impl::CustomNameImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;

pub struct NameTagItem;

impl ItemMetadata for NameTagItem {
    fn ids() -> Box<[u16]> {
        [Item::NAME_TAG.id].into()
    }
}

impl ItemBehaviour for NameTagItem {
    /// Vanilla `NameTagItem.interactLivingEntity`.
    fn use_on_entity(
        &self,
        item: &mut ItemStack,
        player: &Player,
        target: Arc<dyn EntityBase>,
    ) -> bool {
        let entity = target.get_entity();
        let Some(name) = item.get_data_component::<CustomNameImpl>() else {
            return false;
        };
        if !entity.entity_type.saveable {
            return false;
        }
        if target
            .get_living_entity()
            .is_some_and(crate::entity::living::LivingEntity::is_alive)
        {
            entity.set_custom_name(name.name.clone());
            if let Some(mob) = target.get_mob() {
                mob.get_mob_entity()
                    .persistence_required
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            item.decrement_unless_creative(player.gamemode.load(), 1);
        }
        true
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
