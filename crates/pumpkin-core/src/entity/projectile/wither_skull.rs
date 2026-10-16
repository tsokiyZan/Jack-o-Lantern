use std::sync::atomic::{AtomicBool, Ordering};

use pumpkin_data::{damage::DamageType, effect::StatusEffect, potion::Effect, tracked_data};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{Difficulty, math::vector3::Vector3};

use crate::{
    entity::{
        Entity, EntityBase,
        projectile::{
            ProjectileHit,
            hurting_projectile::{HurtingProjectile, HurtingProjectileTick},
        },
    },
    server::Server,
    world::ExplosionInteraction,
};

/// Vanilla `WitherSkull.getInertia` for a dangerous (blue) skull, a float.
const DANGEROUS_INERTIA: f32 = 0.73;

/// Vanilla `WitherSkull`.
pub struct WitherSkullEntity {
    pub hurting: HurtingProjectile,
    pub dangerous: AtomicBool,
}

impl WitherSkullEntity {
    #[must_use]
    pub const fn new(entity: Entity) -> Self {
        Self {
            hurting: HurtingProjectile::new(entity),
            dangerous: AtomicBool::new(false),
        }
    }

    /// Vanilla `WitherBoss.performRangedAttack`'s skull: `entity` is at the head.
    #[must_use]
    pub fn new_shot(
        entity: Entity,
        shooter: &Entity,
        dangerous: bool,
        direction: Vector3<f64>,
    ) -> Self {
        let head = entity.pos.load();
        let skull = Self {
            hurting: HurtingProjectile::new_from_mob(entity, shooter, direction),
            dangerous: AtomicBool::new(false),
        };
        if dangerous {
            skull.set_dangerous(true);
        }
        skull.get_entity().set_pos(head);
        skull
    }

    #[must_use]
    pub fn is_dangerous(&self) -> bool {
        self.dangerous.load(Ordering::Relaxed)
    }

    pub fn set_dangerous(&self, dangerous: bool) {
        self.dangerous.store(dangerous, Ordering::Relaxed);
        self.hurting
            .get_entity()
            .set_synced_data(tracked_data::wither_skull::DATA_DANGEROUS, dangerous);
    }
}

impl EntityBase for WitherSkullEntity {
    fn get_owner_id(&self) -> Option<i32> {
        self.hurting.get_owner_id()
    }

    fn set_owner_id(&self, owner: Option<i32>) {
        self.hurting.set_owner_id(owner);
    }

    /// Vanilla `WitherSkull.getBlockExplosionResistance`: a blue skull breaks what the wither
    /// can.
    fn get_block_explosion_resistance(
        &self,
        _world: &crate::world::World,
        _pos: &pumpkin_util::math::position::BlockPos,
        block: &pumpkin_data::Block,
        resistance: f32,
    ) -> f32 {
        if self.is_dangerous() && crate::entity::boss::wither::WitherEntity::can_destroy(block) {
            resistance.min(0.8)
        } else {
            resistance
        }
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        self.hurting.write_nbt(nbt);
        nbt.put_bool("dangerous", self.is_dangerous());
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        self.hurting.read_nbt(nbt);
        self.dangerous.store(
            nbt.get_bool("dangerous").unwrap_or(false),
            Ordering::Relaxed,
        );
    }

    fn init_data_tracker(&self) {
        let entity = self.get_entity();
        entity.set_synced_data(
            tracked_data::wither_skull::DATA_DANGEROUS,
            self.is_dangerous(),
        );
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        let defaults = HurtingProjectileTick::default();
        self.hurting.tick(
            caller,
            server,
            &HurtingProjectileTick {
                inertia: if self.is_dangerous() {
                    DANGEROUS_INERTIA
                } else {
                    defaults.inertia
                },
                should_burn: false,
                ..defaults
            },
        );
    }

    fn get_entity(&self) -> &Entity {
        self.hurting.get_entity()
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        let world = self.get_entity().world.load();

        if let ProjectileHit::Entity { ref entity, .. } = hit {
            let difficulty = world.level_info.load().difficulty;

            // Vanilla `WitherSkull.onHitEntity`: `witherSkull(this, owner)` for 8 with a living
            // owner, who heals 5 on a kill; plain `magic` for 5 without one.
            let this = world.get_entity_by_id(self.get_entity().entity_id);
            let owner = self
                .get_owner_id()
                .and_then(|id| world.get_entity_by_id(id))
                .filter(|owner| owner.get_living_entity().is_some());
            let was_hurt = owner.as_ref().map_or_else(
                || entity.damage(entity.as_ref(), 5.0, DamageType::MAGIC),
                |owner| {
                    let hurt = entity.damage_with_context(
                        entity.as_ref(),
                        8.0,
                        DamageType::WITHER_SKULL,
                        None,
                        this.as_deref(),
                        Some(owner.as_ref()),
                    );
                    // `isAlive`: a living target with no health left counts as dead.
                    let alive = entity.get_living_entity().map_or_else(
                        || entity.get_entity().is_alive(),
                        crate::entity::living::LivingEntity::is_alive,
                    );
                    // TODO: vanilla `EnchantmentHelper.doPostAttackEffects` when the target lives.
                    if hurt
                        && !alive
                        && let Some(living_owner) = owner.get_living_entity()
                    {
                        living_owner.heal(5.0);
                    }
                    hurt
                },
            );

            if was_hurt && let Some(living) = entity.get_living_entity() {
                let duration = match difficulty {
                    Difficulty::Hard => 800,   // 40 seconds
                    Difficulty::Normal => 200, // 10 seconds
                    Difficulty::Easy | Difficulty::Peaceful => 0,
                };

                if duration > 0 {
                    let effect = Effect {
                        effect_type: &StatusEffect::WITHER,
                        duration,
                        amplifier: 1,
                        ambient: false,
                        show_particles: true,
                        show_icon: true,
                        blend: true,
                    };
                    if let Some(player) = entity.get_player() {
                        player.add_effect(effect);
                    } else {
                        living.add_effect(effect);
                    }
                }
            }
        }

        // Vanilla `WitherSkull.onHit`: at the skull's own position, with the skull as source.
        let this = world.get_entity_by_id(self.get_entity().entity_id);
        world.explode(
            this,
            self.get_entity().pos.load(),
            1.0,
            ExplosionInteraction::Mob,
        );
        self.get_entity().remove();
    }
}
