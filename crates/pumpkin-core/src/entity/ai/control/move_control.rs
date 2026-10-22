use crate::entity::Entity;
use crate::entity::ai::control::{Control, MoveControlTrait, rotlerp};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::living::LivingEntity;
use crate::entity::mob::Mob;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::{self, position::BlockPos, vector3::Vector3};
use std::sync::atomic::Ordering;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    #[default]
    Wait,
    MoveTo,
    Strafe,
    Jumping,
}

pub struct MoveControl {
    pub wanted_x: f64,
    pub wanted_y: f64,
    pub wanted_z: f64,
    pub speed_modifier: f64,
    pub strafe_forwards: f32,
    pub strafe_right: f32,
    pub operation: Operation,
}

impl Default for MoveControl {
    fn default() -> Self {
        Self {
            wanted_x: 0.0,
            wanted_y: 0.0,
            wanted_z: 0.0,
            speed_modifier: 0.0,
            strafe_forwards: 0.0,
            strafe_right: 0.0,
            operation: Operation::Wait,
        }
    }
}

impl Control for MoveControl {}

impl MoveControlTrait for MoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        let living_entity = &mob_entity.living_entity;
        let entity = &living_entity.entity;
        match self.operation {
            Operation::Strafe => {
                let speed = living_entity.get_attribute_value(&Attributes::MOVEMENT_SPEED) as f32;
                let speed_modified = self.speed_modifier as f32 * speed;
                let mut xa = self.strafe_forwards;
                let mut za = self.strafe_right;
                #[expect(
                    clippy::imprecise_flops,
                    reason = "vanilla rounds Mth.sqrt(xa * xa + za * za), not a hypot"
                )]
                let mut dist = (xa * xa + za * za).sqrt();
                if dist < 1.0 {
                    dist = 1.0;
                }
                dist = speed_modified / dist;
                xa *= dist;
                za *= dist;
                let yaw_radians = entity.yaw.load() * (std::f64::consts::PI / 180.0) as f32;
                let sin = math::sin(yaw_radians);
                let cos = math::cos(yaw_radians);
                let dx = xa * cos - za * sin;
                let dz = za * cos + xa * sin;
                if !Self::is_walkable(mob, dx, dz) {
                    self.strafe_forwards = 1.0;
                    self.strafe_right = 0.0;
                }
                mob_entity.set_speed(speed_modified);
                let input = living_entity.movement_input.load();
                living_entity.movement_input.store(Vector3::new(
                    f64::from(self.strafe_right),
                    input.y,
                    f64::from(self.strafe_forwards),
                ));
                self.operation = Operation::Wait;
            }
            Operation::MoveTo => {
                self.operation = Operation::Wait;
                let pos = entity.pos.load();
                let xd = self.wanted_x - pos.x;
                let zd = self.wanted_z - pos.z;
                let yd = self.wanted_y - pos.y;
                let dd = xd * xd + yd * yd + zd * zd;
                if dd < f64::from(MIN_SPEED_SQR) {
                    set_zza(living_entity, 0.0);
                    return;
                }

                let y_rot_d = (math::atan2(zd, xd) * f64::from(180.0f32)
                    / f64::from(std::f32::consts::PI)) as f32
                    - 90.0;
                entity
                    .yaw
                    .store(rotlerp(entity.yaw.load(), y_rot_d, MAX_TURN));
                mob_entity.set_speed(
                    (self.speed_modifier
                        * living_entity.get_attribute_value(&Attributes::MOVEMENT_SPEED))
                        as f32,
                );

                // `mob.maxUpStep()`; a player-controlled mob never steers through this.
                let max_up_step =
                    living_entity.get_attribute_value(&Attributes::STEP_HEIGHT) as f32;
                let width = entity.entity_dimension.load().width;
                if yd > f64::from(max_up_step) && xd * xd + zd * zd < f64::from(width.max(1.0))
                    || Self::is_on_step_to_jump(entity)
                {
                    mob_entity.jump();
                    self.operation = Operation::Jumping;
                }
            }
            Operation::Jumping => {
                mob_entity.set_speed(
                    (self.speed_modifier
                        * living_entity.get_attribute_value(&Attributes::MOVEMENT_SPEED))
                        as f32,
                );
                // Mobs are always affected by fluids.
                if entity.on_ground.load(Ordering::Relaxed)
                    || entity.touching_water.load(Ordering::Relaxed)
                    || entity.touching_lava.load(Ordering::Relaxed)
                {
                    self.operation = Operation::Wait;
                }
            }
            Operation::Wait => set_zza(living_entity, 0.0),
        }
    }

    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed_modifier: f64) {
        self.wanted_x = x;
        self.wanted_y = y;
        self.wanted_z = z;
        self.speed_modifier = speed_modifier;
        if self.operation != Operation::Jumping {
            self.operation = Operation::MoveTo;
        }
    }

    fn strafe(&mut self, forwards: f32, right: f32) {
        self.operation = Operation::Strafe;
        self.strafe_forwards = forwards;
        self.strafe_right = right;
        self.speed_modifier = 0.25;
    }

    fn has_wanted(&self) -> bool {
        self.operation == Operation::MoveTo
    }
}

/// `MoveControl.MIN_SPEED_SQR`.
const MIN_SPEED_SQR: f32 = 2.500_000_3E-7;
/// `MoveControl.MAX_TURN`.
const MAX_TURN: f32 = 90.0;

/// Vanilla `Mob.setZza`.
fn set_zza(living_entity: &LivingEntity, zza: f32) {
    let input = living_entity.movement_input.load();
    living_entity
        .movement_input
        .store(Vector3::new(input.x, input.y, f64::from(zza)));
}

impl MoveControl {
    /// The second half of `MOVE_TO`'s jump test: the block the mob stands in has a collision
    /// shape whose top is above the mob, and is not a door or fence.
    fn is_on_step_to_jump(entity: &Entity) -> bool {
        let block_pos = entity.block_pos.load();
        let world = entity.world.load();
        let (block, state) = world.get_block_and_state(&block_pos);
        if block.has_tag(&tag::Block::MINECRAFT_DOORS)
            || block.has_tag(&tag::Block::MINECRAFT_FENCES)
        {
            return false;
        }
        let max_y = state
            .get_block_collision_shapes_at(&block_pos)
            .map(|shape| shape.max.y)
            .reduce(f64::max);
        max_y.is_some_and(|max_y| entity.pos.load().y < max_y + f64::from(block_pos.0.y))
    }

    /// `MoveControl.isWalkable`: whether the block the strafe heads into is walkable for the
    /// mob's node evaluator.
    fn is_walkable(mob: &dyn Mob, dx: f32, dz: f32) -> bool {
        let entity = &mob.get_mob_entity().living_entity.entity;
        let pos = entity.pos.load();
        let target = BlockPos::floored(
            pos.x + f64::from(dx),
            f64::from(entity.block_pos.load().0.y),
            pos.z + f64::from(dz),
        );
        mob.get_mob_entity()
            .own_navigation()
            .get_path_type_at(entity, target)
            .is_none_or(|path_type| path_type == PathType::Walkable)
    }

    #[must_use]
    pub fn has_wanted(&self) -> bool {
        self.operation == Operation::MoveTo
    }

    #[must_use]
    pub const fn get_speed_modifier(&self) -> f64 {
        self.speed_modifier
    }

    pub fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed_modifier: f64) {
        self.wanted_x = x;
        self.wanted_y = y;
        self.wanted_z = z;
        self.speed_modifier = speed_modifier;
        if self.operation != Operation::Jumping {
            self.operation = Operation::MoveTo;
        }
    }

    pub const fn strafe(&mut self, forwards: f32, right: f32) {
        self.operation = Operation::Strafe;
        self.strafe_forwards = forwards;
        self.strafe_right = right;
        self.speed_modifier = 0.25;
    }
}
