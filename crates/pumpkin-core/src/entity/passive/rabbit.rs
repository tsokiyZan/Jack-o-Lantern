use std::any::Any;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use crate::block::blocks::plant::crop::CropBlockBase;
use crate::block::blocks::plant::crop::carrot::CarrotBlock;
use crate::entity::ai::control::jump_control::JumpControlTrait;
use crate::entity::ai::control::move_control::{MoveControl, Operation};
use crate::entity::ai::control::{Control, MoveControlTrait};
use crate::entity::ai::goal::climb_on_top_of_powder_snow::ClimbOnTopOfPowderSnowGoal;
use crate::entity::ai::goal::move_to_target_pos::{MoveToTargetPos, MoveToTargetPosGoal};
use crate::entity::ai::goal::revenge::RevengeGoal;
use crate::entity::ai::goal::{Controls, Goal, ParentHandle};
use crate::world::World;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::entity_status::EntityStatus;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockStateId};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        active_target::ActiveTargetGoal, avoid_entity::AvoidEntityGoal, breed::BreedGoal,
        escape_danger::EscapeDangerGoal, look_at_entity::LookAtEntityGoal,
        melee_attack::MeleeAttackGoal, swim::SwimGoal, tempt::TemptGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

const TEMPT_ITEMS: &[&Item] = &[&Item::CARROT, &Item::GOLDEN_CARROT, &Item::DANDELION];

/// Vanilla `Rabbit.STROLL_SPEED_MOD`.
const STROLL_SPEED_MOD: f64 = 0.6;
/// Vanilla `Rabbit.FLEE_SPEED_MOD`.
const FLEE_SPEED_MOD: f64 = 2.2;
/// Vanilla `Rabbit.BABY_JUMP_HEIGHT`.
const BABY_JUMP_HEIGHT: f64 = 0.5;
/// Vanilla `Rabbit.ADULT_JUMP_HEIGHT`.
const ADULT_JUMP_HEIGHT: f64 = 1.5;
/// Vanilla `Rabbit.JUMP_DELAY_TICKS`.
const JUMP_DELAY_TICKS: i32 = 10;
/// Vanilla `Rabbit.PANIC_JUMP_DELAY_TICKS`.
const PANIC_JUMP_DELAY_TICKS: i32 = 3;
/// Vanilla `Rabbit.JUMP_DURATION_IN_TICKS`.
const JUMP_DURATION_IN_TICKS: i32 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum RabbitVariant {
    #[default]
    Brown = 0,
    White = 1,
    Black = 2,
    WhiteSplotched = 3,
    Gold = 4,
    Salt = 5,
    Evil = 99,
}

impl RabbitVariant {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::White,
            2 => Self::Black,
            3 => Self::WhiteSplotched,
            4 => Self::Gold,
            5 => Self::Salt,
            99 => Self::Evil,
            _ => Self::Brown,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub fn random_variant() -> Self {
        let mut rng = rand::rng();
        match rng.random_range(0..6) {
            1 => Self::White,
            2 => Self::Black,
            3 => Self::WhiteSplotched,
            4 => Self::Gold,
            5 => Self::Salt,
            _ => Self::Brown,
        }
    }
}

pub struct RabbitEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub variant: AtomicI32,
    pub more_carrot_ticks: AtomicI32,
    jump_ticks: AtomicI32,
    jump_duration: AtomicI32,
    was_on_ground: AtomicBool,
    jump_delay_ticks: AtomicI32,
}

impl RabbitEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        let variant = RabbitVariant::random_variant();
        let rabbit = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            variant: AtomicI32::new(variant.id()),
            more_carrot_ticks: AtomicI32::new(0),
            jump_ticks: AtomicI32::new(0),
            jump_duration: AtomicI32::new(0),
            was_on_ground: AtomicBool::new(false),
            jump_delay_ticks: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(rabbit);
        *mob_arc
            .mob_entity
            .jump_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(RabbitJumpControl::default());
        *mob_arc
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(RabbitMoveControl::new(Arc::downgrade(&mob_arc)));
        mob_arc.set_speed_modifier(0.0);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(1, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(1, Box::new(ClimbOnTopOfPowderSnowGoal));
            goal_selector.add_goal(
                1,
                EscapeDangerGoal::new(FLEE_SPEED_MOD).ticking_with(rabbit_panic_tick),
            );
            goal_selector.add_goal(2, BreedGoal::new(0.8));
            goal_selector.add_goal(3, Box::new(TemptGoal::new(1.0, TEMPT_ITEMS, false)));
            // RabbitAvoidEntityGoal: only while not the killer bunny.
            goal_selector.add_goal(
                4,
                Box::new(
                    AvoidEntityGoal::new(&EntityType::PLAYER, 8.0, 2.2, 2.2)
                        .gated_by(is_not_evil_rabbit),
                ),
            );
            goal_selector.add_goal(
                4,
                Box::new(
                    AvoidEntityGoal::new(&EntityType::WOLF, 10.0, 2.2, 2.2)
                        .gated_by(is_not_evil_rabbit),
                ),
            );
            goal_selector.add_goal(
                4,
                Box::new(
                    AvoidEntityGoal::any_mob(4.0, 2.2, 2.2)
                        .avoiding_only(|target| {
                            target.entity.entity_type.category
                                == &pumpkin_data::entity::MobCategory::MONSTER
                        })
                        .gated_by(is_not_evil_rabbit),
                ),
            );
            goal_selector.add_goal(5, RaidGardenGoal::new());
            goal_selector.add_goal(
                6,
                Box::new(WanderAroundGoal::avoiding_water(STROLL_SPEED_MOD)),
            );
            goal_selector.add_goal(
                11,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 10.0),
            );
        };

        // The field `idleAnimationTimeout = this.random.nextInt(40) + 180` only feeds the
        // client's animation, but its initializer draws on the server too. Java runs it after
        // `Mob`'s constructor has called `registerGoals`; nothing between it and the end of
        // vanilla's constructor draws.
        mob_arc.get_entity().random().next_bounded_i32(40);

        mob_arc
    }

    #[must_use]
    pub fn get_variant(&self) -> RabbitVariant {
        RabbitVariant::from_id(self.variant.load(Ordering::Relaxed))
    }

    pub fn set_variant(&self, variant: RabbitVariant) {
        self.variant.store(variant.id(), Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::rabbit::DATA_TYPE_ID,
            VarInt(variant.id()),
        );

        if variant == RabbitVariant::Evil {
            let mut goal_selector = self
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            goal_selector.add_goal(4, Box::new(MeleeAttackGoal::new(1.4, true)));

            let mut target_selector = self
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            target_selector.add_goal(1, Box::new(RevengeGoal::new(true).alerting_others()));
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&self.mob_entity, &EntityType::PLAYER, true),
            );
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&self.mob_entity, &EntityType::WOLF, true),
            );
        }
    }
}

impl RabbitEntity {
    fn with_move_control<R>(&self, f: impl FnOnce(&mut RabbitMoveControl) -> R) -> Option<R> {
        let mut guard = self
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let control: &mut dyn Any = &mut **guard;
        control.downcast_mut::<RabbitMoveControl>().map(f)
    }

    fn with_jump_control<R>(&self, f: impl FnOnce(&mut RabbitJumpControl) -> R) -> Option<R> {
        let mut guard = self
            .mob_entity
            .jump_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let control: &mut dyn Any = &mut **guard;
        control.downcast_mut::<RabbitJumpControl>().map(f)
    }

    /// Vanilla `RabbitJumpControl.wantJump`.
    fn want_jump(&self) -> bool {
        self.with_jump_control(|control| control.jump)
            .unwrap_or(false)
    }

    /// Vanilla `moveControl.getSpeedModifier()`.
    fn move_speed_modifier(&self) -> f64 {
        self.with_move_control(|control| control.move_control.speed_modifier)
            .unwrap_or(0.0)
    }

    /// Vanilla `path.getNextEntityPos(this)` of an unfinished navigation path.
    fn next_path_pos(&self) -> Option<Vector3<f64>> {
        let width = self.get_entity().entity_dimension.load().width;
        self.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_path()
            .filter(|path| !path.is_done())
            .and_then(|path| path.get_next_entity_pos(width))
    }

    /// Vanilla `Rabbit.getJumpPower`: higher when the path or the wanted position climbs,
    /// or when the rabbit runs into a wall.
    fn get_jump_power(&self) -> f32 {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        let (speed_modifier, wanted_y) = self
            .with_move_control(|control| {
                (
                    control.move_control.speed_modifier,
                    control.move_control.wanted_y,
                )
            })
            .unwrap_or_default();
        let mut base_jump_power = if speed_modifier <= STROLL_SPEED_MOD {
            0.2f32
        } else {
            0.3
        };

        let y = entity.pos.load().y;
        if self.next_path_pos().is_some_and(|pos| pos.y > y + 0.5) {
            base_jump_power = 0.5;
        }

        if entity.horizontal_collision.load(Ordering::Relaxed)
            || living.jumping.load(Ordering::Relaxed) && wanted_y > y + 0.5
        {
            base_jump_power = 0.5;
        }

        living.get_jump_velocity(base_jump_power / 0.42)
    }

    /// Vanilla `Rabbit.setSpeedModifier`.
    fn set_speed_modifier(&self, speed: f64) {
        self.with_move_control(|control| control.set_speed_modifier(self, speed));
    }

    /// Vanilla `Rabbit.setJumping`: every jump start plays the jump sound.
    fn set_jumping(&self, jump: bool) {
        self.mob_entity
            .living_entity
            .jumping
            .store(jump, Ordering::Relaxed);
        if jump {
            // Vanilla rounds each product, so no fused multiply-add.
            let pitch = {
                let mut random = self.get_entity().random();
                ((random.next_f32() - random.next_f32()) * 0.2 + 1.0) * 0.8
            };
            self.play_sound_at_self(Sound::EntityRabbitJump, self.sound_volume(), pitch);
        }
    }

    /// Vanilla `Rabbit.startJumping`.
    fn start_jumping(&self) {
        self.set_jumping(true);
        self.jump_duration
            .store(JUMP_DURATION_IN_TICKS, Ordering::Relaxed);
        self.jump_ticks.store(0, Ordering::Relaxed);
    }

    /// Vanilla `Rabbit.facePoint`.
    fn face_point(&self, face_x: f64, face_z: f64) {
        let entity = self.get_entity();
        let pos = entity.pos.load();
        entity.yaw.store(
            (math::atan2(face_z - pos.z, face_x - pos.x) * f64::from(180.0f32)
                / f64::from(std::f32::consts::PI)) as f32
                - 90.0,
        );
    }

    /// Vanilla `Rabbit.checkLandingDelay`, with `setLandingDelay` and `disableJumpControl`.
    fn check_landing_delay(&self) {
        let delay = if self.move_speed_modifier() < FLEE_SPEED_MOD {
            JUMP_DELAY_TICKS
        } else {
            PANIC_JUMP_DELAY_TICKS
        };
        self.jump_delay_ticks.store(delay, Ordering::Relaxed);
        self.with_jump_control(|control| control.can_jump = false);
    }
}

impl AgeableMob for RabbitEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for RabbitEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_RABBIT_FOOD)
            || TEMPT_ITEMS.iter().any(|i| i.id == item_stack.item.id)
    }
}

/// Vanilla `Rabbit.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::RABBIT_BABY_DIMENSIONS;

impl Mob for RabbitEntity {
    /// Vanilla `Rabbit.playAttackSound`: only the killer bunny, with a voice pitch drawn even
    /// when it is silent.
    fn play_attack_sound(&self) {
        if self.get_variant() == RabbitVariant::Evil {
            let pitch = {
                let mut random = self.get_entity().random();
                (random.next_f32() - random.next_f32()) * 0.2 + 1.0
            };
            self.play_sound_at_self(pumpkin_data::sound::Sound::EntityRabbitAttack, 1.0, pitch);
        }
    }

    /// Vanilla `Rabbit.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
        nbt.put_int("RabbitType", self.get_variant().id());
        nbt.put_int(
            "MoreCarrotTicks",
            self.more_carrot_ticks.load(Ordering::Relaxed),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        if let Some(rabbit_type) = nbt.get_int("RabbitType") {
            self.set_variant(RabbitVariant::from_id(rabbit_type));
        }
        if let Some(ticks) = nbt.get_int("MoreCarrotTicks") {
            self.more_carrot_ticks.store(ticks, Ordering::Relaxed);
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();
    }

    /// Vanilla `Rabbit.customServerAiStep`: the hop timing.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if self.jump_delay_ticks.load(Ordering::Relaxed) > 0 {
            self.jump_delay_ticks.fetch_sub(1, Ordering::Relaxed);
        }

        let ticks = self.more_carrot_ticks.load(Ordering::Relaxed);
        if ticks > 0 {
            let ticks = ticks - self.get_entity().random().next_bounded_i32(3);
            self.more_carrot_ticks
                .store(ticks.max(0), Ordering::Relaxed);
        }

        let entity = self.get_entity();
        if entity.on_ground.load(Ordering::Relaxed) {
            if !self.was_on_ground.load(Ordering::Relaxed) {
                self.set_jumping(false);
                self.check_landing_delay();
            }

            if self.get_variant() == RabbitVariant::Evil
                && self.jump_delay_ticks.load(Ordering::Relaxed) == 0
                && let Some(target) = self.get_target()
            {
                let target_pos = target.get_entity().pos.load();
                if entity.pos.load().squared_distance_to_vec(&target_pos) < 16.0 {
                    self.face_point(target_pos.x, target_pos.z);
                    self.with_move_control(|control| {
                        let speed_modifier = control.move_control.speed_modifier;
                        control.set_wanted_position(
                            target_pos.x,
                            target_pos.y,
                            target_pos.z,
                            speed_modifier,
                        );
                    });
                    self.start_jumping();
                    self.was_on_ground.store(true, Ordering::Relaxed);
                }
            }

            let (want_jump, can_jump) = self
                .with_jump_control(|control| (control.jump, control.can_jump))
                .unwrap_or_default();
            if !want_jump {
                let wanted = self
                    .with_move_control(|control| {
                        let move_control = &control.move_control;
                        move_control.has_wanted().then_some(Vector3::new(
                            move_control.wanted_x,
                            move_control.wanted_y,
                            move_control.wanted_z,
                        ))
                    })
                    .flatten();
                if let Some(wanted) = wanted
                    && self.jump_delay_ticks.load(Ordering::Relaxed) == 0
                {
                    let pos = self.next_path_pos().unwrap_or(wanted);
                    self.face_point(pos.x, pos.z);
                    self.start_jumping();
                }
            } else if !can_jump {
                self.with_jump_control(|control| control.can_jump = true);
            }
        }

        self.was_on_ground
            .store(entity.on_ground.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    /// Vanilla `Rabbit.jumpFromGround`: a hop from standstill gets a push forwards.
    fn jump_from_ground(&self) {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        living.jump_with_power(self.get_jump_power());
        if self.move_speed_modifier() > 0.0 {
            let current = entity.velocity.load().horizontal_length_squared();
            if current < 0.01 {
                let height = if entity.is_baby() {
                    BABY_JUMP_HEIGHT
                } else {
                    ADULT_JUMP_HEIGHT
                };
                entity
                    .update_velocity_from_input(Vector3::new(0.0, height, 1.0), f64::from(0.1f32));
            }
        }

        entity
            .world
            .load()
            .broadcast_entity_event(entity, EntityStatus::Jump, None);
    }

    /// The end of vanilla `Rabbit.aiStep`: a hop lasts at most `JUMP_DURATION_IN_TICKS`.
    fn post_tick(&self) {
        let jump_ticks = self.jump_ticks.load(Ordering::Relaxed);
        let jump_duration = self.jump_duration.load(Ordering::Relaxed);
        if jump_ticks != jump_duration {
            self.jump_ticks.store(jump_ticks + 1, Ordering::Relaxed);
        } else if jump_duration != 0 {
            self.jump_ticks.store(0, Ordering::Relaxed);
            self.jump_duration.store(0, Ordering::Relaxed);
            self.set_jumping(false);
        }
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::rabbit::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::rabbit::DATA_TYPE_ID,
            VarInt(self.get_variant().id()),
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        self.animal_interact(player, item_stack, Sound::EntityRabbitAmbient)
    }
}

/// Vanilla `RabbitPanicGoal.tick`: keeps the flee speed for the next hops.
fn rabbit_panic_tick(mob: &dyn Mob, speed_modifier: f64) {
    if let Some(rabbit) = mob.cast_any().downcast_ref::<RabbitEntity>() {
        rabbit.set_speed_modifier(speed_modifier);
    }
}

/// Vanilla `Rabbit.RabbitJumpControl`: a jump request starts a hop, and the jump stays on
/// until the rabbit lands or the hop times out.
#[derive(Default)]
struct RabbitJumpControl {
    jump: bool,
    can_jump: bool,
}

impl Control for RabbitJumpControl {}

impl JumpControlTrait for RabbitJumpControl {
    fn jump(&mut self) {
        self.jump = true;
    }

    fn tick(&mut self, mob: &dyn Mob) {
        if self.jump {
            if let Some(rabbit) = mob.cast_any().downcast_ref::<RabbitEntity>() {
                rabbit.start_jumping();
            }
            self.jump = false;
        }
    }
}

/// Vanilla `Rabbit.RabbitMoveControl`: a rabbit only moves while it hops, at the speed
/// it was last asked for.
struct RabbitMoveControl {
    move_control: MoveControl,
    next_jump_speed: f64,
    rabbit: Weak<RabbitEntity>,
}

impl RabbitMoveControl {
    fn new(rabbit: Weak<RabbitEntity>) -> Self {
        Self {
            move_control: MoveControl::default(),
            next_jump_speed: 0.0,
            rabbit,
        }
    }

    /// Vanilla `Rabbit.setSpeedModifier`, which sets the speed on this move control.
    fn set_speed_modifier(&mut self, mob: &dyn Mob, speed: f64) {
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_speed(speed);
        let MoveControl {
            wanted_x,
            wanted_y,
            wanted_z,
            ..
        } = self.move_control;
        self.set_wanted_position(wanted_x, wanted_y, wanted_z, speed);
    }
}

impl Control for RabbitMoveControl {}

impl MoveControlTrait for RabbitMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let living = &mob.get_mob_entity().living_entity;
        let Some(rabbit) = mob.cast_any().downcast_ref::<RabbitEntity>() else {
            return;
        };
        if living.entity.on_ground.load(Ordering::Relaxed)
            && !living.jumping.load(Ordering::Relaxed)
            && !rabbit.want_jump()
        {
            self.set_speed_modifier(mob, 0.0);
        } else if self.move_control.has_wanted()
            || self.move_control.operation == Operation::Jumping
        {
            self.set_speed_modifier(mob, self.next_jump_speed);
        }

        MoveControlTrait::tick(&mut self.move_control, mob);
    }

    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, mut speed_modifier: f64) {
        if self
            .rabbit
            .upgrade()
            .is_some_and(|rabbit| rabbit.get_entity().is_in_water())
        {
            speed_modifier = 1.5;
        }

        MoveControlTrait::set_wanted_position(&mut self.move_control, x, y, z, speed_modifier);
        if speed_modifier > 0.0 {
            self.next_jump_speed = speed_modifier;
        }
    }

    fn strafe(&mut self, forward: f32, right: f32) {
        MoveControlTrait::strafe(&mut self.move_control, forward, right);
    }

    fn has_wanted(&self) -> bool {
        MoveControlTrait::has_wanted(&self.move_control)
    }
}

/// `RabbitAvoidEntityGoal`: the killer bunny doesn't flee.
fn is_not_evil_rabbit(mob: &dyn Mob) -> bool {
    mob.cast_any()
        .downcast_ref::<RabbitEntity>()
        .is_none_or(|rabbit| rabbit.get_variant() != RabbitVariant::Evil)
}

/// Vanilla `Rabbit.RaidGardenGoal`: a rabbit that wants food walks to a ripe carrot and
/// nibbles it down a stage (or eats it up).
struct RaidGardenGoal {
    goal: MoveToTargetPosGoal<Self>,
    wants_to_raid: AtomicBool,
    can_raid: AtomicBool,
}

impl RaidGardenGoal {
    #[expect(
        clippy::unnecessary_box_returns,
        reason = "the goal's parent handle points into the box"
    )]
    fn new() -> Box<Self> {
        let mut this = Box::new(Self {
            goal: MoveToTargetPosGoal::new(ParentHandle::none(), f64::from(0.7f32), 16, 1),
            wants_to_raid: AtomicBool::new(false),
            can_raid: AtomicBool::new(false),
        });
        // SAFETY: `this` heap allocation address is pinned in Box and outlives `ParentHandle` references.
        this.goal.move_to_target_pos = unsafe { ParentHandle::new(&this) };
        this
    }
}

impl MoveToTargetPos for RaidGardenGoal {
    /// Vanilla's `isValidTarget` also remembers that a raid is possible.
    fn is_target_pos(&self, world: Arc<World>, block_pos: BlockPos) -> bool {
        if world
            .get_block(&block_pos)
            .has_tag(&tag::Block::MINECRAFT_SUPPORTS_CROPS)
            && self.wants_to_raid.load(Ordering::Relaxed)
            && !self.can_raid.load(Ordering::Relaxed)
        {
            let (crop, state) = world.get_block_and_state_id(&block_pos.up());
            if crop == &Block::CARROTS && CarrotBlock.get_age(state, crop) >= CarrotBlock.max_age()
            {
                self.can_raid.store(true, Ordering::Relaxed);
                return true;
            }
        }
        false
    }
}

impl Goal for RaidGardenGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if self.goal.cooldown <= 0 {
            if !mob
                .get_entity()
                .world
                .load()
                .level_info
                .load()
                .game_rules
                .mob_griefing
            {
                return false;
            }
            self.can_raid.store(false, Ordering::Relaxed);
            let wants_more_food = mob
                .cast_any()
                .downcast_ref::<RabbitEntity>()
                .is_some_and(|rabbit| rabbit.more_carrot_ticks.load(Ordering::Relaxed) <= 0);
            self.wants_to_raid.store(wants_more_food, Ordering::Relaxed);
        }
        self.goal.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_raid.load(Ordering::Relaxed) && self.goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.goal.start(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.goal.tick(mob);
        let target = self.goal.target_pos;
        mob.get_mob_entity()
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .look_at_with_range(
                f64::from(target.0.x) + 0.5,
                f64::from(target.0.y + 1),
                f64::from(target.0.z) + 0.5,
                10.0,
                mob.get_max_look_pitch_change(),
            );
        if !self.goal.is_reached_target() {
            return;
        }
        let world = mob.get_entity().world.load_full();
        let crops_pos = target.up();
        let (crop, state) = world.get_block_and_state_id(&crops_pos);
        if self.can_raid.load(Ordering::Relaxed) && crop == &Block::CARROTS {
            let age = CarrotBlock.get_age(state, crop);
            if age == 0 {
                world.set_block_state(&crops_pos, BlockStateId::AIR, BlockFlags::NOTIFY_LISTENERS);
                world.break_block(&crops_pos, None, BlockFlags::NOTIFY_ALL);
            } else {
                world.set_block_state(
                    &crops_pos,
                    CarrotBlock.state_with_age(crop, state, age - 1),
                    BlockFlags::NOTIFY_LISTENERS,
                );
                world.sync_world_event(
                    WorldEvent::ParticlesAndSoundDestroyBlock,
                    crops_pos,
                    i32::from(state.as_u16()),
                );
            }
            if let Some(rabbit) = mob.cast_any().downcast_ref::<RabbitEntity>() {
                rabbit.more_carrot_ticks.store(40, Ordering::Relaxed);
            }
        }
        self.can_raid.store(false, Ordering::Relaxed);
        self.goal.cooldown = 10;
    }

    fn should_run_every_tick(&self) -> bool {
        self.goal.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}
