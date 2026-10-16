use std::sync::Arc;

use crossbeam::atomic::AtomicCell;
use uuid::Uuid;

use crate::{
    entity::{Entity, EntityBase},
    world::World,
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stored {
    uuid: Uuid,
    /// The live entity it last resolved to, by runtime id.
    id: Option<i32>,
}

/// Vanilla `EntityReference`: an entity known by its UUID, resolved to the live entity, in any
/// dimension, when it's needed. It survives saving and the entity unloading.
pub struct EntityReference(AtomicCell<Option<Stored>>);

impl EntityReference {
    #[must_use]
    pub const fn none() -> Self {
        Self(AtomicCell::new(None))
    }

    /// Vanilla `EntityReference.of(entity)`.
    #[must_use]
    pub fn of(entity: Option<&Entity>) -> Self {
        let reference = Self::none();
        reference.set(entity);
        reference
    }

    pub fn set(&self, entity: Option<&Entity>) {
        self.0.store(entity.map(|entity| Stored {
            uuid: entity.entity_uuid,
            id: Some(entity.entity_id),
        }));
    }

    /// Vanilla `EntityReference.of(uuid)`.
    pub fn set_uuid(&self, uuid: Option<Uuid>) {
        self.0.store(uuid.map(|uuid| Stored { uuid, id: None }));
    }

    #[must_use]
    pub fn uuid(&self) -> Option<Uuid> {
        self.0.load().map(|stored| stored.uuid)
    }

    /// Vanilla `EntityReference.matches`.
    #[must_use]
    pub fn matches(&self, entity: &Entity) -> bool {
        self.uuid() == Some(entity.entity_uuid)
    }

    /// Vanilla `EntityReference.getEntity(level, Entity.class)`: the stored entity while it is
    /// still in a world, otherwise whatever entity now has the UUID.
    #[must_use]
    pub fn get(&self, world: &World) -> Option<Arc<dyn EntityBase>> {
        let stored = self.0.load()?;
        if let Some(id) = stored.id {
            if let Some(entity) = find_in_any_world(world, |world| world.get_entity_by_id(id))
                .filter(|entity| entity.get_entity().entity_uuid == stored.uuid)
            {
                return Some(entity);
            }
            self.0.store(Some(Stored {
                uuid: stored.uuid,
                id: None,
            }));
        }
        let entity = find_in_any_world(world, |world| world.get_entity_by_uuid(stored.uuid))?;
        self.0.store(Some(Stored {
            uuid: stored.uuid,
            id: Some(entity.get_entity().entity_id),
        }));
        Some(entity)
    }
}

/// Vanilla `Level.getEntityInAnyDimension`: `world` first, then the server's other worlds.
fn find_in_any_world(
    world: &World,
    find: impl Fn(&World) -> Option<Arc<dyn EntityBase>>,
) -> Option<Arc<dyn EntityBase>> {
    if let Some(entity) = find(world) {
        return Some(entity);
    }
    let server = world.server.upgrade()?;
    server
        .worlds
        .load()
        .iter()
        .filter(|other| !std::ptr::eq(other.as_ref(), world))
        .find_map(|other| find(other))
}
