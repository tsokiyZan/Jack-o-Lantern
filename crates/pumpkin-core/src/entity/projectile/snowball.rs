use std::sync::atomic::AtomicBool;

use crate::entity::projectile::ProjectileHit;
use crate::{
    entity::{Entity, EntityBase, projectile::ThrownItemEntity},
    server::Server,
};
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::math::vector3::Vector3;

const GRAVITY: f64 = 0.03;

pub struct SnowballEntity {
    pub thrown: ThrownItemEntity,
}

impl SnowballEntity {
    pub fn new(entity: Entity) -> Self {
        // Keep the velocity initialization
        entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));

        // Initialize without owner
        let thrown = ThrownItemEntity {
            entity,
            owner_id: crossbeam::atomic::AtomicCell::new(None),
            collides_with_projectiles: false,
            has_hit: AtomicBool::new(false),
            last_deflected_by: crossbeam::atomic::AtomicCell::new(None),
            gravity: GRAVITY,
            left_owner: std::sync::atomic::AtomicBool::new(false),
        };

        Self { thrown }
    }

    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        thrown.entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        Self { thrown }
    }
}

impl EntityBase for SnowballEntity {
    fn get_owner_id(&self) -> Option<i32> {
        self.thrown.owner_id.load()
    }

    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        self.thrown.process_tick(caller);
    }

    fn get_entity(&self) -> &Entity {
        self.thrown.get_entity()
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: crate::entity::projectile::ProjectileHit) {
        let world = self.get_entity().world.load();

        // Always send particle status regardless of what was hit
        world.send_entity_status(
            self.get_entity(),
            EntityStatus::Death,
            Some(ActorEventID::Death),
        );

        // Handle entity-specific damage
        // Vanilla `Snowball.onHitEntity`: `thrown(this, owner)`, 3 to a blaze and 0 to the rest,
        // which still knocks them back.
        if let ProjectileHit::Entity { ref entity, .. } = hit {
            let is_blaze = entity.get_entity().entity_type.id == EntityType::BLAZE.id;
            let damage = if is_blaze { 3.0 } else { 0.0 };
            let this = world.get_entity_by_id(self.get_entity().entity_id);
            let owner = self
                .thrown
                .owner_id
                .load()
                .and_then(|id| world.get_entity_by_id(id));
            entity.damage_with_context(
                entity.as_ref(),
                damage,
                DamageType::THROWN,
                None,
                this.as_deref(),
                owner.as_deref(),
            );
        }
    }
}
