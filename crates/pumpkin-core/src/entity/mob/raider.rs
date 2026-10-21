use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;

use crate::entity::EntityBase;
use crate::entity::ai::goal::{Controls, Goal};
use crate::entity::ai::pathfinder::NavigatorGoal;
use crate::entity::ai::pathfinder::path::Path;
use crate::entity::ai::util::default_random_pos;
use crate::entity::item::ItemEntity;
use crate::entity::mob::Mob;
use crate::entity::mob::patrol::{PatrolData, PatrollingMonster};
use pumpkin_util::random::RandomImpl;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[must_use]
pub fn create_ominous_banner() -> ItemStack {
    use pumpkin_data::data_component::DataComponent;
    use pumpkin_data::data_component_impl::{CustomNameImpl, DataComponentImpl};
    let mut stack = ItemStack::new(1, &Item::WHITE_BANNER);
    stack.patch.push((
        DataComponent::CustomName,
        Some(
            CustomNameImpl {
                name: TextComponent::translate("block.minecraft.ominous_banner", []),
            }
            .to_dyn(),
        ),
    ));
    stack
}

/// Vanilla `ItemStack.matches(stack, Raid.getOminousBannerInstance(...))`.
#[must_use]
pub fn is_ominous_banner(stack: &ItemStack) -> bool {
    stack.are_equal(&create_ominous_banner())
}

pub struct RaiderData {
    pub patrol_data: PatrolData,
    pub wave: AtomicI32,
    pub can_join_raid: AtomicBool,
    pub ticks_outside_raid: AtomicI32,
    pub is_celebrating: AtomicBool,
    pub raid_id: AtomicCell<Option<i32>>,
}

impl Default for RaiderData {
    fn default() -> Self {
        Self {
            patrol_data: PatrolData::default(),
            wave: AtomicI32::new(0),
            can_join_raid: AtomicBool::new(false),
            ticks_outside_raid: AtomicI32::new(0),
            is_celebrating: AtomicBool::new(false),
            raid_id: AtomicCell::new(None),
        }
    }
}

pub trait Raider: PatrollingMonster {
    fn get_raider_data(&self) -> &RaiderData;

    fn can_join_raid(&self) -> bool {
        self.get_raider_data().can_join_raid.load(Ordering::Relaxed)
    }

    fn set_can_join_raid(&self, can_join: bool) {
        self.get_raider_data()
            .can_join_raid
            .store(can_join, Ordering::Relaxed);
    }

    fn get_wave(&self) -> i32 {
        self.get_raider_data().wave.load(Ordering::Relaxed)
    }

    fn set_wave(&self, wave: i32) {
        self.get_raider_data().wave.store(wave, Ordering::Relaxed);
    }

    fn is_celebrating(&self) -> bool {
        self.get_raider_data()
            .is_celebrating
            .load(Ordering::Relaxed)
    }

    fn set_celebrating(&self, celebrating: bool) {
        self.get_raider_data()
            .is_celebrating
            .store(celebrating, Ordering::Relaxed);
        let entity = &self.get_mob_entity().living_entity.entity;
        entity.set_synced_data(tracked_data::pillager::IS_CELEBRATING, celebrating);
    }

    fn has_active_raid(&self) -> bool {
        self.get_raider_data().raid_id.load().is_some()
    }

    fn is_captain(&self) -> bool {
        self.is_patrol_leader()
    }

    fn get_celebrate_sound(&self) -> Sound;

    fn apply_raid_buffs(&self, _wave: i32, _is_captain: bool) {}

    fn write_raider_nbt(&self, nbt: &mut NbtCompound) {
        self.write_patrol_nbt(nbt);
        let data = self.get_raider_data();
        nbt.put_int("Wave", data.wave.load(Ordering::Relaxed));
        nbt.put_bool("CanJoinRaid", data.can_join_raid.load(Ordering::Relaxed));
        if let Some(raid_id) = data.raid_id.load() {
            nbt.put_int("RaidId", raid_id);
        }
    }

    fn read_raider_nbt(&self, nbt: &NbtCompound) {
        self.read_patrol_nbt(nbt);
        let data = self.get_raider_data();
        if let Some(wave) = nbt.get_int("Wave") {
            data.wave.store(wave, Ordering::Relaxed);
        }
        if let Some(can_join) = nbt.get_bool("CanJoinRaid") {
            data.can_join_raid.store(can_join, Ordering::Relaxed);
        }
        if let Some(raid_id) = nbt.get_int("RaidId") {
            data.raid_id.store(Some(raid_id));
        }
    }
}

/// Goal for raiders holding ground during patrols when spotting distant targets, alerting nearby raiders.
pub struct HoldGroundAttackGoal {
    hostile_radius_sqr: f64,
}

impl HoldGroundAttackGoal {
    #[must_use]
    pub fn new(hostile_radius: f32) -> Self {
        Self {
            hostile_radius_sqr: f64::from(hostile_radius * hostile_radius),
        }
    }
}

impl Goal for HoldGroundAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(raider) = mob.as_raider() else {
            return false;
        };
        if raider.has_active_raid() || !raider.is_patrolling() {
            return false;
        }
        let target = mob.get_mob_entity().get_target().clone();
        target.is_some()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let target = mob.get_mob_entity().get_target().clone();
        target.is_some()
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();

        let target = mob.get_mob_entity().get_target().clone();
        if let Some(target) = target {
            let entity = mob.get_entity();
            let world = entity.world.load();
            let bb = entity.bounding_box.load().expand(8.0, 8.0, 8.0);
            let nearby = world.get_entities_at_box(&bb);

            for cand in nearby {
                if cand.get_entity().entity_id != entity.entity_id
                    && let Some(cand_mob) = cand.get_mob()
                    && cand_mob.as_raider().is_some()
                {
                    cand_mob.get_mob_entity().set_target(Some(target.clone()));
                }
            }
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let target = mob.get_mob_entity().get_target().clone();
        if let Some(target) = target {
            let mob_pos = mob.get_entity().pos.load();
            let target_pos = target.get_entity().pos.load();
            let dist_sq = mob_pos.squared_distance_to_vec(&target_pos);

            if dist_sq > self.hostile_radius_sqr {
                mob.get_mob_entity()
                    .look_control
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .look_at_entity_with_range(&target, 30.0, 30.0);
            }
        }
    }
}

/// Vanilla `Raider.ObtainRaidLeaderBannerGoal`: during a raid whose wave has no living
/// leader, walks to a reachable dropped ominous banner and picks it up.
#[derive(Default)]
pub struct ObtainRaidLeaderBannerGoal {
    /// Vanilla `unreachableBannerCache`: item entity id to the game time it is retried.
    unreachable_banner_cache: HashMap<i32, i64>,
    path_to_banner: Option<Path>,
    pursued_banner: Option<Arc<dyn EntityBase>>,
}

/// Vanilla `Raider.ALLOWED_ITEMS`.
fn is_allowed_banner(entity: &dyn EntityBase) -> bool {
    entity
        .cast_any()
        .downcast_ref::<ItemEntity>()
        .is_some_and(|item| {
            item.get_pickup_delay() == 0
                && item.get_entity().is_alive()
                && is_ominous_banner(
                    &item
                        .get_item_stack()
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                )
        })
}

/// The raid `mob` is in, while it is active.
fn active_raid_of(mob: &dyn Mob, raider: &dyn Raider) -> Option<(bool, Option<Uuid>)> {
    let raid_id = raider.get_raider_data().raid_id.load()?;
    let world = mob.get_entity().world.load();
    let raids = world
        .raids
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let raid = raids.get(raid_id).filter(|raid| raid.is_active())?;
    Some((
        raid.is_over(),
        raid.group_to_leader_map.get(&raider.get_wave()).copied(),
    ))
}

/// Vanilla `ObtainRaidLeaderBannerGoal.cannotPickUpBanner`.
fn cannot_pick_up_banner(mob: &dyn Mob) -> bool {
    let Some(raider) = mob.as_raider() else {
        return true;
    };
    let Some((is_over, leader)) = active_raid_of(mob, raider) else {
        return true;
    };
    if is_over || !raider.can_be_leader() {
        return true;
    }
    let wears_banner = mob
        .get_mob_entity()
        .living_entity
        .entity_equipment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&EquipmentSlot::HEAD)
        .are_equal(&create_ominous_banner());
    if wears_banner {
        return true;
    }
    leader.is_some_and(|leader| {
        mob.get_entity()
            .world
            .load()
            .get_entity_by_uuid(leader)
            .is_some_and(|leader| leader.get_entity().is_alive())
    })
}

impl Goal for ObtainRaidLeaderBannerGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if cannot_pick_up_banner(mob) {
            return false;
        }
        let mob_entity = mob.get_mob_entity();
        let entity = mob.get_entity();
        let world = entity.world.load();
        let game_time = world.get_world_age();
        let follow_range = mob_entity
            .living_entity
            .get_attribute_value(&pumpkin_data::attributes::Attributes::FOLLOW_RANGE);
        let mut temp_cache = HashMap::new();
        let banners = world
            .get_entities_at_box(&entity.bounding_box.load().expand(
                follow_range,
                8.0,
                follow_range,
            ))
            .into_iter()
            .filter(|item| is_allowed_banner(item.as_ref()));
        for banner in banners {
            let banner_id = banner.get_entity().entity_id;
            let unreachable_until = self
                .unreachable_banner_cache
                .get(&banner_id)
                .copied()
                .unwrap_or(i64::MIN);
            if game_time < unreachable_until {
                temp_cache.insert(banner_id, unreachable_until);
                continue;
            }
            let path = mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .create_path(&mob_entity.living_entity, banner.get_entity().pos.load(), 1);
            if let Some(path) = path.filter(Path::can_reach) {
                self.path_to_banner = Some(path);
                self.pursued_banner = Some(banner);
                return true;
            }
            temp_cache.insert(banner_id, game_time + 600);
        }
        self.unreachable_banner_cache = temp_cache;
        false
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let Some(banner) = &self.pursued_banner else {
            return false;
        };
        if self.path_to_banner.is_none() || banner.get_entity().is_removed() {
            return false;
        }
        // `pathToBanner.isDone()`: the navigation follows that path.
        let path_done = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_done();
        !path_done && !cannot_pick_up_banner(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        let mob_entity = mob.get_mob_entity();
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .move_to_path(self.path_to_banner.clone(), 1.15, &mob_entity.living_entity);
    }

    fn stop(&mut self, _mob: &dyn Mob) {
        self.path_to_banner = None;
        self.pursued_banner = None;
    }

    fn tick(&mut self, mob: &dyn Mob) {
        if let Some(banner) = &self.pursued_banner
            && banner
                .get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&mob.get_entity().pos.load())
                < 1.414 * 1.414
            && let Some(item) = banner.get_item_entity()
        {
            mob.mob_pick_up_item(item);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `Raider.pickUpItem`: in an active raid whose wave has no leader, an ominous banner
/// makes this raider the leader; anything else goes through `Mob.pickUpItem`.
pub fn pick_up_item<M: Mob + ?Sized>(mob: &M, item: &ItemEntity) {
    let stack = item
        .get_item_stack()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let Some(raider) = mob.as_raider() else {
        crate::entity::mob::equipment::pick_up_item(mob, item);
        return;
    };
    let entity = mob.get_entity();
    let world = entity.world.load();
    let leads = raider
        .get_raider_data()
        .raid_id
        .load()
        .is_some_and(|raid_id| {
            world
                .raids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(raid_id)
                .is_some_and(|raid| {
                    raid.is_active() && !raid.group_to_leader_map.contains_key(&raider.get_wave())
                })
        });
    if !leads || !is_ominous_banner(&stack) {
        crate::entity::mob::equipment::pick_up_item(mob, item);
        return;
    }
    let mob_entity = mob.get_mob_entity();
    let current = mob_entity.get_item_slot(&EquipmentSlot::HEAD);
    let drop_chance = mob_entity.drop_chance(&EquipmentSlot::HEAD);
    if !current.is_empty() && (entity.random().next_f32() - 0.1).max(0.0) < drop_chance {
        entity.spawn_at_location(current);
    }
    mob.set_item_slot_and_notify(&EquipmentSlot::HEAD, stack.clone());
    mob_entity
        .living_entity
        .pickup(item.get_entity(), u32::from(stack.item_count));
    item.get_entity().remove();
    // Vanilla `Raid.setLeader`, whose banner is the one just worn.
    if let Some(raid_id) = raider.get_raider_data().raid_id.load()
        && let Some(raid) = world
            .raids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(raid_id)
    {
        raid.group_to_leader_map
            .insert(raider.get_wave(), entity.entity_uuid);
    }
    mob_entity.set_guaranteed_drop(&EquipmentSlot::HEAD);
    raider.set_patrol_leader(true);
}

/// Goal for raiders celebrating victory.
pub struct RaiderCelebrationGoal;

impl Goal for RaiderCelebrationGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(raider) = mob.as_raider() else {
            return false;
        };
        let target = mob.get_mob_entity().get_target().clone();
        target.is_none() && raider.is_celebrating()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let Some(raider) = mob.as_raider() else {
            return false;
        };
        let target = mob.get_mob_entity().get_target().clone();
        target.is_none() && raider.is_celebrating()
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(raider) = mob.as_raider() {
            raider.set_celebrating(true);
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(raider) = mob.as_raider() {
            raider.set_celebrating(false);
        }
    }

    /// Vanilla `Raider.RaiderCelebration.tick`.
    fn tick(&mut self, mob: &dyn Mob) {
        let Some(raider) = mob.as_raider() else {
            return;
        };
        let entity = mob.get_entity();

        if !entity.is_silent() && entity.random().next_bounded_i32(self.get_tick_count(100)) == 0 {
            mob.make_sound(raider.get_celebrate_sound());
        }

        if !mob.is_passenger() && entity.random().next_bounded_i32(self.get_tick_count(50)) == 0 {
            mob.get_mob_entity().jump();
        }
    }
}

/// Vanilla `Raider.RaiderMoveThroughVillageGoal`: during a raid, walk from home to home.
///
/// Pumpkin keeps no `PoiTypes.HOME` (bed) records, so `PoiManager.getRandom` finds nothing
/// here and the goal never starts, as in a vanilla village without beds.
pub struct RaiderMoveThroughVillageGoal {
    speed_modifier: f64,
    distance_to_poi: i32,
    poi_pos: BlockPos,
    visited: Vec<BlockPos>,
    stuck: bool,
}

impl RaiderMoveThroughVillageGoal {
    #[must_use]
    pub const fn new(speed_modifier: f64, distance_to_poi: i32) -> Self {
        Self {
            speed_modifier,
            distance_to_poi,
            poi_pos: BlockPos::ZERO,
            visited: Vec::new(),
            stuck: false,
        }
    }

    /// Vanilla `isValidRaid`.
    fn is_valid_raid(mob: &dyn Mob) -> bool {
        mob.as_raider()
            .and_then(|raider| active_raid_of(mob, raider))
            .is_some_and(|(is_over, _)| !is_over)
    }

    /// Vanilla `hasSuitablePoi`.
    fn has_suitable_poi(&mut self, mob: &dyn Mob) -> bool {
        let Some(home_pos) = Self::random_home_poi(mob, |pos| self.has_not_visited(pos)) else {
            return false;
        };
        self.poi_pos = home_pos;
        true
    }

    /// Vanilla `PoiManager.getRandom(HOME, filter, ANY, blockPosition(), 48, raider.random)`.
    /// Pumpkin keeps no home POIs, so none is found, and the shuffle of the empty list
    /// draws nothing.
    fn random_home_poi(_mob: &dyn Mob, _filter: impl Fn(&BlockPos) -> bool) -> Option<BlockPos> {
        None
    }

    fn has_not_visited(&self, poi: &BlockPos) -> bool {
        !self.visited.contains(poi)
    }

    /// Vanilla `updateVisited`.
    fn update_visited(&mut self) {
        if self.visited.len() > 2 {
            self.visited.remove(0);
        }
    }

    /// Vanilla `BlockPos.closerToCenterThan(position, distance)`.
    fn poi_closer_to_center_than(&self, position: Vector3<f64>, distance: f64) -> bool {
        self.poi_pos
            .to_centered_f64()
            .squared_distance_to_vec(&position)
            < distance * distance
    }

    /// Vanilla `navigation.moveTo(x, y, z, speedModifier)`.
    fn move_to(&self, mob: &dyn Mob, target: Vector3<f64>) {
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_progress(NavigatorGoal::new(
                mob.get_entity().pos.load(),
                target,
                self.speed_modifier,
            ));
    }
}

impl Goal for RaiderMoveThroughVillageGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.update_visited();
        Self::is_valid_raid(mob)
            && self.has_suitable_poi(mob)
            && mob.get_mob_entity().get_target().is_none()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        if mob.is_navigator_idle() {
            return false;
        }
        let distance = f64::from(mob.get_entity().width()) + f64::from(self.distance_to_poi);
        mob.get_mob_entity().get_target().is_none()
            && !self.poi_closer_to_center_than(mob.get_entity().pos.load(), distance)
            && !self.stuck
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity()
            .living_entity
            .no_action_time
            .store(0, Ordering::Relaxed);
        let poi = self.poi_pos.0;
        self.move_to(
            mob,
            Vector3::new(f64::from(poi.x), f64::from(poi.y), f64::from(poi.z)),
        );
        self.stuck = false;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if self
            .poi_closer_to_center_than(mob.get_entity().pos.load(), f64::from(self.distance_to_poi))
        {
            self.visited.push(self.poi_pos);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }

    /// Vanilla `tick` (Raider.java:539-554): `DefaultRandomPos.getPosTowards` on the
    /// raider's own random, narrow first and then wide.
    fn tick(&mut self, mob: &dyn Mob) {
        if !mob.is_navigator_idle() {
            return;
        }
        // `Vec3.atBottomCenterOf(poiPos)`.
        let poi = self.poi_pos.0;
        let poi_vec = Vector3::new(
            f64::from(poi.x) + 0.5,
            f64::from(poi.y),
            f64::from(poi.z) + 0.5,
        );
        let next_pos = default_random_pos::get_pos_towards(
            mob,
            16,
            7,
            poi_vec,
            f64::from(std::f32::consts::PI / 10.0),
        )
        .or_else(|| {
            default_random_pos::get_pos_towards(
                mob,
                8,
                7,
                poi_vec,
                f64::from(std::f32::consts::FRAC_PI_2),
            )
        });
        let Some(next_pos) = next_pos else {
            self.stuck = true;
            return;
        };
        self.move_to(mob, next_pos);
    }
}

pub use crate::entity::ai::goal::pathfind_to_raid::PathfindToRaidGoal;

/// Vanilla `PatrollingMonster.registerGoals` then `Raider.registerGoals`, which every raider
/// runs before registering its own goals.
pub fn add_raider_goals(goal_selector: &mut crate::entity::ai::goal::goal_selector::GoalSelector) {
    goal_selector.add_goal(
        4,
        Box::new(crate::entity::mob::patrol::LongDistancePatrolGoal::new(
            0.7, 0.595,
        )),
    );
    goal_selector.add_goal(1, Box::new(ObtainRaidLeaderBannerGoal::default()));
    goal_selector.add_goal(3, Box::new(PathfindToRaidGoal::default()));
    goal_selector.add_goal(4, Box::new(RaiderMoveThroughVillageGoal::new(1.05, 1)));
    goal_selector.add_goal(5, Box::new(RaiderCelebrationGoal));
}

/// Vanilla `Raider` class membership.
#[must_use]
pub fn is_raider(entity_type: &'static EntityType) -> bool {
    use pumpkin_data::tag::Taggable;
    entity_type.has_tag(&pumpkin_data::tag::EntityType::MINECRAFT_RAIDERS)
}

/// Vanilla `HurtByTargetGoal(this, Raider.class).setAlertOthers()`.
#[must_use]
pub fn raider_hurt_by_target() -> Box<crate::entity::ai::goal::revenge::RevengeGoal> {
    Box::new(
        crate::entity::ai::goal::revenge::RevengeGoal::new(true)
            .ignoring(is_raider)
            .alerting_others(),
    )
}

/// Vanilla `AbstractVillager` class membership, as a target predicate.
#[must_use]
pub fn is_abstract_villager(
    target: &crate::entity::living::LivingEntity,
    _world: &crate::world::World,
) -> bool {
    let entity_type = target.entity.entity_type;
    entity_type == &EntityType::VILLAGER || entity_type == &EntityType::WANDERING_TRADER
}
