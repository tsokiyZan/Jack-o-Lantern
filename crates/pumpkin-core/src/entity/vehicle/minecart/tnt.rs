use std::sync::atomic::{AtomicI32, Ordering};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::entity::Entity;

pub(super) struct TntMinecart {
    fuse: AtomicI32,
    explosion_power: AtomicCell<f32>,
    explosion_speed_factor: AtomicCell<f32>,
}

impl TntMinecart {
    pub(super) const fn new() -> Self {
        Self {
            fuse: AtomicI32::new(-1),
            explosion_power: AtomicCell::new(4.0),
            explosion_speed_factor: AtomicCell::new(1.0),
        }
    }

    /// Vanilla `MinecartTNT.isPrimed`.
    pub(super) fn is_primed(&self) -> bool {
        self.fuse.load(Ordering::Relaxed) > -1
    }

    /// Vanilla `MinecartTNT.shouldBlockExplode` and the matching resistance override: a primed
    /// cart spares rails and the block under them.
    pub(super) fn spares_block(
        &self,
        world: &crate::world::World,
        pos: &pumpkin_util::math::position::BlockPos,
        block: &pumpkin_data::Block,
    ) -> bool {
        use pumpkin_data::tag::Taggable;
        self.is_primed()
            && (block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_RAILS)
                || world
                    .get_block(&pos.up())
                    .has_tag(&pumpkin_data::tag::Block::MINECRAFT_RAILS))
    }

    pub(super) fn prime(&self, entity: &Entity, fuse: i32) {
        if self.fuse.load(Ordering::Relaxed) >= 0
            || !entity
                .world
                .load()
                .level_info
                .load()
                .game_rules
                .tnt_explodes
        {
            return;
        }

        self.fuse.store(fuse, Ordering::Relaxed);
        let world = entity.world.load();
        world.send_entity_status(
            entity,
            EntityStatus::TntPrime,
            Some(ActorEventID::PrimeTNTCart),
        );
        world.play_sound(
            Sound::EntityTntPrimed,
            SoundCategory::Blocks,
            &entity.pos.load(),
        );
    }

    pub(super) fn tick(&self, entity: &Entity) -> bool {
        let fuse = self.fuse.load(Ordering::Relaxed);
        if fuse > 0 {
            self.fuse.store(fuse - 1, Ordering::Relaxed);
            let mut smoke_pos = entity.pos.load();
            smoke_pos.y += 0.5;
            entity.world.load().spawn_client_particle(
                smoke_pos,
                Vector3::new(0.0, 0.0, 0.0),
                0.0,
                1,
                Particle::Smoke,
            );
        } else if fuse == 0 {
            let velocity = entity.velocity.load();
            self.explode(
                entity,
                None,
                velocity.x * velocity.x + velocity.z * velocity.z,
            );
            return true;
        }
        false
    }

    /// Vanilla `MinecartTNT.explode(damageSource, speedSqr)`.
    pub(super) fn explode(
        &self,
        entity: &Entity,
        damage_source: Option<crate::world::ExplosionDamageSource>,
        horizontal_speed_squared: f64,
    ) {
        let world = entity.world.load();
        if !world.level_info.load().game_rules.tnt_explodes {
            if self.fuse.load(Ordering::Relaxed) > -1 {
                entity.remove();
            }
            return;
        }

        let power = Self::explosion_strength(
            self.explosion_power.load(),
            self.explosion_speed_factor.load(),
            horizontal_speed_squared,
            entity.random().next_f64(),
        );
        // The cart itself is the source, and goes only after the explosion, while it still
        // spares rails as primed.
        let source = world.get_entity_by_id(entity.entity_id);
        world.explode_full(
            source,
            damage_source,
            None,
            entity.pos.load(),
            power,
            false,
            crate::world::ExplosionInteraction::Tnt,
            crate::world::ExplosionEffects::DEFAULT,
        );
        entity.remove();
    }

    /// Vanilla `MinecartTNT.fuse`: `-1` until primed.
    pub(super) fn fuse(&self) -> i32 {
        self.fuse.load(Ordering::Relaxed)
    }

    pub(super) fn set_fuse(&self, fuse: i32) {
        self.fuse.store(fuse, Ordering::Relaxed);
    }

    pub(super) fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int("fuse", self.fuse.load(Ordering::Relaxed));
        let power = self.explosion_power.load();
        if power != 4.0 {
            nbt.put_float("explosion_power", power);
        }
        let speed_factor = self.explosion_speed_factor.load();
        if speed_factor != 1.0 {
            nbt.put_float("explosion_speed_factor", speed_factor);
        }
    }

    pub(super) fn read_nbt(&self, nbt: &NbtCompound) {
        self.fuse
            .store(nbt.get_int("fuse").unwrap_or(-1), Ordering::Relaxed);
        self.explosion_power.store(
            nbt.get_float("explosion_power")
                .unwrap_or(4.0)
                .clamp(0.0, 128.0),
        );
        self.explosion_speed_factor.store(
            nbt.get_float("explosion_speed_factor")
                .unwrap_or(1.0)
                .clamp(0.0, 128.0),
        );
    }

    fn explosion_strength(
        base: f32,
        speed_factor: f32,
        horizontal_speed_squared: f64,
        random: f64,
    ) -> f32 {
        // Vanilla `(float) (explosionPowerBase + explosionSpeedFactor * random.nextDouble() * 1.5 * speed)`,
        // in double.
        let speed = horizontal_speed_squared.sqrt().min(5.0);
        (f64::from(base) + f64::from(speed_factor) * random * 1.5 * speed) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::TntMinecart;

    #[test]
    fn tnt_minecart_explosion_bonus_is_speed_capped() {
        assert_eq!(TntMinecart::explosion_strength(4.0, 1.0, 100.0, 1.0), 11.5);
        assert_eq!(TntMinecart::explosion_strength(4.0, 1.0, 100.0, 0.0), 4.0);
    }
}
