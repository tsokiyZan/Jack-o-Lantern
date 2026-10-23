//! Vanilla `Bucketable`'s shared helpers, for mobs that can be carried in a bucket.

use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use pumpkin_data::data_component_impl::{BucketEntityDataImpl, CustomNameImpl};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::mob::Mob;
use crate::entity::player::Player;
use crate::item::interaction_result::InteractionResult;

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

/// Vanilla `Bucketable.bucketMobPickup` for a mob picked up with a water bucket.
///
/// The plugin event (whose cancellation leaves the mob be), the pickup sound, a `bucket_item`
/// filled by `save_to_bucket_tag` replacing the used bucket (`createFilledResult`), the lead
/// dropped and the mob gone. Returns whether it picked the mob up, `Optional.of(SUCCESS)`.
pub fn bucket_mob_pickup(
    mob: &dyn Mob,
    player: &Arc<Player>,
    item_stack: &mut ItemStack,
    bucket_item: &'static Item,
    pickup_sound: Sound,
    save_to_bucket_tag: impl FnOnce(&mut ItemStack),
) -> bool {
    let entity = mob.get_entity();
    if item_stack.item != &Item::WATER_BUCKET || !entity.is_alive() {
        return false;
    }
    let world = entity.world.load();
    if let Some(server) = world.server.upgrade() {
        let mut event =
            crate::plugin::api::events::player::player_bucket_entity::PlayerBucketEntityEvent {
                player: player.clone(),
                entity_id: entity.entity_id,
                bucket_item: bucket_item.registry_key.to_string(),
                cancelled: false,
            };
        server.plugin_manager.fire_blocking(&server, &mut event);
        if event.cancelled {
            return false;
        }
    }
    mob.play_sound_at_self(pickup_sound, 1.0, 1.0);
    let mut bucket = ItemStack::new(1, bucket_item);
    save_to_bucket_tag(&mut bucket);
    crate::item::create_filled_result(item_stack, player, bucket, false);
    // `leashable.dropLeash()`.
    crate::entity::leashable::drop_leash(mob, true, true);
    entity.remove();
    true
}

/// Vanilla `AbstractFish.saveToBucketTag`: only `saveDefaultDataToBucketTag`.
pub fn save_fish_to_bucket_tag(mob: &dyn Mob, bucket: &mut ItemStack) {
    let tag = save_default_data_to_bucket_tag(mob, bucket);
    bucket.set_data_component(BucketEntityDataImpl { nbt: Some(tag) });
}

/// Vanilla `AbstractFish.mobInteract`: `bucketMobPickup`, else `Mob.mobInteract`'s pass.
/// `save_to_bucket_tag` is the fish's `saveToBucketTag` (`save_fish_to_bucket_tag` unless the
/// fish adds components).
pub fn fish_mob_interact(
    mob: &dyn Mob,
    player: &Arc<Player>,
    item_stack: &mut ItemStack,
    bucket_item: &'static Item,
    save_to_bucket_tag: impl FnOnce(&mut ItemStack),
) -> InteractionResult {
    if bucket_mob_pickup(
        mob,
        player,
        item_stack,
        bucket_item,
        Sound::ItemBucketFillFish,
        save_to_bucket_tag,
    ) {
        InteractionResult::SUCCESS
    } else {
        InteractionResult::PASS
    }
}
