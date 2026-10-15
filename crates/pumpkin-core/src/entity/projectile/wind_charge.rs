use pumpkin_data::damage::DamageType;
use pumpkin_data::tag;
use pumpkin_util::math::vector3::Vector3;
use std::sync::LazyLock;
use std::{
    f64,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

use crate::{
    entity::{
        Entity, EntityBase,
        living::LivingEntity,
        projectile::{ProjectileHit, ThrownItemEntity},
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
    thrown_item_entity: ThrownItemEntity,
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
            thrown_item_entity,
        }
    }

    #[must_use]
    pub const fn new_breeze(thrown_item_entity: ThrownItemEntity) -> Self {
        Self {
            kind: WindChargeKind::Breeze,
            thrown_item_entity,
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
    fn get_owner_id(&self) -> Option<i32> {
        self.thrown_item_entity.owner_id.load()
    }

    fn set_owner_id(&self, owner: Option<i32>) {
        self.thrown_item_entity.owner_id.store(owner);
    }

    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        self.thrown_item_entity.process_tick(caller);

        if let Some(cooldown) = self.deflect_cooldown() {
            let cooldown_ticks = cooldown.load(Ordering::Relaxed);
            if cooldown_ticks > 0 {
                cooldown.store(cooldown_ticks - 1, Ordering::Relaxed);
            }
        }
    }

    fn get_entity(&self) -> &Entity {
        &self.thrown_item_entity.entity
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
                    .thrown_item_entity
                    .owner_id
                    .load()
                    .and_then(|id| world.get_entity_by_id(id))
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
                let _ = entity.damage_with_context(
                    entity.as_ref(),
                    1.0,
                    DamageType::WIND_CHARGE,
                    None,
                    Some(self.get_entity()),
                    owner.as_deref(),
                );
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
