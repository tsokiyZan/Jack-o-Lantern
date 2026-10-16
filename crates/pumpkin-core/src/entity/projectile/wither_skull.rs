use std::sync::atomic::{AtomicBool, Ordering};

use pumpkin_data::{damage::DamageType, effect::StatusEffect, potion::Effect, tracked_data};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{Difficulty, math::vector3::Vector3};

use crate::{
    entity::{
        Entity, EntityBase,
        projectile::{ProjectileHit, ThrownItemEntity},
    },
    server::Server,
    world::ExplosionInteraction,
};

const GRAVITY: f64 = 0.0;

pub struct WitherSkullEntity {
    pub thrown: ThrownItemEntity,
    pub dangerous: AtomicBool,
}

impl WitherSkullEntity {
    #[must_use]
    pub const fn new(entity: Entity) -> Self {
        let thrown = ThrownItemEntity {
            entity,
            owner_id: crossbeam::atomic::AtomicCell::new(None),
            collides_with_projectiles: false,
            has_hit: AtomicBool::new(false),
            last_deflected_by: crossbeam::atomic::AtomicCell::new(None),
            gravity: GRAVITY,
            left_owner: std::sync::atomic::AtomicBool::new(false),
        };

        Self {
            thrown,
            dangerous: AtomicBool::new(false),
        }
    }

    #[must_use]
    pub fn new_shot(
        entity: Entity,
        shooter: &Entity,
        dangerous: bool,
        direction: Vector3<f64>,
    ) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        let speed = 0.95;
        let vel = direction.normalize().multiply(speed, speed, speed);
        thrown.entity.velocity.store(vel);

        let len = vel.horizontal_length();
        thrown.entity.set_rotation(
            vel.x.atan2(vel.z) as f32 * 57.295_776,
            vel.y.atan2(len) as f32 * 57.295_776,
        );

        Self {
            thrown,
            dangerous: AtomicBool::new(dangerous),
        }
    }

    #[must_use]
    pub fn is_dangerous(&self) -> bool {
        self.dangerous.load(Ordering::Relaxed)
    }

    pub fn set_dangerous(&self, dangerous: bool) {
        self.dangerous.store(dangerous, Ordering::Relaxed);
        self.thrown
            .entity
            .set_synced_data(tracked_data::wither_skull::DATA_DANGEROUS, dangerous);
    }
}

impl EntityBase for WitherSkullEntity {
    fn get_owner_id(&self) -> Option<i32> {
        self.thrown.owner_id.load()
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
        nbt.put_bool("dangerous", self.is_dangerous());
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        if let Some(dangerous) = nbt.get_bool("dangerous") {
            self.dangerous.store(dangerous, Ordering::Relaxed);
        }
    }

    fn init_data_tracker(&self) {
        let entity = self.get_entity();
        entity.set_synced_data(
            tracked_data::wither_skull::DATA_DANGEROUS,
            self.is_dangerous(),
        );
    }

    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        self.thrown.process_tick(caller);
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
                    if hurt
                        && !entity.get_entity().is_alive()
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
    }
}
