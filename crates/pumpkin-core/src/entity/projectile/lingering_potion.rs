use std::sync::RwLock;

use crate::entity::projectile::splash_potion::on_hit_thrown_potion;
use crate::{
    entity::{Entity, EntityBase, projectile::ThrownItemEntity},
    server::Server,
};
use pumpkin_data::item_stack::ItemStack;

use pumpkin_util::math::vector3::Vector3;
use uuid::Uuid;

const GRAVITY: f64 = 0.05;

pub struct LingeringPotionEntity {
    pub thrown: ThrownItemEntity,
    pub item_stack: RwLock<ItemStack>,
}

impl LingeringPotionEntity {
    pub fn new(entity: Entity) -> Self {
        entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        let thrown = ThrownItemEntity::unowned(entity, GRAVITY);

        Self {
            thrown,
            item_stack: RwLock::new(ItemStack::new(
                1,
                &pumpkin_data::item::Item::LINGERING_POTION,
            )),
        }
    }

    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        thrown.entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        Self {
            thrown,
            item_stack: RwLock::new(ItemStack::new(
                1,
                &pumpkin_data::item::Item::LINGERING_POTION,
            )),
        }
    }

    pub fn set_item_stack(&self, item_stack: ItemStack) {
        let mut write = self
            .item_stack
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *write = item_stack;
    }
}

impl EntityBase for LingeringPotionEntity {
    fn get_projectile(&self) -> Option<&crate::entity::projectile::Projectile> {
        Some(&self.thrown.projectile)
    }

    fn init_data_tracker(&self) {
        let entity = self.get_entity();
        let stack = self
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Sync the item stack so the client renders the correct potion type
        entity.set_synced_data(
            pumpkin_data::tracked_data::lingering_potion::ITEM_STACK,
            pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer::from(stack.clone()),
        );
    }

    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        self.thrown.process_tick(caller);
    }

    /// Vanilla `ThrowableItemProjectile.addAdditionalSaveData`: the `Item`.
    fn write_custom_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        let mut item = pumpkin_nbt::compound::NbtCompound::new();
        self.item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .write_item_stack(&mut item);
        nbt.put_compound("Item", item);
    }

    /// Vanilla `ThrowableItemProjectile.readAdditionalSaveData`: `setItem` with the saved
    /// `Item`, or the default item.
    fn read_custom_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        let mut stack = nbt
            .get_compound("Item")
            .and_then(ItemStack::read_item_stack)
            .unwrap_or_else(|| ItemStack::new(1, &pumpkin_data::item::Item::LINGERING_POTION));
        // `source.copyWithCount(1)`.
        stack.item_count = 1;
        self.set_item_stack(stack.clone());
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::lingering_potion::ITEM_STACK,
            pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer::from(stack),
        );
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
        let stack = self
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        on_hit_thrown_potion(self, &stack, &hit, || {
            self.on_hit_as_potion(&stack, &hit);
        });
    }
}

impl LingeringPotionEntity {
    /// Vanilla `ThrownLingeringPotion.onHitAsPotion`: an area effect cloud at the entity
    /// struck, or the potion itself; radius 3 shrinking 0.5 per use, 600 ticks after a 10-tick
    /// wait.
    fn on_hit_as_potion(&self, stack: &ItemStack, hit: &crate::entity::projectile::ProjectileHit) {
        let world = self.get_entity().world.load();
        if let Some(server) = world.server.upgrade() {
            let mut event = crate::plugin::api::events::entity::lingering_potion_splash::LingeringPotionSplashEvent::new(
                self.get_entity().entity_id,
                self.get_entity().block_pos.load(),
                stack.item.registry_key.to_string(),
            );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
        }

        let cloud_pos = match hit {
            crate::entity::projectile::ProjectileHit::Entity { entity, .. } => {
                entity.get_entity().pos.load()
            }
            crate::entity::projectile::ProjectileHit::Block { .. } => self.get_entity().pos.load(),
        };
        let cloud_entity = crate::entity::Entity::from_uuid(
            Uuid::new_v4(),
            world.clone(),
            cloud_pos,
            &pumpkin_data::entity::EntityType::AREA_EFFECT_CLOUD,
        );
        let cloud = crate::entity::area_effect_cloud::AreaEffectCloudEntity::create(
            cloud_entity,
            stack.clone(),
            crate::item::potion::PotionContents::read_potion_effects(stack),
            600,
            3.0,
            20,
            10,
            -0.5,
            0,
        );

        world.spawn_entity(cloud);
    }
}
