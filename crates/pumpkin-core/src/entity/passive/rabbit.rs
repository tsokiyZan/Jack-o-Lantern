use std::sync::{
    Arc, Weak,
    atomic::{AtomicI32, Ordering},
};

use crate::entity::ai::goal::climb_on_top_of_powder_snow::ClimbOnTopOfPowderSnowGoal;
use crate::entity::ai::goal::revenge::RevengeGoal;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::EntityDimensions;
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
            // TODO: 5 RaidGardenGoal is not ported.
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
