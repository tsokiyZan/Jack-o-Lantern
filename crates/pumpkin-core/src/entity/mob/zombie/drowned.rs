use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_util::Hand;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, Weak};

use crate::entity::ai::control::move_control::{MoveControl, Operation};
use crate::entity::ai::control::{Control, MoveControlTrait, rotlerp};
use crate::entity::ai::goal::active_target::ActiveTargetGoal;
use crate::entity::ai::goal::move_to_target_pos::{MoveToTargetPos, MoveToTargetPosGoal};
use crate::entity::ai::goal::ranged_attack::RangedAttackGoal;
use crate::entity::ai::goal::revenge::RevengeGoal;
use crate::entity::ai::goal::wander_around::WanderAroundGoal;
use crate::entity::ai::goal::zombie_attack::ZombieAttackGoal;
use crate::entity::ai::goal::{Controls, Goal, ParentHandle};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::pathfinder::{AmphibiousPathNavigation, Navigator, NavigatorGoal};
use crate::entity::ai::util::default_random_pos;
use crate::entity::living::LivingEntity;
use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::zombie::{
    ZombieEntityBase, baby_turtle_on_land_target, is_abstract_villager,
};
use crate::entity::projectile::arrow::ArrowPickup;
use crate::entity::projectile::trident::TridentEntity;
use crate::entity::spawn_util::is_face_full_up;
use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity, RangedAttackMob},
};
use crate::world::World;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math;

pub struct DrownedEntity {
    entity: Arc<ZombieEntityBase>,
    searching_for_land: AtomicBool,
}

impl DrownedEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        // The behaviour goals come once the drowned exists, which the trident goal needs.
        let entity =
            ZombieEntityBase::with_behaviour_goals(entity, false, BABY_DIMENSIONS, |_, _, _| {});
        let mob_entity = &entity.mob_entity;
        // Vanilla `Drowned.createNavigation`, then the constructor; after the navigator,
        // which holds the maluses.
        *mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            Navigator::new(AmphibiousPathNavigation::new(false).steered_by_move_control());
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(DrownedMoveControl::default());
        mob_entity.set_pathfinding_malus(PathType::Water, 0.0);
        let drowned = Arc::new(Self {
            entity,
            searching_for_land: AtomicBool::new(false),
        });
        drowned.add_behaviour_goals();
        drowned
    }

    /// Vanilla `Drowned.addBehaviourGoals`.
    fn add_behaviour_goals(self: &Arc<Self>) {
        let mob = &self.entity.mob_entity;
        let ranged: Weak<dyn RangedAttackMob> = {
            let this: Arc<dyn RangedAttackMob> = self.clone();
            Arc::downgrade(&this)
        };
        let sea_level = mob.living_entity.entity.world.load().sea_level;
        let mut goal_selector = mob
            .goals_selector
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut target_selector = mob
            .target_selector
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        goal_selector.add_goal(1, Box::new(DrownedGoToWaterGoal::new(1.0)));
        goal_selector.add_goal(
            2,
            Box::new(DrownedTridentAttackGoal::new(ranged, 1.0, 40, 10.0)),
        );
        goal_selector.add_goal(
            2,
            ZombieAttackGoal::new(1.0, false).gated_by(|mob| {
                mob.get_mob_entity()
                    .get_target()
                    .as_deref()
                    .is_some_and(|target| {
                        let world = mob.get_entity().world.load();
                        target
                            .get_living_entity()
                            .is_some_and(|target| ok_target(target, &world))
                    })
            }),
        );
        goal_selector.add_goal(5, DrownedGoToBeachGoal::new(1.0));
        goal_selector.add_goal(6, Box::new(DrownedSwimUpGoal::new(1.0, sea_level)));
        goal_selector.add_goal(7, Box::new(WanderAroundGoal::new(1.0)));

        target_selector.add_goal(
            1,
            Box::new(
                RevengeGoal::new(true)
                    .ignoring(|entity_type| entity_type == &EntityType::DROWNED)
                    .alerting_others_except(|entity_type| {
                        entity_type == &EntityType::ZOMBIFIED_PIGLIN
                    }),
            ),
        );
        target_selector.add_goal(
            2,
            Box::new(ActiveTargetGoal::new(
                mob,
                &EntityType::PLAYER,
                10,
                true,
                false,
                Some(ok_target),
            )),
        );
        target_selector.add_goal(
            3,
            ActiveTargetGoal::predicated(mob, 10, false, is_abstract_villager),
        );
        target_selector.add_goal(
            3,
            ActiveTargetGoal::with_default(mob, &EntityType::IRON_GOLEM, true),
        );
        target_selector.add_goal(
            3,
            ActiveTargetGoal::with_default(mob, &EntityType::AXOLOTL, true),
        );
        target_selector.add_goal(5, Box::new(baby_turtle_on_land_target(mob)));
    }

    fn from_mob(mob: &dyn Mob) -> Option<&Self> {
        mob.cast_any().downcast_ref::<Self>()
    }

    /// Vanilla `Drowned.wantsToSwim`.
    fn wants_to_swim(&self) -> bool {
        self.is_searching_for_land()
            || self
                .entity
                .mob_entity
                .get_target()
                .is_some_and(|target| target.get_entity().is_in_water())
    }

    fn is_searching_for_land(&self) -> bool {
        self.searching_for_land.load(Ordering::Relaxed)
    }

    fn set_searching_for_land(&self, searching_for_land: bool) {
        self.searching_for_land
            .store(searching_for_land, Ordering::Relaxed);
    }

    /// Vanilla `Drowned.closeToNextPos`: within 2 blocks of the path's final target.
    fn close_to_next_pos(&self) -> bool {
        let entity = self.get_entity();
        let pos = entity.pos.load();
        self.entity
            .mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_path()
            .is_some_and(|path| {
                let target = path.get_target().0;
                pos.squared_distance_to(
                    f64::from(target.x),
                    f64::from(target.y),
                    f64::from(target.z),
                ) < 4.0
            })
    }
}

/// Vanilla `Drowned.okTarget`: by day, drowned only go after targets in water.
fn ok_target(target: &LivingEntity, world: &World) -> bool {
    !world.is_bright_outside() || target.is_in_water()
}

/// Vanilla `Drowned.NAUTILUS_SHELL_CHANCE`.
const NAUTILUS_SHELL_CHANCE: f32 = 0.03;

/// Vanilla `Drowned.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    EntityDimensions::scalable(0.49, 0.98).with_eye_height(0.775);

impl Mob for DrownedEntity {
    crate::impl_zombie_mob!(entity);

    /// Vanilla `Drowned.finalizeSpawn`.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        let group_data = self
            .entity
            .finalize_zombie_spawn(self, world, group_data, false);
        let mob_entity = &self.entity.mob_entity;
        if mob_entity
            .get_item_slot(&EquipmentSlot::OFF_HAND)
            .is_empty()
            && self.get_random().random::<f32>() < NAUTILUS_SHELL_CHANCE
        {
            mob_entity.set_item_slot(
                &EquipmentSlot::OFF_HAND,
                ItemStack::new(1, &Item::NAUTILUS_SHELL),
            );
            mob_entity.set_guaranteed_drop(&EquipmentSlot::OFF_HAND);
        }
        // TODO: the zombie nautilus jockey needs `ZombieNautilus` and the `NATURAL` and
        // `STRUCTURE` spawn reasons.
        group_data
    }

    /// Vanilla `Drowned.populateDefaultEquipmentSlots`: no armor, sometimes a trident or
    /// a fishing rod.
    fn populate_default_equipment_slots(
        &self,
        _world: &Arc<World>,
        _difficulty: &RegionalDifficulty,
    ) {
        let mut random = self.get_random();
        if f64::from(random.random::<f32>()) > 0.9 {
            let item = if random.random_range(0..16) < 10 {
                &Item::TRIDENT
            } else {
                &Item::FISHING_ROD
            };
            self.entity
                .mob_entity
                .set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, item));
        }
    }

    /// Vanilla `Drowned.getPreferredWeaponType`.
    fn get_preferred_weapon_type(&self) -> Option<&'static pumpkin_data::tag::Tag> {
        Some(&pumpkin_data::tag::Item::MINECRAFT_DROWNED_PREFERRED_WEAPONS)
    }

    /// Vanilla `Drowned.canReplaceCurrentItem`: the nautilus shell stays.
    fn can_replace_current_item(
        &self,
        new_item: &ItemStack,
        current_item: &ItemStack,
        slot: &EquipmentSlot,
    ) -> bool {
        current_item.get_item() != &Item::NAUTILUS_SHELL
            && crate::entity::mob::equipment::can_replace_current_item(
                &self.entity.mob_entity,
                self.get_preferred_weapon_type(),
                new_item,
                current_item,
                slot,
            )
    }

    fn on_attack(&self, target: &dyn EntityBase) {
        self.entity.on_attack(self, target);
    }

    /// Vanilla `Drowned.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        Some(
            if self.get_mob_entity().living_entity.entity.is_in_water() {
                Sound::EntityDrownedAmbientWater
            } else {
                Sound::EntityDrownedAmbient
            },
        )
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    /// Vanilla `Drowned.travelInWater`: a drowned that wants to swim drifts on its
    /// move control's push instead of the usual water physics.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let living = &self.entity.mob_entity.living_entity;
        let entity = &living.entity;
        if !(entity.is_under_water() && self.wants_to_swim()) {
            return false;
        }
        entity.update_velocity_from_input(living.movement_input.load(), f64::from(0.01f32));
        living.make_move(caller);
        entity.velocity.store(entity.velocity.load() * 0.9);
        true
    }

    /// Vanilla `Drowned.isPushedByFluid`.
    fn mob_is_pushed_by_fluids(&self) -> bool {
        !self.get_entity().is_swimming()
    }

    /// Vanilla `Drowned.updateSwimming`.
    fn update_swimming(&self) {
        let entity = self.get_entity();
        entity.set_swimming(
            !self.entity.mob_entity.is_no_ai() && entity.is_under_water() && self.wants_to_swim(),
        );
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.entity.write_zombie_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.entity.read_zombie_nbt(self, nbt);
    }
}

impl RangedAttackMob for DrownedEntity {
    /// Vanilla `Drowned.performRangedAttack`.
    fn perform_ranged_attack(&self, target: &Arc<dyn EntityBase>, _power: f32) {
        let living = &self.entity.mob_entity.living_entity;
        let entity = &living.entity;
        let main_hand = living.held_item(self);
        let trident_item = if main_hand.item.id == Item::TRIDENT.id {
            main_hand
        } else {
            ItemStack::new(1, &Item::TRIDENT)
        };
        let world = entity.world.load_full();
        let trident = TridentEntity::new_shot(
            Entity::new(world.clone(), entity.pos.load(), &EntityType::TRIDENT),
            entity,
            trident_item.clone(),
            ArrowPickup::Disallowed,
        );
        let pos = entity.pos.load();
        let target_entity = target.get_entity();
        let target_pos = target_entity.pos.load();
        let xd = target_pos.x - pos.x;
        // `target.getY(0.3333333333333333)`.
        let yd = target_pos.y + f64::from(target_entity.height()) * 0.333_333_333_333_333_3
            - trident.get_entity().pos.load().y;
        let zd = target_pos.z - pos.z;
        #[expect(
            clippy::imprecise_flops,
            reason = "vanilla rounds Math.sqrt(xd * xd + zd * zd), not a hypot"
        )]
        let distance_to_target = (xd * xd + zd * zd).sqrt();
        // `Projectile.spawnProjectileUsingShoot`: shoot, add, then the enchantment effects.
        trident.set_velocity(
            xd,
            yd + distance_to_target * f64::from(0.2f32),
            zd,
            f64::from(1.6f32),
            f64::from(self.ranged_attack_uncertainty(&world)),
        );
        let trident = Arc::new(trident);
        world.spawn_entity(trident.clone());
        trident.apply_on_projectile_spawned(&trident_item);

        if !entity.is_silent() {
            let pitch = 1.0 / (self.get_random().random::<f32>() * 0.4 + 0.8);
            world.play_sound_fine(
                Sound::EntityDrownedShoot,
                self.sound_source(),
                &pos,
                1.0,
                pitch,
            );
        }
    }
}

/// Vanilla `Drowned.DrownedMoveControl`: swims towards the waypoint while the drowned
/// wants to swim in water, and otherwise walks like any mob, falling a little faster.
#[derive(Default)]
struct DrownedMoveControl {
    base: MoveControl,
}

impl Control for DrownedMoveControl {}

impl MoveControlTrait for DrownedMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let Some(drowned) = DrownedEntity::from_mob(mob) else {
            self.base.tick(mob);
            return;
        };
        let mob_entity = mob.get_mob_entity();
        let living = &mob_entity.living_entity;
        let entity = &living.entity;
        if drowned.wants_to_swim() && entity.is_in_water() {
            let y = entity.pos.load().y;
            if mob_entity
                .get_target()
                .is_some_and(|target| target.get_entity().pos.load().y > y)
                || drowned.is_searching_for_land()
            {
                entity
                    .velocity
                    .store(entity.velocity.load() + Vector3::new(0.0, 0.002, 0.0));
            }

            let navigation_done = mob_entity
                .navigator
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_done();
            if self.base.operation != Operation::MoveTo || navigation_done {
                mob_entity.set_speed(0.0);
                return;
            }

            let pos = entity.pos.load();
            let xd = self.base.wanted_x - pos.x;
            let mut yd = self.base.wanted_y - pos.y;
            let zd = self.base.wanted_z - pos.z;
            let dd = (xd * xd + yd * yd + zd * zd).sqrt();
            yd /= dd;
            let y_rot_d = (math::atan2(zd, xd) * f64::from(180.0f32)
                / f64::from(std::f32::consts::PI)) as f32
                - 90.0;
            let yaw = rotlerp(entity.yaw.load(), y_rot_d, 90.0);
            entity.yaw.store(yaw);
            entity.body_yaw.store(yaw);
            let target_speed = (self.base.speed_modifier
                * living.get_attribute_value(&Attributes::MOVEMENT_SPEED))
                as f32;
            let new_speed = math::lerp(0.125, living.speed.load(), target_speed);
            mob_entity.set_speed(new_speed);
            let new_speed = f64::from(new_speed);
            entity.velocity.store(
                entity.velocity.load()
                    + Vector3::new(
                        new_speed * xd * 0.005,
                        new_speed * yd * 0.1,
                        new_speed * zd * 0.005,
                    ),
            );
        } else {
            if !entity.on_ground.load(Ordering::Relaxed) {
                entity
                    .velocity
                    .store(entity.velocity.load() + Vector3::new(0.0, -0.008, 0.0));
            }
            self.base.tick(mob);
        }
    }

    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed_modifier: f64) {
        self.base.set_wanted_position(x, y, z, speed_modifier);
    }

    fn strafe(&mut self, forward: f32, right: f32) {
        self.base.strafe(forward, right);
    }

    fn has_wanted(&self) -> bool {
        self.base.has_wanted()
    }
}

/// Vanilla `Drowned.DrownedGoToWaterGoal`: by day, a drowned on land heads for water.
struct DrownedGoToWaterGoal {
    wanted: Vector3<f64>,
    speed_modifier: f64,
}

impl DrownedGoToWaterGoal {
    const fn new(speed_modifier: f64) -> Self {
        Self {
            wanted: Vector3::new(0.0, 0.0, 0.0),
            speed_modifier,
        }
    }

    fn get_water_pos(mob: &dyn Mob) -> Option<Vector3<f64>> {
        let mut random = mob.get_random();
        let pos = mob.get_entity().block_pos.load();
        let world = mob.get_entity().world.load();
        for _ in 0..10 {
            let x = random.random_range(0..20) - 10;
            let y = 2 - random.random_range(0..8);
            let z = random.random_range(0..20) - 10;
            let random_pos = pos.add(x, y, z);
            if world.get_block(&random_pos) == &pumpkin_data::Block::WATER {
                // `Vec3.atBottomCenterOf`.
                return Some(Vector3::new(
                    f64::from(random_pos.0.x) + 0.5,
                    f64::from(random_pos.0.y),
                    f64::from(random_pos.0.z) + 0.5,
                ));
            }
        }
        None
    }
}

impl Goal for DrownedGoToWaterGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if !mob.get_entity().world.load().is_bright_outside() || mob.get_entity().is_in_water() {
            return false;
        }
        let Some(pos) = Self::get_water_pos(mob) else {
            return false;
        };
        self.wanted = pos;
        true
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        !mob.is_navigator_idle()
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_progress(NavigatorGoal::new(
                mob.get_entity().pos.load(),
                self.wanted,
                self.speed_modifier,
            ));
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `Drowned.DrownedTridentAttackGoal`: a `RangedAttackGoal` while holding a trident.
struct DrownedTridentAttackGoal {
    goal: RangedAttackGoal,
}

impl DrownedTridentAttackGoal {
    fn new(
        mob: Weak<dyn RangedAttackMob>,
        speed_modifier: f64,
        attack_interval: i32,
        attack_radius: f32,
    ) -> Self {
        Self {
            goal: RangedAttackGoal::new(mob, speed_modifier, attack_interval, attack_radius)
                .gated_by(|mob| main_hand_item(mob).item.id == Item::TRIDENT.id),
        }
    }
}

fn main_hand_item(mob: &dyn Mob) -> ItemStack {
    mob.get_mob_entity()
        .living_entity
        .entity_equipment
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&EquipmentSlot::MAIN_HAND)
}

impl Goal for DrownedTridentAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.goal.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.goal.start(mob);
        let mob_entity = mob.get_mob_entity();
        mob_entity.set_attacking(true);
        // `startUsingItem(MAIN_HAND)`.
        let living = &mob_entity.living_entity;
        let stack = main_hand_item(mob);
        let using_item = living
            .item_in_use
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if !stack.is_empty() && !using_item {
            let duration = mob
                .get_entity()
                .world
                .load()
                .server
                .upgrade()
                .and_then(|server| server.item_registry.get_use_duration(stack.item.id))
                .unwrap_or(0);
            living.set_active_hand(Hand::Right, stack, duration);
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.goal.stop(mob);
        let mob_entity = mob.get_mob_entity();
        mob_entity.living_entity.clear_active_hand();
        mob_entity.set_attacking(false);
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

/// Vanilla `Drowned.DrownedGoToBeachGoal`: at night, a drowned near the surface heads
/// for a dry spot it can stand on.
struct DrownedGoToBeachGoal {
    goal: MoveToTargetPosGoal<Self>,
}

impl DrownedGoToBeachGoal {
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
}

impl MoveToTargetPos for DrownedGoToBeachGoal {
    fn is_target_pos(&self, world: Arc<World>, block_pos: BlockPos) -> bool {
        let above = block_pos.up();
        world.get_block_state(&above).is_air()
            && world.get_block_state(&above.up()).is_air()
            && is_face_full_up(world.get_block_state(&block_pos))
    }
}

impl Goal for DrownedGoToBeachGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let entity = mob.get_entity();
        let world = entity.world.load();
        self.goal.can_start(mob)
            && !world.is_bright_outside()
            && entity.is_in_water()
            && entity.pos.load().y >= f64::from(world.sea_level - 3)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(drowned) = DrownedEntity::from_mob(mob) {
            drowned.set_searching_for_land(false);
        }
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

/// Vanilla `Drowned.DrownedSwimUpGoal`: at night, a deep drowned swims up towards the
/// surface. It claims no controls, so it runs alongside the others.
struct DrownedSwimUpGoal {
    speed_modifier: f64,
    sea_level: i32,
    stuck: bool,
}

impl DrownedSwimUpGoal {
    const fn new(speed_modifier: f64, sea_level: i32) -> Self {
        Self {
            speed_modifier,
            sea_level,
            stuck: false,
        }
    }
}

impl Goal for DrownedSwimUpGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let entity = mob.get_entity();
        !entity.world.load().is_bright_outside()
            && entity.is_in_water()
            && entity.pos.load().y < f64::from(self.sea_level - 2)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_start(mob) && !self.stuck
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(drowned) = DrownedEntity::from_mob(mob) else {
            return;
        };
        let pos = mob.get_entity().pos.load();
        if pos.y < f64::from(self.sea_level - 1)
            && (mob.is_navigator_idle() || drowned.close_to_next_pos())
        {
            let Some(next_pos) = default_random_pos::get_pos_towards(
                mob,
                4,
                8,
                Vector3::new(pos.x, f64::from(self.sea_level - 1), pos.z),
                f64::from(std::f32::consts::FRAC_PI_2),
            ) else {
                self.stuck = true;
                return;
            };
            mob.get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .set_progress(NavigatorGoal::new(pos, next_pos, self.speed_modifier));
        }
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(drowned) = DrownedEntity::from_mob(mob) {
            drowned.set_searching_for_land(true);
        }
        self.stuck = false;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(drowned) = DrownedEntity::from_mob(mob) {
            drowned.set_searching_for_land(false);
        }
    }
}
