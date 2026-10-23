//! Vanilla `ConversionTracker` and `Mob.convertTo` with `ConversionType.SINGLE`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};

use pumpkin_data::entity::EntityType;
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::EntityBase;
use crate::entity::mob::{Mob, convert_common};
use pumpkin_data::item_stack::ItemStack;
use uuid::Uuid;

/// Vanilla `ConversionTracker`: a mob that stays afflicted long enough starts converting,
/// and converts once the conversion time runs out.
pub struct ConversionTracker {
    conversion_id: TrackedData,
    affliction_time_tag: &'static str,
    total_affliction_time: i32,
    conversion_time_tag: &'static str,
    total_conversion_time: i32,
    affliction_time: AtomicI32,
    conversion_time: AtomicI32,
    /// The synced `conversion_id` flag.
    converting: AtomicBool,
}

impl ConversionTracker {
    #[must_use]
    pub const fn new(
        conversion_id: TrackedData,
        affliction_time_tag: &'static str,
        total_affliction_time: i32,
        conversion_time_tag: &'static str,
        total_conversion_time: i32,
    ) -> Self {
        Self {
            conversion_id,
            affliction_time_tag,
            total_affliction_time,
            conversion_time_tag,
            total_conversion_time,
            affliction_time: AtomicI32::new(0),
            conversion_time: AtomicI32::new(0),
            converting: AtomicBool::new(false),
        }
    }

    /// Vanilla `tick`. `afflicted` is the tracker's `isAfflicted`; `do_conversion` runs once
    /// the conversion time is up.
    pub fn tick(&self, mob: &dyn Mob, afflicted: bool, do_conversion: impl FnOnce()) {
        let mob_entity = mob.get_mob_entity();
        if !mob_entity.living_entity.is_alive() || mob_entity.is_no_ai() {
            return;
        }
        if !afflicted {
            self.affliction_time.store(-1, Relaxed);
            self.set_converting(mob, false);
        } else if self.is_converting() {
            if self.conversion_time.fetch_sub(1, Relaxed) - 1 < 0 {
                do_conversion();
            }
        } else if self.affliction_time.fetch_add(1, Relaxed) + 1 >= self.total_affliction_time {
            self.start_conversion(mob, self.total_conversion_time);
        }
    }

    #[must_use]
    pub fn is_converting(&self) -> bool {
        self.converting.load(Relaxed)
    }

    fn set_converting(&self, mob: &dyn Mob, converting: bool) {
        self.converting.store(converting, Relaxed);
        mob.get_entity()
            .set_synced_data(self.conversion_id, converting);
    }

    pub fn start_conversion(&self, mob: &dyn Mob, time: i32) {
        self.conversion_time.store(time, Relaxed);
        self.set_converting(mob, true);
    }

    pub fn write_nbt(&self, nbt: &mut NbtCompound, afflicted: bool) {
        nbt.put_int(
            self.conversion_time_tag,
            if self.is_converting() {
                self.conversion_time.load(Relaxed)
            } else {
                -1
            },
        );
        nbt.put_int(
            self.affliction_time_tag,
            if afflicted {
                self.affliction_time.load(Relaxed)
            } else {
                -1
            },
        );
    }

    pub fn read_nbt(&self, mob: &dyn Mob, nbt: &NbtCompound) {
        self.affliction_time
            .store(nbt.get_int(self.affliction_time_tag).unwrap_or(0), Relaxed);
        match nbt.get_int(self.conversion_time_tag).unwrap_or(-1) {
            -1 => self.set_converting(mob, false),
            time => self.start_conversion(mob, time),
        }
    }
}

/// Vanilla `ConversionParams`, without the type: `convert_to` is always `SINGLE`, and the cube
/// split runs `SPLIT_ON_DEATH` itself. `team` is the team's name.
#[derive(Clone)]
pub struct ConversionParams {
    pub keep_equipment: bool,
    pub preserve_can_pick_up_loot: bool,
    pub team: Option<String>,
}

impl ConversionParams {
    /// Vanilla `ConversionParams.single`: the team is `mob.getTeam()`, read before anything
    /// changes.
    #[must_use]
    pub fn single(mob: &dyn Mob, keep_equipment: bool, preserve_can_pick_up_loot: bool) -> Self {
        Self {
            keep_equipment,
            preserve_can_pick_up_loot,
            team: mob.get_team_name(),
        }
    }
}

/// Vanilla `Mob.convertTo` for a `ConversionType.SINGLE` conversion: builds the new mob,
/// moves everything over, lets `after_conversion` finish it, adds it to the world and
/// discards `from`.
pub fn convert_to(
    from: &dyn Mob,
    entity_type: &'static EntityType,
    params: &ConversionParams,
    after_conversion: impl FnOnce(&dyn Mob),
) -> Option<Arc<dyn EntityBase>> {
    let old_entity = from.get_entity();
    if old_entity.is_removed() {
        return None;
    }
    let world = old_entity.world.load_full();
    let converted = crate::entity::r#type::from_type(
        entity_type,
        old_entity.pos.load(),
        &world,
        Uuid::new_v4(),
    );
    let to = converted.get_mob()?;
    convert_single_type(from, &converted, to, params);
    after_conversion(to);
    world.spawn_entity(converted.clone());
    old_entity.remove();
    Some(converted)
}

/// Vanilla `ConversionType.SINGLE.convert`.
fn convert_single_type(
    from: &dyn Mob,
    converted: &Arc<dyn EntityBase>,
    to: &dyn Mob,
    params: &ConversionParams,
) {
    let (old, new) = (from.get_mob_entity(), to.get_mob_entity());
    let (old_living, new_living) = (&old.living_entity, &new.living_entity);
    let (old_entity, new_entity) = (&old_living.entity, &new_living.entity);

    let root_passenger = old_entity.get_first_passenger();
    // `copyPosition`.
    new_entity.set_pos(old_entity.pos.load());
    new_entity.set_rotation(old_entity.yaw.load(), old_entity.pitch.load());
    new_entity.velocity.store(old_entity.velocity.load());
    if let Some(rider) = root_passenger {
        // `rootPassenger.stopRiding()`, then `boardingCooldown = 0`, which undoes the 60 ticks
        // the dismount set.
        old_entity.remove_passenger(rider.get_entity().entity_id);
        rider.get_entity().riding_cooldown.store(0, Relaxed);
        // `to` is brand new, so it normally carries nobody.
        let to_passengers = new_entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for passenger in to_passengers {
            new_entity.remove_passenger(passenger.get_entity().entity_id);
            passenger.get_entity().remove();
        }
        // `rootPassenger.startRiding(to)`, before `to` is in the world: `canRide` (not
        // sneaking, no boarding cooldown) and `canAddPassenger` (no passenger yet), with the
        // game event. A riding player gets `startRiding`'s packets now, and `to`'s spawn
        // pairing then carries its passenger list, as `addFreshEntity` after the mount does.
        let rider_entity = rider.get_entity();
        if !rider_entity.is_removed()
            && !rider_entity.is_sneaking()
            && rider_entity.riding_cooldown.load(Relaxed) <= 0
            && new_entity
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        {
            new_entity.add_passenger(converted.clone(), rider);
        }
    }
    if let Some(vehicle) = old_entity.get_vehicle() {
        vehicle.get_entity().remove_passenger(old_entity.entity_id);
        // Vanilla mounts `to` here and the `setLeashedTo` below dismounts it again; Pumpkin
        // mounts at spawn, after the leash, so a leashed mob skips the mount instead.
        if crate::entity::leashable::leash_holder(old_entity).is_none() {
            new.set_pending_vehicle(vehicle);
        }
    }

    if params.keep_equipment {
        let mut old_equipment = old_living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let old_chances = old_living
            .equipment_drop_chances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut new_equipment = new_living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut new_chances = new_living
            .equipment_drop_chances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (slot, stack) in &mut old_equipment.equipment {
            if stack.is_empty() {
                continue;
            }
            // `copyAndClear`.
            new_equipment.put(slot, std::mem::replace(stack, ItemStack::EMPTY.clone()));
            match old_chances.get(slot) {
                Some(chance) => new_chances.insert(slot.clone(), *chance),
                None => new_chances.remove(slot),
            };
        }
    }

    new_living
        .entity
        .fall_distance
        .store(old_living.entity.fall_distance.load());
    // Vanilla `setSharedFlag(7, ...)`, not `set_fall_flying`, which asserts a player's
    // glide state actually changes.
    let fall_flying = old_entity.is_fall_flying();
    new_entity.fall_flying.store(fall_flying, Relaxed);
    new_entity.set_flag(crate::entity::Flag::FallFlying, fall_flying);
    // Vanilla copies only the memory time, not the player.
    new_living.last_hurt_by_player_memory_time.store(
        old_living.last_hurt_by_player_memory_time.load(Relaxed),
        Relaxed,
    );
    new_living
        .hurt_time
        .store(old_living.hurt_time.load(Relaxed), Relaxed);
    new_entity.body_yaw.store(old_entity.body_yaw.load());
    new_entity
        .on_ground
        .store(old_entity.on_ground.load(Relaxed), Relaxed);
    // Only the position: vanilla sets no pose here.
    if let Some(pos) = old_living.get_sleeping_pos() {
        new_living.set_sleeping_pos(pos);
    }
    if let Some(holder) = crate::entity::leashable::leash_holder(old_entity) {
        crate::entity::leashable::set_leashed_to(to, holder, true);
    }

    convert_common(from, to, params);
}

/// `convert_to` with `ConversionParams.single(keep_equipment, preserve_can_pick_up_loot)`.
pub fn convert_single(
    from: &dyn Mob,
    to_type: &'static EntityType,
    keep_equipment: bool,
    preserve_can_pick_up_loot: bool,
    after_conversion: impl FnOnce(&dyn Mob),
) -> Option<Arc<dyn EntityBase>> {
    convert_to(
        from,
        to_type,
        &ConversionParams::single(from, keep_equipment, preserve_can_pick_up_loot),
        after_conversion,
    )
}
