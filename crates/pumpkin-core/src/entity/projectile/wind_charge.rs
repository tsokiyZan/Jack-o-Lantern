use pumpkin_data::damage::DamageType;
use pumpkin_data::tag;
use pumpkin_data::tag::Taggable;
use pumpkin_util::math::vector3::Vector3;
use std::sync::LazyLock;
use std::{
    f64,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

use crate::enchantment::EnchantmentHelper;
use crate::entity::damage_source::DamageSource;
use crate::{
    entity::{
        Entity, EntityBase,
        living::LivingEntity,
        projectile::{
            ProjectileHit, ThrownItemEntity,
            hurting_projectile::{HurtingProjectile, HurtingProjectileTick},
        },
        projectile_deflection::ProjectileDeflectionType,
    },
    server::Server,
    world::SimpleExplosionDamageCalculator,
};

const DEFAULT_DEFLECT_COOLDOWN: u8 = 5;
pub const WIND_CHARGE_GRAVITY: f64 = 0.0;

enum WindChargeKind {
    Normal { deflect_cooldown: AtomicU8 },
    Breeze,
}

pub struct WindChargeEntity {
    kind: WindChargeKind,
    /// `AbstractWindCharge extends AbstractHurtingProjectile`.
    hurting: HurtingProjectile,
}

pub static WIND_CHARGE_EXPLOSION_DAMAGE_CALCULATOR: LazyLock<Arc<SimpleExplosionDamageCalculator>> =
    LazyLock::new(|| {
        Arc::new(SimpleExplosionDamageCalculator::new(
            true,
            false,
            Some(1.22),
            Some(&tag::Block::MINECRAFT_BLOCKS_WIND_CHARGE_EXPLOSIONS),
        ))
    });

pub static BREEZE_WIND_CHARGE_EXPLOSION_DAMAGE_CALCULATOR: LazyLock<
    Arc<SimpleExplosionDamageCalculator>,
> = LazyLock::new(|| {
    Arc::new(SimpleExplosionDamageCalculator::new(
        true,
        false,
        None,
        Some(&tag::Block::MINECRAFT_BLOCKS_WIND_CHARGE_EXPLOSIONS),
    ))
});

impl WindChargeEntity {
    #[must_use]
    pub const fn new_normal(thrown_item_entity: ThrownItemEntity) -> Self {
        Self {
            kind: WindChargeKind::Normal {
                deflect_cooldown: AtomicU8::new(DEFAULT_DEFLECT_COOLDOWN),
            },
            // `AbstractWindCharge`'s constructors: `accelerationPower = 0.0`.
            hurting: HurtingProjectile::with_thrown(thrown_item_entity, 0.0),
        }
    }

    #[must_use]
    pub const fn new_breeze(thrown_item_entity: ThrownItemEntity) -> Self {
        Self {
            kind: WindChargeKind::Breeze,
            hurting: HurtingProjectile::with_thrown(thrown_item_entity, 0.0),
        }
    }

    pub const fn deflect_cooldown(&self) -> Option<&AtomicU8> {
        if let WindChargeKind::Normal {
            deflect_cooldown, ..
        } = &self.kind
        {
            Some(deflect_cooldown)
        } else {
            None
        }
    }

    pub fn create_explosion(&self, position: Vector3<f64>) {
        let (power, calculator, effects) = match self.kind {
            WindChargeKind::Normal { .. } => (
                1.2,
                WIND_CHARGE_EXPLOSION_DAMAGE_CALCULATOR.clone(),
                crate::world::ExplosionEffects::WIND_CHARGE,
            ),
            WindChargeKind::Breeze => (
                3.0,
                BREEZE_WIND_CHARGE_EXPLOSION_DAMAGE_CALCULATOR.clone(),
                crate::world::ExplosionEffects::BREEZE_WIND_CHARGE,
            ),
        };
        let world = self.get_entity().world.load();
        world.explode_full(
            world.get_entity_by_id(self.get_entity().entity_id),
            None,
            Some(calculator),
            position,
            power,
            false,
            crate::world::ExplosionInteraction::Trigger,
            effects,
        );
    }

    pub fn deflect(
        &mut self,
        deflection: &ProjectileDeflectionType,
        deflector: Option<&dyn EntityBase>,
    ) -> bool {
        if let Some(cooldown) = self.deflect_cooldown()
            && cooldown.load(Ordering::Relaxed) > 0
        {
            return false;
        }

        deflection.deflect(self, deflector);
        true
    }
}

impl EntityBase for WindChargeEntity {
    /// Vanilla `WindCharge.deflect`: not within its first ticks (`noDeflectTicks`); then
    /// `Projectile.deflect(AIM_DEFLECT, ...)`, along the attacker's look and owned by it, and
    /// `AbstractHurtingProjectile.onDeflection(true)`.
    fn deflect_by_attack(&self, deflector: &dyn EntityBase, power: f64) -> bool {
        if let Some(cooldown) = self.deflect_cooldown()
            && cooldown.load(Ordering::Relaxed) > 0
        {
            return false;
        }
        let entity = self.get_entity();
        let deflector_entity = deflector.get_entity();
        entity
            .velocity
            .store(deflector_entity.get_look_angle() * power);
        entity.velocity_dirty.store(true, Ordering::SeqCst);
        self.hurting
            .thrown
            .projectile
            .set_owner(Some(deflector_entity));
        self.hurting.on_deflection(true);
        true
    }

    /// Vanilla `AbstractWindCharge.canCollideWith`: wind charges pass through each other.
    fn can_collide_with(&self, caller: &dyn EntityBase, other: &dyn EntityBase) -> bool {
        let other_type = other.get_entity().entity_type;
        other_type != &pumpkin_data::entity::EntityType::WIND_CHARGE
            && other_type != &pumpkin_data::entity::EntityType::BREEZE_WIND_CHARGE
            && other.is_collidable(Some(caller))
            && caller.get_entity().root_vehicle_id() != other.get_entity().root_vehicle_id()
    }

    /// Vanilla `Projectile.isPickable`: only redirectable projectiles.
    fn can_hit(&self) -> bool {
        self.get_entity()
            .entity_type
            .has_tag(&pumpkin_data::tag::EntityType::MINECRAFT_REDIRECTABLE_PROJECTILE)
    }

    /// `AbstractHurtingProjectile.addAdditionalSaveData`: `acceleration_power`.
    fn write_custom_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.hurting.write_nbt(nbt);
    }

    /// `AbstractHurtingProjectile.readAdditionalSaveData`: `acceleration_power`, 0.1 when absent.
    fn read_custom_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.hurting.read_nbt(nbt);
    }

    fn get_projectile(&self) -> Option<&crate::entity::projectile::Projectile> {
        Some(&self.hurting.thrown.projectile)
    }

    /// Vanilla `WindCharge.tick`: `AbstractWindCharge.tick` (an explosion once 30 blocks above
    /// the build height, else `AbstractHurtingProjectile.tick` with inertia 1 and no burning),
    /// then the `noDeflectTicks` count-down.
    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        let entity = self.get_entity();
        if entity.block_pos.load().0.y > entity.world.load().get_top_y() + 30 {
            self.create_explosion(entity.pos.load());
            entity.remove();
        } else {
            self.hurting.tick(
                caller,
                server,
                &HurtingProjectileTick {
                    inertia: 1.0,
                    liquid_inertia: 1.0,
                    should_burn: false,
                },
            );
        }

        if let Some(cooldown) = self.deflect_cooldown() {
            let cooldown_ticks = cooldown.load(Ordering::Relaxed);
            if cooldown_ticks > 0 {
                cooldown.store(cooldown_ticks - 1, Ordering::Relaxed);
            }
        }
    }

    fn get_entity(&self) -> &Entity {
        &self.hurting.thrown.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        match hit {
            // Vanilla `AbstractWindCharge.onHitEntity`: `windCharge(this, owner)` for 1, then
            // the burst at the charge itself.
            ProjectileHit::Entity { ref entity, .. } => {
                let world = self.get_entity().world.load();
                let owner = self
                    .hurting
                    .thrown
                    .projectile
                    .get_owner(&self.hurting.thrown.entity)
                    .filter(|owner| owner.get_living_entity().is_some());
                if let Some(owner_living) = owner.as_ref().and_then(|o| o.get_living_entity()) {
                    // `setLastHurtMob`.
                    if entity.get_living_entity().is_some() {
                        owner_living
                            .last_attacking_id
                            .store(entity.get_entity().entity_id, Ordering::Relaxed);
                        owner_living.last_attack_time.store(
                            owner_living.entity.tick_count.load(Ordering::Relaxed),
                            Ordering::Relaxed,
                        );
                    }
                }
                let damage_source = DamageSource {
                    damage_type: DamageType::WIND_CHARGE,
                    direct: Some(self.get_entity()),
                    causing: owner.as_deref(),
                };
                if entity.damage_with_context(
                    entity.as_ref(),
                    1.0,
                    damage_source.damage_type,
                    None,
                    damage_source.direct,
                    damage_source.causing,
                ) && entity.get_living_entity().is_some()
                {
                    EnchantmentHelper::do_post_attack_effects(
                        &world,
                        entity.as_ref(),
                        &damage_source,
                    );
                }
                self.create_explosion(self.get_entity().pos.load());
            }
            // Vanilla `AbstractWindCharge.onHitBlock`: a quarter block out of the hit face.
            ProjectileHit::Block { face, hit_pos, .. } => {
                let normal = face.to_offset();
                self.create_explosion(Vector3::new(
                    hit_pos.x + f64::from(normal.x) * 0.25,
                    hit_pos.y + f64::from(normal.y) * 0.25,
                    hit_pos.z + f64::from(normal.z) * 0.25,
                ));
            }
        }
    }
}
