use crate::entity::projectile::ProjectileHit;
use crate::{
    entity::{
        Entity, EntityBase, EntityType, mob::endermite::EndermiteEntity,
        projectile::ThrownItemEntity,
    },
    server::Server,
};
use pumpkin_data::damage::DamageType;
use pumpkin_data::dimension::Dimension;
use pumpkin_data::entity::{EntityPose, EntityStatus};
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use std::sync::atomic::Ordering;

const GRAVITY: f64 = 0.03;
const PARTICLE_COUNT: i32 = 32;
const ENDERMITE_SPAWN_CHANCE: f32 = 0.05;

pub struct EnderPearlEntity {
    pub thrown: ThrownItemEntity,
}

impl EnderPearlEntity {
    pub fn new(entity: Entity) -> Self {
        entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));

        let thrown = ThrownItemEntity::unowned(entity, GRAVITY);

        Self { thrown }
    }

    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        thrown.entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        Self { thrown }
    }
}

impl EnderPearlEntity {
    /// Vanilla `ThrownEnderpearl.isAllowedToTeleportOwner`: an owner in the pearl's level must
    /// be alive (a living one also awake); one in another level must be able to use a portal.
    fn is_allowed_to_teleport_owner(owner: &dyn EntityBase, level: &crate::world::World) -> bool {
        let owner_entity = owner.get_entity();
        if owner_entity.world.load().dimension == level.dimension {
            owner_entity.is_alive()
                && owner.get_living_entity().is_none_or(|living| {
                    living.health.load() > 0.0 && owner_entity.pose.load() != EntityPose::Sleeping
                })
        } else {
            owner.can_use_portal(true)
        }
    }
}

impl EntityBase for EnderPearlEntity {
    /// Vanilla `ThrowableProjectile.canUsePortal`.
    fn can_use_portal(&self, _ignore_passenger: bool) -> bool {
        true
    }

    /// Vanilla `ThrownEnderpearl.canTeleport`: out of the End, a player's pearl only goes once
    /// its owner has seen the credits.
    fn can_teleport(&self, from: &crate::world::World, to: &crate::world::World) -> bool {
        let base = self.get_entity().can_teleport_base(from, to);
        if from.dimension == Dimension::THE_END
            && to.dimension == Dimension::OVERWORLD
            && let Some(owner) = self.thrown.projectile.get_owner(&self.thrown.entity)
            && let Some(player) = owner.get_player()
        {
            return base && player.seen_credits.load(Ordering::Relaxed);
        }
        base
    }

    fn get_projectile(&self) -> Option<&crate::entity::projectile::Projectile> {
        Some(&self.thrown.projectile)
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
        let entity = self.get_entity();
        let world = entity.world.load();

        let attacker = self.thrown.projectile.get_owner(&self.thrown.entity);

        // Spawn portal particles at hit position
        let hit_pos = hit.hit_pos();
        // Vanilla `ThrownEnderpearl.onHit`: per particle `getY() + random.nextDouble() * 2.0`,
        // then `nextGaussian()` for the x and z speeds, from the pearl's random.
        for _ in 0..PARTICLE_COUNT {
            let (dy, speed_x, speed_z) = {
                let mut random = entity.random();
                let dy = random.next_f64() * 2.0;
                let speed_x = random.next_gaussian();
                let speed_z = random.next_gaussian();
                (dy, speed_x, speed_z)
            };
            let offset = Vector3::new(0.0, dy, 0.0);
            let speed = Vector3::new(speed_x, 0.0, speed_z);

            world.spawn_particle(
                hit_pos.add(&offset),
                Vector3::new(speed.x as f32, speed.y as f32, speed.z as f32),
                1.0,
                1,
                Particle::Portal,
            );
        }

        let owner = attacker.clone();
        let teleport_pos = entity.last_pos.load();

        if let (
            ProjectileHit::Entity {
                entity: hit_entity,
                hit_pos,
                ..
            },
            Some(owner),
        ) = (&hit, attacker)
        {
            let victim_ref = &**hit_entity;
            hit_entity.damage_with_context(
                victim_ref,
                0.0,
                DamageType::THROWN,
                Some(*hit_pos),
                Some(owner.get_entity()),
                Some(victim_ref),
            );
        }

        if let Some(owner) = owner
            && Self::is_allowed_to_teleport_owner(owner.as_ref(), &world)
        {
            // Vanilla only spawns endermites behind, and hurts, a teleported player.
            let is_player = owner.get_player().is_some();
            let should_spawn_endermite =
                is_player && entity.random().next_f32() < ENDERMITE_SPAWN_CHANCE;
            if world.should_spawn_monsters() && should_spawn_endermite {
                let entity = Entity::new(
                    world.clone(),
                    owner.get_entity().pos.load(),
                    &EntityType::ENDERMITE,
                );
                let endermite = EndermiteEntity::new(entity);
                world.spawn_entity(endermite);
            }

            // `if (isOnPortalCooldown()) owner.setPortalCooldown()`, for a player owner.
            if is_player && entity.portal_cooldown.load(Ordering::Relaxed) > 0 {
                owner
                    .get_entity()
                    .portal_cooldown
                    .store(owner.dimension_changing_delay(), Ordering::Relaxed);
            }

            // In vanilla, teleport handles everything including sound
            owner.teleport(
                teleport_pos,
                Some(owner.get_entity().yaw.load()),
                Some(owner.get_entity().pitch.load()),
                world.clone(),
            );

            // Vanilla `resetFallDistance` and `resetCurrentImpulseContext` on the teleported owner.
            owner.get_entity().reset_fall_distance();
            if let Some(living) = owner.get_living_entity() {
                living.reset_current_impulse_context();
            }

            // Play teleport sound at new position
            world.play_sound(
                Sound::EntityPlayerTeleport,
                SoundCategory::Players,
                &teleport_pos,
            );

            if is_player {
                owner.damage(
                    owner.as_ref(),
                    5.0,
                    pumpkin_data::damage::DamageType::ENDER_PEARL,
                );
            }
        }

        world.send_entity_status(entity, EntityStatus::Death, Some(ActorEventID::Death));
    }
}
