//! Vanilla `Bucketable`'s shared helpers, for mobs that can be carried in a bucket.

use std::sync::atomic::Ordering::Relaxed;

use pumpkin_data::data_component_impl::CustomNameImpl;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::mob::Mob;
use crate::entity::player::Player;

/// Vanilla `Bucketable.saveDefaultDataToBucketTag`: copies the custom name onto `bucket`
/// and returns the start of its `bucket_entity_data`, for the mob to add its own keys.
pub fn save_default_data_to_bucket_tag(mob: &dyn Mob, bucket: &mut ItemStack) -> NbtCompound {
    let mob_entity = mob.get_mob_entity();
    let entity = mob.get_entity();
    if let Some(name) = &**entity.custom_name.load() {
        bucket.set_data_component(CustomNameImpl { name: name.clone() });
    }
    let mut tag = NbtCompound::new();
    if mob_entity.is_no_ai() {
        tag.put_bool("NoAI", true);
    }
    if entity.is_silent() {
        tag.put_bool("Silent", true);
    }
    if entity.has_no_gravity() {
        tag.put_bool("NoGravity", true);
    }
    if entity.glowing.load(Relaxed) {
        tag.put_bool("Glowing", true);
    }
    if entity.invulnerable.load(Relaxed) {
        tag.put_bool("Invulnerable", true);
    }
    if mob_entity.persistence_required.load(Relaxed) {
        tag.put_bool("PersistenceRequired", true);
    }
    tag.put_float("Health", mob_entity.living_entity.health.load());
    tag
}

/// Vanilla `Bucketable.loadDefaultDataFromBucketTag`.
pub fn load_default_data_from_bucket_tag(mob: &dyn Mob, tag: &NbtCompound) {
    let mob_entity = mob.get_mob_entity();
    let entity = mob.get_entity();
    if let Some(no_ai) = tag.get_bool("NoAI") {
        mob_entity.set_no_ai(no_ai);
    }
    if let Some(silent) = tag.get_bool("Silent") {
        entity.set_silent(silent);
    }
    if let Some(no_gravity) = tag.get_bool("NoGravity") {
        entity.set_has_no_gravity(no_gravity);
    }
    if let Some(glowing) = tag.get_bool("Glowing") {
        entity.set_glowing(glowing);
    }
    if let Some(invulnerable) = tag.get_bool("Invulnerable") {
        entity.set_invulnerable(invulnerable);
    }
    if tag.get_bool("PersistenceRequired") == Some(true) {
        mob_entity.persistence_required.store(true, Relaxed);
    }
    if let Some(health) = tag.get_float("Health") {
        mob_entity.living_entity.set_health(health);
    }
}

/// Vanilla `ItemUtils.createFilledResult` without the creative stack limit, as
/// `Bucketable.bucketMobPickup` calls it: `stack` becomes `filled`, or `filled` goes to
/// the inventory or the ground.
pub fn create_filled_result(stack: &mut ItemStack, player: &Player, mut filled: ItemStack) {
    stack.decrement_unless_creative(player.gamemode.load(), 1);
    if stack.is_empty() {
        *stack = filled;
    } else if !player.inventory.insert_stack_anywhere(&mut filled) {
        player.drop_item(filled);
    }
}
