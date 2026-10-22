use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_data::world::WorldEvent;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::atan2;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::{BlockPos, for_each_block_intersected_between};
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::chunk::ChunkHeightmapType;

use crate::entity::ai::control::move_control::Operation;
use crate::entity::ai::control::{Control, MoveControlTrait};
use crate::entity::ai::goal::active_target::ActiveTargetGoal;
use crate::entity::living::LivingEntity;
use crate::entity::projectile::fireball::FireballEntity;
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{Controls, Goal},
    mob::{Mob, MobEntity},
};
use crate::world::World;

/// `180.0F / (float) Math.PI`, vanilla's float radians-to-degrees factor.
pub(crate) const RAD_TO_DEG: f32 = 180.0 / std::f32::consts::PI;
/// Vanilla's `maxDist = 64.0` in `faceMovementDirection` and the fireball goal, squared.
const MAX_TARGET_DISTANCE_SQ: f64 = 4096.0;

pub struct GhastEntity {
    pub mob_entity: MobEntity,
    pub is_charging: AtomicBool,
    pub explosion_power: AtomicU8,
}

impl GhastEntity {
    pub const DEFAULT_EXPLOSION_POWER: u8 = 1;
    pub const XP_REWARD: u32 = 5;

    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        mob_entity.set_own_move_control(Box::new(GhastMoveControl::new(false, |_| false)));
        let ghast = Self {
            mob_entity,
            is_charging: AtomicBool::new(false),
            explosion_power: AtomicU8::new(Self::DEFAULT_EXPLOSION_POWER),
        };

        let mob_arc = Arc::new(ghast);

        // Vanilla `Ghast.registerGoals`.
        mob_arc
            .mob_entity
            .add_goal(5, RandomFloatAroundGoal::new(0));
        mob_arc.mob_entity.add_goal(7, GhastLookGoal);
        mob_arc
            .mob_entity
            .add_goal(7, GhastShootFireballGoal::default());
        let ghast = Arc::downgrade(&mob_arc);
        mob_arc.mob_entity.add_target_goal(
            1,
            ActiveTargetGoal::new(
                &mob_arc.mob_entity,
                &EntityType::PLAYER,
                10,
                true,
                false,
                Some(move |target: &LivingEntity, _world: &World| {
                    ghast.upgrade().is_some_and(|ghast| {
                        (target.entity.pos.load().y - ghast.get_entity().pos.load().y).abs() <= 4.0
                    })
                }),
            ),
        );

        mob_arc
    }

    pub fn set_charging(&self, charging: bool) {
        self.is_charging.store(charging, Ordering::Relaxed);
        let entity = &self.mob_entity.living_entity.entity;
        entity.set_synced_data(tracked_data::ghast::DATA_IS_CHARGING, charging);
    }

    #[must_use]
    pub fn is_charging(&self) -> bool {
        self.is_charging.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn get_explosion_power(&self) -> u8 {
        self.explosion_power.load(Ordering::Relaxed)
    }

    pub fn set_explosion_power(&self, power: u8) {
        self.explosion_power.store(power, Ordering::Relaxed);
    }

    #[must_use]
    pub fn check_ghast_spawn_rules(world: &World, pos: &BlockPos) -> bool {
        if world.level_info.load().difficulty == pumpkin_util::Difficulty::Peaceful {
            return false;
        }
        if rand::random_range(0..20) != 0 {
            return false;
        }
        let state = world.get_block_state(pos);
        state.is_air()
    }
}

impl Mob for GhastEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `Ghast.travel`.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        self.mob_entity.living_entity.travel_flying(caller, 0.02);
        true
    }

    /// Vanilla `Ghast.checkFallDamage` does nothing.
    fn mob_check_fall_damage(
        &self,
        _caller: &dyn crate::entity::EntityBase,
        _ya: f64,
        _on_ground: bool,
        _state: &'static pumpkin_data::BlockState,
        _pos: pumpkin_util::math::position::BlockPos,
    ) {
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        if self.is_charging() {
            entity.set_synced_data(tracked_data::ghast::DATA_IS_CHARGING, true);
        }
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        let power = self.get_explosion_power();
        nbt.put_byte("ExplosionPower", i8::try_from(power).unwrap_or(1));
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if let Some(power) = nbt.get_byte("ExplosionPower") {
            self.set_explosion_power(u8::try_from(power).unwrap_or(Self::DEFAULT_EXPLOSION_POWER));
        }
    }

    fn modify_incoming_damage(&self, amount: f32, damage_type: DamageType) -> f32 {
        if damage_type.id == DamageType::FIREBALL.id {
            1000.0
        } else {
            amount
        }
    }

    fn xp_reward(&self) -> u32 {
        Self::XP_REWARD
    }
}

/// Vanilla `Ghast.faceMovementDirection`: face the target within 64 blocks, otherwise the
/// way the mob is drifting.
pub fn face_movement_direction(mob: &dyn Mob) {
    let mob_entity = mob.get_mob_entity();
    let entity = &mob_entity.living_entity.entity;
    let pos = entity.pos.load();
    let y_rot = if let Some(target) = mob.get_target() {
        let target_pos = target.get_entity().pos.load();
        if target_pos.squared_distance_to_vec(&pos) >= MAX_TARGET_DISTANCE_SQ {
            return;
        }
        -(atan2(target_pos.x - pos.x, target_pos.z - pos.z) as f32) * RAD_TO_DEG
    } else {
        let movement = entity.velocity.load();
        -(atan2(movement.x, movement.z) as f32) * RAD_TO_DEG
    };
    entity.yaw.store(y_rot);
    entity.body_yaw.store(y_rot);
}

/// Vanilla `Ghast.GhastMoveControl`.
///
/// Every few ticks pushes the mob towards the wanted position, and gives up when the way
/// there is blocked. The happy ghast uses it too, `careful` and stopped while on its
/// still timeout.
pub struct GhastMoveControl {
    wanted_x: f64,
    wanted_y: f64,
    wanted_z: f64,
    speed_modifier: f64,
    operation: Operation,
    float_duration: i32,
    careful: bool,
    should_be_stopped: fn(&dyn Mob) -> bool,
}

impl GhastMoveControl {
    #[must_use]
    pub fn new(careful: bool, should_be_stopped: fn(&dyn Mob) -> bool) -> Self {
        Self {
            wanted_x: 0.0,
            wanted_y: 0.0,
            wanted_z: 0.0,
            speed_modifier: 0.0,
            operation: Operation::Wait,
            float_duration: 0,
            careful,
            should_be_stopped,
        }
    }

    fn can_reach(&self, mob: &dyn Mob, travel: Vector3<f64>) -> bool {
        let entity = mob.get_entity();
        let world = entity.world.load();
        let aabb = entity.bounding_box.load();
        let aabb_at_destination = aabb.shift(travel);
        if self.careful {
            let inflated = aabb_at_destination.expand_all(1.0);
            let all_possible = BlockPos::between_closed(
                BlockPos::floored_v(inflated.min),
                BlockPos::floored_v(inflated.max),
            )
            .all(|pos| self.block_traversal_possible(entity, &world, None, pos, false, false));
            if !all_possible {
                return false;
            }
        }

        let is_in_water = entity.is_in_water();
        let is_in_lava = entity.touching_lava.load(Ordering::Relaxed);
        let start = entity.pos.load();
        let end = start + travel;
        for_each_block_intersected_between(start, end, &aabb_at_destination, |pos, _| {
            aabb.intersects(&BoundingBox::from_block(&pos))
                || self.block_traversal_possible(
                    entity,
                    &world,
                    Some((start, end)),
                    pos,
                    is_in_water,
                    is_in_lava,
                )
        })
    }

    /// `start_end` is vanilla's `start` and `end`, which are both null or both set.
    fn block_traversal_possible(
        &self,
        entity: &Entity,
        world: &World,
        start_end: Option<(Vector3<f64>, Vector3<f64>)>,
        pos: BlockPos,
        can_path_through_water: bool,
        can_path_through_lava: bool,
    ) -> bool {
        let (block, state) = world.get_block_and_state(&pos);
        if state.is_air() {
            return true;
        }

        let mut shapes = state
            .get_block_collision_shapes_at(&pos)
            .map(|shape| shape.at_pos(pos));
        let path_no_collisions = match start_end {
            Some((start, end)) => !entity.collided_with_shape_moving_from(start, end, shapes),
            None => shapes.next().is_none(),
        };
        if !self.careful {
            return path_no_collisions;
        }

        if block.has_tag(&tag::Block::MINECRAFT_HAPPY_GHAST_AVOIDS) {
            return false;
        }

        let (fluid, fluid_state) = world.get_fluid_and_fluid_state(&pos);
        if !fluid_state.is_empty
            && start_end.is_none_or(|(start, end)| {
                entity.collided_with_fluid(fluid, &fluid_state, &pos, start, end)
            })
        {
            if fluid.has_tag(&tag::Fluid::MINECRAFT_WATER) {
                return can_path_through_water;
            }
            if fluid.has_tag(&tag::Fluid::MINECRAFT_LAVA) {
                return can_path_through_lava;
            }
        }

        path_no_collisions
    }
}

impl Control for GhastMoveControl {}

impl MoveControlTrait for GhastMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        if (self.should_be_stopped)(mob) {
            self.operation = Operation::Wait;
            mob_entity.stop_in_place();
        }

        if self.operation != Operation::MoveTo {
            return;
        }
        let float_duration = self.float_duration;
        self.float_duration -= 1;
        if float_duration > 0 {
            return;
        }
        self.float_duration += mob.get_entity().random().next_bounded_i32(5) + 2;
        let living = &mob_entity.living_entity;
        let entity = &living.entity;
        let pos = entity.pos.load();
        let travel = Vector3::new(
            self.wanted_x - pos.x,
            self.wanted_y - pos.y,
            self.wanted_z - pos.z,
        );
        if self.can_reach(mob, travel) {
            let speed = living.get_attribute_value(&Attributes::FLYING_SPEED) * 5.0 / 3.0;
            entity
                .velocity
                .store(entity.velocity.load() + travel.normalize() * speed);
        } else {
            self.operation = Operation::Wait;
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

    fn has_wanted(&self) -> bool {
        self.operation == Operation::MoveTo
    }

    fn wanted_position(&self) -> Vector3<f64> {
        Vector3::new(self.wanted_x, self.wanted_y, self.wanted_z)
    }

    fn set_wait(&mut self) {
        self.operation = Operation::Wait;
    }
}

/// Vanilla `Ghast.GhastLookGoal`.
pub struct GhastLookGoal;

impl Goal for GhastLookGoal {
    fn can_start(&mut self, _mob: &dyn Mob) -> bool {
        true
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn tick(&mut self, mob: &dyn Mob) {
        face_movement_direction(mob);
    }

    fn controls(&self) -> Controls {
        Controls::LOOK
    }
}

/// Vanilla `Ghast.GhastShootFireballGoal`.
#[derive(Default)]
pub struct GhastShootFireballGoal {
    charge_time: i32,
}

fn as_ghast(mob: &dyn Mob) -> Option<&GhastEntity> {
    mob.cast_any().downcast_ref::<GhastEntity>()
}

impl Goal for GhastShootFireballGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.get_target().is_some()
    }

    fn start(&mut self, _mob: &dyn Mob) {
        self.charge_time = 0;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(ghast) = as_ghast(mob) {
            ghast.set_charging(false);
        }
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(ghast) = as_ghast(mob) else {
            return;
        };
        let Some(target) = ghast.get_target() else {
            return;
        };

        let entity = &ghast.mob_entity.living_entity.entity;
        let target_entity = target.get_entity();
        let ghast_pos = entity.pos.load();
        let target_pos = target_entity.pos.load();
        if target_pos.squared_distance_to_vec(&ghast_pos) < MAX_TARGET_DISTANCE_SQ
            && entity.has_line_of_sight(target_entity)
        {
            let world = entity.world.load_full();
            self.charge_time += 1;
            if self.charge_time == 10 && !entity.is_silent() {
                world.sync_world_event(WorldEvent::SoundGhastWarning, entity.block_pos.load(), 0);
            }

            if self.charge_time == 20 {
                // `getViewVector(1.0F)`, which for a living entity follows the head.
                let view =
                    Entity::calculate_view_vector(entity.pitch.load(), entity.head_yaw.load());
                let ghast_half_y = ghast_pos.y + f64::from(entity.height()) * 0.5;
                let direction = Vector3::new(
                    target_pos.x - (ghast_pos.x + view.x * 4.0),
                    target_pos.y + f64::from(target_entity.height()) * 0.5 - (0.5 + ghast_half_y),
                    target_pos.z - (ghast_pos.z + view.z * 4.0),
                );
                if !entity.is_silent() {
                    world.sync_world_event(
                        WorldEvent::SoundGhastFireball,
                        entity.block_pos.load(),
                        0,
                    );
                }

                // Vanilla positions the fireball at the ghast, then offsets it by the view.
                let spawn_pos = Vector3::new(
                    ghast_pos.x + view.x * 4.0,
                    ghast_half_y + 0.5,
                    ghast_pos.z + view.z * 4.0,
                );
                let fireball_base = Entity::from_uuid(
                    uuid::Uuid::new_v4(),
                    world.clone(),
                    spawn_pos,
                    &EntityType::FIREBALL,
                );
                let fireball = FireballEntity::new_from_mob(
                    fireball_base,
                    entity,
                    direction.normalize(),
                    f32::from(ghast.get_explosion_power()),
                );
                fireball.get_entity().set_pos(spawn_pos);
                world.spawn_entity(Arc::new(fireball));
                self.charge_time = -40;
            }
        } else if self.charge_time > 0 {
            self.charge_time -= 1;
        }

        ghast.set_charging(self.charge_time > 10);
    }

    fn controls(&self) -> Controls {
        Controls::empty()
    }
}

/// Vanilla `Ghast.RandomFloatAroundGoal`: when the mob has nowhere to go, or its wanted
/// position is under a block or over 60 blocks away, picks a new one within 16 blocks.
pub struct RandomFloatAroundGoal {
    distance_to_blocks: i32,
}

impl RandomFloatAroundGoal {
    /// Vanilla `RandomFloatAroundGoal.MAX_ATTEMPTS`.
    const MAX_ATTEMPTS: i32 = 64;

    #[must_use]
    pub const fn new(distance_to_blocks: i32) -> Self {
        Self { distance_to_blocks }
    }

    /// Vanilla `getSuitableFlyToPosition`.
    fn get_suitable_fly_to_position(mob: &dyn Mob, distance_to_blocks: i32) -> Vector3<f64> {
        let entity = mob.get_entity();
        let world = entity.world.load();
        let center = entity.pos.load();
        let mut result = None;

        for _ in 0..Self::MAX_ATTEMPTS {
            result = Self::choose_random_position_with_restriction(mob, center);
            if let Some(result) = result
                && Self::is_good_target(&world, result, distance_to_blocks)
            {
                return result;
            }
        }

        let mut result =
            result.unwrap_or_else(|| Self::choose_random_position(center, &mut *entity.random()));
        let pos = BlockPos::floored_v(result);
        // `level.getHeight` gives the first free block, one above Pumpkin's height.
        let height_y =
            world.get_heightmap_height(ChunkHeightmapType::MotionBlocking, pos.0.x, pos.0.z) + 1;
        if height_y < pos.0.y && height_y > world.dimension.min_y {
            result.y = center.y - (center.y - result.y).abs();
        }
        result
    }

    /// Vanilla `isGoodTarget`: an air block with a non-air block within
    /// `distance_to_blocks - 1` blocks in some direction.
    fn is_good_target(world: &World, target: Vector3<f64>, distance_to_blocks: i32) -> bool {
        if distance_to_blocks <= 0 {
            return true;
        }

        let pos = BlockPos::floored_v(target);
        if !world.get_block_state(&pos).is_air() {
            return false;
        }

        pumpkin_data::BlockDirection::all().iter().any(|direction| {
            (1..distance_to_blocks).any(|i| {
                !world
                    .get_block_state(&pos.offset_dir(direction.to_offset(), i))
                    .is_air()
            })
        })
    }

    /// Vanilla `chooseRandomPosition`, in vanilla's float offsets.
    fn choose_random_position(center: Vector3<f64>, random: &mut impl RandomImpl) -> Vector3<f64> {
        let mut offset = || f64::from((random.next_f32() * 2.0 - 1.0) * 16.0);
        let x = center.x + offset();
        let y = center.y + offset();
        let z = center.z + offset();
        Vector3::new(x, y, z)
    }

    /// Vanilla `chooseRandomPositionWithRestriction`: `None` outside the mob's home.
    fn choose_random_position_with_restriction(
        mob: &dyn Mob,
        center: Vector3<f64>,
    ) -> Option<Vector3<f64>> {
        let target = Self::choose_random_position(center, &mut *mob.get_entity().random());
        let mob_entity = mob.get_mob_entity();
        (!mob_entity.has_position_target()
            || mob_entity.is_in_position_target_range_pos(&BlockPos::floored_v(target)))
        .then_some(target)
    }
}

impl Goal for RandomFloatAroundGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        // `this.ghast.getMoveControl()`.
        let Some(wanted) = mob
            .get_mob_entity()
            .with_move_control(|control| control.has_wanted().then(|| control.wanted_position()))
        else {
            return true;
        };

        let pos = mob.get_entity().pos.load();
        let xd = wanted.x - pos.x;
        let yd = wanted.y - pos.y;
        let zd = wanted.z - pos.z;
        let dd = xd * xd + yd * yd + zd * zd;
        dd < 1.0 || dd > 3600.0
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }

    fn start(&mut self, mob: &dyn Mob) {
        let result = Self::get_suitable_fly_to_position(mob, self.distance_to_blocks);
        mob.get_mob_entity().with_move_control(|control| {
            control.set_wanted_position(result.x, result.y, result.z, 1.0);
        });
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}
