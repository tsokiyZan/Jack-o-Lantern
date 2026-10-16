use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::entity::{Entity, EntityBase, living::LivingEntity};
use pumpkin_data::{
    damage::DamageType,
    tag::{self, Taggable},
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

pub struct EndCrystalEntity {
    entity: Entity,
    beam_target: Mutex<Option<BlockPos>>,
    show_bottom: AtomicBool,
    invulnerable: AtomicBool,
}

impl EndCrystalEntity {
    #[must_use]
    pub const fn new(entity: Entity) -> Self {
        Self {
            entity,
            beam_target: Mutex::new(None),
            show_bottom: AtomicBool::new(false),
            invulnerable: AtomicBool::new(false),
        }
    }

    pub fn set_show_bottom(&self, show_bottom: bool) {
        self.show_bottom.store(show_bottom, Ordering::Relaxed);
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::end_crystal::SHOW_BOTTOM,
            show_bottom,
        );
    }

    pub fn show_bottom(&self) -> bool {
        self.show_bottom.load(Ordering::Relaxed)
    }

    pub fn set_beam_target(&self, beam_target: Option<BlockPos>) {
        *self
            .beam_target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = beam_target;
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::end_crystal::BEAM_TARGET,
            beam_target,
        );
    }

    pub fn beam_target(&self) -> Option<BlockPos> {
        *self
            .beam_target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn set_invulnerable(&self, invulnerable: bool) {
        self.invulnerable.store(invulnerable, Ordering::Relaxed);
    }

    pub fn is_invulnerable(&self) -> bool {
        self.invulnerable.load(Ordering::Relaxed)
    }
}

impl EntityBase for EndCrystalEntity {
    /// Vanilla `isPickable`.
    fn can_hit(&self) -> bool {
        true
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        _amount: f32,
        damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        _source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if self.is_invulnerable() {
            return false;
        }
        // Vanilla `EndCrystal.hurtServer`: the dragon can't break its own crystals.
        if cause.is_some_and(|cause| {
            cause.get_entity().entity_type == &pumpkin_data::entity::EntityType::ENDER_DRAGON
        }) {
            return false;
        }
        if self.entity.is_removed() {
            return true;
        }

        let world = self.entity.world.load();
        let this = world.get_entity_by_id(self.entity.entity_id);
        self.entity.remove();
        if !damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_EXPLOSION) {
            // Credited to whoever broke it, when anyone did.
            let damage_source = cause.map(|cause| {
                crate::world::ExplosionDamageSource::explosion(
                    this.clone(),
                    world.get_entity_by_id(cause.get_entity().entity_id),
                )
            });
            world.explode_full(
                this,
                damage_source,
                None,
                self.entity.pos.load(),
                6.0,
                false,
                crate::world::ExplosionInteraction::Block,
                crate::world::ExplosionEffects::DEFAULT,
            );
        }

        let dragon_uuid = world.dragon_fight.as_ref().and_then(|fight_mutex| {
            fight_mutex
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .on_crystal_destroyed(&world, self.entity.entity_uuid)
        });
        if let Some(dragon) = dragon_uuid.and_then(|uuid| world.get_entity_by_uuid(uuid))
            && let Some(dragon) = dragon
                .cast_any()
                .downcast_ref::<crate::entity::boss::ender_dragon::EnderDragonEntity>(
            )
        {
            dragon.on_crystal_destroyed(self, cause);
        }

        true
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}
