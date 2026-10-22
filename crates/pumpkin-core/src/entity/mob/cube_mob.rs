//! Vanilla `AbstractCubeMob` with its `Slime`, `MagmaCube` and `SulfurCube` subclasses.

mod sulfur_cube;

use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
use std::sync::{Arc, Weak};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::{EntityPose, EntityType};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_data::{Block, BlockState};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::wrap_degrees_90;
use pumpkin_util::random::RandomImpl;

use crate::enchantment::EnchantmentHelper;
use crate::entity::ageable::{AgeableData, AgeableMob};
use crate::entity::ai::control::look_control::LookControl;
use crate::entity::ai::control::{Control, MoveControlTrait, rotlerp};
use crate::entity::ai::goal::{Controls, Goal, active_target::ActiveTargetGoal, to_goal_ticks};
use crate::entity::custom_sound::CustomSound;
use crate::entity::damage_source::DamageSource;
use crate::entity::living::LivingEntity;
use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::{Mob, MobEntity, convert_common, mob_hurt_server, sounds};
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase, do_teams_allow_damage};
use crate::world::World;

/// Which `AbstractCubeMob` subclass a cube mob is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CubeKind {
    Slime,
    MagmaCube,
    SulfurCube,
}

pub struct CubeMobEntity {
    mob_entity: MobEntity,
    kind: CubeKind,
    /// This mob, for the goals vanilla `registerGoals` adds again after construction.
    weak: Weak<Self>,
    ageable: AgeableData,
    /// `SulfurCube`'s own state.
    sulfur: Option<sulfur_cube::SulfurCubeData>,
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
        let cube = Arc::new_cyclic(|weak| Self {
            mob_entity: MobEntity::new(entity),
            kind,
            weak: weak.clone(),
            ageable: AgeableData::default(),
            sulfur: (kind == CubeKind::SulfurCube).then(sulfur_cube::SulfurCubeData::default),
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
        cube.mob_entity
            .set_own_move_control(Box::new(CubeMobMoveControl {
                cube: cube.weak.clone(),
            }));
        cube.register_goals();
        cube
    }

    /// Vanilla `registerGoals`, which `SulfurCube` runs again when its body empties.
    fn register_goals(&self) {
        // `CubeMobFloatGoal`'s constructor.
        self.mob_entity.own_navigation().set_can_float(true);
        let weak = &self.weak;
        let mut goals = self
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
        if self.kind == CubeKind::SulfurCube {
            sulfur_cube::add_behaviour_goals(&mut goals, weak);
            // `SulfurCube.addTargetingGoals` adds none.
            return;
        }
        goals.add_goal(
            2,
            Box::new(CubeMobAttackGoal {
                cube: weak.clone(),
                grow_tired_timer: 0,
            }),
        );
        drop(goals);
        // `addTargetingGoals`.
        let mut targets = self
            .mob_entity
            .target_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let height_check = weak.clone();
        targets.add_goal(
            1,
            Box::new(ActiveTargetGoal::new(
                &self.mob_entity,
                &EntityType::PLAYER,
                10,
                true,
                false,
                Some(move |target: &LivingEntity, _world: &World| {
                    height_check.upgrade().is_some_and(|cube| {
                        (target.entity.pos.load().y - cube.get_entity().pos.load().y).abs() <= 4.0
                    })
                }),
            )),
        );
        targets.add_goal(
            3,
            ActiveTargetGoal::with_default(&self.mob_entity, &EntityType::IRON_GOLEM, true),
        );
    }

    /// Vanilla `setSize`, with the `Slime`, `MagmaCube` and `SulfurCube` overrides.
    pub fn set_size(&self, size: i32, update_health: bool) {
        let actual_size = size.clamp(1, 127);
        let living = &self.mob_entity.living_entity;
        self.size.store(actual_size, Relaxed);
        living
            .entity
            .set_synced_data(tracked_data::abstract_cube_mob::ID_SIZE, actual_size);
        EntityBase::refresh_dimensions(self);
        // `setCubeMobHealth`.
        let max_health = if self.kind == CubeKind::SulfurCube {
            4 * actual_size
        } else {
            actual_size * actual_size
        };
        living.set_attribute_base(&Attributes::MAX_HEALTH, f64::from(max_health));
        living.set_attribute_base(
            &Attributes::MOVEMENT_SPEED,
            f64::from(0.2f32 + 0.1f32 * actual_size as f32),
        );
        if update_health {
            living.set_health(living.get_attribute_value(&Attributes::MAX_HEALTH) as f32);
        }
        match self.kind {
            CubeKind::Slime => {
                living.set_attribute_base(&Attributes::ATTACK_DAMAGE, f64::from(actual_size));
            }
            CubeKind::MagmaCube => {
                // Vanilla uses the size before clamping here.
                living.set_attribute_base(&Attributes::ARMOR, f64::from(size * 3));
                living.set_attribute_base(&Attributes::ATTACK_DAMAGE, f64::from(actual_size));
            }
            CubeKind::SulfurCube => {
                // Vanilla compares the size before clamping.
                if update_health && size == 1 && !self.is_baby() {
                    self.set_baby(true);
                }
            }
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
        let delay = self.get_entity().random().next_bounded_i32(20) + 10;
        match self.kind {
            CubeKind::Slime | CubeKind::SulfurCube => delay,
            CubeKind::MagmaCube => delay * 4,
        }
    }

    fn get_sound_volume(&self) -> f32 {
        0.4 * self.get_size() as f32
    }

    fn get_sound_pitch(&self) -> f32 {
        let pitch_adjuster = if self.is_tiny() { 1.4 } else { 0.8 };
        let mut rng = self.get_entity().random();
        ((rng.next_f32() - rng.next_f32()) * 0.2 + 1.0) * pitch_adjuster
    }

    fn get_jump_sound(&self) -> Sound {
        match self.kind {
            CubeKind::Slime if self.is_tiny() => Sound::EntitySlimeJumpSmall,
            CubeKind::Slime => Sound::EntitySlimeJump,
            CubeKind::MagmaCube => Sound::EntityMagmaCubeJump,
            CubeKind::SulfurCube if self.is_tiny() => Sound::EntitySmallSulfurCubeJump,
            CubeKind::SulfurCube => Sound::EntitySulfurCubeJump,
        }
    }

    fn get_squish_sound(&self) -> Sound {
        match (self.kind, self.is_tiny()) {
            (CubeKind::Slime, true) => Sound::EntitySlimeSquishSmall,
            (CubeKind::Slime, false) => Sound::EntitySlimeSquish,
            (CubeKind::MagmaCube, true) => Sound::EntityMagmaCubeSquishSmall,
            (CubeKind::MagmaCube, false) => Sound::EntityMagmaCubeSquish,
            (CubeKind::SulfurCube, true) => Sound::EntitySmallSulfurCubeSquish,
            (CubeKind::SulfurCube, false) if self.has_body_item() => Sound::EntitySulfurCubeBounce,
            (CubeKind::SulfurCube, false) => Sound::EntitySulfurCubeSquish,
        }
    }

    /// Vanilla `Entity.playSound`.
    fn play_sound(&self, sound: Sound, volume: f32, pitch: f32) {
        let entity = self.get_entity();
        if !entity.silent.load(Relaxed) {
            entity.world.load().play_sound_fine(
                sound,
                Mob::sound_source(self),
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
            CubeKind::SulfurCube => false,
        }
    }

    fn get_attack_damage(&self) -> f32 {
        let damage = self
            .mob_entity
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32;
        match self.kind {
            CubeKind::Slime | CubeKind::SulfurCube => damage,
            CubeKind::MagmaCube => damage + 2.0,
        }
    }

    /// Vanilla `dealDamage`.
    fn deal_damage(&self, target: &dyn EntityBase) {
        let entity = self.get_entity();
        let damage_source = DamageSource {
            damage_type: DamageType::MOB_ATTACK,
            direct: Some(self),
            causing: Some(self),
        };
        if entity.is_alive()
            && do_teams_allow_damage(self, target)
            && self.mob_entity.is_in_attack_range(target)
            && entity.has_line_of_sight(target.get_entity())
            && target.damage_with_context(
                target,
                self.get_attack_damage(),
                damage_source.damage_type,
                None,
                damage_source.direct,
                damage_source.causing,
            )
        {
            let pitch = {
                let mut rng = self.get_entity().random();
                (rng.next_f32() - rng.next_f32()) * 0.2 + 1.0
            };
            self.play_sound(Sound::EntitySlimeAttack, 1.0, pitch);
            EnchantmentHelper::do_post_attack_effects(
                &entity.world.load_full(),
                target,
                &damage_source,
            );
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
            (CubeKind::SulfurCube, true) => Sound::EntitySmallSulfurCubeDeath,
            (CubeKind::SulfurCube, false) => Sound::EntitySulfurCubeDeath,
        })
    }

    fn hurt_sound(&self) -> Option<Sound> {
        Some(match (self.kind, self.is_tiny()) {
            (CubeKind::Slime, true) => Sound::EntitySlimeHurtSmall,
            (CubeKind::Slime, false) => Sound::EntitySlimeHurt,
            (CubeKind::MagmaCube, true) => Sound::EntityMagmaCubeHurtSmall,
            (CubeKind::MagmaCube, false) => Sound::EntityMagmaCubeHurt,
            (CubeKind::SulfurCube, true) => Sound::EntitySmallSulfurCubeHurt,
            (CubeKind::SulfurCube, false) => Sound::EntitySulfurCubeHurt,
        })
    }
}

impl AgeableMob for CubeMobEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable
    }

    /// `Slime` and `MagmaCube` override `canBeABaby` to false.
    fn can_be_a_baby(&self) -> bool {
        self.kind == CubeKind::SulfurCube
    }

    /// Vanilla `SulfurCube.ageBoundaryReached`: a grown up cube takes the adult size.
    fn age_boundary_reached(&self) {
        if self.kind == CubeKind::SulfurCube && !self.is_baby() {
            self.set_size(sulfur_cube::MAX_SIZE, true);
        }
    }
}

impl Mob for CubeMobEntity {
    /// Vanilla `SulfurCube.maxUpStep`: none while it carries a block.
    fn mob_max_up_step(&self) -> f32 {
        if self.has_body_item() {
            0.0
        } else {
            crate::entity::living_max_up_step(
                &self.get_mob_entity().living_entity,
                self.get_controlling_passenger(),
            )
        }
    }

    /// Vanilla `AbstractCubeMob.getPassengerAttachmentPoint`: on top, sunk in by its size.
    fn mob_passenger_attachment_point(
        &self,
        _passenger: &dyn EntityBase,
        dimensions: EntityDimensions,
        scale: f32,
    ) -> pumpkin_util::math::vector3::Vector3<f64> {
        pumpkin_util::math::vector3::Vector3::new(
            0.0,
            f64::from(dimensions.height)
                - 0.015_625 * f64::from(self.get_size()) * f64::from(scale),
            0.0,
        )
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn sound_volume(&self) -> f32 {
        self.get_sound_volume()
    }

    /// `AbstractCubeMob` plays in the hostile category, `SulfurCube` in the neutral one.
    fn sound_source(&self) -> SoundCategory {
        if self.kind == CubeKind::SulfurCube {
            SoundCategory::Neutral
        } else {
            SoundCategory::Hostile
        }
    }

    /// Vanilla `AbstractCubeMob.getMaxHeadXRot`.
    fn get_max_look_pitch_change(&self) -> f32 {
        0.0
    }

    fn default_dimensions(&self, _pose: EntityPose) -> EntityDimensions {
        Entity::type_dimensions(self.get_entity().entity_type).scale(self.get_size() as f32)
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    /// Only `SulfurCube` keeps the `AgeableMob` state Pumpkin tracks.
    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        (self.kind == CubeKind::SulfurCube).then_some(self)
    }

    /// Vanilla `getBreedOffspring`, which only `SulfurCube` overrides: a size 1 baby.
    fn spawn_as_baby(&self) -> bool {
        if self.kind != CubeKind::SulfurCube {
            return false;
        }
        self.set_size(sulfur_cube::MIN_SIZE, true);
        self.set_baby(true);
        true
    }

    fn get_base_experience_reward(&self) -> u32 {
        if self.kind == CubeKind::SulfurCube {
            return self.sulfur_base_experience_reward();
        }
        super::mob_base_experience_reward(self)
    }

    /// `setSize` sets `xpReward` to the size.
    fn xp_reward(&self) -> u32 {
        self.get_size() as u32
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int("Size", self.get_size() - 1);
        nbt.put_bool("wasOnGround", self.was_on_ground.load(Relaxed));
        self.sulfur_write_nbt(nbt);
    }

    fn mob_read_nbt_before_super(&self, nbt: &NbtCompound) {
        self.sulfur_read_nbt(nbt);
        self.set_size(nbt.get_int("Size").unwrap_or(0) + 1, false);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.was_on_ground
            .store(nbt.get_bool("wasOnGround").unwrap_or(false), Relaxed);
    }

    /// Vanilla `finalizeSpawn`: the `AgeableMob` group, then `setSpawnSize`.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        if self.kind == CubeKind::SulfurCube {
            let group_data = self.finalize_ageable_spawn(group_data);
            self.mob_entity.finalize_spawn_base();
            // `SulfurCube.setSpawnSize`.
            self.set_size(
                if self.is_baby() {
                    sulfur_cube::MIN_SIZE
                } else {
                    sulfur_cube::MAX_SIZE
                },
                true,
            );
            return Some(group_data);
        }
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
        // `level.getRandom()`.
        let mut rng = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut size_scale = rng.next_bounded_i32(3);
        if size_scale < 2 && rng.next_f32() < 0.5 * difficulty.special_multiplier {
            size_scale += 1;
        }
        drop(rng);
        self.set_size(1 << size_scale, true);
        Some(group_data)
    }

    /// The part of vanilla `tick` before `super.tick()`.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        if self.kind == CubeKind::SulfurCube {
            self.tick_fuse();
            self.prime_when_on_powered_position();
        }
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
                let mut rng = self.get_entity().random();
                // The landing particles are the client's, but the server draws their random
                // numbers too.
                let mut i = 0;
                while (i as f32) < size * 16.0 {
                    rng.next_f32();
                    rng.next_f32();
                    i += 1;
                }
                ((rng.next_f32() - rng.next_f32()) * 0.2 + 1.0) / 0.8
            };
            self.play_sound(self.get_squish_sound(), self.get_sound_volume(), pitch);
            self.target_squish.store(-0.5);
        } else if !on_ground && was_on_ground {
            self.target_squish.store(1.0);
        }
        self.was_on_ground.store(on_ground, Relaxed);
        // `decreaseSquish`.
        let decrease = match self.kind {
            CubeKind::Slime | CubeKind::SulfurCube => 0.6,
            CubeKind::MagmaCube => 0.9,
        };
        self.target_squish
            .store(self.target_squish.load() * decrease);
        if self.kind == CubeKind::SulfurCube {
            self.touch_nearby_players();
        }
    }

    fn collect_equipment_changes_override(
        &self,
        last: &[ItemStack; 8],
        touched: &mut Vec<Attributes>,
    ) {
        self.collect_body_equipment_changes(last, touched);
    }

    /// The end of vanilla `AgeableMob.aiStep`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        if self.kind == CubeKind::SulfurCube {
            self.ageable_ai_step();
        }
    }

    /// Vanilla `SulfurCube.canPickUpLoot`: an empty body, whatever the saved flag says.
    fn can_pick_up_loot(&self, _brain: &crate::entity::ai::brain::Brain) -> bool {
        if self.kind == CubeKind::SulfurCube {
            !self.has_body_item()
        } else {
            self.mob_entity.can_pick_up_loot()
        }
    }

    /// Vanilla `SulfurCube.canHoldItem`.
    fn can_hold_item(&self, stack: &ItemStack) -> bool {
        self.kind != CubeKind::SulfurCube || self.sulfur_can_hold_item(stack)
    }

    /// Vanilla `SulfurCube.pickUpItem`.
    fn mob_pick_up_item(&self, item: &crate::entity::item::ItemEntity) {
        if self.kind == CubeKind::SulfurCube {
            self.sulfur_pick_up_item(item);
        } else {
            crate::entity::mob::equipment::pick_up_item(self, item);
        }
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if let Some(sulfur) = &self.sulfur {
            sulfur.custom_server_ai_step();
        }
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

    /// Vanilla `playerTouch`. `SulfurCube.playerPush` runs from `touch_nearby_players`.
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

    /// Vanilla `remove`: a killed cube mob splits into cubes of half its size.
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
        let count = self.sulfur.as_ref().map_or_else(
            || 2 + self.get_entity().random().next_bounded_i32(3),
            sulfur_cube::SulfurCubeData::split_count,
        );
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
            let y_rot = self.get_entity().random().next_f32() * 360.0;
            child_entity.set_rotation(y_rot, 0.0);
            if self.kind == CubeKind::SulfurCube {
                cube.set_baby(true);
            }
            world.spawn_entity(child);
        }
    }

    fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if self.kind == CubeKind::SulfurCube
            && self.sulfur_hurt_server(caller, amount, damage_type, position, source, cause)
        {
            return true;
        }
        mob_hurt_server(self, caller, amount, damage_type, position, source, cause)
    }

    fn custom_knockback(
        &self,
        power: f64,
        xd: f64,
        zd: f64,
        attacker: Option<&Entity>,
        damage: f32,
        comes_from_effect: bool,
    ) -> bool {
        self.kind == CubeKind::SulfurCube
            && self.sulfur_knockback(power, xd, zd, attacker, damage, comes_from_effect)
    }

    fn mob_interact(
        &self,
        player: &Arc<Player>,
        hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        if self.kind == CubeKind::SulfurCube {
            return self.sulfur_mob_interact(player, hand, item_stack);
        }
        InteractionResult::PASS
    }

    /// Vanilla `SulfurCube.canBeLeashed`: only while it carries a body item. Slimes and
    /// magma cubes are enemies.
    fn can_be_leashed(&self) -> bool {
        self.kind == CubeKind::SulfurCube && self.has_body_item()
    }

    fn do_push(&self, entity: &dyn EntityBase) {
        if self.kind == CubeKind::SulfurCube {
            self.apply_contact_damage(entity);
        }
    }

    fn requires_custom_persistence(&self) -> bool {
        let entity = self.get_entity();
        entity.has_vehicle()
            || entity.is_leashed()
            || (self.kind == CubeKind::SulfurCube
                && (self.has_body_item() || self.is_from_bucket()))
    }

    fn can_breathe_underwater(&self) -> bool {
        self.has_body_item()
            || self
                .get_entity()
                .entity_type
                .has_tag(&tag::EntityType::MINECRAFT_CAN_BREATHE_UNDER_WATER)
    }

    fn can_freeze(&self) -> bool {
        !self.has_body_item()
    }

    fn omnidirectional_air_mover(&self) -> bool {
        self.has_body_item()
    }

    fn fluid_jump_threshold(&self) -> Option<f64> {
        (self.kind == CubeKind::SulfurCube)
            .then(|| f64::from(self.get_entity().entity_dimension.load().height) * 0.2)
    }

    fn travel_in_fluid_end(&self, water: bool) {
        if self.kind == CubeKind::SulfurCube {
            self.float_in_fluid(water);
        }
    }

    fn custom_look_control_tick(&self, _look_control: &mut LookControl) -> bool {
        if !self.has_body_item() {
            return false;
        }
        // `SulfurCubeLookControl.tick`: square up to the nearest quarter turn.
        let entity = self.get_entity();
        let close_angle = wrap_degrees_90(entity.yaw.load());
        let y_rot = entity.yaw.load() - close_angle;
        entity.yaw.store(y_rot);
        entity.head_yaw.store(y_rot);
        true
    }

    fn play_step_sound(
        &self,
        caller: &dyn EntityBase,
        _pos: &BlockPos,
        _block: &'static Block,
        state: &'static BlockState,
    ) {
        if self.has_body_item() {
            return;
        }
        match sounds::step_sound(self.get_entity().entity_type) {
            sounds::StepSound::Block => self.get_entity().play_step_sound_default(caller, state),
            sounds::StepSound::Silent => {}
            sounds::StepSound::Fixed(sound, volume) => {
                caller.play_sound_at_self(sound, volume, 1.0);
            }
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
        // `SulfurCubeMobMoveControl.tick`: a cube holding a block stays put.
        if cube.has_body_item() {
            return;
        }
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
        cube.get_target()
            .is_some_and(|target| Mob::can_attack(&*cube, target.as_ref()))
    }

    fn start(&mut self, _mob: &dyn Mob) {
        self.grow_tired_timer = to_goal_ticks(300);
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        let Some(cube) = self.cube.upgrade() else {
            return false;
        };
        let Some(target) = cube.get_target() else {
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
        if let Some(target) = cube.get_target() {
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
        let roll = cube.get_entity().random().next_f32();
        if roll < 0.8 {
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
            cube.get_target().is_none()
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
                let mut rng = cube.get_entity().random();
                (40 + rng.next_bounded_i32(60), rng.next_bounded_i32(360))
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
