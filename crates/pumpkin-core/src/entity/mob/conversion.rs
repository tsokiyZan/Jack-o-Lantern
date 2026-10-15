//! Vanilla `Mob.convertTo` with `ConversionType.SINGLE`: one mob turning into another.

use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use pumpkin_data::entity::EntityType;
use pumpkin_data::item_stack::ItemStack;
use uuid::Uuid;

use crate::entity::EntityBase;
use crate::entity::mob::{Mob, convert_common};

/// Vanilla `ConversionParams`, without the scoreboard team, which Pumpkin does not carry over.
#[derive(Clone, Copy)]
pub struct ConversionParams {
    pub keep_equipment: bool,
    pub preserve_can_pick_up_loot: bool,
}

/// Vanilla `Mob.convertTo` for a `ConversionType.SINGLE` conversion: builds the new mob,
/// moves everything over, lets `after_conversion` finish it, adds it to the world and
/// discards `from`.
pub fn convert_to(
    from: &dyn Mob,
    entity_type: &'static EntityType,
    params: ConversionParams,
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
    convert_single(from, to, params);
    after_conversion(to);
    world.spawn_entity(converted.clone());
    old_entity.remove();
    Some(converted)
}

/// Vanilla `ConversionType.SINGLE.convert`.
fn convert_single(from: &dyn Mob, to: &dyn Mob, params: ConversionParams) {
    let (old, new) = (from.get_mob_entity(), to.get_mob_entity());
    let (old_living, new_living) = (&old.living_entity, &new.living_entity);
    let (old_entity, new_entity) = (&old_living.entity, &new_living.entity);

    let root_passenger = old_entity.get_first_passenger();
    // `copyPosition`.
    new_entity.set_pos(old_entity.pos.load());
    new_entity.set_rotation(old_entity.yaw.load(), old_entity.pitch.load());
    new_entity.velocity.store(old_entity.velocity.load());
    if let Some(rider) = root_passenger {
        old_entity.remove_passenger(rider.get_entity().entity_id);
        new.add_pending_rider(rider);
    }
    if let Some(vehicle) = old_entity.get_vehicle() {
        vehicle.get_entity().remove_passenger(old_entity.entity_id);
        new.set_pending_vehicle(vehicle);
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
        .fall_distance
        .store(old_living.fall_distance.load());
    new_entity.set_fall_flying(old_entity.is_fall_flying());
    new_living
        .last_hurt_by_player_id
        .store(old_living.last_hurt_by_player_id.load(Relaxed), Relaxed);
    new_living
        .last_hurt_by_player_time
        .store(old_living.last_hurt_by_player_time.load(Relaxed), Relaxed);
    new_living
        .hurt_time
        .store(old_living.hurt_time.load(Relaxed), Relaxed);
    new_entity.body_yaw.store(old_entity.body_yaw.load());
    new_entity
        .on_ground
        .store(old_entity.on_ground.load(Relaxed), Relaxed);
    let leash_holder = old_entity
        .leashed_to
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(holder) = leash_holder {
        new_entity.leash_to(holder);
    }

    convert_common(from, to, params);
}
