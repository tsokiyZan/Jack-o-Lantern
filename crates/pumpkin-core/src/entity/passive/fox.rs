use std::sync::{
    Arc, Weak,
    atomic::{AtomicI32, AtomicU8, Ordering},
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::Block;
use pumpkin_data::block_properties::NetherWartLikeProperties;
use pumpkin_data::data_component_impl::{ConsumableImpl, EquipmentSlot, FoodImpl};
use pumpkin_data::entity::{EntityPose, EntityStatus, EntityType};
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::world::WorldEvent;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::world::BlockFlags;
use uuid::Uuid;

use crate::block::blocks::plant::cave_vines::CaveVinesBlock;
use crate::entity::ai::control::move_control::MoveControl;
use crate::entity::ai::control::{Control, MoveControlTrait};
use crate::entity::ai::goal::active_target::ActiveTargetGoal;
use crate::entity::ai::goal::avoid_entity::AvoidEntityGoal;
use crate::entity::ai::goal::breed::BreedGoal;
use crate::entity::ai::goal::climb_on_top_of_powder_snow::ClimbOnTopOfPowderSnowGoal;
use crate::entity::ai::goal::escape_danger::EscapeDangerGoal;
use crate::entity::ai::goal::flee_sun::FleeSunGoal;
use crate::entity::ai::goal::follow_parent::FollowParentGoal;
use crate::entity::ai::goal::leap_at_target::LeapAtTargetGoal;
use crate::entity::ai::goal::look_at_entity::LookAtEntityGoal;
use crate::entity::ai::goal::melee_attack::MeleeAttackGoal;
use crate::entity::ai::goal::move_to_target_pos::{MoveToTargetPos, MoveToTargetPosGoal};
use crate::entity::ai::goal::stroll_through_village::StrollThroughVillageGoal;
use crate::entity::ai::goal::swim::SwimGoal;
use crate::entity::ai::goal::wander_around::WanderAroundGoal;
use crate::entity::ai::goal::{Controls, Goal, ParentHandle, to_goal_ticks};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::item::ItemEntity;
use crate::entity::living::LivingEntity;
use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::{Mob, MobEntity, is_monster, set_mob_target_base};
use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};
use crate::world::World;

/// Vanilla `Fox.FLAG_SITTING`.
const FLAG_SITTING: u8 = 1;
/// Vanilla `Fox.FLAG_CROUCHING`.
const FLAG_CROUCHING: u8 = 4;
/// Vanilla `Fox.FLAG_INTERESTED`.
const FLAG_INTERESTED: u8 = 8;
/// Vanilla `Fox.FLAG_POUNCING`.
const FLAG_POUNCING: u8 = 16;
/// Vanilla `Fox.FLAG_SLEEPING`.
const FLAG_SLEEPING: u8 = 32;
/// Vanilla `Fox.FLAG_FACEPLANTED`.
const FLAG_FACEPLANTED: u8 = 64;
/// Vanilla `Fox.FLAG_DEFENDING`.
const FLAG_DEFENDING: u8 = 128;
/// Vanilla `Fox.MIN_TICKS_BEFORE_EAT`.
const MIN_TICKS_BEFORE_EAT: i32 = 600;
/// Vanilla `Fox.MAX_CROUCH_AMOUNT`.
const MAX_CROUCH_AMOUNT: f32 = 5.0;
/// How long after its last hurt a mob still counts as an attacker for
/// `TRUSTED_TARGET_SELECTOR`.
const TRUSTED_TARGET_MEMORY_TICKS: i32 = 600;
/// The pickup delay `spitOutItem` gives the item the fox drops.
const SPIT_PICKUP_DELAY: u8 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum FoxVariant {
    #[default]
    Red = 0,
    Snow = 1,
}

impl FoxVariant {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Snow,
            _ => Self::Red,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Snow => "snow",
        }
    }

    /// Vanilla `Fox.Variant.CODEC`: the serialized name, if it is one.
    #[must_use]
    pub fn from_serialized_name(name: &str) -> Option<Self> {
        match name {
            "red" => Some(Self::Red),
            "snow" => Some(Self::Snow),
            _ => None,
        }
    }

    #[must_use]
    pub fn from_name(s: &str) -> Self {
        match s {
            "snow" | "minecraft:snow" => Self::Snow,
            _ => Self::Red,
        }
    }

    /// Vanilla `Fox.Variant.byBiome`.
    #[must_use]
    pub fn by_biome(world: &World, pos: &BlockPos) -> Self {
        if world
            .get_zoomed_biome(pos)
            .has_tag(&tag::WorldgenBiome::MINECRAFT_SPAWNS_SNOW_FOXES)
        {
            Self::Snow
        } else {
            Self::Red
        }
    }
}

/// The three target goals vanilla's fox builds in `registerGoals` and only adds in
/// `setTargetGoals`.
#[derive(Clone, Copy)]
enum FoxTargetGoal {
    Land = 1,
    TurtleEgg = 2,
    Fish = 4,
}

impl FoxTargetGoal {
    fn create(self, mob: &MobEntity) -> ActiveTargetGoal {
        match self {
            Self::Land => *ActiveTargetGoal::predicated(mob, 10, false, |target, _world| {
                is_stalkable_prey(target.entity.entity_type)
            }),
            Self::TurtleEgg => ActiveTargetGoal::new(
                mob,
                &EntityType::TURTLE,
                10,
                false,
                false,
                // Vanilla `Turtle.BABY_ON_LAND_SELECTOR`.
                Some(|target: &LivingEntity, _world: &World| {
                    target.entity.is_baby() && !target.entity.is_in_water()
                }),
            ),
            Self::Fish => *ActiveTargetGoal::predicated(mob, 20, false, |target, _world| {
                is_schooling_fish(target.entity.entity_type)
            }),
        }
    }
}

/// Vanilla `Fox.STALKABLE_PREY`.
fn is_stalkable_prey(entity_type: &EntityType) -> bool {
    entity_type == &EntityType::CHICKEN || entity_type == &EntityType::RABBIT
}

/// Vanilla `instanceof AbstractSchoolingFish`.
fn is_schooling_fish(entity_type: &EntityType) -> bool {
    [
        &EntityType::COD,
        &EntityType::SALMON,
        &EntityType::TROPICAL_FISH,
    ]
    .contains(&entity_type)
}

fn as_fox(mob: &dyn Mob) -> Option<&FoxEntity> {
    mob.cast_any().downcast_ref::<FoxEntity>()
}

fn not_defending(mob: &dyn Mob) -> bool {
    as_fox(mob).is_some_and(|fox| !fox.is_defending())
}

/// Represents a Fox, a passive/neutral mob.
///
/// Wiki: <https://minecraft.wiki/w/Fox>
pub struct FoxEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub variant: AtomicI32,
    pub flags: AtomicU8,
    /// Vanilla `DATA_TRUSTED_ID_0` and `DATA_TRUSTED_ID_1`.
    trusted: [AtomicCell<Option<Uuid>>; 2],
    interested_angle: AtomicCell<f32>,
    interested_angle_o: AtomicCell<f32>,
    crouch_amount: AtomicCell<f32>,
    crouch_amount_o: AtomicCell<f32>,
    ticks_since_eaten: AtomicI32,
    /// The [`FoxTargetGoal`]s already in the target selector; vanilla's selector is a set,
    /// so `setTargetGoals` never adds the same goal twice.
    target_goals_added: AtomicU8,
}

impl FoxEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        // Vanilla `Fox`'s constructor.
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(FoxMoveControl::default());
        mob_entity.set_pathfinding_malus(PathType::DangerOther, 0.0);
        mob_entity.set_pathfinding_malus(PathType::DamageOther, 0.0);
        mob_entity.set_can_pick_up_loot(true);
        mob_entity.set_required_path_length(32.0);
        let fox = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            variant: AtomicI32::new(FoxVariant::Red.id()),
            flags: AtomicU8::new(0),
            trusted: [AtomicCell::new(None), AtomicCell::new(None)],
            interested_angle: AtomicCell::new(0.0),
            interested_angle_o: AtomicCell::new(0.0),
            crouch_amount: AtomicCell::new(0.0),
            crouch_amount_o: AtomicCell::new(0.0),
            ticks_since_eaten: AtomicI32::new(0),
            target_goals_added: AtomicU8::new(0),
        };
        let mob_arc = Arc::new(fox);
        mob_arc.register_goals();
        mob_arc
    }

    /// Vanilla `Fox.registerGoals`.
    fn register_goals(self: &Arc<Self>) {
        let fox_weak = Arc::downgrade(self);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = self.clone();
            Arc::downgrade(&mob_arc)
        };
        let mob_entity = &self.mob_entity;

        {
            let mut goal_selector = mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(0, Box::new(FoxFloatGoal::new(mob_entity)));
            goal_selector.add_goal(0, Box::new(ClimbOnTopOfPowderSnowGoal));
            goal_selector.add_goal(1, Box::new(FaceplantGoal { countdown: 0 }));
            goal_selector.add_goal(
                2,
                EscapeDangerGoal::new(2.2).panics_only_when(not_defending),
            );
            goal_selector.add_goal(3, Box::new(FoxBreedGoal::new(1.0)));
            let player_fox = fox_weak.clone();
            goal_selector.add_goal(
                4,
                Box::new(
                    AvoidEntityGoal::new(&EntityType::PLAYER, 16.0, 1.6, 1.4).avoiding_only(
                        move |target| {
                            // Vanilla `AVOID_PLAYERS`; the creative and spectator check is the
                            // goal's own.
                            !target.entity.is_sneaking()
                                && player_fox.upgrade().is_some_and(|fox| {
                                    !fox.trusts(&target.entity.entity_uuid) && !fox.is_defending()
                                })
                        },
                    ),
                ),
            );
            let wolf_fox = fox_weak.clone();
            goal_selector.add_goal(
                4,
                Box::new(
                    AvoidEntityGoal::new(&EntityType::WOLF, 8.0, 1.6, 1.4).avoiding_only(
                        move |target| {
                            !crate::entity::mob::is_tame(target)
                                && wolf_fox.upgrade().is_some_and(|fox| !fox.is_defending())
                        },
                    ),
                ),
            );
            let bear_fox = fox_weak.clone();
            goal_selector.add_goal(
                4,
                Box::new(
                    AvoidEntityGoal::new(&EntityType::POLAR_BEAR, 8.0, 1.6, 1.4).avoiding_only(
                        move |_target| bear_fox.upgrade().is_some_and(|fox| !fox.is_defending()),
                    ),
                ),
            );
            goal_selector.add_goal(5, Box::new(StalkPreyGoal));
            goal_selector.add_goal(6, Box::new(FoxPounceGoal));
            goal_selector.add_goal(6, Box::new(SeekShelterGoal::new(1.25)));
            goal_selector.add_goal(
                7,
                Box::new(FoxMeleeAttackGoal {
                    goal: MeleeAttackGoal::new(f64::from(1.2f32), true).attacking_with(fox_bite),
                }),
            );
            goal_selector.add_goal(7, Box::new(SleepGoal::new(self)));
            goal_selector.add_goal(
                8,
                Box::new(FoxFollowParentGoal {
                    goal: FollowParentGoal::new(1.25),
                }),
            );
            goal_selector.add_goal(
                9,
                Box::new(FoxStrollThroughVillageGoal {
                    goal: StrollThroughVillageGoal::new(200),
                }),
            );
            goal_selector.add_goal(10, FoxEatBerriesGoal::new(f64::from(1.2f32), 12, 1));
            goal_selector.add_goal(10, Box::new(LeapAtTargetGoal::new(0.4)));
            goal_selector.add_goal(11, Box::new(WanderAroundGoal::avoiding_water(1.0)));
            goal_selector.add_goal(11, Box::new(FoxSearchForItemsGoal));
            goal_selector.add_goal(
                12,
                Box::new(FoxLookAtPlayerGoal {
                    goal: LookAtEntityGoal::new(mob_weak, &EntityType::PLAYER, 24.0, 0.02, false),
                }),
            );
            goal_selector.add_goal(13, Box::new(PerchAndSearchGoal::default()));
        };

        mob_entity.add_target_goal(3, DefendTrustedTargetGoal::new(mob_entity, fox_weak));
    }

    /// Vanilla `Fox.setTargetGoals`.
    fn set_target_goals(&self) {
        if self.get_variant() == FoxVariant::Red {
            self.add_target_goal(4, FoxTargetGoal::Land);
            self.add_target_goal(4, FoxTargetGoal::TurtleEgg);
            self.add_target_goal(6, FoxTargetGoal::Fish);
        } else {
            self.add_target_goal(4, FoxTargetGoal::Fish);
            self.add_target_goal(6, FoxTargetGoal::Land);
            self.add_target_goal(6, FoxTargetGoal::TurtleEgg);
        }
    }

    fn add_target_goal(&self, priority: u8, goal: FoxTargetGoal) {
        let bit = goal as u8;
        if self.target_goals_added.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
            self.mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .add_goal(priority, Box::new(goal.create(&self.mob_entity)));
        }
    }

    #[must_use]
    pub fn get_variant(&self) -> FoxVariant {
        FoxVariant::from_id(self.variant.load(Ordering::Relaxed))
    }

    pub fn set_variant(&self, variant: FoxVariant) {
        self.variant.store(variant.id(), Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::fox::TYPE_ID,
            VarInt(variant.id()),
        );
    }

    /// Vanilla `Fox.getTrustedEntities`.
    fn trusted_entities(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.trusted.iter().filter_map(AtomicCell::load)
    }

    /// Vanilla `Fox.addTrustedEntity`: fills the first slot, then keeps overwriting the second.
    fn add_trusted_entity(&self, uuid: Uuid) {
        if self.trusted[0].load().is_some() {
            self.set_trusted(1, Some(uuid));
        } else {
            self.set_trusted(0, Some(uuid));
        }
    }

    fn set_trusted(&self, index: usize, uuid: Option<Uuid>) {
        self.trusted[index].store(uuid);
        let tracked = if index == 0 {
            pumpkin_data::tracked_data::fox::TRUSTED_ID_0
        } else {
            pumpkin_data::tracked_data::fox::TRUSTED_ID_1
        };
        self.get_entity().set_synced_data(tracked, uuid);
    }

    /// Vanilla `Fox.clearTrusted`.
    fn clear_trusted(&self) {
        self.set_trusted(0, None);
        self.set_trusted(1, None);
    }

    /// Vanilla `Fox.trusts`.
    #[must_use]
    pub fn trusts(&self, uuid: &Uuid) -> bool {
        self.trusted_entities().any(|trusted| trusted == *uuid)
    }

    fn get_flag(&self, flag: u8) -> bool {
        (self.flags.load(Ordering::Relaxed) & flag) != 0
    }

    fn set_flag(&self, flag: u8, value: bool) {
        let current = self.flags.load(Ordering::Relaxed);
        let new_flags = if value {
            current | flag
        } else {
            current & !flag
        };
        // `SynchedEntityData.set` only sends a changed value.
        if new_flags != current {
            self.flags.store(new_flags, Ordering::Relaxed);
            self.get_entity()
                .set_synced_data(pumpkin_data::tracked_data::fox::FLAGS_ID, new_flags);
        }
    }

    #[must_use]
    pub fn is_sitting(&self) -> bool {
        self.get_flag(FLAG_SITTING)
    }

    pub fn set_sitting(&self, val: bool) {
        self.set_flag(FLAG_SITTING, val);
    }

    #[must_use]
    pub fn is_crouching(&self) -> bool {
        self.get_flag(FLAG_CROUCHING)
    }

    pub fn set_crouching(&self, val: bool) {
        self.set_flag(FLAG_CROUCHING, val);
    }

    #[must_use]
    pub fn is_interested(&self) -> bool {
        self.get_flag(FLAG_INTERESTED)
    }

    pub fn set_interested(&self, val: bool) {
        self.set_flag(FLAG_INTERESTED, val);
    }

    #[must_use]
    pub fn is_pouncing(&self) -> bool {
        self.get_flag(FLAG_POUNCING)
    }

    pub fn set_pouncing(&self, val: bool) {
        self.set_flag(FLAG_POUNCING, val);
    }

    #[must_use]
    pub fn is_sleeping(&self) -> bool {
        self.get_flag(FLAG_SLEEPING)
    }

    pub fn set_sleeping(&self, val: bool) {
        self.set_flag(FLAG_SLEEPING, val);
    }

    #[must_use]
    pub fn is_faceplanted(&self) -> bool {
        self.get_flag(FLAG_FACEPLANTED)
    }

    pub fn set_faceplanted(&self, val: bool) {
        self.set_flag(FLAG_FACEPLANTED, val);
    }

    #[must_use]
    pub fn is_defending(&self) -> bool {
        self.get_flag(FLAG_DEFENDING)
    }

    pub fn set_defending(&self, val: bool) {
        self.set_flag(FLAG_DEFENDING, val);
    }

    /// Vanilla `Fox.wakeUp`.
    fn wake_up(&self) {
        self.set_sleeping(false);
    }

    /// Vanilla `Fox.clearStates`.
    pub fn clear_states(&self) {
        self.set_interested(false);
        self.set_crouching(false);
        self.set_sitting(false);
        self.set_sleeping(false);
        self.set_defending(false);
        self.set_faceplanted(false);
    }

    /// Vanilla `Fox.canMove`.
    #[must_use]
    pub fn can_move(&self) -> bool {
        !self.is_sleeping() && !self.is_sitting() && !self.is_faceplanted()
    }

    /// Vanilla `Fox.isFullyCrouched`.
    #[must_use]
    pub fn is_fully_crouched(&self) -> bool {
        self.crouch_amount.load() == MAX_CROUCH_AMOUNT
    }

    fn main_hand(&self) -> ItemStack {
        self.mob_entity
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::MAIN_HAND)
    }

    fn target_is_alive(&self) -> bool {
        self.mob_entity.get_target().is_some_and(|target| {
            target
                .get_living_entity()
                .is_some_and(LivingEntity::is_alive)
        })
    }

    /// Vanilla `Fox.canEat`.
    fn can_eat(&self, item_in_mouth: &ItemStack) -> bool {
        is_consumable_food(item_in_mouth)
            && self.mob_entity.get_target().is_none()
            && self.get_entity().on_ground.load(Ordering::Relaxed)
            && !self.is_sleeping()
    }

    /// Vanilla `Fox.canHoldItem`.
    fn fox_can_hold_item(&self, stack: &ItemStack) -> bool {
        let held = self.main_hand();
        held.is_empty()
            || self.ticks_since_eaten.load(Ordering::Relaxed) > 0
                && is_consumable_food(stack)
                && !is_consumable_food(&held)
    }

    /// Vanilla `Fox.spitOutItem`.
    fn spit_out_item(&self, stack: ItemStack) {
        if stack.is_empty() {
            return;
        }
        let entity = self.get_entity();
        let pos = entity.pos.load();
        let look = entity.get_look_angle();
        let world = entity.world.load_full();
        let item = ItemEntity::new(
            Entity::new(
                world.clone(),
                Vector3::new(pos.x + look.x, pos.y + 1.0, pos.z + look.z),
                &EntityType::ITEM,
            ),
            stack,
        );
        item.set_pickup_delay(SPIT_PICKUP_DELAY);
        // TODO: vanilla also records the fox as the item's thrower, which Pumpkin's item
        // entities do not track yet.
        self.play_sound_at_self(Sound::EntityFoxSpit, 1.0, 1.0);
        world.spawn_entity(Arc::new(item));
    }

    /// Vanilla `Fox.dropItemStack`: a plain item entity at the fox, free to pick up.
    fn drop_item_stack(&self, stack: ItemStack) {
        let entity = self.get_entity();
        let world = entity.world.load_full();
        let item = ItemEntity::new(
            Entity::new(world.clone(), entity.pos.load(), &EntityType::ITEM),
            stack,
        );
        item.set_pickup_delay(0);
        world.spawn_entity(Arc::new(item));
    }

    /// Vanilla `Fox.pickUpItem`.
    fn fox_pick_up_item(&self, item_entity: &ItemEntity) {
        let mut item_stack = item_entity
            .get_item_stack()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if !self.fox_can_hold_item(&item_stack) {
            return;
        }
        let count = item_stack.item_count;
        if count > 1 {
            self.drop_item_stack(item_stack.split(count - 1));
        }
        self.spit_out_item(self.main_hand());
        let held = item_stack.split(1);
        self.set_item_slot_and_notify(&EquipmentSlot::MAIN_HAND, held);
        self.mob_entity
            .set_guaranteed_drop(&EquipmentSlot::MAIN_HAND);
        // Vanilla takes the count left after the split: none.
        self.mob_entity
            .living_entity
            .pickup(item_entity.get_entity(), u32::from(item_stack.item_count));
        item_entity.get_entity().remove();
        self.ticks_since_eaten.store(0, Ordering::Relaxed);
    }

    /// Vanilla `Fox.isPathClear`: the three blocks above each sixth of the way to `target`
    /// can be replaced.
    fn is_path_clear(&self, target: &Entity) -> bool {
        let entity = self.get_entity();
        let pos = entity.pos.load();
        let target_pos = target.pos.load();
        let zdiff = target_pos.z - pos.z;
        let xdiff = target_pos.x - pos.x;
        let slope = zdiff / xdiff;
        let world = entity.world.load();

        for i in 0..6 {
            let fraction = f64::from(i as f32 / 6.0);
            let z = if slope == 0.0 { 0.0 } else { zdiff * fraction };
            let x = if slope == 0.0 {
                xdiff * fraction
            } else {
                z / slope
            };

            for j in 1..4 {
                let check = BlockPos::floored(pos.x + x, pos.y + f64::from(j), pos.z + z);
                if !world.get_block_state(&check).replaceable() {
                    return false;
                }
            }
        }

        true
    }

    /// Vanilla `Fox.FoxBehaviorGoal.hasShelter`.
    fn has_shelter(&self) -> bool {
        let entity = self.get_entity();
        let pos = entity.pos.load();
        let fox_pos = BlockPos::floored(pos.x, entity.bounding_box.load().max.y, pos.z);
        !entity.world.load().can_see_sky(&fox_pos) && self.get_walk_target_value(&fox_pos) >= 0.0
    }

    /// Vanilla `Fox.FoxBehaviorGoal.alertable`: something within 12/6/12 that a fox keeps an
    /// eye on.
    fn alertable(&self) -> bool {
        let entity = self.get_entity();
        let world = entity.world.load();
        let area = entity.bounding_box.load().expand(12.0, 6.0, 12.0);
        let alertable_targeting = TargetPredicate::create_attackable()
            .set_base_max_distance(12.0)
            .ignore_visibility();
        world.get_all_at_box(&area).iter().any(|candidate| {
            candidate.get_living_entity().is_some()
                && self.is_alertable_entity(candidate.as_ref())
                && alertable_targeting.test(&world, Some(self), candidate.as_ref())
        })
    }

    /// Vanilla `Fox.FoxAlertableEntitiesSelector`.
    fn is_alertable_entity(&self, target: &dyn EntityBase) -> bool {
        let target_entity = target.get_entity();
        let entity_type = target_entity.entity_type;
        if entity_type == &EntityType::FOX {
            return false;
        }
        if is_stalkable_prey(entity_type) || is_monster(entity_type) {
            return true;
        }
        if let Some(tamable) = target.get_mob().and_then(Mob::as_tamable) {
            return !tamable.is_tame();
        }
        if let Some(player) = target.get_player()
            && (player.is_spectator() || player.is_creative())
        {
            return false;
        }
        if self.trusts(&target_entity.entity_uuid) {
            return false;
        }
        !is_sleeping(target) && !target_entity.is_sneaking()
    }
}

/// Vanilla `Fox.isConsumableFood`.
fn is_consumable_food(stack: &ItemStack) -> bool {
    stack.get_data_component::<FoodImpl>().is_some()
        && stack.get_data_component::<ConsumableImpl>().is_some()
}

/// Vanilla `LivingEntity.isSleeping` for any entity.
fn is_sleeping(entity: &dyn EntityBase) -> bool {
    entity.get_player().map_or_else(
        || entity.get_mob().is_some_and(Mob::is_sleeping),
        Player::is_sleeping,
    )
}

/// Vanilla `FoxMeleeAttackGoal.checkAndPerformAttack`: a bite, without a swing.
fn fox_bite(mob: &dyn Mob, target: &dyn EntityBase) {
    let caller: &dyn EntityBase = mob;
    if mob.get_mob_entity().try_attack(caller, target) {
        mob.on_attack(target);
    }
    caller.play_sound_at_self(Sound::EntityFoxBite, 1.0, 1.0);
}

/// Vanilla `Fox.FoxBreedGoal.breed`'s part for the offspring: a parent's variant, and trust
/// for the players who bred it.
fn fox_offspring(mob: &dyn Mob, partner: &dyn EntityBase, offspring: &dyn EntityBase) {
    let (Some(fox), Some(partner_fox), Some(baby)) = (
        as_fox(mob),
        partner.get_mob().and_then(as_fox),
        offspring.get_mob().and_then(as_fox),
    ) else {
        return;
    };
    // Vanilla `Fox.getBreedOffspring`.
    let use_own = fox.get_entity().random().next_bool();
    let variant = if use_own {
        fox.get_variant()
    } else {
        partner_fox.get_variant()
    };
    baby.set_variant(variant);

    // `getLoveCause`: the breeding player, only while they are online.
    let world = fox.get_entity().world.load();
    let love_cause = |animal: &MobEntity| {
        animal
            .breeder
            .load()
            .filter(|uuid| world.get_player_by_uuid(*uuid).is_some())
    };
    let animal_love_cause = love_cause(&fox.mob_entity);
    let partner_love_cause = love_cause(&partner_fox.mob_entity);
    if let Some(cause) = animal_love_cause {
        baby.add_trusted_entity(cause);
    }
    if let Some(cause) = partner_love_cause
        && animal_love_cause != partner_love_cause
    {
        baby.add_trusted_entity(cause);
    }
}

impl AgeableMob for FoxEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for FoxEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_FOX_FOOD)
    }
}

/// Vanilla `Fox.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::FOX_BABY_DIMENSIONS;

impl Mob for FoxEntity {
    /// Vanilla `Fox.getAmbientSound`: a fox alone at night sometimes screeches.
    fn ambient_sound(&self) -> Option<Sound> {
        if self.is_sleeping() {
            return Some(Sound::EntityFoxSleep);
        }
        let entity = self.get_entity();
        let world = entity.world.load();
        if !world.is_bright_outside() && entity.random().next_f32() < 0.1 {
            let area = entity.bounding_box.load().expand(16.0, 16.0, 16.0);
            if world
                .get_players_at_box(&area)
                .iter()
                .all(|player| player.is_spectator())
            {
                return Some(Sound::EntityFoxScreech);
            }
        }
        Some(Sound::EntityFoxAmbient)
    }

    /// Vanilla `Fox.playAmbientSound`: the screech is played at volume 2, anything else
    /// through `Mob.playAmbientSound`, which asks for the sound again.
    fn play_ambient_sound(&self) {
        if self.ambient_sound() == Some(Sound::EntityFoxScreech) {
            self.play_sound_at_self(Sound::EntityFoxScreech, 2.0, self.voice_pitch());
        } else if let Some(sound) = self.ambient_sound() {
            self.make_sound(sound);
        }
    }

    /// Vanilla `Fox.getDefaultDimensions`.
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

    /// Vanilla `Fox.addAdditionalSaveData`.
    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        let trusted = self
            .trusted_entities()
            .map(|uuid| {
                let (most, least) = uuid.as_u64_pair();
                NbtTag::IntArray(vec![
                    (most >> 32) as i32,
                    most as i32,
                    (least >> 32) as i32,
                    least as i32,
                ])
            })
            .collect();
        nbt.put("Trusted", NbtTag::List(trusted));
        nbt.put_bool("Sleeping", self.is_sleeping());
        nbt.put_string("Type", self.get_variant().as_str().to_string());
        nbt.put_bool("Sitting", self.is_sitting());
        nbt.put_bool("Crouching", self.is_crouching());
    }

    /// Vanilla `Fox.readAdditionalSaveData`.
    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.clear_trusted();
        if let Some(trusted) = nbt.get_list("Trusted") {
            for tag in trusted {
                if let NbtTag::IntArray(ints) = tag
                    && let [a, b, c, d] = ints[..]
                {
                    let most = (u64::from(a as u32) << 32) | u64::from(b as u32);
                    let least = (u64::from(c as u32) << 32) | u64::from(d as u32);
                    self.add_trusted_entity(Uuid::from_u64_pair(most, least));
                }
            }
        }
        self.set_sleeping(nbt.get_bool("Sleeping").unwrap_or(false));
        self.set_variant(
            nbt.get_string("Type")
                .and_then(FoxVariant::from_serialized_name)
                .unwrap_or_default(),
        );
        self.set_sitting(nbt.get_bool("Sitting").unwrap_or(false));
        self.set_crouching(nbt.get_bool("Crouching").unwrap_or(false));
        self.set_target_goals();
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_set_variant_name(&self, name: &str) {
        self.set_variant(FoxVariant::from_name(name));
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();
    }

    /// The start of vanilla `Fox.aiStep`, before `super.aiStep()`.
    fn ai_step(&self, _caller: &dyn EntityBase) {
        let living = &self.mob_entity.living_entity;
        if living.is_alive() && !self.mob_entity.is_no_ai() {
            let ticks_since_eaten = self.ticks_since_eaten.fetch_add(1, Ordering::Relaxed) + 1;
            let item_in_mouth = self.main_hand();
            if self.can_eat(&item_in_mouth) {
                if ticks_since_eaten > MIN_TICKS_BEFORE_EAT {
                    let remaining_food = living.finish_using_item(self, item_in_mouth);
                    // Vanilla eats the stack in the slot in place.
                    self.mob_entity
                        .set_item_slot(&EquipmentSlot::MAIN_HAND, remaining_food);
                    self.ticks_since_eaten.store(0, Ordering::Relaxed);
                } else if ticks_since_eaten > 560 && self.get_entity().random().next_f32() < 0.1 {
                    self.play_sound_at_self(Sound::EntityFoxEat, 1.0, 1.0);
                    let entity = self.get_entity();
                    entity
                        .world
                        .load()
                        .send_entity_status(entity, EntityStatus::FoxEat, None);
                }
            }

            if !self.target_is_alive() {
                self.set_crouching(false);
                self.set_interested(false);
            }
        }

        // `isImmobile` is `isDeadOrDying` for a fox.
        if self.is_sleeping() || living.health.load() <= 0.0 {
            living.jumping.store(false, Ordering::Relaxed);
            let input = living.movement_input.load();
            living.movement_input.store(Vector3::new(0.0, input.y, 0.0));
        }
    }

    /// The rest of vanilla `Fox.aiStep` after `super.aiStep()`, then the end of `Fox.tick`.
    fn post_tick(&self) {
        if self.is_defending() && self.get_entity().random().next_f32() < 0.05 {
            self.play_sound_at_self(Sound::EntityFoxAggro, 1.0, 1.0);
        }

        let entity = self.get_entity();
        if !self.mob_entity.is_no_ai() {
            let world = entity.world.load();
            let in_water = entity.is_in_water();
            if in_water || self.mob_entity.get_target().is_some() || world.is_thundering() {
                self.wake_up();
            }
            if in_water || self.is_sleeping() {
                self.set_sitting(false);
            }
            // `this.level().getRandom().nextFloat() < 0.2F`.
            if self.is_faceplanted()
                && world
                    .random
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .next_f32()
                    < 0.2
            {
                let pos = entity.block_pos.load();
                let state = world.get_block_state_id(&pos);
                world.sync_world_event(
                    WorldEvent::ParticlesAndSoundDestroyBlock,
                    pos,
                    i32::from(state.as_u16()),
                );
            }
        }

        let interested_angle = self.interested_angle.load();
        self.interested_angle_o.store(interested_angle);
        let wanted_angle = if self.is_interested() { 1.0 } else { 0.0 };
        self.interested_angle
            .store(interested_angle + (wanted_angle - interested_angle) * 0.4);

        let crouch_amount = self.crouch_amount.load();
        self.crouch_amount_o.store(crouch_amount);
        if self.is_crouching() {
            self.crouch_amount
                .store((crouch_amount + 0.2).min(MAX_CROUCH_AMOUNT));
        } else {
            self.crouch_amount.store(0.0);
        }
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::fox::BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::fox::TYPE_ID,
            VarInt(self.get_variant().id()),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::fox::FLAGS_ID,
            self.flags.load(Ordering::Relaxed),
        );
        for (index, trusted) in self.trusted.iter().enumerate() {
            if let Some(uuid) = trusted.load() {
                self.set_trusted(index, Some(uuid));
            }
        }
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        self.animal_interact(player, item_stack, Sound::EntityFoxAmbient)
    }

    /// Vanilla `FoxLookControl.tick`: a sleeping fox keeps its head still.
    fn skips_look_control_tick(&self) -> bool {
        self.is_sleeping()
    }

    /// Vanilla `FoxLookControl.resetXRotOnTick`.
    fn look_control_resets_x_rot(&self) -> bool {
        !self.is_pouncing()
            && !self.is_crouching()
            && !self.is_interested()
            && !self.is_faceplanted()
    }

    fn is_sleeping(&self) -> bool {
        Self::is_sleeping(self)
    }

    /// Vanilla `Fox.setTarget`.
    fn set_mob_target(&self, target: Option<Arc<dyn EntityBase>>) {
        if self.is_defending() && target.is_none() {
            self.set_defending(false);
        }
        set_mob_target_base(self, target);
    }

    /// Vanilla `Fox.canHoldItem`.
    fn can_hold_item(&self, stack: &ItemStack) -> bool {
        self.fox_can_hold_item(stack)
    }

    /// Vanilla `Fox.pickUpItem`.
    fn mob_pick_up_item(&self, item: &ItemEntity) {
        self.fox_pick_up_item(item);
    }

    /// Vanilla `Fox.dropAllDeathLoot`: whatever is in the fox's mouth always drops.
    fn before_drop_all_death_loot(&self) {
        let stack = self.main_hand();
        if stack.is_empty() {
            return;
        }
        // Vanilla `spawnAtLocation`: at the fox, with the default pickup delay.
        let entity = self.get_entity();
        let world = entity.world.load_full();
        world.spawn_entity(Arc::new(ItemEntity::new(
            Entity::new(world.clone(), entity.pos.load(), &EntityType::ITEM),
            stack,
        )));
        self.mob_entity
            .set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::EMPTY.clone());
    }

    /// Vanilla `Fox.populateDefaultEquipmentSlots`: sometimes a fox spawns with something in
    /// its mouth.
    fn populate_default_equipment_slots(
        &self,
        world: &Arc<World>,
        _difficulty: &RegionalDifficulty,
    ) {
        // `finalizeSpawn` passes `level.getRandom()`.
        let level_random = || {
            world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        };
        if level_random().next_f32() >= 0.2 {
            return;
        }
        let odds = level_random().next_f32();
        let held_in_mouth = if odds < 0.05 {
            &Item::EMERALD
        } else if odds < 0.2 {
            &Item::EGG
        } else if odds < 0.4 {
            if level_random().next_bool() {
                &Item::RABBIT_FOOT
            } else {
                &Item::RABBIT_HIDE
            }
        } else if odds < 0.6 {
            &Item::WHEAT
        } else if odds < 0.8 {
            &Item::LEATHER
        } else {
            &Item::FEATHER
        };
        self.mob_entity
            .set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, held_in_mouth));
    }

    /// Vanilla `Fox.finalizeSpawn`.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        let entity = self.get_entity();
        let mut variant = FoxVariant::by_biome(world, &entity.block_pos.load());
        let mut is_baby = false;
        let group_size = if let Some(SpawnGroupData::Fox {
            variant: group_variant,
            group_size,
        }) = group_data
        {
            variant = group_variant;
            is_baby = group_size >= 2;
            group_size
        } else {
            0
        };

        self.set_variant(variant);
        if is_baby {
            self.set_age(-24000);
        }
        self.set_target_goals();
        let difficulty = RegionalDifficulty::at(world, entity.pos.load());
        self.populate_default_equipment_slots(world, &difficulty);

        // `FoxGroupData` is an `AgeableMobGroupData(false)`: `AgeableMob.finalizeSpawn` only
        // counts the group.
        self.mob_entity.finalize_spawn_base();
        Some(SpawnGroupData::Fox {
            variant,
            group_size: group_size + 1,
        })
    }
}

/// Vanilla `Fox.FoxMoveControl`: a sleeping, sitting or faceplanted fox stays put.
#[derive(Default)]
struct FoxMoveControl {
    move_control: MoveControl,
}

impl Control for FoxMoveControl {}

impl MoveControlTrait for FoxMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        if as_fox(mob).is_some_and(FoxEntity::can_move) {
            self.move_control.tick(mob);
        }
    }

    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed_modifier: f64) {
        MoveControlTrait::set_wanted_position(&mut self.move_control, x, y, z, speed_modifier);
    }

    fn strafe(&mut self, forward: f32, right: f32) {
        MoveControlTrait::strafe(&mut self.move_control, forward, right);
    }

    fn has_wanted(&self) -> bool {
        MoveControlTrait::has_wanted(&self.move_control)
    }
}

/// Vanilla `Fox.FoxFloatGoal`: floats in shallower water than other mobs, dropping
/// whatever it was doing.
struct FoxFloatGoal {
    float: Box<SwimGoal>,
}

impl FoxFloatGoal {
    fn new(mob: &MobEntity) -> Self {
        Self {
            float: SwimGoal::new(mob),
        }
    }

    fn can_use(mob: &dyn Mob) -> bool {
        let entity = mob.get_entity();
        entity.touching_water.load(Ordering::Relaxed) && entity.water_height.load() > 0.25
            || entity.touching_lava.load(Ordering::Relaxed)
    }
}

impl Goal for FoxFloatGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        Self::can_use(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        Self::can_use(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.float.start(mob);
        if let Some(fox) = as_fox(mob) {
            fox.clear_states();
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.float.tick(mob);
    }

    fn should_run_every_tick(&self) -> bool {
        self.float.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        self.float.controls()
    }
}

/// Vanilla `Fox.FaceplantGoal`: a fox that pounced into snow stays stuck for a moment.
struct FaceplantGoal {
    countdown: i32,
}

impl Goal for FaceplantGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_fox(mob).is_some_and(FoxEntity::is_faceplanted)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_start(mob) && self.countdown > 0
    }

    fn start(&mut self, _mob: &dyn Mob) {
        self.countdown = self.get_tick_count(40);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(fox) = as_fox(mob) {
            fox.set_faceplanted(false);
        }
    }

    fn tick(&mut self, _mob: &dyn Mob) {
        self.countdown -= 1;
    }

    fn controls(&self) -> Controls {
        Controls::LOOK | Controls::JUMP | Controls::MOVE
    }
}

/// Vanilla `Fox.FoxBreedGoal`.
struct FoxBreedGoal {
    breed: Box<BreedGoal>,
}

impl FoxBreedGoal {
    fn new(speed: f64) -> Self {
        Self {
            breed: BreedGoal::new(speed).with_offspring(fox_offspring),
        }
    }
}

impl Goal for FoxBreedGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.breed.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.breed.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(fox) = as_fox(mob) {
            fox.clear_states();
        }
        if let Some(partner) = self.breed.partner().and_then(|partner| partner.get_mob())
            && let Some(partner) = as_fox(partner)
        {
            partner.clear_states();
        }
        self.breed.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.breed.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.breed.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.breed.controls()
    }
}

/// Vanilla `Fox.StalkPreyGoal`: creeps up on a chicken or rabbit until close enough to crouch.
struct StalkPreyGoal;

impl StalkPreyGoal {
    fn look_at_target(fox: &FoxEntity, target: &Arc<dyn EntityBase>) {
        fox.mob_entity
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .look_at_entity_with_range(
                target,
                fox.get_max_head_rotation(),
                fox.get_max_look_pitch_change(),
            );
    }

    fn stop_navigation(fox: &FoxEntity) {
        fox.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
    }
}

impl Goal for StalkPreyGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(fox) = as_fox(mob) else {
            return false;
        };
        if fox.is_sleeping() {
            return false;
        }
        let Some(target) = fox.mob_entity.get_target() else {
            return false;
        };
        target
            .get_living_entity()
            .is_some_and(LivingEntity::is_alive)
            && is_stalkable_prey(target.get_entity().entity_type)
            && fox
                .get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&target.get_entity().pos.load())
                > 36.0
            && !fox.is_crouching()
            && !fox.is_interested()
            && !fox.mob_entity.living_entity.jumping.load(Ordering::Relaxed)
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(fox) = as_fox(mob) {
            fox.set_sitting(false);
            fox.set_faceplanted(false);
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        let Some(fox) = as_fox(mob) else {
            return;
        };
        if let Some(target) = fox.mob_entity.get_target()
            && fox.is_path_clear(target.get_entity())
        {
            fox.set_interested(true);
            fox.set_crouching(true);
            Self::stop_navigation(fox);
            Self::look_at_target(fox, &target);
        } else {
            fox.set_interested(false);
            fox.set_crouching(false);
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(fox) = as_fox(mob) else {
            return;
        };
        let Some(target) = fox.mob_entity.get_target() else {
            return;
        };
        Self::look_at_target(fox, &target);
        let target_pos = target.get_entity().pos.load();
        if fox
            .get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&target_pos)
            <= 36.0
        {
            fox.set_interested(true);
            fox.set_crouching(true);
            Self::stop_navigation(fox);
        } else {
            let living = &fox.mob_entity.living_entity;
            let mut navigator = fox
                .mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Vanilla `moveTo(target, 1.5)` paths with a reach of 1.
            let path = navigator.create_path(living, target_pos, 1);
            if path.is_some() {
                navigator.move_to_path(path, 1.5, living);
            }
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}

/// Vanilla `Fox.FoxPounceGoal`: a fully crouched fox leaps onto its prey.
struct FoxPounceGoal;

impl FoxPounceGoal {
    fn look_at_target(fox: &FoxEntity, target: &Arc<dyn EntityBase>) {
        fox.mob_entity
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .look_at_entity_with_range(target, 60.0, 30.0);
    }
}

impl Goal for FoxPounceGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(fox) = as_fox(mob) else {
            return false;
        };
        if !fox.is_fully_crouched() {
            return false;
        }
        let Some(target) = fox.mob_entity.get_target() else {
            return false;
        };
        if !target
            .get_living_entity()
            .is_some_and(LivingEntity::is_alive)
        {
            return false;
        }
        // Vanilla also refuses a target whose motion direction differs from its facing,
        // which only boats and minecarts have.
        let has_clear_path = fox.is_path_clear(target.get_entity());
        if !has_clear_path {
            let living = &fox.mob_entity.living_entity;
            fox.mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .create_path(living, target.get_entity().pos.load(), 0);
            fox.set_crouching(false);
            fox.set_interested(false);
        }
        has_clear_path
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let Some(fox) = as_fox(mob) else {
            return false;
        };
        if !fox.target_is_alive() {
            return false;
        }
        let entity = fox.get_entity();
        let yd = entity.velocity.load().y;
        (yd * yd >= f64::from(0.05f32)
            || entity.pitch.load().abs() >= 15.0
            || !entity.on_ground.load(Ordering::Relaxed))
            && !fox.is_faceplanted()
    }

    fn can_stop(&self) -> bool {
        false
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(fox) = as_fox(mob) else {
            return;
        };
        let living = &fox.mob_entity.living_entity;
        living.jumping.store(true, Ordering::Relaxed);
        fox.set_pouncing(true);
        fox.set_interested(false);
        if let Some(target) = fox.mob_entity.get_target() {
            Self::look_at_target(fox, &target);
            let entity = fox.get_entity();
            let pos = entity.pos.load();
            let target_pos = target.get_entity().pos.load();
            let delta = target_pos - pos;
            // Vanilla `Vec3.normalize`.
            let dist = (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
            let (ux, uz) = if dist < f64::from(1.0E-5f32) {
                (0.0, 0.0)
            } else {
                (delta.x / dist, delta.z / dist)
            };
            let velocity = entity.velocity.load();
            entity.set_velocity(Vector3::new(
                velocity.x + ux * 0.8,
                velocity.y + 0.9,
                velocity.z + uz * 0.8,
            ));
        }
        fox.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
    }

    fn stop(&mut self, mob: &dyn Mob) {
        let Some(fox) = as_fox(mob) else {
            return;
        };
        fox.set_crouching(false);
        fox.crouch_amount.store(0.0);
        fox.crouch_amount_o.store(0.0);
        fox.set_interested(false);
        fox.set_pouncing(false);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(fox) = as_fox(mob) else {
            return;
        };
        let target = fox.mob_entity.get_target();
        if let Some(target) = &target {
            Self::look_at_target(fox, target);
        }

        let entity = fox.get_entity();
        if !fox.is_faceplanted() {
            let movement = entity.velocity.load();
            let x_rot = entity.pitch.load();
            if movement.y * movement.y < f64::from(0.03f32) && x_rot != 0.0 {
                entity.set_pitch(pumpkin_util::math::rot_lerp(0.2, x_rot, 0.0));
            } else {
                #[expect(
                    clippy::imprecise_flops,
                    reason = "vanilla rounds Math.sqrt(x * x + z * z), not a hypot"
                )]
                let direction = (movement.x * movement.x + movement.z * movement.z).sqrt();
                let jumping = fox.mob_entity.living_entity.jumping.load(Ordering::Relaxed);
                let upwards_bias = if jumping && movement.y > 0.0 {
                    6.5f32
                } else {
                    1.0f32
                };
                let biased_y = movement.y * f64::from(upwards_bias);
                #[expect(
                    clippy::imprecise_flops,
                    reason = "vanilla rounds Math.sqrt(a * a + b * b), not a hypot"
                )]
                let len = (direction * direction + biased_y * biased_y).sqrt();
                if len > f64::from(1.0E-5f32) {
                    let rotation =
                        (-biased_y).signum_java() * (direction / len).acos() * f64::from(180.0f32)
                            / f64::from(std::f32::consts::PI);
                    entity.set_pitch(rotation as f32);
                }
            }
        }

        if let Some(target) = &target
            && distance_to(entity, target.get_entity()) <= 2.0
        {
            let caller: &dyn EntityBase = fox;
            if fox.mob_entity.try_attack(caller, target.as_ref()) {
                fox.on_attack(target.as_ref());
            }
        } else if entity.pitch.load() > 0.0
            && entity.on_ground.load(Ordering::Relaxed)
            && entity.velocity.load().y as f32 != 0.0
            && entity.world.load().get_block(&entity.block_pos.load()) == &Block::SNOW
        {
            entity.set_pitch(60.0);
            fox.set_mob_target(None);
            fox.set_faceplanted(true);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::JUMP
    }
}

/// Java's `Math.signum`, which keeps a zero as is.
trait SignumJava {
    fn signum_java(self) -> Self;
}

impl SignumJava for f64 {
    fn signum_java(self) -> Self {
        if self == 0.0 || self.is_nan() {
            self
        } else {
            self.signum()
        }
    }
}

/// Vanilla `Entity.distanceTo`, in floats.
fn distance_to(entity: &Entity, other: &Entity) -> f32 {
    let pos = entity.pos.load();
    let other_pos = other.pos.load();
    let xd = (pos.x - other_pos.x) as f32;
    let yd = (pos.y - other_pos.y) as f32;
    let zd = (pos.z - other_pos.z) as f32;
    (xd * xd + yd * yd + zd * zd).sqrt()
}

/// Vanilla `Fox.SeekShelterGoal`: heads under cover in daylight or a thunderstorm.
struct SeekShelterGoal {
    flee_sun: FleeSunGoal,
    interval: i32,
}

impl SeekShelterGoal {
    const fn new(speed_modifier: f64) -> Self {
        Self {
            flee_sun: FleeSunGoal::new(speed_modifier),
            interval: to_goal_ticks(100),
        }
    }
}

impl Goal for SeekShelterGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(fox) = as_fox(mob) else {
            return false;
        };
        if fox.is_sleeping() || fox.mob_entity.get_target().is_some() {
            return false;
        }
        let entity = fox.get_entity();
        let world = entity.world.load();
        let pos = entity.block_pos.load();
        if world.is_thundering() && world.can_see_sky(&pos) {
            return self.flee_sun.set_wanted_pos(mob);
        }
        if self.interval > 0 {
            self.interval -= 1;
            return false;
        }
        self.interval = 100;
        world.is_bright_outside()
            && world.can_see_sky(&pos)
            && !world.is_village(&pos)
            && self.flee_sun.set_wanted_pos(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.flee_sun.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(fox) = as_fox(mob) {
            fox.clear_states();
        }
        self.flee_sun.start(mob);
    }

    fn controls(&self) -> Controls {
        self.flee_sun.controls()
    }
}

/// Vanilla `Fox.FoxMeleeAttackGoal`.
struct FoxMeleeAttackGoal {
    goal: MeleeAttackGoal,
}

impl Goal for FoxMeleeAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_fox(mob).is_some_and(|fox| {
            !fox.is_sitting() && !fox.is_sleeping() && !fox.is_crouching() && !fox.is_faceplanted()
        }) && self.goal.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(fox) = as_fox(mob) {
            fox.set_interested(false);
        }
        self.goal.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.goal.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.goal.tick(mob);
    }

    fn should_run_every_tick(&self) -> bool {
        self.goal.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}

/// Vanilla `Fox.SleepGoal`: naps through the day somewhere sheltered and quiet.
struct SleepGoal {
    countdown: i32,
}

impl SleepGoal {
    /// Vanilla `SleepGoal.WAIT_TIME_BEFORE_SLEEP`.
    const WAIT_TIME_BEFORE_SLEEP: i32 = to_goal_ticks(140);

    fn new(fox: &FoxEntity) -> Self {
        Self {
            countdown: fox
                .get_entity()
                .random()
                .next_bounded_i32(Self::WAIT_TIME_BEFORE_SLEEP),
        }
    }

    fn can_sleep(&mut self, fox: &FoxEntity) -> bool {
        if self.countdown > 0 {
            self.countdown -= 1;
            return false;
        }
        fox.get_entity().world.load().is_bright_outside()
            && fox.has_shelter()
            && !fox.alertable()
            && !fox.get_entity().is_in_powder_snow()
    }
}

impl Goal for SleepGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(fox) = as_fox(mob) else {
            return false;
        };
        let input = fox.mob_entity.living_entity.movement_input.load();
        if input.x != 0.0 || input.y != 0.0 || input.z != 0.0 {
            return false;
        }
        self.can_sleep(fox) || fox.is_sleeping()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        as_fox(mob).is_some_and(|fox| self.can_sleep(fox))
    }

    fn stop(&mut self, mob: &dyn Mob) {
        let Some(fox) = as_fox(mob) else {
            return;
        };
        self.countdown = fox
            .get_entity()
            .random()
            .next_bounded_i32(Self::WAIT_TIME_BEFORE_SLEEP);
        fox.clear_states();
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(fox) = as_fox(mob) else {
            return;
        };
        fox.set_sitting(false);
        fox.set_crouching(false);
        fox.set_interested(false);
        fox.mob_entity
            .living_entity
            .jumping
            .store(false, Ordering::Relaxed);
        fox.set_sleeping(true);
        fox.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
        let pos = fox.get_entity().pos.load();
        fox.mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_wanted_position(pos.x, pos.y, pos.z, 0.0);
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK | Controls::JUMP
    }
}

/// Vanilla `Fox.FoxFollowParentGoal`.
struct FoxFollowParentGoal {
    goal: FollowParentGoal,
}

impl Goal for FoxFollowParentGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        not_defending(mob) && self.goal.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        not_defending(mob) && self.goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(fox) = as_fox(mob) {
            fox.clear_states();
        }
        self.goal.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.goal.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.goal.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}

/// Vanilla `Fox.FoxStrollThroughVillageGoal`.
struct FoxStrollThroughVillageGoal {
    goal: StrollThroughVillageGoal,
}

impl FoxStrollThroughVillageGoal {
    /// Vanilla `canFoxMove`.
    fn can_fox_move(mob: &dyn Mob) -> bool {
        as_fox(mob).is_some_and(|fox| {
            !fox.is_sleeping()
                && !fox.is_sitting()
                && !fox.is_defending()
                && fox.mob_entity.get_target().is_none()
        })
    }
}

impl Goal for FoxStrollThroughVillageGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.goal.can_start(mob) && Self::can_fox_move(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.goal.should_continue(mob) && Self::can_fox_move(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(fox) = as_fox(mob) {
            fox.clear_states();
        }
        self.goal.start(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.goal.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}

/// Vanilla `Fox.FoxEatBerriesGoal`: walks to a ripe sweet berry bush or glow berries and
/// picks them.
struct FoxEatBerriesGoal {
    goal: MoveToTargetPosGoal<Self>,
    ticks_waited: i32,
}

impl FoxEatBerriesGoal {
    /// Vanilla `FoxEatBerriesGoal.WAIT_TICKS`.
    const WAIT_TICKS: i32 = 40;

    #[expect(
        clippy::unnecessary_box_returns,
        reason = "the goal's parent handle points into the box"
    )]
    fn new(speed_modifier: f64, search_range: i32, vertical_search_range: i32) -> Box<Self> {
        let mut this = Box::new(Self {
            goal: MoveToTargetPosGoal::new(
                ParentHandle::none(),
                speed_modifier,
                search_range,
                vertical_search_range,
            ),
            ticks_waited: 0,
        });
        // SAFETY: `this` heap allocation address is pinned in Box and outlives `ParentHandle` references.
        this.goal.move_to_target_pos = unsafe { ParentHandle::new(&this) };
        this
    }

    /// Vanilla `onReachedTarget`.
    fn on_reached_target(&self, fox: &FoxEntity) {
        let world = fox.get_entity().world.load_full();
        if !world.level_info.load().game_rules.mob_griefing {
            return;
        }
        let pos = self.goal.target_pos;
        let (block, state) = world.get_block_and_state_id(&pos);
        if block == &Block::SWEET_BERRY_BUSH {
            Self::pick_sweet_berries(fox, &world, pos, state);
        } else if CaveVinesBlock::has_glow_berries(block, state) {
            CaveVinesBlock::use_berries(&world, &pos, Some(fox));
        }
    }

    /// Vanilla `pickSweetBerries`.
    fn pick_sweet_berries(
        fox: &FoxEntity,
        world: &Arc<World>,
        pos: BlockPos,
        state: pumpkin_data::BlockStateId,
    ) {
        let mut props = NetherWartLikeProperties::from_state_id(state);
        let age = props.age;
        // `1 + Fox.this.level().getRandom().nextInt(2) + (age == 3 ? 1 : 0)`.
        let mut count = 1
            + world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .next_bounded_i32(2) as u8
            + u8::from(age == 3);
        if fox.main_hand().is_empty() {
            fox.mob_entity.set_item_slot(
                &EquipmentSlot::MAIN_HAND,
                ItemStack::new(1, &Item::SWEET_BERRIES),
            );
            count -= 1;
        }
        if count > 0 {
            world.drop_stack(&pos, ItemStack::new(count, &Item::SWEET_BERRIES));
        }
        fox.play_sound_at_self(Sound::BlockSweetBerryBushPickBerries, 1.0, 1.0);
        props.age = 1;
        world.set_block_state(
            &pos,
            props.to_state_id(&Block::SWEET_BERRY_BUSH),
            BlockFlags::NOTIFY_LISTENERS,
        );
        world.game_event(
            GameEvent::BlockChange,
            pos.to_centered_f64(),
            crate::world::game_event::GameEventContext::of_entity(fox),
        );
    }
}

impl MoveToTargetPos for FoxEatBerriesGoal {
    /// Vanilla `isValidTarget`.
    fn is_target_pos(&self, world: Arc<World>, block_pos: BlockPos) -> bool {
        let (block, state) = world.get_block_and_state_id(&block_pos);
        block == &Block::SWEET_BERRY_BUSH && NetherWartLikeProperties::from_state_id(state).age >= 2
            || CaveVinesBlock::has_glow_berries(block, state)
    }

    fn get_desired_distance_to_target(&self) -> f64 {
        2.0
    }

    fn should_recalculate_path(&self, trying_time: i32) -> bool {
        trying_time % 100 == 0
    }
}

impl Goal for FoxEatBerriesGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_fox(mob).is_some_and(|fox| !fox.is_sleeping()) && self.goal.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.ticks_waited = 0;
        if let Some(fox) = as_fox(mob) {
            fox.set_sitting(false);
        }
        self.goal.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.goal.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        if let Some(fox) = as_fox(mob) {
            if self.goal.is_reached_target() {
                if self.ticks_waited >= Self::WAIT_TICKS {
                    self.on_reached_target(fox);
                } else {
                    self.ticks_waited += 1;
                }
            } else if fox.get_entity().random().next_f32() < 0.05 {
                fox.play_sound_at_self(Sound::EntityFoxSniff, 1.0, 1.0);
            }
        }
        self.goal.tick(mob);
    }

    fn should_run_every_tick(&self) -> bool {
        self.goal.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}

/// Vanilla `Fox.FoxSearchForItemsGoal`: an empty-mouthed fox walks to a dropped item.
struct FoxSearchForItemsGoal;

impl FoxSearchForItemsGoal {
    /// Vanilla `getEntitiesOfClass(ItemEntity.class, inflate(8), ALLOWED_ITEMS)`.
    fn allowed_items(fox: &FoxEntity) -> Vec<Arc<dyn EntityBase>> {
        let entity = fox.get_entity();
        let area = entity.bounding_box.load().expand(8.0, 8.0, 8.0);
        entity
            .world
            .load()
            .get_entities_at_box(&area)
            .into_iter()
            .filter(|candidate| {
                candidate.get_item_entity().is_some_and(|item| {
                    item.get_pickup_delay() == 0
                        && !item.get_entity().removed.load(Ordering::Relaxed)
                })
            })
            .collect()
    }

    fn move_to(fox: &FoxEntity, item: &Arc<dyn EntityBase>) {
        let living = &fox.mob_entity.living_entity;
        let mut navigator = fox
            .mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Vanilla `moveTo(item, 1.2F)` paths with a reach of 1.
        let path = navigator.create_path(living, item.get_entity().pos.load(), 1);
        if path.is_some() {
            navigator.move_to_path(path, f64::from(1.2f32), living);
        }
    }
}

impl Goal for FoxSearchForItemsGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(fox) = as_fox(mob) else {
            return false;
        };
        if !fox.main_hand().is_empty() {
            return false;
        }
        if fox.mob_entity.get_target().is_some()
            || fox
                .mob_entity
                .living_entity
                .get_last_hurt_by_mob()
                .is_some()
        {
            return false;
        }
        if !fox.can_move() {
            return false;
        }
        if fox
            .get_entity()
            .random()
            .next_bounded_i32(to_goal_ticks(10))
            != 0
        {
            return false;
        }
        !Self::allowed_items(fox).is_empty() && fox.main_hand().is_empty()
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(fox) = as_fox(mob) else {
            return;
        };
        let items = Self::allowed_items(fox);
        if fox.main_hand().is_empty()
            && let Some(item) = items.first()
        {
            Self::move_to(fox, item);
        }
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(fox) = as_fox(mob) else {
            return;
        };
        if let Some(item) = Self::allowed_items(fox).first() {
            Self::move_to(fox, item);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `Fox.FoxLookAtPlayerGoal`.
struct FoxLookAtPlayerGoal {
    goal: LookAtEntityGoal,
}

impl FoxLookAtPlayerGoal {
    fn fox_allows(mob: &dyn Mob) -> bool {
        as_fox(mob).is_some_and(|fox| !fox.is_faceplanted() && !fox.is_interested())
    }
}

impl Goal for FoxLookAtPlayerGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.goal.can_start(mob) && Self::fox_allows(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.goal.should_continue(mob) && Self::fox_allows(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.goal.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.goal.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.goal.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}

/// Vanilla `Fox.PerchAndSearchGoal`: sits down and looks around now and then.
#[derive(Default)]
struct PerchAndSearchGoal {
    rel_x: f64,
    rel_z: f64,
    look_time: i32,
    looks_remaining: i32,
}

impl PerchAndSearchGoal {
    /// Vanilla `resetLook`.
    fn reset_look(&mut self, fox: &FoxEntity) {
        let (rnd, extra) = {
            let mut random = fox.get_entity().random();
            let rnd = std::f64::consts::TAU * random.next_f64();
            (rnd, random.next_bounded_i32(20))
        };
        self.rel_x = rnd.cos();
        self.rel_z = rnd.sin();
        self.look_time = self.get_tick_count(80 + extra);
    }
}

impl Goal for PerchAndSearchGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(fox) = as_fox(mob) else {
            return false;
        };
        fox.mob_entity
            .living_entity
            .get_last_hurt_by_mob()
            .is_none()
            && {
                let roll = fox.get_entity().random().next_f32();
                roll < 0.02
            }
            && !fox.is_sleeping()
            && fox.mob_entity.get_target().is_none()
            && fox.is_navigator_idle()
            && !fox.alertable()
            && !fox.is_pouncing()
            && !fox.is_crouching()
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        self.looks_remaining > 0
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(fox) = as_fox(mob) else {
            return;
        };
        self.reset_look(fox);
        self.looks_remaining = 2 + fox.get_entity().random().next_bounded_i32(3);
        fox.set_sitting(true);
        fox.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(fox) = as_fox(mob) {
            fox.set_sitting(false);
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(fox) = as_fox(mob) else {
            return;
        };
        self.look_time -= 1;
        if self.look_time <= 0 {
            self.looks_remaining -= 1;
            self.reset_look(fox);
        }
        let entity = fox.get_entity();
        let pos = entity.pos.load();
        fox.mob_entity
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .look_at_with_range(
                pos.x + self.rel_x,
                entity.get_eye_y(),
                pos.z + self.rel_z,
                fox.get_max_head_rotation(),
                fox.get_max_look_pitch_change(),
            );
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}

/// Vanilla `Fox.DefendTrustedTargetGoal`: turns on whatever last hurt someone the fox trusts.
struct DefendTrustedTargetGoal {
    goal: Box<ActiveTargetGoal>,
    fox: Weak<FoxEntity>,
    /// Vanilla `targetConditions`, ranged once at construction.
    target_conditions: TargetPredicate,
    trusted_last_hurt_by: Option<Arc<dyn EntityBase>>,
    trusted_last_hurt: Option<Arc<dyn EntityBase>>,
    timestamp: i32,
}

impl DefendTrustedTargetGoal {
    fn new(mob: &MobEntity, fox: Weak<FoxEntity>) -> Self {
        let target_conditions = TargetPredicate::create_attackable().set_base_max_distance(
            mob.living_entity
                .get_attribute_value(&pumpkin_data::attributes::Attributes::FOLLOW_RANGE),
        );
        Self {
            goal: ActiveTargetGoal::predicated(mob, 10, false, |_target, _world| false),
            fox,
            target_conditions,
            trusted_last_hurt_by: None,
            trusted_last_hurt: None,
            timestamp: 0,
        }
    }

    /// Vanilla `TRUSTED_TARGET_SELECTOR && !trusts(target)`: the target has itself hurt a
    /// mob that is still alive.
    fn is_trusted_target(fox: &FoxEntity, target: &dyn EntityBase) -> bool {
        let Some(living) = target.get_living_entity() else {
            return false;
        };
        let last_hurt_mob = living.last_attacking_id.load(Ordering::Relaxed);
        // `lastHurtMobTimestamp < tickCount + 600` always holds: any remembered victim counts.
        let remembers_victim = last_hurt_mob != 0
            && living.last_attack_time.load(Ordering::Relaxed)
                < living.entity.tick_count.load(Ordering::Relaxed) + TRUSTED_TARGET_MEMORY_TICKS
            && living
                .entity
                .world
                .load()
                .get_entity_by_id(last_hurt_mob)
                .is_some_and(|victim| {
                    victim
                        .get_living_entity()
                        .is_some_and(LivingEntity::is_alive)
                });
        remembers_victim && !fox.trusts(&living.entity.entity_uuid)
    }

    /// Vanilla `TargetGoal.canAttack` with this goal's conditions.
    fn can_attack(&self, fox: &FoxEntity, target: &dyn EntityBase) -> bool {
        Self::is_trusted_target(fox, target)
            && self
                .target_conditions
                .test(&fox.get_entity().world.load(), Some(fox), target)
            && fox
                .mob_entity
                .is_in_position_target_range_pos(&target.get_entity().block_pos.load())
    }
}

impl Goal for DefendTrustedTargetGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if mob
            .get_entity()
            .random()
            .next_bounded_i32(to_goal_ticks(10))
            != 0
        {
            return false;
        }
        let Some(fox) = self.fox.upgrade() else {
            return false;
        };
        let world = fox.get_entity().world.load();
        for trusted in fox.trusted_entities() {
            let trusted_entity = world.get_player_by_uuid(trusted).map_or_else(
                || world.get_entity_by_uuid(trusted),
                |player| Some(player as Arc<dyn EntityBase>),
            );
            let Some(trusted_entity) =
                trusted_entity.filter(|entity| entity.get_living_entity().is_some())
            else {
                continue;
            };
            let Some(living) = trusted_entity.get_living_entity() else {
                continue;
            };
            let timestamp = living.last_attacked_time.load(Ordering::Relaxed);
            self.trusted_last_hurt_by = living.get_last_hurt_by_mob();
            self.trusted_last_hurt = Some(trusted_entity);
            return timestamp != self.timestamp
                && self
                    .trusted_last_hurt_by
                    .as_deref()
                    .is_some_and(|target| self.can_attack(&fox, target));
        }
        false
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.goal.set_target(self.trusted_last_hurt_by.clone());
        if let Some(living) = self
            .trusted_last_hurt
            .as_ref()
            .and_then(|trusted| trusted.get_living_entity())
        {
            self.timestamp = living.last_attacked_time.load(Ordering::Relaxed);
        }
        if let Some(fox) = as_fox(mob) {
            fox.play_sound_at_self(Sound::EntityFoxAggro, 1.0, 1.0);
            fox.set_defending(true);
            fox.wake_up();
        }
        self.goal.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.goal.stop(mob);
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}
