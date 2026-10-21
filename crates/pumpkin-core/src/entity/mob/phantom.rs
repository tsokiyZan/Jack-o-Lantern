use std::sync::atomic::{AtomicI32, Ordering::Relaxed};
use std::sync::{Arc, PoisonError};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::entity::{EntityPose, EntityType};
use pumpkin_data::sound::Sound;
use pumpkin_data::tracked_data;
use pumpkin_data::world::WorldEvent;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::{
    approach, approach_degrees, atan2, cos, degrees_difference_abs, sin, wrap_degrees,
};
use pumpkin_util::random::RandomImpl;
use pumpkin_world::chunk::ChunkHeightmapType;

use crate::entity::ai::control::body_rotation_control::BodyRotationControlTrait;
use crate::entity::ai::control::{Control, MoveControlTrait};
use crate::entity::ai::goal::{Controls, Goal, to_goal_ticks};
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::ai::util::goal_utils;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::passive::cat::CatEntity;
use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
};
use crate::world::World;

/// Vanilla `Phantom.TICKS_PER_FLAP`: `Mth.ceil(24.166098F)`.
pub const TICKS_PER_FLAP: i32 = 25;
/// `(float) (Math.PI / 180.0)`, vanilla's float degrees-to-radians factor.
const DEG_TO_RAD: f32 = (std::f64::consts::PI / 180.0) as f32;

/// Vanilla `Phantom.AttackPhase`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AttackPhase {
    Circle,
    Swoop,
}

pub struct PhantomEntity {
    pub mob_entity: MobEntity,
    /// Vanilla `ID_SIZE`.
    size: AtomicI32,
    move_target_point: AtomicCell<Vector3<f64>>,
    anchor_point: AtomicCell<Option<BlockPos>>,
    attack_phase: AtomicCell<AttackPhase>,
}

impl PhantomEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `Phantom`'s constructor and `createBodyControl`; its look control is
        // `skips_look_control_tick`.
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(PhantomMoveControl::default());
        *mob_entity
            .body_rotation_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(PhantomBodyRotationControl);
        let phantom = Arc::new(Self {
            mob_entity,
            size: AtomicI32::new(0),
            move_target_point: AtomicCell::new(Vector3::new(0.0, 0.0, 0.0)),
            anchor_point: AtomicCell::new(None),
            attack_phase: AtomicCell::new(AttackPhase::Circle),
        });

        // Vanilla `registerGoals`.
        phantom
            .mob_entity
            .add_goal(1, PhantomAttackStrategyGoal::default());
        phantom
            .mob_entity
            .add_goal(2, PhantomSweepAttackGoal::default());
        phantom
            .mob_entity
            .add_goal(3, PhantomCircleAroundAnchorGoal::default());
        phantom
            .mob_entity
            .add_target_goal(1, PhantomAttackPlayerTargetGoal::default());

        phantom
    }

    /// Vanilla `setPhantomSize`, with the `onSyncedDataUpdated` that a changed size fires.
    pub fn set_phantom_size(&self, size: i32) {
        let size = size.clamp(0, 64);
        if self.size.swap(size, Relaxed) != size {
            self.get_entity()
                .set_synced_data(tracked_data::phantom::ID_SIZE, size);
            self.update_phantom_size_info();
        }
    }

    fn update_phantom_size_info(&self) {
        self.refresh_dimensions();
        self.mob_entity.living_entity.set_attribute_base(
            &Attributes::ATTACK_DAMAGE,
            f64::from(6 + self.get_phantom_size()),
        );
    }

    #[must_use]
    pub fn get_phantom_size(&self) -> i32 {
        self.size.load(Relaxed)
    }

    /// Vanilla `getUniqueFlapTickOffset`.
    fn get_unique_flap_tick_offset(&self) -> i32 {
        self.get_entity().entity_id.wrapping_mul(3)
    }

    /// Vanilla `Phantom.canAttack(level, target, conditions)`.
    fn can_attack_with(&self, target: &dyn EntityBase, conditions: &TargetPredicate) -> bool {
        conditions.test(&self.get_entity().world.load(), Some(self), target)
    }

    fn touching_target(&self) -> bool {
        self.move_target_point
            .load()
            .squared_distance_to_vec(&self.get_entity().pos.load())
            < 4.0
    }
}

fn as_phantom(mob: &dyn Mob) -> Option<&PhantomEntity> {
    mob.cast_any().downcast_ref::<PhantomEntity>()
}

impl Mob for PhantomEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `Phantom.isFlapping`.
    fn is_flapping(&self) -> bool {
        self.get_unique_flap_tick_offset()
            .wrapping_add(self.get_entity().tick_count.load(Relaxed))
            % TICKS_PER_FLAP
            == 0
    }

    /// Vanilla `Phantom.getDefaultDimensions`: one size step adds 15%.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        self.get_entity()
            .get_dimensions(pose)
            .scale(self.age_scale())
            .scale(1.0 + 0.15 * self.get_phantom_size() as f32)
    }

    /// Vanilla `Phantom.PhantomLookControl.tick` does nothing, leaving the pitch the move
    /// control set.
    fn skips_look_control_tick(&self) -> bool {
        true
    }

    /// Vanilla `Phantom.checkFallDamage` does nothing.
    fn mob_check_fall_damage(
        &self,
        _caller: &dyn crate::entity::EntityBase,
        _ya: f64,
        _on_ground: bool,
        _state: &'static pumpkin_data::BlockState,
        _pos: pumpkin_util::math::position::BlockPos,
    ) {
    }

    /// Vanilla `Phantom.travel`.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        self.mob_entity.living_entity.travel_flying(caller, 0.2);
        true
    }

    fn finalize_spawn(
        &self,
        _world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.anchor_point
            .store(Some(self.get_entity().block_pos.load().up_height(5)));
        self.set_phantom_size(0);
        self.mob_entity.finalize_spawn_base();
        group_data
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        if let Some(pos) = self.anchor_point.load() {
            nbt.put(
                "anchor_pos",
                NbtTag::IntArray(vec![pos.0.x, pos.0.y, pos.0.z]),
            );
        }
        nbt.put_int("size", self.get_phantom_size());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.anchor_point
            .store(nbt.get_int_array("anchor_pos").and_then(|pos| match pos {
                &[x, y, z] => Some(BlockPos::new(x, y, z)),
                _ => None,
            }));
        self.set_phantom_size(nbt.get_int("size").unwrap_or(0));
    }
}

/// Vanilla `Phantom.PhantomMoveControl`: steers towards `moveTargetPoint`, speeding up
/// on straight runs and slowing down in turns.
struct PhantomMoveControl {
    speed: f32,
}

impl Default for PhantomMoveControl {
    fn default() -> Self {
        Self { speed: 0.1 }
    }
}

impl Control for PhantomMoveControl {}

impl MoveControlTrait for PhantomMoveControl {
    #[expect(
        clippy::imprecise_flops,
        reason = "vanilla rounds Math.sqrt of the sums, not a hypot"
    )]
    fn tick(&mut self, mob: &dyn Mob) {
        let Some(phantom) = as_phantom(mob) else {
            return;
        };
        let entity = phantom.get_entity();
        if entity.horizontal_collision.load(Relaxed) {
            entity.yaw.store(entity.yaw.load() + 180.0);
            self.speed = 0.1;
        }

        let target = phantom.move_target_point.load();
        let pos = entity.pos.load();
        let mut tdx = target.x - pos.x;
        let tdy = target.y - pos.y;
        let mut tdz = target.z - pos.z;
        let mut sd = (tdx * tdx + tdz * tdz).sqrt();
        if sd.abs() <= f64::from(1.0E-5f32) {
            return;
        }
        let y_relative_scale = 1.0 - (tdy * f64::from(0.7f32)).abs() / sd;
        tdx *= y_relative_scale;
        tdz *= y_relative_scale;
        sd = (tdx * tdx + tdz * tdz).sqrt();
        let sd2 = (tdx * tdx + tdz * tdz + tdy * tdy).sqrt();
        let prev = entity.yaw.load();
        let angle = atan2(tdz, tdx) as f32;
        let a = wrap_degrees(entity.yaw.load() + 90.0);
        let b = wrap_degrees(angle * (180.0 / std::f32::consts::PI));
        let yaw = approach_degrees(a, b, 4.0) - 90.0;
        entity.yaw.store(yaw);
        entity.body_yaw.store(yaw);
        self.speed = if degrees_difference_abs(prev, yaw) < 3.0 {
            approach(self.speed, 1.8, 0.005 * (1.8 / self.speed))
        } else {
            approach(self.speed, 0.2, 0.025)
        };

        let x_rot_d =
            (-(atan2(-tdy, sd) * f64::from(180.0f32) / f64::from(std::f32::consts::PI))) as f32;
        entity.set_pitch(x_rot_d);
        let move_angle = yaw + 90.0;
        let txd = f64::from(self.speed * cos(move_angle * DEG_TO_RAD)) * (tdx / sd2).abs();
        let tzd = f64::from(self.speed * sin(move_angle * DEG_TO_RAD)) * (tdz / sd2).abs();
        let tyd = f64::from(self.speed * sin(x_rot_d * DEG_TO_RAD)) * (tdy / sd2).abs();
        let movement = entity.velocity.load();
        entity.velocity.store(Vector3::new(
            movement.x + (txd - movement.x) * 0.2,
            movement.y + (tyd - movement.y) * 0.2,
            movement.z + (tzd - movement.z) * 0.2,
        ));
    }
}

/// Vanilla `Phantom.PhantomBodyRotationControl`: the head follows the body a tick late
/// and the body faces the flight direction.
struct PhantomBodyRotationControl;

impl Control for PhantomBodyRotationControl {}

impl BodyRotationControlTrait for PhantomBodyRotationControl {
    fn client_tick(&mut self, mob: &dyn Mob) {
        let entity = mob.get_entity();
        entity.head_yaw.store(entity.body_yaw.load());
        entity.body_yaw.store(entity.yaw.load());
    }
}

/// Vanilla `Phantom.PhantomAttackPlayerTargetGoal`: every few seconds, targets the
/// highest player it may attack within 16 blocks across and 64 up or down.
struct PhantomAttackPlayerTargetGoal {
    attack_targeting: TargetPredicate,
    next_scan_tick: i32,
}

impl Default for PhantomAttackPlayerTargetGoal {
    fn default() -> Self {
        Self {
            attack_targeting: TargetPredicate::create_attackable().set_base_max_distance(64.0),
            next_scan_tick: to_goal_ticks(20),
        }
    }
}

impl Goal for PhantomAttackPlayerTargetGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if self.next_scan_tick > 0 {
            self.next_scan_tick -= 1;
            return false;
        }
        self.next_scan_tick = to_goal_ticks(60);
        let Some(phantom) = as_phantom(mob) else {
            return false;
        };
        let entity = phantom.get_entity();
        let world = entity.world.load();
        // Vanilla `getNearbyPlayers`: players standing in the box that pass the conditions.
        let area = entity.bounding_box.load().expand(16.0, 64.0, 16.0);
        let mut players: Vec<Arc<dyn EntityBase>> = world
            .players
            .load()
            .iter()
            .filter(|player| area.contains(player.get_entity().pos.load()))
            .map(|player| Arc::clone(player) as Arc<dyn EntityBase>)
            .filter(|player| {
                self.attack_targeting
                    .test(&world, Some(phantom), player.as_ref())
            })
            .collect();
        players.sort_by(|a, b| {
            b.get_entity()
                .pos
                .load()
                .y
                .total_cmp(&a.get_entity().pos.load().y)
        });
        let default_conditions = TargetPredicate::default();
        for player in players {
            if phantom.can_attack_with(player.as_ref(), &default_conditions) {
                phantom.set_mob_target(Some(player));
                return true;
            }
        }
        false
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        has_attackable_target(mob)
    }
}

/// Whether the phantom's target passes vanilla's `TargetingConditions.DEFAULT`.
fn has_attackable_target(mob: &dyn Mob) -> bool {
    let Some(phantom) = as_phantom(mob) else {
        return false;
    };
    mob.get_target()
        .is_some_and(|target| phantom.can_attack_with(target.as_ref(), &TargetPredicate::default()))
}

/// Vanilla `Phantom.PhantomAttackStrategyGoal`: circles high above the target, then
/// swoops at it every 8 to 12 seconds.
#[derive(Default)]
struct PhantomAttackStrategyGoal {
    next_sweep_tick: i32,
}

impl PhantomAttackStrategyGoal {
    fn set_anchor_above_target(mob: &dyn Mob, phantom: &PhantomEntity) {
        if phantom.anchor_point.load().is_none() {
            return;
        }
        let Some(target) = phantom.get_target() else {
            return;
        };
        let mut anchor = target
            .get_entity()
            .block_pos
            .load()
            .up_height(20 + mob.get_entity().random().next_bounded_i32(20));
        let sea_level = phantom.get_entity().world.load().sea_level;
        if anchor.0.y < sea_level {
            anchor = BlockPos::new(anchor.0.x, sea_level + 1, anchor.0.z);
        }
        phantom.anchor_point.store(Some(anchor));
    }
}

impl Goal for PhantomAttackStrategyGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        has_attackable_target(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(phantom) = as_phantom(mob) else {
            return;
        };
        self.next_sweep_tick = self.get_tick_count(10);
        phantom.attack_phase.store(AttackPhase::Circle);
        Self::set_anchor_above_target(mob, phantom);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        let Some(phantom) = as_phantom(mob) else {
            return;
        };
        if let Some(anchor) = phantom.anchor_point.load() {
            // Vanilla `getHeightmapPos` gives the first free block, one above Pumpkin's height.
            let surface = phantom.get_entity().world.load().get_heightmap_height(
                ChunkHeightmapType::MotionBlocking,
                anchor.0.x,
                anchor.0.z,
            ) + 1;
            let anchor = BlockPos::new(
                anchor.0.x,
                surface + 10 + mob.get_entity().random().next_bounded_i32(20),
                anchor.0.z,
            );
            phantom.anchor_point.store(Some(anchor));
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(phantom) = as_phantom(mob) else {
            return;
        };
        if phantom.attack_phase.load() != AttackPhase::Circle {
            return;
        }
        self.next_sweep_tick -= 1;
        if self.next_sweep_tick <= 0 {
            phantom.attack_phase.store(AttackPhase::Swoop);
            Self::set_anchor_above_target(mob, phantom);
            let delay = 8 + mob.get_entity().random().next_bounded_i32(4);
            self.next_sweep_tick = self.get_tick_count(delay * 20);
            let pitch = 0.95 + mob.get_entity().random().next_f32() * 0.1;
            phantom.play_sound_at_self(Sound::EntityPhantomSwoop, 10.0, pitch);
        }
    }
}

/// Vanilla `Phantom.PhantomSweepAttackGoal`: dives at the target, bites on contact and
/// pulls up when it hits a wall, gets hurt or a cat hisses nearby.
#[derive(Default)]
struct PhantomSweepAttackGoal {
    is_scared_of_cat: bool,
    cat_search_tick: i32,
}

/// Vanilla `PhantomSweepAttackGoal.CAT_SEARCH_TICK_DELAY`.
const CAT_SEARCH_TICK_DELAY: i32 = 20;

impl Goal for PhantomSweepAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_phantom(mob).is_some_and(|phantom| {
            phantom.get_target().is_some() && phantom.attack_phase.load() == AttackPhase::Swoop
        })
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let Some(target) = mob.get_target() else {
            return false;
        };
        if !target
            .get_living_entity()
            .is_some_and(crate::entity::living::LivingEntity::is_alive)
        {
            return false;
        }
        if target.is_spectator()
            || target
                .get_player()
                .is_some_and(crate::entity::player::Player::is_creative)
        {
            return false;
        }
        if !self.can_start(mob) {
            return false;
        }

        let entity = mob.get_entity();
        let tick_count = entity.tick_count.load(Relaxed);
        if tick_count > self.cat_search_tick {
            self.cat_search_tick = tick_count + CAT_SEARCH_TICK_DELAY;
            let area = entity.bounding_box.load().expand(16.0, 16.0, 16.0);
            let cats: Vec<Arc<dyn EntityBase>> = goal_utils::get_entities_of_class(
                &entity.world.load(),
                Some(&EntityType::CAT),
                &area,
            )
            .into_iter()
            .filter(|cat| {
                cat.get_living_entity()
                    .is_some_and(crate::entity::living::LivingEntity::is_alive)
            })
            .collect();
            for cat in &cats {
                if let Some(cat) = cat.cast_any().downcast_ref::<CatEntity>() {
                    cat.hiss();
                }
            }
            self.is_scared_of_cat = !cats.is_empty();
        }

        !self.is_scared_of_cat
    }

    fn stop(&mut self, mob: &dyn Mob) {
        mob.set_mob_target(None);
        if let Some(phantom) = as_phantom(mob) {
            phantom.attack_phase.store(AttackPhase::Circle);
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(phantom) = as_phantom(mob) else {
            return;
        };
        let Some(target) = phantom.get_target() else {
            return;
        };
        let target_entity = target.get_entity();
        let target_pos = target_entity.pos.load();
        phantom.move_target_point.store(Vector3::new(
            target_pos.x,
            target_pos.y + f64::from(target_entity.height()) * 0.5,
            target_pos.z,
        ));
        let entity = phantom.get_entity();
        let inflate = f64::from(0.2f32);
        if entity
            .bounding_box
            .load()
            .expand(inflate, inflate, inflate)
            .intersects(&target_entity.bounding_box.load())
        {
            phantom.mob_entity.try_attack(phantom, target.as_ref());
            phantom.attack_phase.store(AttackPhase::Circle);
            if !entity.is_silent() {
                entity.world.load().sync_world_event(
                    WorldEvent::SoundPhantomBite,
                    entity.block_pos.load(),
                    0,
                );
            }
        } else if entity.horizontal_collision.load(Relaxed)
            || phantom.mob_entity.living_entity.was_hurt_recently()
        {
            phantom.attack_phase.store(AttackPhase::Circle);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `Phantom.PhantomCircleAroundAnchorGoal`: flies rings around the anchor point,
/// now and then changing height, radius, direction or position on the ring.
#[derive(Default)]
struct PhantomCircleAroundAnchorGoal {
    angle: f32,
    distance: f32,
    height: f32,
    clockwise: f32,
}

impl PhantomCircleAroundAnchorGoal {
    fn select_next(&mut self, phantom: &PhantomEntity) {
        let anchor = phantom.anchor_point.load().unwrap_or_else(|| {
            let pos = phantom.get_entity().block_pos.load();
            phantom.anchor_point.store(Some(pos));
            pos
        });
        self.angle += self.clockwise * 15.0 * DEG_TO_RAD;
        phantom.move_target_point.store(Vector3::new(
            f64::from(anchor.0.x) + f64::from(self.distance * cos(self.angle)),
            f64::from(anchor.0.y) + f64::from(-4.0 + self.height),
            f64::from(anchor.0.z) + f64::from(self.distance * sin(self.angle)),
        ));
    }
}

impl Goal for PhantomCircleAroundAnchorGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_phantom(mob).is_some_and(|phantom| {
            phantom.get_target().is_none() || phantom.attack_phase.load() == AttackPhase::Circle
        })
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(phantom) = as_phantom(mob) else {
            return;
        };
        let mut random = mob.get_entity().random();
        self.distance = 5.0 + random.next_f32() * 10.0;
        self.height = -4.0 + random.next_f32() * 9.0;
        self.clockwise = if random.next_bool() { 1.0 } else { -1.0 };
        drop(random);
        self.select_next(phantom);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(phantom) = as_phantom(mob) else {
            return;
        };
        let entity = mob.get_entity();
        if entity.random().next_bounded_i32(self.get_tick_count(350)) == 0 {
            self.height = -4.0 + entity.random().next_f32() * 9.0;
        }

        if entity.random().next_bounded_i32(self.get_tick_count(250)) == 0 {
            self.distance += 1.0;
            if self.distance > 15.0 {
                self.distance = 5.0;
                self.clockwise = -self.clockwise;
            }
        }

        if entity.random().next_bounded_i32(self.get_tick_count(450)) == 0 {
            self.angle = entity.random().next_f32() * 2.0 * std::f32::consts::PI;
            self.select_next(phantom);
        }

        if phantom.touching_target() {
            self.select_next(phantom);
        }

        let entity = phantom.get_entity();
        let world = entity.world.load();
        let y = entity.pos.load().y;
        if phantom.move_target_point.load().y < y
            && !world
                .get_block_state(&entity.block_pos.load().down())
                .is_air()
        {
            self.height = self.height.max(1.0);
            self.select_next(phantom);
        }

        if phantom.move_target_point.load().y > y
            && !world
                .get_block_state(&entity.block_pos.load().up())
                .is_air()
        {
            self.height = self.height.min(-1.0);
            self.select_next(phantom);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}
