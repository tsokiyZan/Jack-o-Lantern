use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::sync::{
    Arc, Mutex, PoisonError, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use crate::block::blocks::turtle_egg::TurtleEggBlock;
use crate::entity::ai::control::move_control::Operation;
use crate::entity::ai::control::{Control, MoveControlTrait, rotlerp};
use crate::entity::ai::goal::breed::award_breeding;
use crate::entity::ai::goal::move_to_target_pos::{MoveToTargetPos, MoveToTargetPosGoal};
use crate::entity::ai::goal::{Controls, Goal, ParentHandle, to_goal_ticks};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::pathfinder::{AmphibiousPathNavigation, Navigator, NavigatorGoal};
use crate::entity::ai::util::default_random_pos;
use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::world::World;
use crossbeam::atomic::AtomicCell;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::block_properties::TurtleEggProperties;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockStateId};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::world::BlockFlags;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        breed::BreedGoal, escape_danger::EscapeDangerGoal, look_at_entity::LookAtEntityGoal,
        tempt::TemptGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

pub struct TurtleEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub has_egg: AtomicBool,
    pub laying_egg: AtomicBool,
    lay_egg_counter: AtomicI32,
    home_pos: AtomicCell<BlockPos>,
    travel_pos: Mutex<Option<BlockPos>>,
    /// `travel_pos.is_some()`, shared with the navigation.
    travelling: Arc<AtomicBool>,
    going_home: AtomicBool,
}

impl TurtleEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let travelling = Arc::new(AtomicBool::new(false));
        // Vanilla `createNavigation`, before the path costs.
        mob_entity.set_own_navigation(Navigator::new(
            AmphibiousPathNavigation::turtle(travelling.clone()).steered_by_move_control(),
        ));
        set_animal_pathfinding_malus(&mob_entity);
        // Vanilla `Turtle`'s constructor.
        mob_entity.set_pathfinding_malus(PathType::Water, 0.0);
        mob_entity.set_pathfinding_malus(PathType::DoorIronClosed, -1.0);
        mob_entity.set_pathfinding_malus(PathType::DoorWoodClosed, -1.0);
        mob_entity.set_pathfinding_malus(PathType::DoorOpen, -1.0);
        mob_entity.set_own_move_control(Box::new(TurtleMoveControl::default()));
        let turtle = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            has_egg: AtomicBool::new(false),
            laying_egg: AtomicBool::new(false),
            lay_egg_counter: AtomicI32::new(0),
            home_pos: AtomicCell::new(BlockPos::ZERO),
            travel_pos: Mutex::new(None),
            travelling,
            going_home: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(turtle);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            goal_selector.add_goal(0, EscapeDangerGoal::seeking_water(1.2, 7));
            goal_selector.add_goal(
                1,
                Box::new(TurtleBreedGoal(BreedGoal::with_breed(1.0, turtle_breed))),
            );
            goal_selector.add_goal(1, TurtleLayEggGoal::new(1.0));
            goal_selector.add_goal(
                2,
                Box::new(TemptGoal::with_tag(
                    1.1,
                    &tag::Item::MINECRAFT_TURTLE_FOOD,
                    false,
                )),
            );
            goal_selector.add_goal(3, TurtleGoToWaterGoal::new(1.0));
            goal_selector.add_goal(4, Box::new(TurtleGoHomeGoal::new(1.0)));
            goal_selector.add_goal(7, Box::new(TurtleTravelGoal::new(1.0)));
            goal_selector.add_goal(
                8,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(
                9,
                Box::new(TurtleRandomStrollGoal(WanderAroundGoal::with_interval(
                    1.0, 100,
                ))),
            );
        };

        mob_arc
    }

    fn home_pos(&self) -> BlockPos {
        self.home_pos.load()
    }

    /// Vanilla `homePos.closerToCenterThan(position(), dist)`.
    fn home_closer_than(&self, dist: f64) -> bool {
        closer_to_center_than(self.home_pos(), self.get_entity().pos.load(), dist)
    }

    fn is_going_home(&self) -> bool {
        self.going_home.load(Ordering::Relaxed)
    }

    fn travel_pos(&self) -> Option<BlockPos> {
        *self
            .travel_pos
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn set_travel_pos(&self, pos: Option<BlockPos>) {
        *self
            .travel_pos
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = pos;
        self.travelling.store(pos.is_some(), Ordering::Relaxed);
    }

    #[must_use]
    pub fn has_egg(&self) -> bool {
        self.has_egg.load(Ordering::Relaxed)
    }

    pub fn set_has_egg(&self, has_egg: bool) {
        self.has_egg.store(has_egg, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(pumpkin_data::tracked_data::turtle::HAS_EGG, has_egg);
    }

    #[must_use]
    pub fn is_laying_egg(&self) -> bool {
        self.laying_egg.load(Ordering::Relaxed)
    }

    pub fn set_laying_egg(&self, laying_egg: bool) {
        self.lay_egg_counter
            .store(i32::from(laying_egg), Ordering::Relaxed);
        self.laying_egg.store(laying_egg, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(pumpkin_data::tracked_data::turtle::LAYING_EGG, laying_egg);
    }
}

impl AgeableMob for TurtleEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for TurtleEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_TURTLE_FOOD)
    }
}

/// Vanilla `Turtle.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::TURTLE_BABY_DIMENSIONS;

impl Mob for TurtleEntity {
    /// Vanilla `Turtle.getWalkTargetValue`.
    fn get_walk_target_value(&self, pos: &BlockPos) -> f32 {
        let world = self.get_entity().world.load();
        if (!self.is_going_home() && world.get_fluid(pos).has_tag(&tag::Fluid::MINECRAFT_WATER))
            || TurtleEggBlock::on_sand(world.as_ref(), pos)
        {
            10.0
        } else {
            world.get_light_level_dependent_magic_value(pos) - 0.5
        }
    }

    /// Vanilla `Turtle.getAmbientSoundInterval`.
    fn ambient_sound_interval(&self) -> i32 {
        200
    }

    /// Vanilla `Turtle.isPushedByFluid`.
    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }

    /// Vanilla `Turtle.travelInWater`.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        if !entity.is_in_water() {
            return false;
        }
        entity.update_velocity_from_input(living.movement_input.load(), f64::from(0.1f32));
        living.make_move(caller);
        let mut velocity = entity.velocity.load() * 0.9;
        if self.get_target().is_none() && (!self.is_going_home() || !self.home_closer_than(20.0)) {
            velocity.y -= 0.005;
        }
        entity.velocity.store(velocity);
        true
    }

    /// The end of vanilla `Turtle.aiStep`: digging particles while laying an egg.
    fn post_tick(&self) {
        let entity = self.get_entity();
        let counter = self.lay_egg_counter.load(Ordering::Relaxed);
        if entity.is_alive() && self.is_laying_egg() && counter >= 1 && counter % 5 == 0 {
            let pos = entity.block_pos.load();
            let world = entity.world.load();
            if TurtleEggBlock::on_sand(world.as_ref(), &pos) {
                let below = world.get_block_state_id(&pos.down());
                world.sync_world_event(
                    WorldEvent::ParticlesAndSoundDestroyBlock,
                    pos,
                    i32::from(below.as_u16()),
                );
            }
        }
    }

    /// Vanilla `Turtle.finalizeSpawn`: home is where it spawned.
    fn finalize_spawn(
        &self,
        _world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.home_pos.store(self.get_entity().block_pos.load());
        self.mob_entity.finalize_spawn_base();
        group_data
    }

    /// Vanilla `Turtle.playStepSound`.
    fn play_step_sound(
        &self,
        caller: &dyn crate::entity::EntityBase,
        _pos: &pumpkin_util::math::position::BlockPos,
        _block: &'static pumpkin_data::Block,
        _state: &'static pumpkin_data::BlockState,
    ) {
        let sound = if self.get_entity().is_baby() {
            Sound::EntityTurtleShambleBaby
        } else {
            Sound::EntityTurtleShamble
        };
        caller.play_sound_at_self(sound, 0.15, 1.0);
    }

    /// Vanilla `Turtle.playSwimSound`.
    fn play_swim_sound(&self, caller: &dyn crate::entity::EntityBase, volume: f32) {
        self.get_entity()
            .play_swim_sound_default(caller, volume * 1.5);
    }

    /// Vanilla `Turtle.getAmbientSound`: only adults on land make one.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        let entity = self.get_entity();
        (!entity.is_in_water()
            && entity.on_ground.load(std::sync::atomic::Ordering::Relaxed)
            && !entity.is_baby())
        .then_some(pumpkin_data::sound::Sound::EntityTurtleAmbientLand)
    }

    /// Vanilla `Turtle.getDefaultDimensions`.
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
        let home = self.home_pos();
        nbt.put(
            "home_pos",
            NbtTag::IntArray(vec![home.0.x, home.0.y, home.0.z]),
        );
        nbt.put_bool("has_egg", self.has_egg());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        let home = match nbt.get_int_array("home_pos") {
            Some(&[x, y, z]) => BlockPos::new(x, y, z),
            _ => self.get_entity().block_pos.load(),
        };
        self.home_pos.store(home);
        self.read_ageable_nbt(nbt);
        if let Some(has_egg) = nbt.get_bool("has_egg") {
            self.set_has_egg(has_egg);
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
            entity.set_synced_data(pumpkin_data::tracked_data::turtle::DATA_BABY_ID, true);
        }
        entity.set_synced_data(pumpkin_data::tracked_data::turtle::HAS_EGG, self.has_egg());
        entity.set_synced_data(
            pumpkin_data::tracked_data::turtle::LAYING_EGG,
            self.is_laying_egg(),
        );
    }

    fn mob_interact(
        &self,
        player: &Arc<Player>,
        _hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        self.animal_interact(player, item_stack, Sound::EntityTurtleAmbientLand)
    }
}

/// Vanilla `Vec3i.closerToCenterThan`.
fn closer_to_center_than(block: BlockPos, pos: Vector3<f64>, dist: f64) -> bool {
    Vector3::new(
        f64::from(block.0.x) + 0.5,
        f64::from(block.0.y) + 0.5,
        f64::from(block.0.z) + 0.5,
    )
    .squared_distance_to_vec(&pos)
        < dist * dist
}

fn as_turtle(mob: &dyn Mob) -> Option<&TurtleEntity> {
    mob.cast_any().downcast_ref::<TurtleEntity>()
}

fn navigation_done(mob: &dyn Mob) -> bool {
    mob.get_mob_entity().own_navigation().is_done()
}

fn move_to(mob: &dyn Mob, target: Vector3<f64>, speed: f64) {
    let pos = mob.get_entity().pos.load();
    mob.get_mob_entity()
        .own_navigation()
        .set_progress(NavigatorGoal::new(pos, target, speed));
}

/// Vanilla `Vec3.atBottomCenterOf`.
fn bottom_center(pos: BlockPos) -> Vector3<f64> {
    Vector3::new(
        f64::from(pos.0.x) + 0.5,
        f64::from(pos.0.y),
        f64::from(pos.0.z) + 0.5,
    )
}

/// The `getPosTowards` fallbacks the go-home and travel goals share.
fn pos_towards(mob: &dyn Mob, target: Vector3<f64>) -> Option<Vector3<f64>> {
    default_random_pos::get_pos_towards(
        mob,
        16,
        3,
        target,
        f64::from((std::f64::consts::PI / 10.0) as f32),
    )
    .or_else(|| {
        default_random_pos::get_pos_towards(
            mob,
            8,
            7,
            target,
            f64::from((std::f64::consts::PI / 2.0) as f32),
        )
    })
}

/// Vanilla `Turtle.TurtleBreedGoal.breed`: the turtle carries an egg instead of a baby.
fn turtle_breed(mob: &dyn Mob, mate: &dyn EntityBase) {
    award_breeding(mob, mate);
    if let Some(turtle) = as_turtle(mob) {
        turtle.set_has_egg(true);
    }
    let mob_entity = mob.get_mob_entity();
    mob_entity.breeding_cooldown.store(6000, Ordering::Relaxed);
    mate.set_breeding_cooldown(6000);
    mob_entity.reset_love_ticks();
    mate.reset_love();
    let world = mob.get_entity().world.load_full();
    if world.level_info.load().game_rules.mob_drops {
        ExperienceOrbEntity::spawn(
            &world,
            mob.get_entity().pos.load(),
            // `this.animal.getRandom().nextInt(7) + 1`.
            (mob.get_entity().random().next_bounded_i32(7) + 1) as u32,
        );
    }
}

/// Vanilla `Turtle.TurtleBreedGoal`: no breeding while carrying an egg.
struct TurtleBreedGoal(Box<BreedGoal>);

impl Goal for TurtleBreedGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.0.can_start(mob) && !as_turtle(mob).is_some_and(TurtleEntity::has_egg)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.0.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.0.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.0.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.0.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.0.controls()
    }
}

/// Vanilla `Turtle.TurtleRandomStrollGoal`: only on land, and not while heading home or
/// carrying an egg.
struct TurtleRandomStrollGoal(WanderAroundGoal);

impl Goal for TurtleRandomStrollGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        !mob.get_entity().is_in_water()
            && as_turtle(mob).is_some_and(|turtle| !turtle.is_going_home() && !turtle.has_egg())
            && self.0.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.0.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.0.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.0.stop(mob);
    }

    fn controls(&self) -> Controls {
        self.0.controls()
    }
}

/// Vanilla `Turtle.TurtleGoHomeGoal`: adults carrying an egg, or now and then any adult far
/// from home, swim back towards their home beach.
struct TurtleGoHomeGoal {
    speed_modifier: f64,
    stuck: bool,
    close_to_home_try_ticks: i32,
}

impl TurtleGoHomeGoal {
    const fn new(speed_modifier: f64) -> Self {
        Self {
            speed_modifier,
            stuck: false,
            close_to_home_try_ticks: 0,
        }
    }
}

impl Goal for TurtleGoHomeGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(turtle) = as_turtle(mob) else {
            return false;
        };
        if turtle.get_entity().is_baby() {
            return false;
        }
        if turtle.has_egg() {
            return true;
        }
        let roll = mob
            .get_entity()
            .random()
            .next_bounded_i32(to_goal_ticks(700));
        roll == 0 && !turtle.home_closer_than(64.0)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        as_turtle(mob).is_some_and(|turtle| !turtle.home_closer_than(7.0))
            && !self.stuck
            && self.close_to_home_try_ticks <= self.get_tick_count(600)
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(turtle) = as_turtle(mob) {
            turtle.going_home.store(true, Ordering::Relaxed);
        }
        self.stuck = false;
        self.close_to_home_try_ticks = 0;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(turtle) = as_turtle(mob) {
            turtle.going_home.store(false, Ordering::Relaxed);
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(turtle) = as_turtle(mob) else {
            return;
        };
        let home = turtle.home_pos();
        let close_to_home = turtle.home_closer_than(16.0);
        if close_to_home {
            self.close_to_home_try_ticks += 1;
        }
        if !navigation_done(mob) {
            return;
        }
        let home_vec = bottom_center(home);
        let mut next = pos_towards(mob, home_vec);
        if let Some(pos) = next
            && !close_to_home
            && mob.get_entity().world.load().get_block(&pos.to_block_pos()) != &Block::WATER
        {
            next = default_random_pos::get_pos_towards(
                mob,
                16,
                5,
                home_vec,
                f64::from((std::f64::consts::PI / 2.0) as f32),
            );
        }
        let Some(next) = next else {
            self.stuck = true;
            return;
        };
        move_to(mob, next, self.speed_modifier);
    }
}

/// Vanilla `Turtle.TurtleTravelGoal`: a turtle in water picks a far-off point and swims
/// towards it.
struct TurtleTravelGoal {
    speed_modifier: f64,
    stuck: bool,
}

impl TurtleTravelGoal {
    const fn new(speed_modifier: f64) -> Self {
        Self {
            speed_modifier,
            stuck: false,
        }
    }
}

impl Goal for TurtleTravelGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_turtle(mob).is_some_and(|turtle| {
            !turtle.is_going_home() && !turtle.has_egg() && turtle.get_entity().is_in_water()
        })
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        !navigation_done(mob)
            && !self.stuck
            && as_turtle(mob).is_some_and(|turtle| {
                !turtle.is_going_home() && !turtle.mob_entity.is_in_love() && !turtle.has_egg()
            })
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(turtle) = as_turtle(mob) else {
            return;
        };
        let (xt, mut yt, zt) = {
            let mut random = mob.get_entity().random();
            let xt = random.next_bounded_i32(1025) - 512;
            let yt = random.next_bounded_i32(9) - 4;
            (xt, yt, random.next_bounded_i32(1025) - 512)
        };
        let entity = mob.get_entity();
        let pos = entity.pos.load();
        if f64::from(yt) + pos.y > f64::from(entity.world.load().sea_level - 1) {
            yt = 0;
        }
        turtle.set_travel_pos(Some(BlockPos::new(
            (f64::from(xt) + pos.x).floor() as i32,
            (f64::from(yt) + pos.y).floor() as i32,
            (f64::from(zt) + pos.z).floor() as i32,
        )));
        self.stuck = false;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(turtle) = as_turtle(mob) {
            turtle.set_travel_pos(None);
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(travel_pos) = as_turtle(mob).and_then(TurtleEntity::travel_pos) else {
            self.stuck = true;
            return;
        };
        if !navigation_done(mob) {
            return;
        }
        let next = pos_towards(mob, bottom_center(travel_pos)).filter(|next| {
            // `hasChunksAt(xc - 34, zc - 34, xc + 34, zc + 34)`.
            let world = mob.get_entity().world.load();
            let (xc, zc) = (next.x.floor() as i32, next.z.floor() as i32);
            let y = world.sea_level;
            ((xc - 34) >> 4..=(xc + 34) >> 4).all(|cx| {
                ((zc - 34) >> 4..=(zc + 34) >> 4)
                    .all(|cz| world.is_loaded(&BlockPos::new(cx << 4, y, cz << 4)))
            })
        });
        let Some(next) = next else {
            self.stuck = true;
            return;
        };
        move_to(mob, next, self.speed_modifier);
    }
}

/// Vanilla `Turtle.TurtleGoToWaterGoal`: a turtle on land with no errand heads for water
/// within 24 blocks.
struct TurtleGoToWaterGoal {
    goal: MoveToTargetPosGoal<Self>,
}

impl TurtleGoToWaterGoal {
    #[expect(
        clippy::unnecessary_box_returns,
        reason = "the goal's parent handle points into the box"
    )]
    fn new(speed_modifier: f64) -> Box<Self> {
        let mut this = Box::new(Self {
            goal: MoveToTargetPosGoal::new(ParentHandle::none(), speed_modifier, 24, 1),
        });
        this.goal.lowest_y = -1;
        // SAFETY: `this` heap allocation address is pinned in Box and outlives `ParentHandle` references.
        this.goal.move_to_target_pos = unsafe { ParentHandle::new(&this) };
        this
    }
}

impl MoveToTargetPos for TurtleGoToWaterGoal {
    fn is_target_pos(&self, world: Arc<World>, block_pos: BlockPos) -> bool {
        world.get_block(&block_pos) == &Block::WATER
    }

    fn should_recalculate_path(&self, trying_time: i32) -> bool {
        trying_time % 160 == 0
    }
}

impl Goal for TurtleGoToWaterGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(turtle) = as_turtle(mob) else {
            return false;
        };
        let in_water = mob.get_entity().is_in_water();
        if mob.get_entity().is_baby() && !in_water {
            return self.goal.can_start(mob);
        }
        !turtle.is_going_home() && !in_water && !turtle.has_egg() && self.goal.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        !mob.get_entity().is_in_water()
            && self.goal.trying_time <= 1200
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

/// Vanilla `Turtle.TurtleLayEggGoal`: near home with an egg, a turtle digs into sand and
/// lays one to four eggs.
struct TurtleLayEggGoal {
    goal: MoveToTargetPosGoal<Self>,
}

impl TurtleLayEggGoal {
    #[expect(
        clippy::unnecessary_box_returns,
        reason = "the goal's parent handle points into the box"
    )]
    fn new(speed_modifier: f64) -> Box<Self> {
        let mut this = Box::new(Self {
            goal: MoveToTargetPosGoal::new(ParentHandle::none(), speed_modifier, 16, 1),
        });
        // SAFETY: `this` heap allocation address is pinned in Box and outlives `ParentHandle` references.
        this.goal.move_to_target_pos = unsafe { ParentHandle::new(&this) };
        this
    }

    fn near_home_with_egg(mob: &dyn Mob) -> bool {
        as_turtle(mob).is_some_and(|turtle| turtle.has_egg() && turtle.home_closer_than(9.0))
    }
}

impl MoveToTargetPos for TurtleLayEggGoal {
    fn is_target_pos(&self, world: Arc<World>, block_pos: BlockPos) -> bool {
        world.get_block_state(&block_pos.up()).is_air()
            && TurtleEggBlock::is_sand(world.as_ref(), &block_pos)
    }
}

impl Goal for TurtleLayEggGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        Self::near_home_with_egg(mob) && self.goal.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.goal.should_continue(mob) && Self::near_home_with_egg(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.goal.start(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.goal.tick(mob);
        let Some(turtle) = as_turtle(mob) else {
            return;
        };
        let entity = mob.get_entity();
        if entity.is_in_water() || !self.goal.is_reached_target() {
            return;
        }
        let counter = turtle.lay_egg_counter.load(Ordering::Relaxed);
        if counter < 1 {
            turtle.set_laying_egg(true);
        } else if counter > self.get_tick_count(200) {
            let world = entity.world.load_full();
            let turtle_pos = entity.block_pos.load();
            // `0.9F + level.getRandom().nextFloat() * 0.2F`.
            let pitch = 0.9
                + world
                    .random
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .next_f32()
                    * 0.2;
            world.play_sound_fine(
                Sound::EntityTurtleLayEgg,
                SoundCategory::Blocks,
                &Vector3::new(
                    f64::from(turtle_pos.0.x) + 0.5,
                    f64::from(turtle_pos.0.y) + 0.5,
                    f64::from(turtle_pos.0.z) + 0.5,
                ),
                0.3,
                pitch,
            );
            let egg_pos = self.goal.target_pos.up();
            let mut egg = TurtleEggProperties::default(&Block::TURTLE_EGG);
            // `this.turtle.random.nextInt(4) + 1`.
            egg.eggs = (mob.get_entity().random().next_bounded_i32(4) + 1) as u8;
            let egg_state: BlockStateId = egg.to_state_id(&Block::TURTLE_EGG);
            world.set_block_state(&egg_pos, egg_state, BlockFlags::NOTIFY_ALL);
            turtle.set_has_egg(false);
            turtle.set_laying_egg(false);
            turtle.mob_entity.love_ticks.store(600, Ordering::Relaxed);
        }
        if turtle.is_laying_egg() {
            turtle.lay_egg_counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn should_run_every_tick(&self) -> bool {
        self.goal.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}

/// Vanilla `Turtle.TurtleMoveControl`: slow on land, slower far from home in water, and
/// swims up and down towards the wanted position.
#[derive(Default)]
struct TurtleMoveControl {
    wanted_x: f64,
    wanted_y: f64,
    wanted_z: f64,
    speed_modifier: f64,
    operation: Operation,
}

impl Control for TurtleMoveControl {}

impl TurtleMoveControl {
    fn update_speed(mob: &dyn Mob, turtle: &TurtleEntity) {
        let mob_entity = mob.get_mob_entity();
        let living = &mob_entity.living_entity;
        let entity = &living.entity;
        if entity.is_in_water() {
            let velocity = entity.velocity.load();
            entity
                .velocity
                .store(Vector3::new(velocity.x, velocity.y + 0.005, velocity.z));
            if !turtle.home_closer_than(16.0) {
                mob_entity.set_speed((living.speed.load() / 2.0).max(0.08));
            }
            if entity.is_baby() {
                mob_entity.set_speed((living.speed.load() / 3.0).max(0.06));
            }
        } else if entity.on_ground.load(Ordering::Relaxed) {
            mob_entity.set_speed((living.speed.load() / 2.0).max(0.06));
        }
    }
}

impl MoveControlTrait for TurtleMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let Some(turtle) = as_turtle(mob) else {
            return;
        };
        Self::update_speed(mob, turtle);
        let mob_entity = mob.get_mob_entity();
        if self.operation != Operation::MoveTo || navigation_done(mob) {
            mob_entity.set_speed(0.0);
            return;
        }
        let living = &mob_entity.living_entity;
        let entity = &living.entity;
        let pos = entity.pos.load();
        let xd = self.wanted_x - pos.x;
        let mut yd = self.wanted_y - pos.y;
        let zd = self.wanted_z - pos.z;
        let dd = (xd * xd + yd * yd + zd * zd).sqrt();
        if dd < f64::from(1.0e-5f32) {
            mob_entity.set_speed(0.0);
            return;
        }
        yd /= dd;
        let y_rot_d = (math::atan2(zd, xd) * f64::from(180.0f32) / f64::from(std::f32::consts::PI))
            as f32
            - 90.0;
        let yaw = rotlerp(entity.yaw.load(), y_rot_d, 90.0);
        entity.yaw.store(yaw);
        entity.body_yaw.store(yaw);
        let target_speed =
            (self.speed_modifier * living.get_attribute_value(&Attributes::MOVEMENT_SPEED)) as f32;
        let speed = living.speed.load();
        mob_entity.set_speed(speed + 0.125 * (target_speed - speed));
        let velocity = entity.velocity.load();
        entity.velocity.store(Vector3::new(
            velocity.x,
            velocity.y + f64::from(living.speed.load()) * yd * 0.1,
            velocity.z,
        ));
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
}
