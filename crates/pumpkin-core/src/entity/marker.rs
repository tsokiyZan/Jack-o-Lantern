use std::sync::{Arc, atomic::Ordering};

use crate::{
    entity::{Entity, EntityBase, living::LivingEntity},
    net::{bedrock::BedrockClient, java::JavaClient},
    server::Server,
};
use pumpkin_data::damage::DamageType;
use pumpkin_util::math::vector3::Vector3;

/// Vanilla `Marker`: its `data` is the entity-wide custom data.
pub struct MarkerEntity {
    pub entity: Entity,
}

impl MarkerEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        entity.no_physics.store(true, Ordering::Relaxed);
        Arc::new(Self { entity })
    }
}

impl EntityBase for MarkerEntity {
    /// Vanilla `Marker.getPistonPushReaction`.
    fn piston_push_reaction(&self) -> pumpkin_data::block_state::PistonBehavior {
        pumpkin_data::block_state::PistonBehavior::Ignore
    }

    fn tick(&self, _caller: &dyn EntityBase, _server: &Server) {}

    fn init_data_tracker(&self) {}

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn is_pushable(&self) -> bool {
        false
    }

    fn is_pushed_by_fluids(&self) -> bool {
        false
    }

    fn can_hit(&self) -> bool {
        false
    }

    fn is_immune_to_explosion(&self, _explosion: &crate::world::explosion::Explosion) -> bool {
        true
    }

    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        _amount: f32,
        _damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        _source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) -> bool {
        false
    }

    fn send_java_spawn_packet(&self, _client: &JavaClient) {}

    fn send_bedrock_spawn_packet(&self, _client: &BedrockClient) {}
}
