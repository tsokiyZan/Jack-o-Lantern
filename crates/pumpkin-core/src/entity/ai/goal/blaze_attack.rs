use pumpkin_data::attributes::Attributes;
use pumpkin_data::world::WorldEvent;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use std::sync::Arc;

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{Controls, Goal},
    mob::Mob,
    mob::blaze::BlazeEntity,
    projectile::small_fireball::SmallFireballEntity,
};

pub struct BlazeShootFireballGoal {
    blaze: std::sync::Weak<BlazeEntity>,
    attack_step: i32,
    attack_time: i32,
    last_seen: i32,
}

impl BlazeShootFireballGoal {
    #[must_use]
    pub const fn new(blaze: std::sync::Weak<BlazeEntity>) -> Self {
        Self {
            blaze,
            attack_step: 0,
            attack_time: 0,
            last_seen: 0,
        }
    }

    fn get_follow_distance(blaze: &BlazeEntity) -> f64 {
        blaze
            .entity
            .living_entity
            .get_attribute_value(&Attributes::FOLLOW_RANGE)
    }
}

impl Goal for BlazeShootFireballGoal {
    fn can_start(&mut self, _mob: &dyn Mob) -> bool {
        let Some(blaze) = self.blaze.upgrade() else {
            return false;
        };
        let Some(target) = blaze.entity.get_target() else {
            return false;
        };
        target.get_entity().is_alive() && blaze.can_attack(target.as_ref())
    }

    fn start(&mut self, _mob: &dyn Mob) {
        self.attack_step = 0;
    }

    fn stop(&mut self, _mob: &dyn Mob) {
        if let Some(blaze) = self.blaze.upgrade() {
            blaze.set_charged(false);
        }
        self.last_seen = 0;
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.attack_time -= 1;

        let Some(blaze) = self.blaze.upgrade() else {
            return;
        };

        let target = blaze.entity.get_target();
        let Some(target) = target else {
            return;
        };

        let entity = &blaze.entity.living_entity.entity;
        let has_line_of_sight = mob.has_line_of_sight(target.get_entity());

        if has_line_of_sight {
            self.last_seen = 0;
        } else {
            self.last_seen += 1;
        }

        let blaze_pos = entity.pos.load();
        let target_entity = target.get_entity();
        let target_pos = target_entity.pos.load();

        let dx = target_pos.x - blaze_pos.x;
        let dz = target_pos.z - blaze_pos.z;
        let distance_sq = blaze_pos.squared_distance_to_vec(&target_pos);

        if distance_sq < 4.0 {
            if !has_line_of_sight {
                return;
            }

            if self.attack_time <= 0 {
                self.attack_time = 20;
                let damaged = blaze.entity.try_attack(&*blaze, target.as_ref());
                if damaged {
                    blaze.on_attack(target.as_ref());
                }
            }

            blaze
                .entity
                .move_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .set_wanted_position(target_pos.x, target_pos.y, target_pos.z, 1.0);
        } else if distance_sq < Self::get_follow_distance(&blaze).powi(2) && has_line_of_sight {
            let yd = (target_pos.y + f64::from(target_entity.entity_dimension.load().height) * 0.5)
                - (blaze_pos.y + f64::from(entity.entity_dimension.load().height) * 0.5);

            if self.attack_time <= 0 {
                self.attack_step += 1;
                if self.attack_step == 1 {
                    self.attack_time = 60;
                    blaze.set_charged(true);
                } else if self.attack_step <= 4 {
                    self.attack_time = 6;
                } else {
                    self.attack_time = 100;
                    self.attack_step = 0;
                    blaze.set_charged(false);
                }

                if self.attack_step > 1 {
                    shoot_fireball(&blaze, dx, yd, dz, distance_sq);
                }
            }

            blaze
                .entity
                .look_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .look_at_entity_with_range(&target, 10.0, 10.0);
        } else if self.last_seen < 5 {
            blaze
                .entity
                .move_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .set_wanted_position(target_pos.x, target_pos.y, target_pos.z, 1.0);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}

/// One fireball, aimed with a triangular spread that widens with the distance.
fn shoot_fireball(blaze: &BlazeEntity, dx: f64, yd: f64, dz: f64, distance_sq: f64) {
    let entity = blaze.get_entity();
    let sqd = distance_sq.sqrt().sqrt() * 0.5;
    let world = entity.world.load_full();
    if !entity.is_silent() {
        world.sync_world_event(WorldEvent::SoundBlazeFireball, entity.block_pos.load(), 0);
    }

    let direction = {
        let mut random = entity.random();
        let x = random.next_triangular(dx, 2.297 * sqd);
        let z = random.next_triangular(dz, 2.297 * sqd);
        Vector3::new(x, yd, z)
    };
    let base_entity = Entity::from_uuid(
        uuid::Uuid::new_v4(),
        world.clone(),
        entity.pos.load(),
        &pumpkin_data::entity::EntityType::SMALL_FIREBALL,
    );
    let fireball = SmallFireballEntity::new_from_mob(base_entity, entity, direction.normalize());
    let fireball_pos = fireball.get_entity().pos.load();
    fireball.get_entity().set_pos(Vector3::new(
        fireball_pos.x,
        entity.pos.load().y + f64::from(entity.height()) * 0.5 + 0.5,
        fireball_pos.z,
    ));
    world.spawn_entity(Arc::new(fireball));
}
