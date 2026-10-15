use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use crate::block::blocks::plant::crop::CropBlockBase;
use crate::block::blocks::plant::crop::carrot::CarrotBlock;
use crate::entity::ai::goal::climb_on_top_of_powder_snow::ClimbOnTopOfPowderSnowGoal;
use crate::entity::ai::goal::move_to_target_pos::{MoveToTargetPos, MoveToTargetPosGoal};
use crate::entity::ai::goal::revenge::RevengeGoal;
use crate::entity::ai::goal::{Controls, Goal, ParentHandle};
use crate::world::World;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockStateId};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
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
        };
        let mob_arc = Arc::new(rabbit);
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
            // RabbitPanicGoal
            goal_selector.add_goal(1, EscapeDangerGoal::new(2.2));
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
            goal_selector.add_goal(6, Box::new(WanderAroundGoal::avoiding_water(0.6)));
            goal_selector.add_goal(
                11,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 10.0),
            );
        };

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
const BABY_DIMENSIONS: EntityDimensions =
    EntityDimensions::scalable(0.24, 0.4).with_eye_height(0.39);

impl Mob for RabbitEntity {
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

    /// The `moreCarrotTicks` countdown from vanilla `Rabbit.customServerAiStep`.
    // TODO: the rest of it, the hop timing with RabbitJumpControl and RabbitMoveControl.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        let ticks = self.more_carrot_ticks.load(Ordering::Relaxed);
        if ticks > 0 {
            let ticks = ticks - self.get_random().random_range(0..3);
            self.more_carrot_ticks
                .store(ticks.max(0), Ordering::Relaxed);
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
