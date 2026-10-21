use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use crate::entity::ai::control::move_control::Operation;
use crate::entity::ai::control::{Control, MoveControlTrait};
use crate::entity::ai::goal::track_target::TrackTargetGoal;
use crate::entity::ai::goal::{Controls, Goal, to_goal_ticks};
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::mob::raider::raider_hurt_by_target;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::world::World;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::Sound;
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use uuid::Uuid;

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{active_target::ActiveTargetGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal},
    mob::{Mob, MobEntity},
};

/// Vanilla `Vex.TICKS_PER_FLAP`: `Mth.ceil((float) (Math.PI * 5.0 / 4.0))`.
const TICKS_PER_FLAP: i32 = 4;
/// Vanilla `Vex.FLAG_IS_CHARGING`.
const FLAG_IS_CHARGING: u8 = 1;

pub struct VexEntity {
    pub mob_entity: MobEntity,
    /// Vanilla `DATA_FLAGS_ID`.
    flags: AtomicU8,
    owner: Mutex<Option<Uuid>>,
    bound_origin: Mutex<Option<BlockPos>>,
    has_limited_life: AtomicBool,
    limited_life_ticks: AtomicI32,
}

impl VexEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `Vex`'s constructor.
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(VexMoveControl::default());
        let mob_arc = Arc::new(Self {
            mob_entity,
            flags: AtomicU8::new(0),
            owner: Mutex::new(None),
            bound_origin: Mutex::new(None),
            has_limited_life: AtomicBool::new(false),
            limited_life_ticks: AtomicI32::new(0),
        });
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

            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(4, Box::new(VexChargeAttackGoal));
            goal_selector.add_goal(8, Box::new(VexRandomMoveGoal));
            goal_selector.add_goal(
                9,
                Box::new(LookAtEntityGoal::new(
                    mob_weak.clone(),
                    &EntityType::PLAYER,
                    3.0,
                    1.0,
                    false,
                )),
            );
            goal_selector.add_goal(10, LookAtEntityGoal::at_mobs(8.0));
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // `HurtByTargetGoal(this, Raider.class).setAlertOthers()`.
            target_selector.add_goal(1, raider_hurt_by_target());
            target_selector.add_goal(2, Box::new(VexCopyOwnerTargetGoal::new()));
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true),
            );
        };

        mob_arc
    }

    fn get_vex_flag(&self, flag: u8) -> bool {
        self.flags.load(Ordering::Relaxed) & flag != 0
    }

    fn set_vex_flag(&self, flag: u8, value: bool) {
        let flags = self.flags.load(Ordering::Relaxed);
        let flags = if value { flags | flag } else { flags & !flag };
        self.flags.store(flags, Ordering::Relaxed);
        self.mob_entity
            .living_entity
            .entity
            .set_synced_data(tracked_data::vex::DATA_FLAGS_ID, flags as i8);
    }

    #[must_use]
    pub fn is_charging(&self) -> bool {
        self.get_vex_flag(FLAG_IS_CHARGING)
    }

    pub fn set_is_charging(&self, value: bool) {
        self.set_vex_flag(FLAG_IS_CHARGING, value);
    }

    pub fn set_owner(&self, owner: Uuid) {
        *self.owner.lock().unwrap_or_else(PoisonError::into_inner) = Some(owner);
    }

    /// Vanilla `getOwner`: the owner while it is a living entity in this world.
    fn get_owner(&self) -> Option<Arc<dyn EntityBase>> {
        let owner = (*self.owner.lock().unwrap_or_else(PoisonError::into_inner))?;
        self.mob_entity
            .living_entity
            .entity
            .world
            .load()
            .get_entity_by_uuid(owner)
            .filter(|owner| owner.get_living_entity().is_some())
    }

    #[must_use]
    pub fn get_bound_origin(&self) -> Option<BlockPos> {
        *self
            .bound_origin
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_bound_origin(&self, bound_origin: Option<BlockPos>) {
        *self
            .bound_origin
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = bound_origin;
    }

    pub fn set_limited_life(&self, life_ticks: i32) {
        self.has_limited_life.store(true, Ordering::Relaxed);
        self.limited_life_ticks.store(life_ticks, Ordering::Relaxed);
    }
}

fn as_vex(mob: &dyn Mob) -> Option<&VexEntity> {
    mob.cast_any().downcast_ref::<VexEntity>()
}

fn move_control_has_wanted(mob: &dyn Mob) -> bool {
    mob.get_mob_entity()
        .move_control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .has_wanted()
}

fn set_wanted_position(mob: &dyn Mob, pos: Vector3<f64>, speed_modifier: f64) {
    mob.get_mob_entity()
        .move_control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .set_wanted_position(pos.x, pos.y, pos.z, speed_modifier);
}

impl Mob for VexEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `Vex.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        Some(Sound::EntityVexAmbient)
    }

    /// Vanilla `Vex.isFlapping`.
    fn is_flapping(&self) -> bool {
        self.mob_entity
            .living_entity
            .entity
            .tick_count
            .load(Ordering::Relaxed)
            % TICKS_PER_FLAP
            == 0
    }

    /// The start of vanilla `Vex.tick`: the vex moves through blocks during its tick.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.mob_entity
            .living_entity
            .entity
            .no_physics
            .store(true, Ordering::Relaxed);
    }

    /// The rest of vanilla `Vex.tick`, after `super.tick()`.
    fn post_tick(&self) {
        let entity = &self.mob_entity.living_entity.entity;
        entity.no_physics.store(false, Ordering::Relaxed);
        entity.set_has_no_gravity(true);
        if self.has_limited_life.load(Ordering::Relaxed)
            && self.limited_life_ticks.fetch_sub(1, Ordering::Relaxed) - 1 <= 0
        {
            self.limited_life_ticks.store(20, Ordering::Relaxed);
            self.damage(self, 1.0, DamageType::STARVE);
        }
    }

    /// Vanilla `Vex.finalizeSpawn`: its sword and the enchantments, before `super`.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        let difficulty = crate::entity::mob::equipment::RegionalDifficulty::at(
            world,
            self.get_entity().pos.load(),
        );
        self.populate_default_equipment_slots(world, &difficulty);
        self.populate_default_equipment_enchantments(world, &difficulty);
        self.mob_entity.finalize_spawn_base();
        group_data
    }

    /// Vanilla `Vex.populateDefaultEquipmentSlots`: an iron sword that never drops.
    fn populate_default_equipment_slots(
        &self,
        _world: &Arc<World>,
        _difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
    ) {
        self.mob_entity.set_item_slot(
            &EquipmentSlot::MAIN_HAND,
            pumpkin_data::item_stack::ItemStack::new(1, &pumpkin_data::item::Item::IRON_SWORD),
        );
        self.mob_entity
            .living_entity
            .equipment_drop_chances
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(EquipmentSlot::MAIN_HAND, 0.0);
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        if let Some(pos) = self.get_bound_origin() {
            nbt.put(
                "bound_pos",
                NbtTag::IntArray(vec![pos.0.x, pos.0.y, pos.0.z]),
            );
        }
        if self.has_limited_life.load(Ordering::Relaxed) {
            nbt.put_int(
                "life_ticks",
                self.limited_life_ticks.load(Ordering::Relaxed),
            );
        }
        if let Some(owner) = *self.owner.lock().unwrap_or_else(PoisonError::into_inner) {
            nbt.put_uuid("owner", owner);
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.set_bound_origin(nbt.get_int_array("bound_pos").and_then(|pos| match pos {
            &[x, y, z] => Some(BlockPos::new(x, y, z)),
            _ => None,
        }));
        match nbt.get_int("life_ticks") {
            Some(life_ticks) => self.set_limited_life(life_ticks),
            None => self.has_limited_life.store(false, Ordering::Relaxed),
        }
        *self.owner.lock().unwrap_or_else(PoisonError::into_inner) = nbt.get_uuid("owner");
    }
}

/// Vanilla `Vex.VexMoveControl`: flies straight at the wanted position, ignoring blocks,
/// and brakes once within its own size of it.
#[derive(Default)]
struct VexMoveControl {
    wanted_x: f64,
    wanted_y: f64,
    wanted_z: f64,
    speed_modifier: f64,
    operation: Operation,
}

impl Control for VexMoveControl {}

impl MoveControlTrait for VexMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        if self.operation != Operation::MoveTo {
            return;
        }
        let mob_entity = mob.get_mob_entity();
        let entity = &mob_entity.living_entity.entity;
        let pos = entity.pos.load();
        let delta = Vector3::new(
            self.wanted_x - pos.x,
            self.wanted_y - pos.y,
            self.wanted_z - pos.z,
        );
        let delta_length = delta.length();
        if delta_length < entity.bounding_box.load().get_average_side_length() {
            self.operation = Operation::Wait;
            entity.velocity.store(entity.velocity.load() * 0.5);
            return;
        }
        let velocity = entity.velocity.load() + delta * (self.speed_modifier * 0.05 / delta_length);
        entity.velocity.store(velocity);
        let (x, z) = mob_entity
            .get_target()
            .map_or((velocity.x, velocity.z), |target| {
                let target_pos = target.get_entity().pos.load();
                (target_pos.x - pos.x, target_pos.z - pos.z)
            });
        let yaw = -(math::atan2(x, z) as f32) * (180.0 / std::f32::consts::PI);
        entity.yaw.store(yaw);
        entity.body_yaw.store(yaw);
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

/// Vanilla `Vex.VexChargeAttackGoal`: dashes at the target's eyes and hits it on contact.
struct VexChargeAttackGoal;

fn living_target(mob: &dyn Mob) -> Option<Arc<dyn EntityBase>> {
    mob.get_mob_entity().get_target().filter(|target| {
        target
            .get_living_entity()
            .is_some_and(crate::entity::living::LivingEntity::is_alive)
    })
}

impl Goal for VexChargeAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(target) = living_target(mob) else {
            return false;
        };
        if move_control_has_wanted(mob)
            || mob.get_entity().random().next_bounded_i32(to_goal_ticks(7)) != 0
        {
            return false;
        }
        mob.get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&target.get_entity().pos.load())
            > 4.0
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        move_control_has_wanted(mob)
            && as_vex(mob).is_some_and(VexEntity::is_charging)
            && living_target(mob).is_some()
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(target) = mob.get_mob_entity().get_target() {
            set_wanted_position(mob, target.get_entity().get_eye_pos(), 1.0);
        }
        if let Some(vex) = as_vex(mob) {
            vex.set_is_charging(true);
        }
        let entity = mob.get_entity();
        if !entity.is_silent() {
            entity.world.load().play_sound_fine(
                Sound::EntityVexCharge,
                mob.sound_source(),
                &entity.pos.load(),
                1.0,
                1.0,
            );
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(vex) = as_vex(mob) {
            vex.set_is_charging(false);
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(target) = mob.get_mob_entity().get_target() else {
            return;
        };
        let entity = mob.get_entity();
        let target_entity = target.get_entity();
        if entity
            .bounding_box
            .load()
            .intersects(&target_entity.bounding_box.load())
        {
            let caller: &dyn EntityBase = mob;
            mob.get_mob_entity().try_attack(caller, target.as_ref());
            if let Some(vex) = as_vex(mob) {
                vex.set_is_charging(false);
            }
        } else if entity
            .pos
            .load()
            .squared_distance_to_vec(&target_entity.pos.load())
            < 9.0
        {
            set_wanted_position(mob, target_entity.get_eye_pos(), 1.0);
        }
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `Vex.VexRandomMoveGoal`: drifts to a random air block near its bound origin.
struct VexRandomMoveGoal;

impl Goal for VexRandomMoveGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        !move_control_has_wanted(mob)
            && mob.get_entity().random().next_bounded_i32(to_goal_ticks(7)) == 0
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let entity = mob.get_entity();
        let bound_origin = as_vex(mob)
            .and_then(VexEntity::get_bound_origin)
            .unwrap_or_else(|| entity.block_pos.load());
        let world = entity.world.load();
        for _ in 0..3 {
            let offset = {
                let mut random = entity.random();
                Vector3::new(
                    random.next_bounded_i32(15) - 7,
                    random.next_bounded_i32(11) - 5,
                    random.next_bounded_i32(15) - 7,
                )
            };
            let test_pos = bound_origin.offset(offset);
            if world.get_block_state(&test_pos).is_air() {
                let wanted = Vector3::new(
                    f64::from(test_pos.0.x) + 0.5,
                    f64::from(test_pos.0.y) + 0.5,
                    f64::from(test_pos.0.z) + 0.5,
                );
                set_wanted_position(mob, wanted, 0.25);
                if mob.get_mob_entity().get_target().is_none() {
                    mob.get_mob_entity()
                        .look_control
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .look_at_with_range(wanted.x, wanted.y, wanted.z, 180.0, 20.0);
                }
                break;
            }
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `Vex.VexCopyOwnerTargetGoal`: takes its owner's target, seen or not.
struct VexCopyOwnerTargetGoal {
    track_target_goal: TrackTargetGoal,
    copy_owner_targeting: TargetPredicate,
}

impl VexCopyOwnerTargetGoal {
    fn new() -> Self {
        Self {
            track_target_goal: TrackTargetGoal::with_default(false),
            copy_owner_targeting: TargetPredicate::create_non_attackable()
                .ignore_visibility()
                .ignore_distance_scaling_factor(),
        }
    }

    fn owner_target(mob: &dyn Mob) -> Option<Arc<dyn EntityBase>> {
        as_vex(mob)?
            .get_owner()?
            .get_mob()?
            .get_mob_entity()
            .get_target()
    }
}

impl Goal for VexCopyOwnerTargetGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let target = Self::owner_target(mob);
        self.track_target_goal
            .can_track(mob, target.as_deref(), &self.copy_owner_targeting)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.track_target_goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.set_mob_target(Self::owner_target(mob));
        self.track_target_goal.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.track_target_goal.stop(mob);
    }

    fn controls(&self) -> Controls {
        self.track_target_goal.controls()
    }
}
