use crate::entity::ai::goal::move_to_target_pos::{MoveToTargetPos, MoveToTargetPosGoal};
use crate::entity::ai::goal::{Controls, Goal, ParentHandle};
use crate::entity::ai::pathfinder::node::PathComputationType;
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::pathfinder::{GroundPathNavigation, Navigator};
use crate::entity::attributes::{Modifier, ModifierOperation, send_attribute_updates_for_living};
use crate::entity::mob::{first_passenger_player, is_player_holding, mob_controlling_passenger};
use crate::world::World;
use pumpkin_data::Block;
use pumpkin_data::BlockState;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::entity::EntityPose;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        breed::BreedGoal, escape_danger::EscapeDangerGoal, follow_parent::FollowParentGoal,
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, tempt::TemptGoal,
        wander_around::WanderAroundGoal,
    },
    item_steerable::{ItemBasedSteering, ItemSteerable},
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

const TEMPT_ITEMS: &[&Item] = &[&Item::WARPED_FUNGUS, &Item::WARPED_FUNGUS_ON_A_STICK];

/// Vanilla `Strider.SUFFOCATING_MODIFIER_ID`.
const SUFFOCATING_MODIFIER_ID: &str = "minecraft:suffocating";

pub struct StriderEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub steering: ItemBasedSteering,
    pub saddled: AtomicBool,
    pub suffocating: AtomicBool,
}

impl StriderEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `createNavigation`, before the constructor's path costs.
        *mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Navigator::new(GroundPathNavigation::strider());
        set_animal_pathfinding_malus(&mob_entity);
        // Vanilla `Strider`'s constructor.
        mob_entity.set_pathfinding_malus(PathType::Water, -1.0);
        mob_entity.set_pathfinding_malus(PathType::Lava, 0.0);
        mob_entity.set_pathfinding_malus(PathType::DangerFire, 0.0);
        mob_entity.set_pathfinding_malus(PathType::DamageFire, 0.0);
        let strider = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            steering: ItemBasedSteering::default(),
            saddled: AtomicBool::new(false),
            suffocating: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(strider);
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

            goal_selector.add_goal(1, EscapeDangerGoal::new(1.65));
            goal_selector.add_goal(2, BreedGoal::new(1.0));
            goal_selector.add_goal(3, Box::new(TemptGoal::new(1.4, TEMPT_ITEMS, false)));
            goal_selector.add_goal(4, StriderGoToLavaGoal::new(1.0));
            goal_selector.add_goal(5, Box::new(FollowParentGoal::new(1.0)));
            goal_selector.add_goal(7, Box::new(WanderAroundGoal::with_interval(1.0, 60)));
            goal_selector.add_goal(
                8,
                LookAtEntityGoal::with_default(mob_weak.clone(), &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));
            goal_selector.add_goal(
                9,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::STRIDER, 8.0),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn is_suffocating(&self) -> bool {
        self.suffocating.load(Ordering::Relaxed)
    }

    /// Vanilla `setSuffocating`: a cold strider is slowed by `SUFFOCATING_MODIFIER`.
    pub fn set_suffocating(&self, suffocating: bool) {
        let was_suffocating = self.suffocating.swap(suffocating, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::strider::DATA_SUFFOCATING,
            suffocating,
        );
        if was_suffocating == suffocating {
            return;
        }
        let living = &self.mob_entity.living_entity;
        living.update_attribute(&Attributes::MOVEMENT_SPEED, |speed| {
            if suffocating {
                speed.add_or_replace_modifier(Modifier {
                    id: SUFFOCATING_MODIFIER_ID.to_string(),
                    amount: f64::from(-0.34f32),
                    operation: ModifierOperation::MultiplyBase,
                });
            } else {
                speed.remove_modifier(SUFFOCATING_MODIFIER_ID);
            }
        });
        send_attribute_updates_for_living(living, vec![Attributes::MOVEMENT_SPEED]);
    }

    /// The start of vanilla `Strider.tick`: out of lava and warm blocks, a strider is cold.
    fn update_suffocating(&self) {
        let entity = self.get_entity();
        let world = entity.world.load();
        let warm = |state: &BlockState| {
            Block::from_state_id(state.id).has_tag(&tag::Block::MINECRAFT_STRIDER_WARM_BLOCKS)
        };
        let in_warm_blocks = warm(world.get_block_state(&entity.block_pos.load()))
            || warm(entity.get_block_state_on_legacy())
            || entity.lava_height.load() > 0.0;
        let on_warm_strider = entity.get_vehicle().is_some_and(|vehicle| {
            vehicle
                .get_mob()
                .and_then(|mob| mob.cast_any().downcast_ref::<Self>())
                .is_some_and(|strider| !strider.is_suffocating())
        });
        self.set_suffocating(!in_warm_blocks && !on_warm_strider);
    }
}

impl AgeableMob for StriderEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for StriderEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_STRIDER_FOOD)
            || item_stack.item == &Item::WARPED_FUNGUS
    }
}

/// Vanilla `Strider.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    EntityDimensions::scalable(0.45, 0.85).with_eye_height(0.4375);

impl Mob for StriderEntity {
    /// Vanilla `Strider.getWalkTargetValue`.
    fn get_walk_target_value(&self, pos: &pumpkin_util::math::position::BlockPos) -> f32 {
        let world = self.get_entity().world.load();
        if world.get_fluid(pos).has_tag(&tag::Fluid::MINECRAFT_LAVA) {
            10.0
        } else if self.get_entity().touching_lava.load(Ordering::Relaxed) {
            f32::NEG_INFINITY
        } else {
            0.0
        }
    }

    fn is_sensitive_to_water(&self) -> bool {
        true
    }

    /// Vanilla `Strider.playStepSound`.
    fn play_step_sound(
        &self,
        caller: &dyn crate::entity::EntityBase,
        _pos: &pumpkin_util::math::position::BlockPos,
        _block: &'static pumpkin_data::Block,
        _state: &'static pumpkin_data::BlockState,
    ) {
        let sound = if self.get_entity().touching_lava.load(Ordering::Relaxed) {
            Sound::EntityStriderStepLava
        } else {
            Sound::EntityStriderStep
        };
        caller.play_sound_at_self(sound, 1.0, 1.0);
    }

    /// Vanilla `Strider.getAmbientSound`: quiet while panicking or tempted.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        let goals = self
            .get_mob_entity()
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (!goals.is_running::<EscapeDangerGoal>() && !goals.is_running::<TemptGoal>())
            .then_some(pumpkin_data::sound::Sound::EntityStriderAmbient)
    }

    /// Vanilla `Strider.getDefaultDimensions`.
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
        nbt.put_bool("Saddle", self.is_saddled());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        if let Some(saddle) = nbt.get_bool("Saddle") {
            self.set_saddled(saddle);
        } else if let Some(saddle_byte) = nbt.get_byte("Saddle") {
            self.set_saddled(saddle_byte == 1);
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `Strider.getControllingPassenger`.
    fn get_controlling_passenger(&self) -> Option<Arc<dyn EntityBase>> {
        if self.is_saddled()
            && let Some(player) = first_passenger_player(&self.mob_entity)
            && is_player_holding(player.as_ref(), &Item::WARPED_FUNGUS_ON_A_STICK)
        {
            return Some(player);
        }
        mob_controlling_passenger(&self.mob_entity)
    }

    fn get_item_steerable(&self) -> Option<&dyn ItemSteerable> {
        Some(self)
    }

    fn is_saddled(&self) -> bool {
        self.saddled.load(Ordering::Relaxed)
    }

    fn can_be_saddled(&self) -> bool {
        self.mob_entity.living_entity.entity.is_alive() && !self.is_baby()
    }

    fn set_saddled(&self, saddled: bool) {
        self.saddled.store(saddled, Ordering::Relaxed);
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        if !self.mob_entity.is_no_ai() {
            self.update_suffocating();
        }
        self.ageable_ai_step();
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::strider::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::strider::DATA_SUFFOCATING,
            self.is_suffocating(),
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let item = item_stack.get_item();

        if item == &Item::SADDLE && self.can_be_saddled() && !self.is_saddled() {
            self.set_saddled(true);
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            let entity = self.get_entity();
            let world = entity.world.load();
            let pos = entity.pos.load();
            world.play_sound(Sound::EntityStriderSaddle, SoundCategory::Neutral, &pos);
            return true;
        }

        if self.is_saddled() && !self.is_food(item_stack) {
            let world = player.world();
            let ent = &self.mob_entity.living_entity.entity;
            if let Some(vehicle) = world.get_entity_by_id(ent.entity_id)
                && let Some(passenger) = world.get_player_by_id(player.entity_id())
            {
                ent.add_passenger(vehicle, passenger as Arc<dyn EntityBase>);
                return true;
            }
        }

        self.animal_interact(player, item_stack, Sound::EntityStriderAmbient)
    }
}

impl ItemSteerable for StriderEntity {
    fn boost(&self) -> bool {
        self.steering.boost()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Vanilla `Strider.StriderGoToLavaGoal`: a strider out of lava heads for a lava block
/// with room above it.
struct StriderGoToLavaGoal {
    goal: MoveToTargetPosGoal<Self>,
}

impl StriderGoToLavaGoal {
    #[expect(
        clippy::unnecessary_box_returns,
        reason = "the goal's parent handle points into the box"
    )]
    fn new(speed_modifier: f64) -> Box<Self> {
        let mut this = Box::new(Self {
            goal: MoveToTargetPosGoal::new(ParentHandle::none(), speed_modifier, 8, 2),
        });
        // SAFETY: `this` heap allocation address is pinned in Box and outlives `ParentHandle` references.
        this.goal.move_to_target_pos = unsafe { ParentHandle::new(&this) };
        this
    }

    fn in_lava(mob: &dyn Mob) -> bool {
        mob.get_entity().touching_lava.load(Ordering::Relaxed)
    }
}

impl MoveToTargetPos for StriderGoToLavaGoal {
    fn is_target_pos(&self, world: Arc<World>, block_pos: BlockPos) -> bool {
        let above = block_pos.up();
        let (above_block, above_state) = world.get_block_and_state(&above);
        world.get_block(&block_pos) == &Block::LAVA
            && world.block_registry.is_pathfindable(
                above_block,
                above_state,
                PathComputationType::Land,
            )
    }

    fn get_move_to_target(&self, target_pos: BlockPos) -> BlockPos {
        target_pos
    }

    fn should_recalculate_path(&self, trying_time: i32) -> bool {
        trying_time % 20 == 0
    }
}

impl Goal for StriderGoToLavaGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        !Self::in_lava(mob) && self.goal.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        !Self::in_lava(mob)
            && self.is_target_pos(mob.get_entity().world.load_full(), self.goal.target_pos)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.goal.start(mob);
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
