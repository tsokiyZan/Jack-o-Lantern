use std::sync::{
    Arc, PoisonError, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::entity_status::EntityStatus;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data::dolphin::{GOT_FISH, MOISTNESS_LEVEL};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::VarInt;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crate::entity::ageable::{AgeableData, AgeableMob};
use crate::entity::mob::spawn::SpawnGroupData;
use crate::world::World;
use crate::world::structure_locator::structures_in_tag;
use pumpkin_util::math::boundingbox::EntityDimensions;

use crate::entity::{
    Entity, EntityBase,
    ai::{
        control::{
            look_control::LookControl, smooth_swimming_move_control::SmoothSwimmingMoveControl,
        },
        goal::{
            Controls, Goal, avoid_entity::AvoidEntityGoal, breath_air::BreathAirGoal,
            dolphin_jump::DolphinJumpGoal,
            follow_player_ridden_entity::FollowPlayerRiddenEntityGoal,
            look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal,
            melee_attack::MeleeAttackGoal, revenge::RevengeGoal, to_goal_ticks,
            try_find_water::TryFindWaterGoal, wander_around::WanderAroundGoal,
        },
        pathfinder::{
            Navigator, NavigatorGoal, WaterBoundPathNavigation,
            node::{PathComputationType, PathType},
        },
        util::{default_random_pos, goal_utils},
    },
    item::ItemEntity,
    mob::{Mob, MobEntity},
    player::Player,
    vehicle::boat::BoatEntity,
};

/// Vanilla `Dolphin.TOTAL_AIR_SUPPLY`.
pub const TOTAL_AIR_SUPPLY: i32 = 4800;
/// Vanilla `Dolphin.TOTAL_MOISTNESS_LEVEL`.
const TOTAL_MOISTNESS_LEVEL: i32 = 2400;
/// The `inWaterSpeedModifier` vanilla gives the dolphin's `SmoothSwimmingMoveControl`.
const IN_WATER_SPEED_MODIFIER: f32 = 0.02;
/// Vanilla `SWIM_WITH_PLAYER_TARGETING` range.
const SWIM_WITH_PLAYER_RANGE: f64 = 10.0;
/// The baby chance of the `AgeableMobGroupData` vanilla's `Dolphin.finalizeSpawn` makes.
const BABY_SPAWN_CHANCE: f32 = 0.1;
/// `DolphinSwimToTreasureGoal` needs at least this much air to set off or keep going.
const TREASURE_MIN_AIR: i32 = 100;
/// The `findNearestMapStructure` radius `DolphinSwimToTreasureGoal` searches.
const TREASURE_SEARCH_RADIUS: i32 = 50;
/// The pickup delay vanilla's `Dolphin.ItemGoal.drop` gives a tossed item.
const THROWN_ITEM_PICKUP_DELAY: u8 = 40;

pub struct DolphinEntity {
    pub mob_entity: MobEntity,
    ageable_data: AgeableData,
    pub got_fish: AtomicBool,
    pub moistness_level: AtomicI32,
}

impl DolphinEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `WaterBoundPathNavigation` allows breaching for dolphins only.
        let mut navigator =
            Navigator::new(WaterBoundPathNavigation::new(true).steered_by_move_control());
        // Vanilla `AgeableWaterCreature`'s constructor.
        navigator.set_pathfinding_malus(PathType::Water, 0.0);
        *mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = navigator;
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(SmoothSwimmingMoveControl::new(
            85,
            10,
            IN_WATER_SPEED_MODIFIER,
            0.1,
            true,
        ));
        *mob_entity
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = LookControl::smooth_swimming(10.0);
        mob_entity.set_can_pick_up_loot(true);
        let dolphin = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            got_fish: AtomicBool::new(false),
            moistness_level: AtomicI32::new(TOTAL_MOISTNESS_LEVEL),
        };
        let mob_arc = Arc::new(dolphin);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        // Vanilla `Dolphin.registerGoals`.
        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            goal_selector.add_goal(0, Box::new(BreathAirGoal::new()));
            goal_selector.add_goal(0, Box::new(TryFindWaterGoal::new()));
            goal_selector.add_goal(1, Box::new(DolphinSwimToTreasureGoal::new()));
            goal_selector.add_goal(2, Box::new(DolphinSwimWithPlayerGoal::new(4.0)));
            goal_selector.add_goal(4, Box::new(WanderAroundGoal::swimming(1.0, 10)));
            goal_selector.add_goal(4, Box::new(RandomLookAroundGoal::default()));
            goal_selector.add_goal(
                5,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(5, Box::new(DolphinJumpGoal::new(10)));
            goal_selector.add_goal(6, Box::new(MeleeAttackGoal::new(1.2, true)));
            goal_selector.add_goal(7, Box::new(MoveToItemGoal::new()));
            goal_selector.add_goal(8, Box::new(PlayWithItemsGoal));
            goal_selector.add_goal(
                8,
                Box::new(FollowPlayerRiddenEntityGoal::new(boat_controlling_player)),
            );
            goal_selector.add_goal(
                8,
                Box::new(FollowPlayerRiddenEntityGoal::new(
                    nautilus_controlling_player,
                )),
            );
            goal_selector.add_goal(
                9,
                Box::new(AvoidEntityGoal::new(&EntityType::GUARDIAN, 8.0, 1.0, 1.0)),
            );
        };

        {
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            target_selector.add_goal(
                1,
                Box::new(
                    RevengeGoal::new(true)
                        .ignoring(|entity_type| {
                            goal_utils::is_instance_of(&EntityType::GUARDIAN, entity_type)
                        })
                        .alerting_others(),
                ),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn got_fish(&self) -> bool {
        self.got_fish.load(Ordering::Relaxed)
    }

    pub fn set_got_fish(&self, val: bool) {
        self.got_fish.store(val, Ordering::Relaxed);
        self.get_entity().set_synced_data(GOT_FISH, val);
    }

    /// Vanilla `Dolphin.pickUpItem`: an empty mouth takes the whole stack.
    fn pick_up_item(&self, item: &ItemEntity) {
        if !self.main_hand().is_empty() {
            return;
        }
        let stack = item
            .get_item_stack()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        // `take` tells the clients before the item goes.
        if !self
            .mob_entity
            .living_entity
            .pickup(item.get_entity(), u32::from(stack.item_count))
        {
            return;
        }
        self.mob_entity
            .set_item_slot_and_drop_when_killed(&EquipmentSlot::MAIN_HAND, stack);
        item.get_entity().remove();
    }

    fn main_hand(&self) -> ItemStack {
        self.mob_entity
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&EquipmentSlot::MAIN_HAND)
    }

    fn set_moistness_level(&self, level: i32) {
        if self.moistness_level.swap(level, Ordering::Relaxed) != level {
            self.get_entity()
                .set_synced_data(MOISTNESS_LEVEL, VarInt(level));
        }
    }
}

impl AgeableMob for DolphinEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

/// Vanilla `Dolphin.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = Entity::type_dimensions(&EntityType::DOLPHIN)
    .scale(0.65)
    .with_eye_height(0.093_75);

impl Mob for DolphinEntity {
    /// Vanilla `Dolphin.getDefaultDimensions`.
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

    /// Vanilla `Dolphin.finalizeSpawn`, then `AgeableMob`'s and `Mob`'s.
    fn finalize_spawn(
        &self,
        _world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.mob_entity
            .living_entity
            .breath
            .set_air_supply(self, TOTAL_AIR_SUPPLY);
        self.get_entity().set_pitch(0.0);
        let group_data = group_data.or(Some(SpawnGroupData::ageable(BABY_SPAWN_CHANCE)));
        let group_data = self.finalize_ageable_spawn(group_data);
        self.mob_entity.finalize_spawn_base();
        Some(group_data)
    }

    /// Vanilla `AgeableWaterCreature.getBaseExperienceReward`.
    fn get_base_experience_reward(&self) -> u32 {
        1 + self.get_random().random_range(0..3)
    }

    /// Vanilla `Dolphin.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        Some(if self.get_entity().is_in_water() {
            Sound::EntityDolphinAmbientWater
        } else {
            Sound::EntityDolphinAmbient
        })
    }

    /// Vanilla `Dolphin.canAttack`: babies never attack, otherwise `Mob.canAttack`.
    fn can_attack(&self, target: &dyn EntityBase) -> bool {
        !self.is_baby()
            && target.get_entity().entity_type != &EntityType::GHAST
            && self.mob_entity.living_entity.can_attack(target)
    }

    /// Vanilla `Dolphin.getMaxHeadXRot`.
    fn get_max_look_pitch_change(&self) -> f32 {
        1.0
    }

    /// Vanilla `Dolphin.getMaxHeadYRot`.
    fn get_max_head_rotation(&self) -> f32 {
        1.0
    }

    /// Vanilla `Dolphin.getMaxAirSupply`.
    fn max_air_supply(&self) -> i32 {
        TOTAL_AIR_SUPPLY
    }

    /// Vanilla `Dolphin.increaseAirSupply`: one breath refills it.
    fn increase_air_supply(&self, _current: i32) -> i32 {
        TOTAL_AIR_SUPPLY
    }

    /// Vanilla `Dolphin.tick`: drying out on land, and flopping about there.
    fn mob_tick(&self, caller: &dyn EntityBase) {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        // `AgeableMob.aiStep`, which vanilla runs inside `super.tick()`.
        self.ageable_ai_step();
        if self.mob_entity.is_no_ai() {
            living.breath.set_air_supply(caller, TOTAL_AIR_SUPPLY);
            return;
        }
        if entity.is_in_water_or_rain() {
            self.set_moistness_level(TOTAL_MOISTNESS_LEVEL);
            return;
        }
        self.set_moistness_level(self.moistness_level.load(Ordering::Relaxed) - 1);
        if self.moistness_level.load(Ordering::Relaxed) <= 0 {
            living.damage(caller, 1.0, DamageType::DRY_OUT);
        }
        if entity.on_ground.load(Ordering::Relaxed) {
            let mut rng = self.get_random();
            let velocity = entity.velocity.load();
            // Sent at once, as vanilla flags the hop with needsSync.
            entity.set_velocity(Vector3::new(
                velocity.x + f64::from((rng.random::<f32>() * 2.0 - 1.0) * 0.2),
                velocity.y + 0.5,
                velocity.z + f64::from((rng.random::<f32>() * 2.0 - 1.0) * 0.2),
            ));
            entity.yaw.store(rng.random::<f32>() * 360.0);
            entity.on_ground.store(false, Ordering::Relaxed);
        }
    }

    /// Vanilla `Dolphin.travelInWater`; on land it moves like any other mob.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        if !entity.is_in_water() {
            return false;
        }
        // Vanilla `Dolphin.travelInWater`: `moveRelative(getSpeed(), input)`.
        entity.update_velocity_from_input(living.movement_input.load(), living.get_speed(caller));
        living.make_move(caller);

        let mut velocity = entity.velocity.load() * 0.9;
        let has_target = self
            .mob_entity
            .target
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if !has_target {
            velocity.y -= 0.005;
        }
        entity.velocity.store(velocity);
        true
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }

    /// The looting part of vanilla `Mob.aiStep`, which runs after the move.
    fn post_tick(&self) {
        self.mob_entity
            .for_each_item_in_pickup_reach(|item| self.pick_up_item(item));
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
        nbt.put_bool("GotFish", self.got_fish());
        nbt.put_int("Moistness", self.moistness_level.load(Ordering::Relaxed));
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        self.set_got_fish(nbt.get_bool("GotFish").unwrap_or(false));
        self.set_moistness_level(nbt.get_int("Moistness").unwrap_or(TOTAL_MOISTNESS_LEVEL));
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `Dolphin.mobInteract`: any fish grows a baby up a little, or makes an
    /// adult remember it was fed.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if item_stack.is_empty() || !item_stack.get_item().has_tag(&tag::Item::MINECRAFT_FISHES) {
            return false;
        }
        self.get_entity().play_sound(Sound::EntityDolphinEat);
        item_stack.decrement_unless_creative(player.gamemode.load(), 1);
        if self.can_age_up() {
            self.age_up(
                Self::get_speed_up_seconds_when_feeding(-self.get_age()),
                true,
            );
        } else {
            self.set_got_fish(true);
        }
        true
    }
}

/// Vanilla `Dolphin.DolphinSwimWithPlayerGoal`: swims alongside a nearby swimming
/// player and gives them Dolphin's Grace.
struct DolphinSwimWithPlayerGoal {
    speed_modifier: f64,
    player: Option<Arc<Player>>,
}

impl DolphinSwimWithPlayerGoal {
    const fn new(speed_modifier: f64) -> Self {
        Self {
            speed_modifier,
            player: None,
        }
    }

    fn give_dolphins_grace(player: &Player) {
        player
            .living_entity
            .add_effect(pumpkin_data::potion::Effect {
                effect_type: &StatusEffect::DOLPHINS_GRACE,
                duration: 100,
                amplifier: 0,
                ambient: false,
                show_particles: true,
                show_icon: true,
                blend: true,
            });
    }
}

impl Goal for DolphinSwimWithPlayerGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let entity = mob.get_entity();
        self.player = entity.world.load().get_nearest_player(
            entity.pos.load(),
            SWIM_WITH_PLAYER_RANGE,
            |player| !player.is_spectator() && player.get_entity().is_alive(),
        );
        let Some(player) = &self.player else {
            return false;
        };
        let is_target = mob
            .get_mob_entity()
            .target
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|target| target.get_entity().entity_id == player.entity_id());
        player.is_swimming() && !is_target
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.player.as_ref().is_some_and(|player| {
            player.is_swimming()
                && mob
                    .get_entity()
                    .pos
                    .load()
                    .squared_distance_to_vec(&player.get_entity().pos.load())
                    < 256.0
        })
    }

    fn start(&mut self, _mob: &dyn Mob) {
        if let Some(player) = &self.player {
            Self::give_dolphins_grace(player);
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.player = None;
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(player) = self.player.clone() else {
            return;
        };
        let mob_entity = mob.get_mob_entity();
        let player_entity: Arc<dyn EntityBase> = player.clone();
        // Vanilla looks with `getMaxHeadYRot() + 20` and `getMaxHeadXRot()`, both 1 for dolphins.
        mob_entity
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .look_at_entity_with_range(&player_entity, 21.0, 1.0);

        let pos = mob.get_entity().pos.load();
        let player_pos = player.get_entity().pos.load();
        let mut navigator = mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if pos.squared_distance_to_vec(&player_pos) < 6.25 {
            navigator.stop();
        } else {
            navigator.set_progress(NavigatorGoal::new(pos, player_pos, self.speed_modifier));
        }
        drop(navigator);

        if player.is_swimming() && rand::rng().random_range(0..6) == 0 {
            Self::give_dolphins_grace(&player);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}

/// Vanilla `AbstractBoat.getControllingPassenger`, for boats only.
fn boat_controlling_player(vehicle: &dyn EntityBase) -> Option<Arc<Player>> {
    vehicle.cast_any().downcast_ref::<BoatEntity>()?;
    first_passenger_player(vehicle)
}

/// Vanilla `AbstractNautilus.getControllingPassenger`, for nautiluses only.
fn nautilus_controlling_player(vehicle: &dyn EntityBase) -> Option<Arc<Player>> {
    let entity = vehicle.get_entity();
    if entity.entity_type != &EntityType::NAUTILUS
        && entity.entity_type != &EntityType::ZOMBIE_NAUTILUS
    {
        return None;
    }
    if !vehicle.get_mob().is_some_and(Mob::is_saddled) {
        return None;
    }
    first_passenger_player(vehicle)
}

fn first_passenger_player(vehicle: &dyn EntityBase) -> Option<Arc<Player>> {
    let entity = vehicle.get_entity();
    let passenger = entity.get_first_passenger()?;
    entity
        .world
        .load()
        .get_player_by_id(passenger.get_entity().entity_id)
}

/// Vanilla `Dolphin.ALLOWED_ITEMS`: items a dolphin goes after.
fn allowed_items(mob: &dyn Mob) -> Vec<Arc<dyn EntityBase>> {
    let entity = mob.get_entity();
    let area = entity.bounding_box.load().expand(8.0, 8.0, 8.0);
    entity
        .world
        .load()
        .get_entities_at_box(&area)
        .into_iter()
        .filter(|candidate| {
            candidate.get_item_entity().is_some_and(|item| {
                item.get_pickup_delay() == 0
                    && item.get_entity().is_alive()
                    && item.get_entity().is_in_water()
            })
        })
        .collect()
}

/// Vanilla `Dolphin.ItemGoal.dropItem`: spits out whatever is in its mouth.
fn drop_item(mob: &dyn Mob) -> bool {
    let mob_entity = mob.get_mob_entity();
    let stack = mob_entity.set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::EMPTY.clone());
    if stack.is_empty() {
        return false;
    }

    // Vanilla `Dolphin.ItemGoal.drop`.
    let entity = mob.get_entity();
    let pos = entity.pos.load();
    let hand_pos = Vector3::new(pos.x, entity.get_eye_y() - f64::from(0.3f32), pos.z);
    let mut rng = mob.get_random();
    let dir = rng.random::<f32>() * std::f32::consts::TAU;
    let pow = 0.02 * rng.random::<f32>();
    let (yaw, pitch) = (
        entity.yaw.load().to_radians(),
        entity.pitch.load().to_radians(),
    );
    let velocity = Vector3::new(
        f64::from(0.3 * -yaw.sin() * pitch.cos() + dir.cos() * pow),
        f64::from(0.3 * pitch.sin() * 1.5),
        f64::from(0.3 * yaw.cos() * pitch.cos() + dir.sin() * pow),
    );
    // TODO: vanilla also records the dolphin as the item's thrower, which Pumpkin's
    // item entities do not track yet.
    let world = entity.world.load_full();
    let item = ItemEntity::new_with_velocity(
        Entity::new(world.clone(), hand_pos, &EntityType::ITEM),
        stack,
        velocity,
        THROWN_ITEM_PICKUP_DELAY,
    );
    world.spawn_entity(Arc::new(item));
    true
}

/// Vanilla `Dolphin.MoveToItemGoal`: swims to a nearby item floating in water.
struct MoveToItemGoal {
    /// The world time before which the goal may not start again.
    cooldown: i64,
}

impl MoveToItemGoal {
    const fn new() -> Self {
        Self { cooldown: 0 }
    }

    fn move_to_first(mob: &dyn Mob, items: &[Arc<dyn EntityBase>]) {
        let Some(item) = items.first() else {
            return;
        };
        let pos = mob.get_entity().pos.load();
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_progress(NavigatorGoal::new(
                pos,
                item.get_entity().pos.load(),
                f64::from(1.2f32),
            ));
    }
}

impl Goal for MoveToItemGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if self.cooldown > mob.get_entity().world.load().get_world_age() {
            return false;
        }
        !allowed_items(mob).is_empty()
    }

    fn start(&mut self, mob: &dyn Mob) {
        let items = allowed_items(mob);
        if !items.is_empty() {
            Self::move_to_first(mob, &items);
            mob.get_entity().play_sound(Sound::EntityDolphinPlay);
        }
        self.cooldown = 0;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if drop_item(mob) {
            self.cooldown = mob.get_entity().world.load().get_world_age()
                + i64::from(mob.get_random().random_range(0..100));
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        Self::move_to_first(mob, &allowed_items(mob));
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `Dolphin.PlayWithItemsGoal`: tosses away whatever it carries.
struct PlayWithItemsGoal;

impl Goal for PlayWithItemsGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        !mob.get_mob_entity()
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&EquipmentSlot::MAIN_HAND)
            .is_empty()
    }

    fn stop(&mut self, mob: &dyn Mob) {
        drop_item(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        drop_item(mob);
    }
}

/// Vanilla `Dolphin.DolphinSwimToTreasureGoal`: a fed dolphin leads the way to the
/// nearest ocean ruin or shipwreck.
struct DolphinSwimToTreasureGoal {
    /// Vanilla keeps this on the dolphin as `treasurePos`; only this goal uses it.
    treasure_pos: Option<BlockPos>,
    stuck: bool,
}

impl DolphinSwimToTreasureGoal {
    const fn new() -> Self {
        Self {
            treasure_pos: None,
            stuck: false,
        }
    }

    fn dolphin(mob: &dyn Mob) -> Option<&DolphinEntity> {
        mob.cast_any().downcast_ref::<DolphinEntity>()
    }

    fn has_air(mob: &dyn Mob) -> bool {
        mob.get_mob_entity().living_entity.breath.air() >= TREASURE_MIN_AIR
    }

    /// Whether the dolphin is within 4 blocks of the treasure's column at its own height.
    fn reached(mob: &dyn Mob, treasure: BlockPos) -> bool {
        let pos = mob.get_entity().pos.load();
        let column = BlockPos::new(treasure.0.x, pos.y.floor() as i32, treasure.0.z);
        column.to_centered_f64().squared_distance_to_vec(&pos) < 4.0 * 4.0
    }

    fn broadcast_looking(mob: &dyn Mob) {
        let entity = mob.get_entity();
        entity.world.load().send_entity_status(
            entity,
            EntityStatus::DolphinLookingForTreasure,
            None,
        );
    }

    /// Vanilla `Dolphin.closeToNextPos`.
    fn close_to_next_pos(mob: &dyn Mob) -> bool {
        let target = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_target_pos();
        target.is_some_and(|target| {
            target
                .to_centered_f64()
                .squared_distance_to_vec(&mob.get_entity().pos.load())
                < 12.0 * 12.0
        })
    }
}

impl Goal for DolphinSwimToTreasureGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        Self::dolphin(mob).is_some_and(DolphinEntity::got_fish) && Self::has_air(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.treasure_pos
            .is_some_and(|treasure| !Self::reached(mob, treasure))
            && !self.stuck
            && Self::has_air(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.stuck = false;
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
        let entity = mob.get_entity();
        let structures = structures_in_tag(&tag::WorldgenStructure::MINECRAFT_DOLPHIN_LOCATED);
        let found = entity.world.load().find_nearest_map_structure(
            &structures,
            entity.block_pos.load(),
            TREASURE_SEARCH_RADIUS,
        );
        if let Some((treasure, _)) = found {
            self.treasure_pos = Some(treasure);
            Self::broadcast_looking(mob);
        } else {
            self.stuck = true;
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if (self
            .treasure_pos
            .is_none_or(|treasure| Self::reached(mob, treasure))
            || self.stuck)
            && let Some(dolphin) = Self::dolphin(mob)
        {
            dolphin.set_got_fish(false);
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(treasure) = self.treasure_pos else {
            return;
        };
        let navigation_done = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_done();
        if !Self::close_to_next_pos(mob) && !navigation_done {
            return;
        }

        let world = mob.get_entity().world.load();
        let treasure = treasure.to_centered_f64();
        let mut next = default_random_pos::get_pos_towards(
            mob,
            16,
            1,
            treasure,
            f64::from(std::f32::consts::FRAC_PI_8),
        )
        .or_else(|| {
            default_random_pos::get_pos_towards(
                mob,
                8,
                4,
                treasure,
                f64::from(std::f32::consts::FRAC_PI_2),
            )
        });
        if let Some(pos) = next {
            let block_pos = BlockPos::floored_v(pos);
            let (block, state) = world.get_block_and_state(&block_pos);
            if !goal_utils::is_water(&world, &block_pos)
                || !world
                    .block_registry
                    .is_pathfindable(block, state, PathComputationType::Water)
            {
                next = default_random_pos::get_pos_towards(
                    mob,
                    8,
                    5,
                    treasure,
                    f64::from(std::f32::consts::FRAC_PI_2),
                );
            }
        }
        let Some(next) = next else {
            self.stuck = true;
            return;
        };

        let mob_entity = mob.get_mob_entity();
        // Vanilla looks with `getMaxHeadYRot() + 20` and `getMaxHeadXRot()`, both 1 for dolphins.
        mob_entity
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .look_at_with_range(next.x, next.y, next.z, 21.0, 1.0);
        let pos = mob.get_entity().pos.load();
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_progress(NavigatorGoal::new(pos, next, 1.3));
        if mob.get_random().random_range(0..to_goal_ticks(80)) == 0 {
            Self::broadcast_looking(mob);
        }
    }

    fn can_stop(&self) -> bool {
        false
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}
