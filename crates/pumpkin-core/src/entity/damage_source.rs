use pumpkin_data::damage::DamageType;
use pumpkin_data::item_stack::ItemStack;

use crate::entity::EntityBase;

/// The parts of a vanilla `DamageSource` that callers read: its type, the direct entity and the one
/// responsible.
pub struct DamageSource<'a> {
    pub damage_type: DamageType,
    pub direct: Option<&'a dyn EntityBase>,
    pub causing: Option<&'a dyn EntityBase>,
}

impl DamageSource<'_> {
    /// Vanilla `DamageSource.getWeaponItem`: the direct entity's weapon.
    #[must_use]
    pub fn get_weapon_item(&self) -> Option<ItemStack> {
        self.direct.and_then(EntityBase::get_weapon_item)
    }

    /// Vanilla `DamageSource.isDirect`: the causing and direct entity are the same object.
    #[must_use]
    pub fn is_direct(&self) -> bool {
        match (self.causing, self.direct) {
            (None, None) => true,
            (Some(causing), Some(direct)) => {
                causing.get_entity().entity_id == direct.get_entity().entity_id
            }
            _ => false,
        }
    }
}
