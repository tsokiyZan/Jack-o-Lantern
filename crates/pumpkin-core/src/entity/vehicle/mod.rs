pub mod boat;
pub mod minecart;
#[allow(clippy::module_inception)]
pub mod vehicle;

use crate::entity::EntityBase;

/// Vanilla `AbstractBoat.canVehicleCollide`: boats and minecarts also bump into anything
/// pushable, but never into their own stack.
pub fn can_vehicle_collide(vehicle: &dyn EntityBase, entity: &dyn EntityBase) -> bool {
    (entity.is_collidable(Some(vehicle)) || entity.is_pushable())
        && vehicle.get_entity().root_vehicle_id() != entity.get_entity().root_vehicle_id()
}
