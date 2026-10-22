use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam::atomic::AtomicCell;

use crate::entity::movement_emission::MovementEmission;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase, living::LivingEntity};
use crate::server::Server;

use pumpkin_data::damage::DamageType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_protocol::java::client::play::Metadata;

use pumpkin_util::math::vector3::Vector3;

use crate::entity::vehicle::vehicle::VehicleEntity;

pub struct BoatEntity {
    pub vehicle: VehicleEntity,
    ticks_underwater: AtomicCell<f32>,
    left_paddle_moving: AtomicBool,
    right_paddle_moving: AtomicBool,
}

impl BoatEntity {
    pub const fn new(entity: Entity) -> Self {
        Self {
            vehicle: VehicleEntity::new(entity),
            ticks_underwater: AtomicCell::new(0.0),
            left_paddle_moving: AtomicBool::new(false),
            right_paddle_moving: AtomicBool::new(false),
        }
    }

    pub fn set_paddles(&self, left: bool, right: bool) {
        self.left_paddle_moving.store(left, Ordering::Relaxed);
        self.right_paddle_moving.store(right, Ordering::Relaxed);

        self.vehicle.entity.send_meta_data(
            &[
                Metadata::new(pumpkin_data::tracked_data::boat::ID_PADDLE_LEFT, left),
                Metadata::new(pumpkin_data::tracked_data::boat::ID_PADDLE_RIGHT, right),
            ],
            None,
        );
    }

    fn send_wobble_metadata(&self) {
        self.vehicle.send_wobble_metadata();
    }
}

impl EntityBase for BoatEntity {
    /// Vanilla `VehicleEntity.getDimensionChangingDelay`.
    fn dimension_changing_delay(&self) -> u32 {
        10
    }

    /// Vanilla `AbstractBoat.getPassengerAttachmentPoint`: two riders sit fore and aft (an
    /// animal a little further forward), at the boat's or raft's ride height.
    fn passenger_attachment_point(
        &self,
        passenger: &dyn EntityBase,
        dimensions: pumpkin_util::math::boundingbox::EntityDimensions,
        _scale: f32,
    ) -> pumpkin_util::math::vector3::Vector3<f64> {
        let entity = self.get_entity();
        let name = entity.entity_type.resource_name;
        let raft = name.ends_with("_raft");
        // `AbstractChestBoat.getSinglePassengerXOffset`.
        let mut offset = if name.contains("chest") { 0.15f32 } else { 0.0 };
        let passengers = entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if passengers.len() > 1 {
            let id = passenger.get_entity().entity_id;
            let index = passengers
                .iter()
                .position(|p| p.get_entity().entity_id == id);
            offset = if index == Some(0) { 0.2 } else { -0.6 };
            if passenger.get_mob().is_some_and(|m| m.as_animal().is_some()) {
                offset += 0.2;
            }
        }
        // `rideHeight`: `Raft`/`ChestRaft` and `Boat`/`ChestBoat`.
        let ride_height = if raft {
            dimensions.height * 0.888_888_9
        } else {
            dimensions.height / 3.0
        };
        pumpkin_util::math::vector3::Vector3::new(0.0, f64::from(ride_height), f64::from(offset))
            .y_rot(-entity.yaw.load() * (std::f32::consts::PI / 180.0))
    }

    /// Vanilla `AbstractBoat.positionRider`. Pumpkin's boats don't turn on their own
    /// (`deltaRotation` stays 0), so only `clampRotation` is left.
    fn position_rider(&self, caller: &dyn EntityBase, passenger: &dyn EntityBase) {
        use pumpkin_data::tag::Taggable;
        crate::entity::set_rider_position(caller, passenger);
        let passenger_entity = passenger.get_entity();
        if passenger_entity
            .entity_type
            .has_tag(&pumpkin_data::tag::EntityType::MINECRAFT_CAN_TURN_IN_BOATS)
        {
            return;
        }
        // `clampRotation` with `calculatePassengerBodyYRot`.
        let entity = self.get_entity();
        let max_passengers = if entity.entity_type.resource_name.contains("chest") {
            1
        } else {
            2
        };
        let passenger_count = entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        let yaw = entity.yaw.load();
        let body_yaw = if passenger.get_mob().is_some_and(|m| m.as_animal().is_some())
            && passenger_count == max_passengers
        {
            yaw + if passenger_entity.entity_id % 2 == 0 {
                90.0
            } else {
                270.0
            }
        } else {
            yaw
        };
        passenger_entity.body_yaw.store(body_yaw);
        if let Some(mob) = passenger.get_mob() {
            passenger_entity.yaw.store(body_yaw);
            mob.clamp_head_rotation_to_body();
        } else {
            let passenger_yaw = passenger_entity.yaw.load();
            let delta = pumpkin_util::math::wrap_degrees(passenger_yaw - body_yaw);
            let target_delta = delta.clamp(-105.0, 105.0);
            passenger_entity
                .yaw
                .store(passenger_yaw + target_delta - delta);
        }
    }

    /// Vanilla `AbstractBoat.checkFallDamage`: a boat's fall ends on the ground without
    /// landing on anything, and doesn't add up over water.
    fn check_fall_damage(
        &self,
        _caller: &dyn EntityBase,
        ya: f64,
        on_ground: bool,
        _state: &'static pumpkin_data::BlockState,
        _pos: pumpkin_util::math::position::BlockPos,
    ) {
        let entity = self.get_entity();
        if entity.has_vehicle() {
            return;
        }
        if on_ground {
            entity.reset_fall_distance();
        } else if ya < 0.0 {
            let below = entity.block_pos.load().down();
            let fluid = entity.world.load().get_fluid(&below);
            let water = fluid.id == pumpkin_data::fluid::Fluid::WATER.id
                || fluid.id == pumpkin_data::fluid::Fluid::FLOWING_WATER.id;
            if !water {
                entity
                    .fall_distance
                    .store(entity.fall_distance.load() - f64::from(ya as f32));
            }
        }
    }

    /// Vanilla `AbstractBoat.modifyPassengerFluidInteractionBox`: only what sticks out above
    /// the boat touches fluids. Boats don't track their `status` yet, so being under water is
    /// the entity's own `isUnderWater`.
    fn modify_passenger_fluid_interaction_box(
        &self,
        passenger_box: pumpkin_util::math::boundingbox::BoundingBox,
    ) -> Option<pumpkin_util::math::boundingbox::BoundingBox> {
        let entity = self.get_entity();
        if entity.is_under_water() {
            return Some(passenger_box);
        }
        let boat_box = entity.bounding_box.load();
        if boat_box.max.y >= passenger_box.max.y {
            return None;
        }
        let mut fluid_box = passenger_box;
        fluid_box.min.y = passenger_box.min.y.max(boat_box.max.y);
        Some(fluid_box)
    }

    fn get_entity(&self) -> &Entity {
        &self.vehicle.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.vehicle.tick();
        // `super.tick()`: `Entity.baseTick`, which ends with `Leashable.tickLeash`.
        self.vehicle.entity.tick(caller, server);

        let underwater = self.ticks_underwater.load();
        if self.vehicle.entity.touching_water.load(Ordering::Relaxed) {
            self.ticks_underwater.store((underwater + 1.0).min(60.0));
        } else if underwater > 0.0 {
            self.ticks_underwater.store((underwater - 1.0).max(0.0));
        }
    }

    fn init_data_tracker(&self) {
        self.send_wobble_metadata();
    }

    fn can_hit(&self) -> bool {
        self.vehicle.entity.is_alive()
    }

    /// Vanilla `AbstractBoat.canBeCollidedWith`.
    fn is_collidable(&self, _entity: Option<&dyn EntityBase>) -> bool {
        true
    }

    /// Vanilla `AbstractBoat.canCollideWith` (`canVehicleCollide`).
    fn can_collide_with(&self, caller: &dyn EntityBase, other: &dyn EntityBase) -> bool {
        crate::entity::vehicle::can_vehicle_collide(caller, other)
    }

    fn damage_with_context(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        _damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) -> bool {
        // `AbstractBoat.getDropItem`: each boat and raft drops the item of its own name.
        let Some(drop_item) = pumpkin_data::item::Item::from_registry_key(
            self.vehicle.entity.entity_type.resource_name,
        ) else {
            return false;
        };
        self.vehicle
            .damage_with_context(caller, amount, source, drop_item)
    }

    /// Vanilla `AbstractBoat.interact`: `Entity.interact`'s leashing first, then boarding.
    fn interact(
        &self,
        player: &Arc<Player>,
        hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        let super_interaction = crate::entity::leash::entity_interact_by_id(
            &self.vehicle.entity,
            player,
            hand,
            item_stack,
        );
        if super_interaction != InteractionResult::PASS {
            return super_interaction;
        }
        if player.get_entity().is_sneaking() {
            return InteractionResult::PASS;
        }

        if self.ticks_underwater.load() >= 60.0 {
            return InteractionResult::PASS;
        }

        if self
            .vehicle
            .entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
            >= 2
        {
            return InteractionResult::PASS;
        }

        if player.get_entity().has_vehicle() {
            return InteractionResult::PASS;
        }

        let world = self.vehicle.entity.world.load();
        let Some(vehicle) = world.get_entity_by_id(self.vehicle.entity.entity_id) else {
            return InteractionResult::PASS;
        };

        let Some(passenger) = world.get_player_by_id(player.entity_id()) else {
            return InteractionResult::PASS;
        };

        self.vehicle
            .entity
            .add_passenger(vehicle, passenger as Arc<dyn EntityBase>);

        InteractionResult::SUCCESS
    }

    /// Vanilla `AbstractBoat.getControllingPassenger`.
    fn controlling_passenger(&self) -> Option<Arc<dyn EntityBase>> {
        self.vehicle
            .entity
            .get_first_passenger()
            .filter(|passenger| passenger.get_player().is_some())
    }

    /// Vanilla `AbstractBoat.getMovementEmission`.
    fn movement_emission(&self) -> MovementEmission {
        MovementEmission::Events
    }

    fn set_paddle_state(&self, left: bool, right: bool) {
        self.set_paddles(left, right);
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    /// `AbstractBoat implements Leashable`.
    fn is_leashable(&self) -> bool {
        true
    }

    /// Vanilla `AbstractBoat.supportQuadLeash`.
    fn support_quad_leash(&self) -> bool {
        true
    }

    fn is_pushable(&self) -> bool {
        true
    }
}
