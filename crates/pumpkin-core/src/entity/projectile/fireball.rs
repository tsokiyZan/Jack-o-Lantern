use pumpkin_data::tag::Taggable;
use std::sync::RwLock;
use std::sync::atomic::Ordering;

use pumpkin_data::damage::DamageType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer;
use pumpkin_util::math::atomic_f32::AtomicF32;
use pumpkin_util::math::vector3::Vector3;

use crate::enchantment::EnchantmentHelper;
use crate::entity::damage_source::DamageSource;
use crate::{
    entity::{
        Entity, EntityBase,
        projectile::{
            ProjectileHit,
            hurting_projectile::{HurtingProjectile, HurtingProjectileTick},
        },
    },
    server::Server,
};

pub const MIN_CAMERA_DISTANCE_SQUARED: f32 = 12.25;
pub const DEFAULT_EXPLOSION_POWER: f32 = 1.0;

/// Vanilla `Fireball`: what the large and small fireballs share.
pub struct Fireball {
    pub hurting: HurtingProjectile,
    item_stack: RwLock<ItemStack>,
}

impl Fireball {
    #[must_use]
    pub fn new(hurting: HurtingProjectile) -> Self {
        Self {
            hurting,
            item_stack: RwLock::new(Self::get_default_item()),
        }
    }

    pub const fn get_entity(&self) -> &Entity {
        self.hurting.get_entity()
    }

    #[must_use]
    pub fn get_default_item() -> ItemStack {
        ItemStack::new(1, &Item::FIRE_CHARGE)
    }

    pub fn get_item(&self) -> ItemStack {
        self.item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set_item(&self, source: ItemStack) {
        let new_item = if source.item_count == 0 {
            Self::get_default_item()
        } else {
            let mut item = source;
            item.item_count = 1;
            item
        };
        *self
            .item_stack
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new_item.clone();

        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::fireball::ITEM_STACK,
            ItemStackSerializer::from(new_item),
        );
    }

    pub fn should_render_at_sqr_distance(&self, distance_sqr: f64) -> bool {
        if self.get_entity().tick_count.load(Ordering::Relaxed) < 2
            && distance_sqr < f64::from(MIN_CAMERA_DISTANCE_SQUARED)
        {
            false
        } else {
            let bb_size = self
                .get_entity()
                .bounding_box
                .load()
                .get_average_side_length()
                * 4.0;
            let size = if bb_size.is_nan() { 4.0 } else { bb_size } * 64.0;
            distance_sqr < size * size
        }
    }

    /// Vanilla `DamageSources.fireball`: unattributed without an owner.
    #[must_use]
    pub fn damage_type(owner: Option<&dyn EntityBase>) -> DamageType {
        if owner.is_some() {
            DamageType::FIREBALL
        } else {
            DamageType::UNATTRIBUTED_FIREBALL
        }
    }

    pub fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.hurting
            .tick(caller, server, &HurtingProjectileTick::default());
    }

    pub fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.hurting.write_nbt(nbt);
        let mut item_compound = NbtCompound::new();
        self.get_item().write_item_stack(&mut item_compound);
        nbt.put_compound("Item", item_compound);
    }

    pub fn read_nbt(&self, nbt: &NbtCompound) {
        self.hurting.read_nbt(nbt);
        self.set_item(
            nbt.get_compound("Item")
                .and_then(ItemStack::read_item_stack)
                .unwrap_or_else(Self::get_default_item),
        );
    }

    pub fn init_data_tracker(&self) {
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::fireball::ITEM_STACK,
            ItemStackSerializer::from(self.get_item()),
        );
    }
}

/// Vanilla `LargeFireball`: the ghast's exploding fireball.
pub struct FireballEntity {
    pub fireball: Fireball,
    pub explosion_power: AtomicF32,
}

impl FireballEntity {
    #[must_use]
    pub fn new(entity: Entity) -> Self {
        Self {
            fireball: Fireball::new(HurtingProjectile::new(entity)),
            explosion_power: AtomicF32::new(DEFAULT_EXPLOSION_POWER),
        }
    }

    /// Vanilla `LargeFireball(level, mob, direction, explosionPower)`.
    #[must_use]
    pub fn new_from_mob(
        entity: Entity,
        mob: &Entity,
        direction: Vector3<f64>,
        explosion_power: f32,
    ) -> Self {
        Self {
            fireball: Fireball::new(HurtingProjectile::new_from_mob(entity, mob, direction)),
            explosion_power: AtomicF32::new(explosion_power),
        }
    }

    pub fn get_explosion_power(&self) -> f32 {
        self.explosion_power.load(Ordering::Relaxed)
    }

    pub fn set_explosion_power(&self, power: f32) {
        self.explosion_power.store(power, Ordering::Relaxed);
    }
}

impl EntityBase for FireballEntity {
    /// Vanilla `Projectile.isPickable`: only redirectable projectiles.
    fn can_hit(&self) -> bool {
        self.get_entity()
            .entity_type
            .has_tag(&pumpkin_data::tag::EntityType::MINECRAFT_REDIRECTABLE_PROJECTILE)
    }

    fn get_projectile(&self) -> Option<&crate::entity::projectile::Projectile> {
        Some(&self.fireball.hurting.thrown.projectile)
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        self.fireball.write_nbt(nbt);
        nbt.put_byte("ExplosionPower", self.get_explosion_power() as i8);
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        self.fireball.read_nbt(nbt);
        self.set_explosion_power(f32::from(nbt.get_byte("ExplosionPower").unwrap_or(1)));
    }

    fn init_data_tracker(&self) {
        self.fireball.init_data_tracker();
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.fireball.tick(caller, server);
    }

    fn get_entity(&self) -> &Entity {
        self.fireball.get_entity()
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        let world = self.get_entity().world.load();

        let this = world.get_entity_by_id(self.get_entity().entity_id);
        // Vanilla `LargeFireball.onHitEntity`: `fireball(this, owner)` for 6.
        if let ProjectileHit::Entity { ref entity, .. } = hit {
            let owner = self.fireball.hurting.get_owner();
            let damage_source = DamageSource {
                damage_type: Fireball::damage_type(owner.as_deref()),
                direct: this.as_deref(),
                // `fireball`: an unowned fireball is its own cause.
                causing: owner.as_deref().or(this.as_deref()),
            };
            entity.damage_with_context(
                entity.as_ref(),
                6.0,
                damage_source.damage_type,
                None,
                damage_source.direct,
                damage_source.causing,
            );
            // Unconditional in vanilla, even when the hit did no damage.
            EnchantmentHelper::do_post_attack_effects(&world, entity.as_ref(), &damage_source);
        }

        // Vanilla `LargeFireball.onHit`: at its own position, setting fire with mob griefing.
        let grief = world.level_info.load().game_rules.mob_griefing;
        world.explode_full(
            this,
            None,
            None,
            self.get_entity().pos.load(),
            self.get_explosion_power(),
            grief,
            crate::world::ExplosionInteraction::Mob,
            crate::world::ExplosionEffects::DEFAULT,
        );
        self.get_entity().remove();
    }
}
