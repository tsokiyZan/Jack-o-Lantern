use std::sync::Arc;

use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::particle::Particle;
use pumpkin_data::world::WorldEvent;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use crate::entity::area_effect_cloud::AreaEffectCloudEntity;
use crate::entity::projectile::ProjectileHit;
use crate::entity::projectile::hurting_projectile::{HurtingProjectile, HurtingProjectileTick};
use crate::entity::{Entity, EntityBase};
use crate::server::Server;

/// Vanilla `DragonFireball.SPLASH_RANGE`.
pub const SPLASH_RANGE: f64 = 4.0;

/// Vanilla `DragonFireball`: the ender dragon's fireball, which leaves a cloud of dragon's
/// breath where it lands.
pub struct DragonFireballEntity {
    pub hurting: HurtingProjectile,
}

impl DragonFireballEntity {
    #[must_use]
    pub const fn new(entity: Entity) -> Self {
        Self {
            hurting: HurtingProjectile::new(entity),
        }
    }

    /// Vanilla `new DragonFireball(level, mob, direction)` followed by the dragon's
    /// `snapTo(x, y, z, 0, 0)`: `entity` is already at the start position.
    #[must_use]
    pub fn new_shot(entity: Entity, owner: &Entity, direction: Vector3<f64>) -> Self {
        let start = entity.pos.load();
        let fireball = Self {
            hurting: HurtingProjectile::new_from_mob(entity, owner, direction),
        };
        let entity = fireball.get_entity();
        entity.set_pos(start);
        entity.set_rotation(0.0, 0.0);
        fireball
    }
}

impl EntityBase for DragonFireballEntity {
    fn get_owner_id(&self) -> Option<i32> {
        self.hurting.get_owner_id()
    }

    fn set_owner_id(&self, owner: Option<i32>) {
        self.hurting.set_owner_id(owner);
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        self.hurting.write_nbt(nbt);
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        self.hurting.read_nbt(nbt);
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.hurting.tick(
            caller,
            server,
            &HurtingProjectileTick {
                should_burn: false,
                ..HurtingProjectileTick::default()
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

    /// Vanilla `DragonFireball.onHit`, at the point of impact.
    fn on_hit(&self, hit: ProjectileHit) {
        let owner_id = self.get_owner_id();
        if let ProjectileHit::Entity { ref entity, .. } = hit
            && Some(entity.get_entity().entity_id) == owner_id
        {
            return;
        }

        let entity = self.get_entity();
        let world = entity.world.load_full();
        let pos = hit.hit_pos();
        let dimensions = entity.entity_dimension.load();
        let search_box =
            BoundingBox::new_from_pos(pos.x, pos.y, pos.z, &dimensions).expand(4.0, 2.0, 4.0);
        let mut cloud_pos = pos;
        for candidate in world.get_all_at_box(&search_box) {
            if candidate.get_living_entity().is_some() {
                let candidate_pos = candidate.get_entity().pos.load();
                if candidate_pos.squared_distance_to_vec(&pos) < SPLASH_RANGE * SPLASH_RANGE {
                    cloud_pos = candidate_pos;
                    break;
                }
            }
        }

        let cloud = AreaEffectCloudEntity::with_vanilla_defaults(Entity::new(
            world.clone(),
            cloud_pos,
            &EntityType::AREA_EFFECT_CLOUD,
        ));
        // TODO: vanilla `setOwner(owner)` and `setPotionDurationScale(0.25F)`; Pumpkin's cloud
        // applies its effects without an owner and with a fixed duration scale.
        cloud.set_custom_particle(Particle::DragonBreath, Box::new(1.0f32.to_be_bytes()));
        cloud.set_radius(3.0);
        let duration = 600;
        *cloud
            .duration
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = duration;
        *cloud
            .radius_on_tick
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = (7.0 - 3.0) / duration as f32;
        cloud
            .effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((&StatusEffect::INSTANT_DAMAGE, 1, 1, false, true, true));

        let data = if entity.is_silent() { -1 } else { 1 };
        world.sync_world_event(
            WorldEvent::ParticlesDragonFireballSplash,
            BlockPos::floored_v(pos),
            data,
        );
        world.spawn_entity(Arc::new(cloud));
        entity.remove();
    }
}
