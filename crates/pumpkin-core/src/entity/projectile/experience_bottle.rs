use pumpkin_data::world::WorldEvent;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::projectile::ProjectileHit;
use crate::{
    entity::{Entity, EntityBase, projectile::ThrownItemEntity},
    server::Server,
};

/// Vanilla `ThrownExperienceBottle.getDefaultGravity`.
const GRAVITY: f64 = 0.07;

/// Vanilla `ThrownExperienceBottle`.
pub struct ExperienceBottleEntity {
    pub thrown: ThrownItemEntity,
}

impl ExperienceBottleEntity {
    /// A bottle with no thrower where `entity` stands, as `/summon`, a saved one or a
    /// dispenser makes it.
    pub const fn new(entity: Entity) -> Self {
        Self {
            thrown: ThrownItemEntity::unowned(entity, GRAVITY),
        }
    }

    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        Self {
            thrown: ThrownItemEntity::new(entity, shooter, GRAVITY),
        }
    }
}

impl EntityBase for ExperienceBottleEntity {
    /// Vanilla `ThrowableProjectile.canUsePortal`.
    fn can_use_portal(&self, _ignore_passenger: bool) -> bool {
        true
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

    /// Vanilla `ThrownExperienceBottle.onHit`: the splash, then 3 to 11 experience thrown
    /// back off the block face or against the bottle's flight.
    fn on_hit(&self, hit: ProjectileHit) {
        let entity = self.get_entity();
        let world = entity.world.load();
        let block_pos = entity.block_pos.load();
        world.sync_world_event(
            WorldEvent::ParticlesSpellPotionSplash,
            block_pos,
            -13_083_194,
        );
        if !entity.is_silent() {
            world.sync_world_event(WorldEvent::SoundSpellPotionSplash, block_pos, 0);
        }
        let xp_count = {
            let mut random = entity.random();
            3 + random.next_bounded_i32(5) + random.next_bounded_i32(5)
        };
        let direction = match &hit {
            ProjectileHit::Block { face, .. } => {
                let offset = face.to_offset();
                Vector3::new(
                    f64::from(offset.x),
                    f64::from(offset.y),
                    f64::from(offset.z),
                )
            }
            ProjectileHit::Entity { .. } => entity.velocity.load() * -1.0,
        };
        ExperienceOrbEntity::spawn_with_direction(
            &world,
            hit.hit_pos(),
            direction,
            xp_count as u32,
        );
    }
}
