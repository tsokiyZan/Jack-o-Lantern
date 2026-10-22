use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::sync::{
    Arc, PoisonError,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use pumpkin_data::data_component_impl::BucketEntityDataImpl;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;

use super::tadpole_ai;
use crate::entity::{
    Entity, EntityBase, ageable,
    ai::brain::{Brain, BrainTick, behavior::utils::is_alive},
    ai::control::look_control::LookControl,
    ai::control::smooth_swimming_move_control::SmoothSwimmingMoveControl,
    breath::BreathManager,
    bucketable,
    mob::{Mob, MobEntity, conversion::convert_single},
    passive::fish,
    player::Player,
};

/// Vanilla `Tadpole.ticksToBeFrog`.
const TICKS_TO_BE_FROG: i32 = 24000;
/// Vanilla `Tadpole.DEFAULT_AGE`.
const DEFAULT_AGE: i32 = 0;

/// Represents a Tadpole, the young of a frog, which grows into one after twenty minutes.
///
/// Wiki: <https://minecraft.wiki/w/Tadpole>
pub struct TadpoleEntity {
    pub mob_entity: MobEntity,
    /// Vanilla `Tadpole.age`, separate from `AgeableMob`'s.
    age: AtomicI32,
    age_lock_particle_timer: AtomicI32,
    age_locked: AtomicBool,
}

impl TadpoleEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `AbstractFish`: water navigation and the fish goals, which tadpoles run
        // alongside their brain.
        fish::init(&mob_entity);
        // Vanilla `Tadpole`'s constructor.
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            Box::new(SmoothSwimmingMoveControl::new(85, 10, 0.02, 0.1, true));
        *mob_entity
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = LookControl::smooth_swimming(10.0);
        let tadpole = Self {
            mob_entity,
            age: AtomicI32::new(DEFAULT_AGE),
            age_lock_particle_timer: AtomicI32::new(0),
            age_locked: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(tadpole);
        mob_arc.mob_entity.init_brain(mob_arc.as_ref());
        mob_arc
    }

    /// Vanilla `Tadpole.isAgeLocked`.
    #[must_use]
    pub fn is_age_locked(&self) -> bool {
        self.age_locked.load(Ordering::Relaxed)
    }

    /// Vanilla `Tadpole.setAgeLocked`.
    fn set_age_locked(&self, locked: bool) {
        self.age_locked.store(locked, Ordering::Relaxed);
        self.get_entity()
            .set_synced_data(tracked_data::tadpole::AGE_LOCKED, locked);
    }

    /// Vanilla `Tadpole.setAgeLockedData`.
    fn set_age_locked_data(&self) {
        self.set_age_locked(!self.is_age_locked());
        self.set_age(0);
        self.age_lock_particle_timer.store(40, Ordering::Relaxed);
    }

    fn get_age(&self) -> i32 {
        self.age.load(Ordering::Relaxed)
    }

    /// Vanilla `Tadpole.ageUp(int)`.
    fn age_up_by(&self, ticks_to_age_up: i32) {
        self.set_age(self.get_age() + ticks_to_age_up * 20);
    }

    /// Vanilla `Tadpole.setAge`.
    fn set_age(&self, new_age: i32) {
        self.age.store(new_age, Ordering::Relaxed);
        if new_age >= TICKS_TO_BE_FROG {
            self.age_up();
        }
    }

    /// Vanilla `Tadpole.ageUp()`: turns into a frog.
    fn age_up(&self) {
        let world = self.get_entity().world.load_full();
        convert_single(self, &EntityType::FROG, false, false, |frog| {
            frog.finalize_spawn(
                &world,
                crate::entity::mob::spawn::EntitySpawnReason::Conversion,
                None,
            );
            frog.get_mob_entity()
                .persistence_required
                .store(true, Ordering::Relaxed);
            // TODO: Entity.fudgePositionAfterSizeChange, which needs
            // CollisionGetter.findFreePosition.
            self.play_sound_at_self(Sound::EntityTadpoleGrowUp, 0.15, 1.0);
        });
    }

    /// Vanilla `Tadpole.getTicksLeftUntilAdult`.
    fn get_ticks_left_until_adult(&self) -> i32 {
        (TICKS_TO_BE_FROG - self.get_age()).max(0)
    }

    /// Vanilla `Tadpole.isFood`.
    fn is_food(item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_FROG_FOOD)
    }

    /// Vanilla `Tadpole.feed`. The happy-villager particle is client-side only.
    fn feed(&self, player: &Player, item_stack: &mut ItemStack) {
        item_stack.decrement_unless_creative(player.gamemode.load(), 1);
        self.age_up_by(ageable::get_speed_up_seconds_when_feeding(
            self.get_ticks_left_until_adult(),
        ));
    }

    /// Vanilla `Tadpole.saveToBucketTag`.
    fn save_to_bucket_tag(&self, bucket: &mut ItemStack) {
        let mut tag = bucketable::save_default_data_to_bucket_tag(self, bucket);
        tag.put_int("Age", self.get_age());
        tag.put_bool("AgeLocked", self.is_age_locked());
        bucket.set_data_component(BucketEntityDataImpl { nbt: Some(tag) });
    }

    /// Vanilla `Bucketable.bucketMobPickup` with `ItemUtils.createFilledResult`.
    fn bucket_mob_pickup(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let entity = self.get_entity();
        if item_stack.item != &Item::WATER_BUCKET || !entity.is_alive() {
            return false;
        }
        let world = entity.world.load();
        if let Some(server) = world.server.upgrade() {
            let mut event =
                crate::plugin::api::events::player::player_bucket_entity::PlayerBucketEntityEvent {
                    player: player.clone(),
                    entity_id: entity.entity_id,
                    bucket_item: "tadpole_bucket".to_string(),
                    cancelled: false,
                };
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return false;
            }
        }
        self.play_sound_at_self(Sound::ItemBucketFillTadpole, 1.0, 1.0);
        let mut bucket = ItemStack::new(1, &Item::TADPOLE_BUCKET);
        self.save_to_bucket_tag(&mut bucket);
        crate::item::create_filled_result(item_stack, player, bucket, false);
        if entity.is_leashed() {
            entity.unleash();
            entity.spawn_at_location(ItemStack::new(1, &Item::LEAD));
        }
        entity.remove();
        true
    }
}

impl Mob for TadpoleEntity {
    /// Vanilla `WaterAnimal.checkSpawnObstruction`: water doesn't obstruct it.
    fn check_spawn_obstruction(&self, world: &crate::world::World) -> bool {
        let entity = self.get_entity();
        world.is_unobstructed(Some(entity), &entity.bounding_box.load())
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `WaterAnimal.handleAirSupply`.
    fn handle_air_supply(&self, pre_tick_air: i32) {
        BreathManager::water_animal_handle_air_supply(self, pre_tick_air);
    }

    /// Vanilla `AbstractFish.aiStep`.
    fn ai_step(&self, _caller: &dyn EntityBase) {
        fish::flop(self, Sound::EntityTadpoleFlop);
    }

    /// The rest of vanilla `Tadpole.aiStep`, after `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        if !self.is_age_locked() {
            self.set_age(self.get_age() + 1);
        }
        ageable::make_age_locked_particle(
            self.get_entity(),
            &self.age_lock_particle_timer,
            self.is_age_locked(),
        );
    }

    /// Vanilla `AbstractFish.travelInWater`.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        fish::travel(self, caller)
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }

    /// Vanilla `AbstractFish.requiresCustomPersistence`; tadpoles always count as from
    /// a bucket.
    fn requires_custom_persistence(&self) -> bool {
        true
    }

    /// Vanilla `AbstractFish.removeWhenFarAway`.
    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        false
    }

    /// Vanilla `Tadpole.shouldDropExperience`.
    fn get_base_experience_reward(&self) -> u32 {
        0
    }

    fn make_brain(&self, packed: &crate::entity::ai::brain::memory::PackedMemories) -> Brain {
        tadpole_ai::TADPOLE_PROVIDER.make_brain(self, packed)
    }

    /// Vanilla `Tadpole.customServerAiStep`'s `TadpoleAi.updateActivity`.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        tadpole_ai::update_activity(tick);
    }

    /// Vanilla `Tadpole.customServerAiStep`.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        // Vanilla `AbstractFish.addAdditionalSaveData` writes `fromBucket()`, always true.
        nbt.put_bool("FromBucket", true);
        nbt.put_int("Age", self.get_age());
        nbt.put_bool("AgeLocked", self.is_age_locked());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.set_age(nbt.get_int("Age").unwrap_or(DEFAULT_AGE));
        self.set_age_locked(nbt.get_bool("AgeLocked").unwrap_or(false));
    }

    /// Vanilla `Tadpole.loadFromBucketTag`.
    fn load_from_bucket_tag(&self, tag: &NbtCompound) {
        bucketable::load_default_data_from_bucket_tag(self, tag);
        if let Some(age) = tag.get_int("Age") {
            self.set_age(age);
        }
        self.set_age_locked(tag.get_bool("AgeLocked").unwrap_or(false));
    }

    fn mob_init_data_tracker(&self) {
        self.get_entity()
            .set_synced_data(tracked_data::tadpole::AGE_LOCKED, self.is_age_locked());
    }

    /// Vanilla `Tadpole.mobInteract`.
    fn mob_interact(
        &self,
        player: &Arc<Player>,
        _hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        if Self::is_food(item_stack) && !self.is_age_locked() {
            self.feed(player, item_stack);
            return InteractionResult::SUCCESS;
        }
        if ageable::can_use_golden_dandelion(
            item_stack,
            true,
            self.age_lock_particle_timer.load(Ordering::Relaxed),
            self,
        ) {
            ageable::set_age_locked_by_player(
                self,
                || self.is_age_locked(),
                player,
                item_stack,
                || self.set_age_locked_data(),
            );
            return InteractionResult::SUCCESS;
        }
        // `bucketMobPickup` answers `SUCCESS`, else `super.mobInteract` passes.
        if self.bucket_mob_pickup(player, item_stack) {
            InteractionResult::SUCCESS
        } else {
            InteractionResult::PASS
        }
    }
}
