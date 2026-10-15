//! Vanilla `ConversionTracker` and `Mob.convertTo` with `ConversionType.SINGLE`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};

use pumpkin_data::entity::EntityType;
use pumpkin_data::tracked_data::TrackedData;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::EntityBase;
use crate::entity::mob::{Mob, convert_common};

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

/// Vanilla `Mob.convertTo` with `ConversionParams.single`: replaces `from` with a new
/// `to_type` mob in the same spot. `after_conversion` sees the new mob before it is added
/// to the world.
pub fn convert_single(
    from: &dyn Mob,
    to_type: &'static EntityType,
    keep_equipment: bool,
    preserve_can_pick_up_loot: bool,
    after_conversion: impl FnOnce(&dyn Mob),
) -> Option<Arc<dyn EntityBase>> {
    let from_mob = from.get_mob_entity();
    let from_living = &from_mob.living_entity;
    let from_entity = &from_living.entity;
    if from_entity.is_removed() {
        return None;
    }
    let world = from_entity.world.load_full();
    let converted = crate::entity::r#type::from_type(
        to_type,
        from_entity.pos.load(),
        &world,
        uuid::Uuid::new_v4(),
    );
    let to = converted.get_mob()?;
    let to_mob = to.get_mob_entity();
    let to_living = &to_mob.living_entity;
    let to_entity = &to_living.entity;

    // `ConversionType.SINGLE.convert`.
    let root_passenger = from_entity.get_first_passenger();
    to_entity.set_rotation(from_entity.yaw.load(), from_entity.pitch.load());
    to_entity.velocity.store(from_entity.velocity.load());
    if let Some(passenger) = &root_passenger {
        from_entity.remove_passenger(passenger.get_entity().entity_id);
    }
    let vehicle = from_entity.get_vehicle();
    if let Some(vehicle) = &vehicle {
        vehicle.get_entity().remove_passenger(from_entity.entity_id);
    }
    if keep_equipment {
        // `copyAndClear` empties the old mob's slots.
        let equipment = std::mem::take(
            &mut from_living
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .equipment,
        );
        for (slot, stack) in equipment {
            if stack.is_empty() {
                continue;
            }
            let drop_chance = from_mob.drop_chance(&slot);
            to_mob.set_item_slot(&slot, stack);
            to_living
                .equipment_drop_chances
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(slot.clone(), drop_chance);
            to.on_equip_item(&slot);
        }
    }
    to_living
        .fall_distance
        .store(from_living.fall_distance.load());
    to_living
        .last_hurt_by_player_id
        .store(from_living.last_hurt_by_player_id.load(Relaxed), Relaxed);
    to_living
        .last_hurt_by_player_time
        .store(from_living.last_hurt_by_player_time.load(Relaxed), Relaxed);
    to_living
        .hurt_time
        .store(from_living.hurt_time.load(Relaxed), Relaxed);
    to_entity.body_yaw.store(from_entity.body_yaw.load());
    to_entity
        .on_ground
        .store(from_entity.on_ground.load(Relaxed), Relaxed);
    let leash_holder = from_entity
        .leashed_to
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(holder) = leash_holder {
        to_entity.leash_to(holder);
    }
    convert_common(from, to);
    if preserve_can_pick_up_loot {
        to_mob.set_can_pick_up_loot(from_mob.can_pick_up_loot());
    }

    after_conversion(to);
    world.spawn_entity(converted.clone());
    if let Some(vehicle) = vehicle {
        vehicle
            .get_entity()
            .add_passenger(vehicle.clone(), converted.clone());
    }
    if let Some(passenger) = root_passenger {
        to_entity.add_passenger(converted.clone(), passenger);
    }
    from_entity.remove();
    Some(converted)
}
