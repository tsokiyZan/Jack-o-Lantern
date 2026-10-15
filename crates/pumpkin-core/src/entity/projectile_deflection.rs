use pumpkin_data::tag::{self, Taggable};

use crate::entity::{Entity, EntityBase};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy)]
pub enum ProjectileDeflectionType {
    None,
    Simple,
    Redirected,
    TransferVelocityDirection,
    /// A deflection of the mob's own, such as vanilla `Breeze.PROJECTILE_DEFLECTION`.
    Custom(fn(&dyn EntityBase, Option<&dyn EntityBase>)),
}

impl ProjectileDeflectionType {
    /// Vanilla `Entity.deflection`'s default: projectiles bounce back off the entity
    /// types in `deflects_projectiles`.
    #[must_use]
    pub fn of_entity_type(entity: &Entity) -> Self {
        if entity
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_DEFLECTS_PROJECTILES)
        {
            Self::Simple
        } else {
            Self::None
        }
    }

    pub fn deflect(&self, projectile: &dyn EntityBase, hit_entity: Option<&dyn EntityBase>) {
        match self {
            Self::None => {}
            Self::Custom(deflect) => deflect(projectile, hit_entity),
            Self::Simple => {
                let vel = rand::random::<f32>() * 20.0 + 170.0;

                let current_velocity = projectile
                    .get_entity()
                    .velocity
                    .load()
                    .multiply(-0.5, -0.5, -0.5);

                let entity = projectile.get_entity();
                entity.velocity.store(current_velocity);

                let yaw = entity.yaw.load() + vel;
                let pitch = entity.pitch.load();
                entity.set_rotation(yaw, pitch);
                // TODO: Add entity.lastYaw += vel
                entity.velocity_dirty.store(true, Ordering::Relaxed);
            }
            Self::Redirected => {
                if let Some(hit_entity) = hit_entity {
                    let rotation_vector = hit_entity.get_entity().rotation();

                    let entity = projectile.get_entity();
                    entity.velocity.store(rotation_vector.to_f64());
                    entity.velocity_dirty.store(true, Ordering::Relaxed);
                }
            }
            Self::TransferVelocityDirection => {
                if let Some(hit_entity) = hit_entity {
                    let hit_velocity = hit_entity.get_entity().velocity.load().normalize();

                    let entity = projectile.get_entity();
                    entity.velocity.store(hit_velocity);
                    entity.velocity_dirty.store(true, Ordering::Relaxed);
                }
            }
        }
    }
}
