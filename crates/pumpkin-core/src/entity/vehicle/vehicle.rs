use crossbeam::atomic::AtomicCell;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::entity::Entity;
use pumpkin_protocol::java::client::play::Metadata;

use crate::entity::EntityBase;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::{CustomNameImpl, DataComponentImpl};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::GameMode;

pub struct VehicleEntity {
    pub entity: Entity,
    pub hurt_time: AtomicI32,
    pub hurt_dir: AtomicI32,
    pub damage: AtomicCell<f32>,
}

impl VehicleEntity {
    pub const fn new(entity: Entity) -> Self {
        Self {
            entity,
            hurt_time: AtomicI32::new(0),
            hurt_dir: AtomicI32::new(1),
            damage: AtomicCell::new(0.0),
        }
    }

    pub fn tick(&self) {
        let current_hurt = self.hurt_time.load(Ordering::Relaxed);
        let hurt_ticked = current_hurt > 0;
        if hurt_ticked {
            self.hurt_time.store(current_hurt - 1, Ordering::Relaxed);
        }

        let current_damage = self.damage.load();
        let damage_ticked = current_damage > 0.0;
        if damage_ticked {
            self.damage.store(current_damage - 1.0);
        }

        if hurt_ticked || damage_ticked {
            self.send_wobble_metadata();
        }

        let mut update_event =
            crate::plugin::api::events::vehicle::vehicle_update::VehicleUpdateEvent::new(
                self.entity.entity_id,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut update_event);
        }

        // Coalesce pushed velocity to once per tick (boats use the default push()).
        if self.entity.velocity_dirty.swap(false, Ordering::SeqCst) {
            self.entity.send_velocity();
        }
    }

    pub async fn create(&self) {
        let mut create_event =
            crate::plugin::api::events::vehicle::vehicle_create::VehicleCreateEvent::new(
                self.entity.entity_id,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server.plugin_manager.fire(&server, &mut create_event).await;
        }
    }

    pub async fn move_vehicle(
        &self,
        from: pumpkin_util::math::vector3::Vector3<f64>,
        to: pumpkin_util::math::vector3::Vector3<f64>,
    ) {
        let mut move_event =
            crate::plugin::api::events::vehicle::vehicle_move::VehicleMoveEvent::new(
                self.entity.entity_id,
                from,
                to,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server.plugin_manager.fire(&server, &mut move_event).await;
        }
    }

    pub async fn collide_entity(&self, collided_entity_id: i32) {
        let mut base_event =
            crate::plugin::api::events::vehicle::vehicle_collision::VehicleCollisionEvent::new(
                self.entity.entity_id,
            );
        let mut collide_event = crate::plugin::api::events::vehicle::vehicle_entity_collision::VehicleEntityCollisionEvent::new(
            self.entity.entity_id,
            collided_entity_id,
        );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server.plugin_manager.fire(&server, &mut base_event).await;
            server
                .plugin_manager
                .fire(&server, &mut collide_event)
                .await;
        }
    }

    pub async fn collide_block(&self, block_pos: pumpkin_util::math::position::BlockPos) {
        let mut base_event =
            crate::plugin::api::events::vehicle::vehicle_collision::VehicleCollisionEvent::new(
                self.entity.entity_id,
            );
        let mut collide_event = crate::plugin::api::events::vehicle::vehicle_block_collision::VehicleBlockCollisionEvent::new(
            self.entity.entity_id,
            block_pos,
        );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server.plugin_manager.fire(&server, &mut base_event).await;
            server
                .plugin_manager
                .fire(&server, &mut collide_event)
                .await;
        }
    }

    pub fn set_damage(&self, damage: f32) {
        self.damage.store(damage);
    }

    pub fn get_damage(&self) -> f32 {
        self.damage.load()
    }

    pub fn set_hurt_time(&self, hurt_time: i32) {
        self.hurt_time.store(hurt_time, Ordering::Relaxed);
    }

    pub fn get_hurt_time(&self) -> i32 {
        self.hurt_time.load(Ordering::Relaxed)
    }

    pub fn set_hurt_dir(&self, hurt_dir: i32) {
        self.hurt_dir.store(hurt_dir, Ordering::Relaxed);
    }

    pub fn get_hurt_dir(&self) -> i32 {
        self.hurt_dir.load(Ordering::Relaxed)
    }

    pub fn send_wobble_metadata(&self) {
        self.entity.send_meta_data(
            &[
                Metadata::new(
                    pumpkin_data::tracked_data::boat::ID_HURT,
                    VarInt(self.get_hurt_time()),
                ),
                Metadata::new(
                    pumpkin_data::tracked_data::boat::ID_HURTDIR,
                    VarInt(self.get_hurt_dir()),
                ),
            ],
            None,
        );
        self.entity.send_meta_data(
            &[Metadata::new(
                pumpkin_data::tracked_data::boat::ID_DAMAGE,
                self.get_damage(),
            )],
            None,
        );
    }

    /// `remove(KILLED)`: `AbstractBoat.remove` drops its leash first. Only boats among the
    /// vehicles are `Leashable`, so a leashed vehicle is a boat. `caller` is this vehicle.
    fn remove_destroyed(&self, caller: &dyn EntityBase) {
        if self.entity.is_leashed() {
            crate::entity::leashable::drop_leash(caller, true, true);
        }
        self.entity.remove();
    }

    /// Vanilla `VehicleEntity.destroy(level, dropItem)`: the vehicle dies and, with
    /// `entityDrops`, drops its item carrying its custom name.
    pub fn destroy(&self, caller: &dyn EntityBase, drop_item: &'static Item) {
        self.remove_destroyed(caller);
        let world = self.entity.world.load();
        if world.level_info.load().game_rules.entity_drops {
            let mut stack = ItemStack::new(1, drop_item);
            if let Some(name) = self.entity.custom_name.load().as_ref() {
                stack.patch.push((
                    DataComponent::CustomName,
                    Some(CustomNameImpl { name: name.clone() }.to_dyn()),
                ));
            }
            self.entity.spawn_at_location(stack);
        }
    }

    /// Vanilla `VehicleEntity.hurtServer`; `caller` is this vehicle, `drop_item` its
    /// `getDropItem`.
    pub fn damage_with_context(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        source: Option<&dyn EntityBase>,
        drop_item: &'static Item,
    ) -> bool {
        if !self.entity.is_alive() {
            return true;
        }

        let attacker_id = source.map(|s| s.get_entity().entity_id);
        let mut damage_event =
            crate::plugin::api::events::vehicle::vehicle_damage::VehicleDamageEvent::new(
                self.entity.entity_id,
                amount,
                attacker_id,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut damage_event);
        }
        if damage_event.cancelled {
            return false;
        }

        let new_strength = self.apply_damage_wobble(amount);

        let is_creative = source
            .and_then(|s| s.get_player())
            .is_some_and(|p| p.gamemode.load() == GameMode::Creative);

        if is_creative || new_strength > 40.0 {
            let mut destroy_event =
                crate::plugin::api::events::vehicle::vehicle_destroy::VehicleDestroyEvent::new(
                    self.entity.entity_id,
                    attacker_id,
                );
            if let Some(server) = self.entity.world.load().server.upgrade() {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut destroy_event);
            }
            if destroy_event.cancelled {
                return false;
            }

            if is_creative {
                self.remove_destroyed(caller);
            } else {
                self.destroy(caller, drop_item);
            }
        }

        true
    }

    /// Applies the standard minecart damage wobble without destroying the vehicle.
    /// TNT minecarts use this before deciding whether damage primes or breaks them.
    pub fn apply_damage_wobble(&self, amount: f32) -> f32 {
        let current_side = self.get_hurt_dir();
        self.set_hurt_dir(-current_side);
        self.set_hurt_time(10);
        self.entity.velocity_dirty.store(true, Ordering::SeqCst);

        let current_strength = self.get_damage();
        let new_strength = current_strength + amount * 10.0;
        self.set_damage(new_strength);

        self.send_wobble_metadata();
        new_strength
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_protocol::codec::var_int::VarInt;
    use pumpkin_protocol::java::client::play::Metadata;
    use pumpkin_util::version::JavaMinecraftVersion;

    fn wobble_metadata(version: JavaMinecraftVersion) -> Vec<u8> {
        let mut bytes = Vec::new();
        Metadata::new(pumpkin_data::tracked_data::boat::ID_HURT, VarInt(10))
            .write(&mut bytes, &version)
            .unwrap();
        Metadata::new(pumpkin_data::tracked_data::boat::ID_HURTDIR, VarInt(-1))
            .write(&mut bytes, &version)
            .unwrap();
        Metadata::new(pumpkin_data::tracked_data::boat::ID_DAMAGE, 10.0f32)
            .write(&mut bytes, &version)
            .unwrap();
        bytes
    }

    #[test]
    fn wobble_metadata_serializes_hurt_and_damage_together() {
        let mut expected = vec![8, 1, 10, 9, 1, 0xff, 0xff, 0xff, 0xff, 0x0f, 10, 3];
        expected.extend(10.0f32.to_be_bytes());

        assert_eq!(wobble_metadata(JavaMinecraftVersion::V_1_21_11), expected);
        assert_eq!(wobble_metadata(JavaMinecraftVersion::V_26_2), expected);
    }
}
