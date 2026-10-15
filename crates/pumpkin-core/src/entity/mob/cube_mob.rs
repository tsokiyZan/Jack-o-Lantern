//! Vanilla `AbstractCubeMob` with its `Slime` and `MagmaCube` subclasses.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
use std::sync::{Arc, Weak};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::{EntityPose, EntityType};
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crate::entity::ai::control::{Control, MoveControlTrait, rotlerp};
use crate::entity::ai::goal::{Controls, Goal, active_target::ActiveTargetGoal, to_goal_ticks};
use crate::entity::custom_sound::CustomSound;
use crate::entity::living::LivingEntity;
use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::{Mob, MobEntity, convert_common};
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase, do_teams_allow_damage};
use crate::world::World;

/// Which `AbstractCubeMob` subclass a cube mob is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CubeKind {
    Slime,
    MagmaCube,
}

pub struct CubeMobEntity {
    mob_entity: MobEntity,
    kind: CubeKind,
    /// Vanilla `ID_SIZE`.
    size: AtomicI32,
    pub target_squish: AtomicCell<f32>,
    pub squish: AtomicCell<f32>,
    pub o_squish: AtomicCell<f32>,
    was_on_ground: AtomicBool,
    // `CubeMobMoveControl` state, which the goals set.
    move_y_rot: AtomicCell<f32>,
    jump_delay: AtomicI32,
    is_aggressive: AtomicBool,
    speed_modifier: AtomicCell<f64>,
    /// `operation == MOVE_TO`.
    wants_to_move: AtomicBool,
}

impl CubeMobEntity {
    pub fn new(entity: Entity, kind: CubeKind) -> Arc<Self> {
        // `CubeMobMoveControl`'s constructor.
        let move_y_rot = 180.0 * entity.yaw.load() / std::f32::consts::PI;
        let cube = Arc::new(Self {
            mob_entity: MobEntity::new(entity),
            kind,
            size: AtomicI32::new(1),
            target_squish: AtomicCell::new(0.0),
            squish: AtomicCell::new(0.0),
            o_squish: AtomicCell::new(0.0),
            was_on_ground: AtomicBool::new(false),
            move_y_rot: AtomicCell::new(move_y_rot),
            jump_delay: AtomicI32::new(0),
            is_aggressive: AtomicBool::new(false),
            speed_modifier: AtomicCell::new(0.0),
            wants_to_move: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&cube);
        *cube
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(CubeMobMoveControl { cube: weak.clone() });
        // `CubeMobFloatGoal`'s constructor.
        cube.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_can_float(true);
        let mut goals = cube
            .mob_entity
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        goals.add_goal(1, Box::new(CubeMobFloatGoal { cube: weak.clone() }));
        goals.add_goal(
            4,
            Box::new(CubeMobRandomDirectionGoal {
                cube: weak.clone(),
                chosen_degrees: 0.0,
                next_randomize_time: 0,
            }),
        );
        goals.add_goal(5, Box::new(CubeMobKeepOnJumpingGoal { cube: weak.clone() }));
        // `addBehaviourGoals`.
        goals.add_goal(
            2,
            Box::new(CubeMobAttackGoal {
                cube: weak.clone(),
                grow_tired_timer: 0,
            }),
        );
        drop(goals);
        // `addTargetingGoals`.
        let mut targets = cube
            .mob_entity
            .target_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        targets.add_goal(
            1,
            Box::new(ActiveTargetGoal::new(
                &cube.mob_entity,
                &EntityType::PLAYER,
                10,
                true,
                false,
                Some(move |target: &LivingEntity, _world: &World| {
                    weak.upgrade().is_some_and(|cube| {
                        (target.entity.pos.load().y - cube.get_entity().pos.load().y).abs() <= 4.0
                    })
                }),
            )),
        );
        targets.add_goal(
            3,
            ActiveTargetGoal::with_default(&cube.mob_entity, &EntityType::IRON_GOLEM, true),
        );
        drop(targets);
        cube
    }

    /// Vanilla `setSize`, with the `Slime` and `MagmaCube` overrides.
    pub fn set_size(&self, size: i32, update_health: bool) {
        let actual_size = size.clamp(1, 127);
        let living = &self.mob_entity.living_entity;
        self.size.store(actual_size, Relaxed);
        living
            .entity
            .set_synced_data(tracked_data::abstract_cube_mob::ID_SIZE, actual_size);
        EntityBase::refresh_dimensions(self);
        living.set_attribute_base(
            &Attributes::MAX_HEALTH,
            f64::from(actual_size * actual_size),
        );
        living.set_attribute_base(
            &Attributes::MOVEMENT_SPEED,
            f64::from(0.2f32 + 0.1f32 * actual_size as f32),
        );
        if update_health {
            living.set_health(living.get_attribute_value(&Attributes::MAX_HEALTH) as f32);
        }
        living.set_attribute_base(&Attributes::ATTACK_DAMAGE, f64::from(actual_size));
        if self.kind == CubeKind::MagmaCube {
            // Vanilla uses the size before clamping here.
            living.set_attribute_base(&Attributes::ARMOR, f64::from(size * 3));
        }
    }

    #[must_use]
    pub const fn kind(&self) -> CubeKind {
        self.kind
    }

    pub fn get_size(&self) -> i32 {
        self.size.load(Relaxed)
    }

    pub fn is_tiny(&self) -> bool {
        self.get_size() <= 1
    }

    fn is_in_liquid(&self) -> bool {
        let entity = self.get_entity();
        entity.touching_water.load(Relaxed) || entity.touching_lava.load(Relaxed)
    }

    fn get_jump_delay(&self) -> i32 {
        let delay = self.get_random().random_range(0..20) + 10;
        match self.kind {
            CubeKind::Slime => delay,
            CubeKind::MagmaCube => delay * 4,
        }
    }

    fn get_sound_volume(&self) -> f32 {
        0.4 * self.get_size() as f32
    }

    fn get_sound_pitch(&self) -> f32 {
        let pitch_adjuster = if self.is_tiny() { 1.4 } else { 0.8 };
        let mut rng = self.get_random();
        ((rng.random::<f32>() - rng.random::<f32>()) * 0.2 + 1.0) * pitch_adjuster
    }

    fn get_jump_sound(&self) -> Sound {
        match self.kind {
            CubeKind::Slime if self.is_tiny() => Sound::EntitySlimeJumpSmall,
            CubeKind::Slime => Sound::EntitySlimeJump,
            CubeKind::MagmaCube => Sound::EntityMagmaCubeJump,
        }
    }

    fn get_squish_sound(&self) -> Sound {
        match (self.kind, self.is_tiny()) {
            (CubeKind::Slime, true) => Sound::EntitySlimeSquishSmall,
            (CubeKind::Slime, false) => Sound::EntitySlimeSquish,
            (CubeKind::MagmaCube, true) => Sound::EntityMagmaCubeSquishSmall,
            (CubeKind::MagmaCube, false) => Sound::EntityMagmaCubeSquish,
        }
    }

    /// Vanilla `Entity.playSound` with the cube mob's `HOSTILE` sound source.
    fn play_sound(&self, sound: Sound, volume: f32, pitch: f32) {
        let entity = self.get_entity();
        if !entity.silent.load(Relaxed) {
            entity.world.load().play_sound_fine(
                sound,
                SoundCategory::Hostile,
                &entity.pos.load(),
                volume,
                pitch,
            );
        }
    }

    fn can_deal_damage(&self) -> bool {
        let effective_ai = !self.mob_entity.is_no_ai();
        match self.kind {
            CubeKind::Slime => !self.is_tiny() && effective_ai,
            CubeKind::MagmaCube => effective_ai,
        }
    }

    fn get_attack_damage(&self) -> f32 {
        let damage = self
            .mob_entity
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32;
        match self.kind {
            CubeKind::Slime => damage,
            CubeKind::MagmaCube => damage + 2.0,
        }
    }

    /// Vanilla `dealDamage`.
    fn deal_damage(&self, target: &dyn EntityBase) {
        let entity = self.get_entity();
        if entity.is_alive()
            && do_teams_allow_damage(self, target)
            && self.mob_entity.is_in_attack_range(target)
            && entity.has_line_of_sight(target.get_entity())
            && target.damage_with_context(
                target,
                self.get_attack_damage(),
                DamageType::MOB_ATTACK,
                None,
                Some(self),
                Some(self),
            )
        {
            let pitch = {
                let mut rng = self.get_random();
                (rng.random::<f32>() - rng.random::<f32>()) * 0.2 + 1.0
            };
            self.play_sound(Sound::EntitySlimeAttack, 1.0, pitch);
        }
    }

    // `CubeMobMoveControl.setDirection`.
    fn set_direction(&self, y_rot: f32, is_aggressive: bool) {
        self.move_y_rot.store(y_rot);
        self.is_aggressive.store(is_aggressive, Relaxed);
    }

    // `CubeMobMoveControl.setWantedMovement`.
    fn set_wanted_movement(&self, speed_modifier: f64) {
        self.speed_modifier.store(speed_modifier);
        self.wants_to_move.store(true, Relaxed);
    }
}

impl CustomSound for CubeMobEntity {
    fn death_sound(&self) -> Option<Sound> {
        Some(match (self.kind, self.is_tiny()) {
            (CubeKind::Slime, true) => Sound::EntitySlimeDeathSmall,
            (CubeKind::Slime, false) => Sound::EntitySlimeDeath,
            (CubeKind::MagmaCube, true) => Sound::EntityMagmaCubeDeathSmall,
            (CubeKind::MagmaCube, false) => Sound::EntityMagmaCubeDeath,
        })
    }

    fn hurt_sound(&self) -> Option<Sound> {
        Some(match (self.kind, self.is_tiny()) {
            (CubeKind::Slime, true) => Sound::EntitySlimeHurtSmall,
            (CubeKind::Slime, false) => Sound::EntitySlimeHurt,
            (CubeKind::MagmaCube, true) => Sound::EntityMagmaCubeHurtSmall,
            (CubeKind::MagmaCube, false) => Sound::EntityMagmaCubeHurt,
        })
    }
}

impl Mob for CubeMobEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn sound_volume(&self) -> f32 {
        self.get_sound_volume()
    }

    fn default_dimensions(&self, _pose: EntityPose) -> EntityDimensions {
        Entity::type_dimensions(self.get_entity().entity_type).scale(self.get_size() as f32)
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    /// `setSize` sets `xpReward` to the size.
    fn xp_reward(&self) -> u32 {
        self.get_size() as u32
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int("Size", self.get_size() - 1);
        nbt.put_bool("wasOnGround", self.was_on_ground.load(Relaxed));
    }

    fn mob_read_nbt_before_super(&self, nbt: &NbtCompound) {
        self.set_size(nbt.get_int("Size").unwrap_or(0) + 1, false);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.was_on_ground
            .store(nbt.get_bool("wasOnGround").unwrap_or(false), Relaxed);
    }

    /// Vanilla `finalizeSpawn`: an `AgeableMobGroupData(false)` group, then `setSpawnSize`.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.mob_entity.finalize_spawn_base();
        let mut group_data = match group_data {
            Some(data @ SpawnGroupData::Ageable { .. }) => data,
            _ => SpawnGroupData::Ageable {
                group_size: 0,
                should_spawn_baby: false,
                baby_spawn_chance: 0.05,
            },
        };
        if let SpawnGroupData::Ageable { group_size, .. } = &mut group_data {
            *group_size += 1;
        }
        let difficulty = RegionalDifficulty::at(world, self.get_entity().pos.load());
        let mut rng = rand::rng();
        let mut size_scale = rng.random_range(0..3);
        if size_scale < 2 && rng.random::<f32>() < 0.5 * difficulty.special_multiplier {
            size_scale += 1;
        }
        self.set_size(1 << size_scale, true);
        Some(group_data)
    }

    /// The part of vanilla `tick` before `super.tick()`.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.o_squish.store(self.squish.load());
        self.squish
            .store(self.squish.load() + (self.target_squish.load() - self.squish.load()) * 0.5);
    }

    /// The part of vanilla `tick` after `super.tick()`.
    fn post_tick(&self) {
        let entity = self.get_entity();
        let on_ground = entity.on_ground.load(Relaxed);
        let was_on_ground = self.was_on_ground.load(Relaxed);
        if on_ground && !was_on_ground {
            let size = entity.entity_dimension.load().width * 2.0;
            let pitch = {
                let mut rng = self.get_random();
                // The landing particles are the client's, but the server draws their random
                // numbers too.
                let mut i = 0;
                while (i as f32) < size * 16.0 {
                    rng.random::<f32>();
                    rng.random::<f32>();
                    i += 1;
                }
                ((rng.random::<f32>() - rng.random::<f32>()) * 0.2 + 1.0) / 0.8
            };
            self.play_sound(self.get_squish_sound(), self.get_sound_volume(), pitch);
            self.target_squish.store(-0.5);
        } else if !on_ground && was_on_ground {
            self.target_squish.store(1.0);
        }
        self.was_on_ground.store(on_ground, Relaxed);
        // `decreaseSquish`.
        let decrease = match self.kind {
            CubeKind::Slime => 0.6,
            CubeKind::MagmaCube => 0.9,
        };
        self.target_squish
            .store(self.target_squish.load() * decrease);
    }

    /// Vanilla `jumpFromGround`, which `MagmaCube` raises by its size.
    fn jump_from_ground(&self) {
        let living = &self.mob_entity.living_entity;
        let mut power = living.get_jump_velocity(1.0);
        if self.kind == CubeKind::MagmaCube {
            power += self.get_size() as f32 * 0.1;
        }
        let mut velocity = living.entity.velocity.load();
        velocity.y = f64::from(power);
        living.entity.velocity.store(velocity);
        living.entity.velocity_dirty.store(true, Relaxed);
    }

    /// Vanilla `MagmaCube.jumpInLiquid`.
    fn jump_in_liquid(&self, lava: bool) {
        if self.kind == CubeKind::MagmaCube && lava {
            let entity = self.get_entity();
            let mut velocity = entity.velocity.load();
            velocity.y = f64::from(0.22f32 + self.get_size() as f32 * 0.05);
            entity.velocity.store(velocity);
            entity.velocity_dirty.store(true, Relaxed);
        } else {
            self.mob_entity.jump_in_liquid();
        }
    }

    /// Vanilla `playerTouch`.
    fn mob_player_collision(&self, player: &Arc<Player>) {
        if self.can_deal_damage() {
            self.deal_damage(&**player);
        }
    }

    /// Vanilla `push(Entity)`: iron golems pushing a cube mob get hurt.
    fn mob_push(&self, entity: &dyn EntityBase) {
        if entity.get_entity().entity_type == &EntityType::IRON_GOLEM && self.can_deal_damage() {
            self.deal_damage(entity);
        }
    }

    /// Vanilla `remove`: a killed cube mob splits into 2 to 4 of half its size.
    fn remove_killed(&self) {
        let size = self.get_size();
        if size <= 1 {
            return;
        }
        let entity = self.get_entity();
        let world = entity.world.load_full();
        let pos = entity.pos.load();
        let xz_cube_spawn_offset = entity.entity_dimension.load().width / 2.0;
        let half_size = size / 2;
        // `getSplitCount`.
        let count = 2 + self.get_random().random_range(0..3);
        for i in 0..count {
            let xd = ((i % 2) as f32 - 0.5) * xz_cube_spawn_offset;
            let zd = ((i / 2) as f32 - 0.5) * xz_cube_spawn_offset;
            // `convertTo` with `ConversionType.SPLIT_ON_DEATH`.
            let child = crate::entity::r#type::from_type(
                entity.entity_type,
                pos,
                &world,
                uuid::Uuid::new_v4(),
            );
            let Some(cube) = child.cast_any().downcast_ref::<Self>() else {
                continue;
            };
            convert_common(
                self,
                cube,
                crate::entity::mob::conversion::ConversionParams {
                    keep_equipment: false,
                    preserve_can_pick_up_loot: false,
                },
            );
            // `setUpSplitCube`.
            cube.set_size(half_size, true);
            let child_entity = cube.get_entity();
            child_entity.set_pos(Vector3::new(
                pos.x + f64::from(xd),
                pos.y + 0.5,
                pos.z + f64::from(zd),
            ));
            child_entity.set_rotation(self.get_random().random::<f32>() * 360.0, 0.0);
            world.spawn_entity(child);
        }
    }
}

/// Vanilla `CubeMobMoveControl`.
struct CubeMobMoveControl {
    cube: Weak<CubeMobEntity>,
}

impl Control for CubeMobMoveControl {}

impl MoveControlTrait for CubeMobMoveControl {
    fn tick(&mut self, _mob: &dyn Mob) {
        let Some(cube) = self.cube.upgrade() else {
            return;
        };
        let mob_entity = &cube.mob_entity;
        let living = &mob_entity.living_entity;
        let entity = &living.entity;
        let y_rot = rotlerp(entity.yaw.load(), cube.move_y_rot.load(), 90.0);
        entity.yaw.store(y_rot);
        entity.head_yaw.store(y_rot);
        entity.body_yaw.store(y_rot);
        if !cube.wants_to_move.swap(false, Relaxed) {
            let input = living.movement_input.load();
            living
                .movement_input
                .store(Vector3::new(input.x, input.y, 0.0));
            return;
        }
        let speed = (cube.speed_modifier.load()
            * living.get_attribute_value(&Attributes::MOVEMENT_SPEED)) as f32;
        mob_entity.set_speed(speed);
        if !entity.on_ground.load(Relaxed) {
            return;
        }
        if cube.jump_delay.fetch_sub(1, Relaxed) <= 0 {
            let mut jump_delay = cube.get_jump_delay();
            if cube.is_aggressive.load(Relaxed) {
                jump_delay /= 3;
            }
            cube.jump_delay.store(jump_delay, Relaxed);
            mob_entity.jump();
            // `doPlayJumpSound`.
            if cube.get_size() > 0 {
                cube.play_sound(
                    cube.get_jump_sound(),
                    cube.get_sound_volume(),
                    cube.get_sound_pitch(),
                );
            }
        } else {
            let input = living.movement_input.load();
            living.movement_input.store(Vector3::new(0.0, input.y, 0.0));
            mob_entity.set_speed(0.0);
        }
    }
}

/// Vanilla `CubeMobAttackGoal`.
struct CubeMobAttackGoal {
    cube: Weak<CubeMobEntity>,
    grow_tired_timer: i32,
}

impl Goal for CubeMobAttackGoal {
    fn can_start(&mut self, _mob: &dyn Mob) -> bool {
        let Some(cube) = self.cube.upgrade() else {
            return false;
        };
        cube.mob_entity
            .get_target()
            .is_some_and(|target| Mob::can_attack(&*cube, target.as_ref()))
    }

    fn start(&mut self, _mob: &dyn Mob) {
        self.grow_tired_timer = to_goal_ticks(300);
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        let Some(cube) = self.cube.upgrade() else {
            return false;
        };
        let Some(target) = cube.mob_entity.get_target() else {
            return false;
        };
        if !Mob::can_attack(&*cube, target.as_ref()) {
            return false;
        }
        self.grow_tired_timer -= 1;
        self.grow_tired_timer > 0
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn tick(&mut self, _mob: &dyn Mob) {
        let Some(cube) = self.cube.upgrade() else {
            return;
        };
        if let Some(target) = cube.mob_entity.get_target() {
            cube.mob_entity.look_at(target.as_ref(), 10.0, 10.0);
        }
        cube.set_direction(cube.get_entity().yaw.load(), cube.can_deal_damage());
    }

    fn controls(&self) -> Controls {
        Controls::LOOK
    }
}

/// Vanilla `CubeMobFloatGoal`.
struct CubeMobFloatGoal {
    cube: Weak<CubeMobEntity>,
}

impl Goal for CubeMobFloatGoal {
    fn can_start(&mut self, _mob: &dyn Mob) -> bool {
        self.cube.upgrade().is_some_and(|cube| cube.is_in_liquid())
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn tick(&mut self, _mob: &dyn Mob) {
        let Some(cube) = self.cube.upgrade() else {
            return;
        };
        if cube.get_random().random::<f32>() < 0.8 {
            cube.mob_entity.jump();
        }
        cube.set_wanted_movement(1.2);
    }

    fn controls(&self) -> Controls {
        Controls::JUMP | Controls::MOVE
    }
}

/// Vanilla `CubeMobKeepOnJumpingGoal`.
struct CubeMobKeepOnJumpingGoal {
    cube: Weak<CubeMobEntity>,
}

impl Goal for CubeMobKeepOnJumpingGoal {
    fn can_start(&mut self, _mob: &dyn Mob) -> bool {
        self.cube
            .upgrade()
            .is_some_and(|cube| !cube.get_entity().has_vehicle())
    }

    fn tick(&mut self, _mob: &dyn Mob) {
        if let Some(cube) = self.cube.upgrade() {
            cube.set_wanted_movement(1.0);
        }
    }

    fn controls(&self) -> Controls {
        Controls::JUMP | Controls::MOVE
    }
}

/// Vanilla `CubeMobRandomDirectionGoal`.
struct CubeMobRandomDirectionGoal {
    cube: Weak<CubeMobEntity>,
    chosen_degrees: f32,
    next_randomize_time: i32,
}

impl Goal for CubeMobRandomDirectionGoal {
    fn can_start(&mut self, _mob: &dyn Mob) -> bool {
        self.cube.upgrade().is_some_and(|cube| {
            cube.mob_entity.get_target().is_none()
                && (cube.get_entity().on_ground.load(Relaxed)
                    || cube.is_in_liquid()
                    || cube
                        .mob_entity
                        .living_entity
                        .has_effect(&StatusEffect::LEVITATION))
        })
    }

    fn tick(&mut self, _mob: &dyn Mob) {
        let Some(cube) = self.cube.upgrade() else {
            return;
        };
        self.next_randomize_time -= 1;
        if self.next_randomize_time <= 0 {
            let (ticks, degrees) = {
                let mut rng = cube.get_random();
                (40 + rng.random_range(0..60), rng.random_range(0..360))
            };
            self.next_randomize_time = self.get_tick_count(ticks);
            self.chosen_degrees = degrees as f32;
        }
        cube.set_direction(self.chosen_degrees, false);
    }

    fn controls(&self) -> Controls {
        Controls::LOOK
    }
}
