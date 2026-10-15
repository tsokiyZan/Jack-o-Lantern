use std::sync::Weak;

use pumpkin_data::damage::DamageType;
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::vector3::Vector3;

use super::EnderDragonEntity;
use crate::entity::living::LivingEntity;
use crate::entity::{Entity, EntityBase};

/// Vanilla `EnderDragonPart`: one of the dragon's hitboxes. Parts are not level entities; the
/// client makes its own from the dragon's id, and the server reaches them through the dragon.
pub struct EnderDragonPart {
    entity: Entity,
    parent_mob: Weak<EnderDragonEntity>,
    /// Index into the dragon's `sub_entities`.
    index: usize,
    pub name: &'static str,
}

impl EnderDragonPart {
    #[must_use]
    pub fn new(
        entity: Entity,
        parent_mob: Weak<EnderDragonEntity>,
        index: usize,
        name: &'static str,
        width: f32,
        height: f32,
    ) -> Self {
        let dimensions = EntityDimensions::scalable(width, height);
        let pos = entity.pos.load();
        entity.entity_dimension.store(dimensions);
        entity
            .bounding_box
            .store(BoundingBox::new_from_pos(pos.x, pos.y, pos.z, &dimensions));
        Self {
            entity,
            parent_mob,
            index,
            name,
        }
    }

    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    pub fn set_pos(&self, pos: Vector3<f64>) {
        self.entity.set_pos(pos);
    }
}

impl EntityBase for EnderDragonPart {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    /// Vanilla `isPickable`.
    fn can_hit(&self) -> bool {
        true
    }

    /// Vanilla `hurtServer`: the dragon takes the hit for this part.
    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if self.entity.is_invulnerable_to(&damage_type, cause) {
            return false;
        }
        self.parent_mob.upgrade().is_some_and(|dragon| {
            dragon.hurt(self.index, damage_type, amount, position, source, cause)
        })
    }
}
