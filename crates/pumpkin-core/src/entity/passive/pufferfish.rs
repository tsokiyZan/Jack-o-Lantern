use crate::entity::breath::BreathManager;
use pumpkin_util::random::RandomImpl;
use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicI32, Ordering};

use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag;
use pumpkin_data::tag::Taggable;
use pumpkin_data::tracked_data::pufferfish::PUFF_STATE;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::VarInt;
use pumpkin_protocol::java::client::play::{CGameEvent, GameEvent};
use pumpkin_util::math::boundingbox::EntityDimensions;

use crate::entity::{
    Entity, EntityBase,
    ai::goal::Goal,
    mob::{Mob, MobEntity},
    passive::fish,
    player::Player,
};

pub const STATE_SMALL: i32 = 0;
pub const STATE_MID: i32 = 1;
pub const STATE_FULL: i32 = 2;

/// Represents a Pufferfish, a passive aquatic mob that can inflate when threatened.
///
/// Wiki: <https://minecraft.wiki/w/Pufferfish>
pub struct PufferfishEntity {
    pub mob_entity: MobEntity,
    puff_state: AtomicI32,
    inflate_counter: AtomicI32,
    deflate_timer: AtomicI32,
    pub from_bucket: fish::FromBucket,
}

impl PufferfishEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        fish::init(&mob_entity);
        mob_entity
            .goals_selector
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .add_goal(1, Box::new(PufferfishPuffGoal));
        let pufferfish = Self {
            mob_entity,
            puff_state: AtomicI32::new(STATE_SMALL),
            inflate_counter: AtomicI32::new(0),
            deflate_timer: AtomicI32::new(0),
            from_bucket: fish::FromBucket::default(),
        };
        pufferfish.refresh_dimensions();
        Arc::new(pufferfish)
    }

    #[must_use]
    pub fn get_puff_state(&self) -> i32 {
        self.puff_state.load(Ordering::Relaxed)
    }

    pub fn set_puff_state(&self, state: i32) {
        self.puff_state.store(state, Ordering::Relaxed);
        self.mob_entity
            .living_entity
            .entity
            .set_synced_data(PUFF_STATE, VarInt(state));
        self.refresh_dimensions();
    }

    const fn get_scale(state: i32) -> f32 {
        match state {
            STATE_SMALL => 0.5,
            STATE_MID => 0.7,
            _ => 1.0,
        }
    }

    /// Vanilla's `TARGETING_CONDITIONS` with the `SCARY_MOB` selector.
    fn is_scary(&self, target: &dyn EntityBase) -> bool {
        let target_entity = target.get_entity();
        if target_entity.entity_id == self.mob_entity.living_entity.entity.entity_id
            || !target_entity.is_alive()
            || target.get_living_entity().is_none()
        {
            return false;
        }
        if let Some(player) = target.get_player()
            && (player.is_spectator() || player.is_creative())
        {
            return false;
        }
        !target_entity
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_NOT_SCARY_FOR_PUFFERFISH)
    }

    /// Scary living entities whose box touches this fish's box grown by `inflate`.
    fn scary_entities(&self, inflate: f64) -> Vec<Arc<dyn EntityBase>> {
        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load();
        let search_box = entity.bounding_box.load().expand_all(inflate);
        let mut scary: Vec<Arc<dyn EntityBase>> = world.get_entities_at_box(&search_box);
        scary.extend(
            world
                .get_players_at_box(&search_box)
                .into_iter()
                .map(|player| player as Arc<dyn EntityBase>),
        );
        scary.retain(|target| self.is_scary(target.as_ref()));
        scary
    }

    /// The puff part of vanilla's `Pufferfish.tick`.
    fn tick_puff_state(&self) {
        let entity = &self.mob_entity.living_entity.entity;
        if !entity.is_alive() || self.mob_entity.is_no_ai() {
            return;
        }
        let state = self.get_puff_state();
        let inflate_counter = self.inflate_counter.load(Ordering::Relaxed);
        if inflate_counter > 0 {
            if state == STATE_SMALL {
                entity.play_sound(Sound::EntityPufferFishBlowUp);
                self.set_puff_state(STATE_MID);
            } else if inflate_counter > 40 && state == STATE_MID {
                entity.play_sound(Sound::EntityPufferFishBlowUp);
                self.set_puff_state(STATE_FULL);
            }
            self.inflate_counter.fetch_add(1, Ordering::Relaxed);
        } else if state != STATE_SMALL {
            let deflate_timer = self.deflate_timer.load(Ordering::Relaxed);
            if deflate_timer > 60 && state == STATE_FULL {
                entity.play_sound(Sound::EntityPufferFishBlowOut);
                self.set_puff_state(STATE_MID);
            } else if deflate_timer > 100 && state == STATE_MID {
                entity.play_sound(Sound::EntityPufferFishBlowOut);
                self.set_puff_state(STATE_SMALL);
            }
            self.deflate_timer.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The sting part of vanilla's `Pufferfish.aiStep`.
    fn sting_nearby_mobs(&self, caller: &dyn EntityBase) {
        let entity = &self.mob_entity.living_entity.entity;
        let puff_state = self.get_puff_state();
        if !entity.is_alive() || puff_state <= STATE_SMALL {
            return;
        }
        for target in self.scary_entities(0.3) {
            if target.get_mob().is_some() {
                self.touch(caller, target.as_ref(), puff_state);
            }
        }
    }

    fn touch(&self, caller: &dyn EntityBase, mob: &dyn EntityBase, puff_state: i32) {
        if mob.damage_with_context(
            mob,
            (1 + puff_state) as f32,
            DamageType::MOB_ATTACK,
            None,
            Some(caller),
            Some(caller),
        ) {
            if let Some(living) = mob.get_living_entity() {
                living.add_effect(poison(puff_state));
            }
            self.mob_entity
                .living_entity
                .entity
                .play_sound(Sound::EntityPufferFishSting);
        }
    }
}

const fn poison(puff_state: i32) -> pumpkin_data::potion::Effect {
    pumpkin_data::potion::Effect {
        effect_type: &StatusEffect::POISON,
        duration: 60 * puff_state,
        amplifier: 0,
        ambient: false,
        show_particles: true,
        show_icon: true,
        blend: true,
    }
}

impl Mob for PufferfishEntity {
    /// Vanilla `WaterAnimal.checkSpawnObstruction`: water doesn't obstruct it.
    fn check_spawn_obstruction(&self, world: &crate::world::World) -> bool {
        let entity = self.get_entity();
        world.is_unobstructed(Some(entity), &entity.bounding_box.load())
    }

    /// Vanilla `Pufferfish.getDefaultDimensions`: the puff state scales the fish.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        self.get_entity()
            .get_dimensions(pose)
            .scale(self.age_scale())
            .scale(Self::get_scale(self.get_puff_state()))
    }

    fn set_from_bucket(&self, from_bucket: bool) {
        self.from_bucket.set(self, from_bucket);
    }

    fn requires_custom_persistence(&self) -> bool {
        self.from_bucket.requires_custom_persistence(self)
    }

    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        self.from_bucket.remove_when_far_away(self)
    }

    /// Vanilla `AbstractFish.loadFromBucketTag`.
    fn load_from_bucket_tag(&self, tag: &pumpkin_nbt::compound::NbtCompound) {
        crate::entity::bucketable::load_default_data_from_bucket_tag(self, tag);
    }

    fn mob_interact(
        &self,
        player: &Arc<crate::entity::player::Player>,
        _hand: pumpkin_util::Hand,
        item_stack: &mut pumpkin_data::item_stack::ItemStack,
    ) -> crate::item::interaction_result::InteractionResult {
        crate::entity::bucketable::fish_mob_interact(
            self,
            player,
            item_stack,
            &pumpkin_data::item::Item::PUFFERFISH_BUCKET,
        )
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `WaterAnimal.getBaseExperienceReward`.
    fn get_base_experience_reward(&self) -> u32 {
        1 + self.get_entity().random().next_bounded_i32(3) as u32
    }

    /// Vanilla `WaterAnimal.handleAirSupply`.
    fn handle_air_supply(&self, pre_tick_air: i32) {
        BreathManager::water_animal_handle_air_supply(self, pre_tick_air);
    }

    fn mob_tick(&self, caller: &dyn EntityBase) {
        self.tick_puff_state();
        self.sting_nearby_mobs(caller);
    }

    /// Vanilla `AbstractFish.aiStep`.
    fn ai_step(&self, _caller: &dyn EntityBase) {
        fish::flop(self, Sound::EntityPufferFishFlop);
    }

    /// Vanilla's `Pufferfish.playerTouch`.
    fn mob_player_collision(&self, player: &Arc<Player>) {
        let puff_state = self.get_puff_state();
        if puff_state > STATE_SMALL
            && player.damage_with_context(
                player.as_ref(),
                (1 + puff_state) as f32,
                DamageType::MOB_ATTACK,
                None,
                Some(self),
                Some(self),
            )
        {
            player.try_send_client_packet(&CGameEvent::new(
                GameEvent::PlayPufferfishStringSound,
                0.0,
            ));
            player.living_entity.add_effect(poison(puff_state));
        }
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.from_bucket.write_nbt(nbt);
        nbt.put_int("PuffState", self.get_puff_state());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.from_bucket.read_nbt(self, nbt);
        self.set_puff_state(
            nbt.get_int("PuffState")
                .unwrap_or(STATE_SMALL)
                .min(STATE_FULL),
        );
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        fish::travel(self, caller)
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }
}

/// Vanilla's `Pufferfish.PufferfishPuffGoal`: inflate while anything scary is
/// within two blocks.
struct PufferfishPuffGoal;

impl Goal for PufferfishPuffGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.cast_any()
            .downcast_ref::<PufferfishEntity>()
            .is_some_and(|fish| !fish.scary_entities(2.0).is_empty())
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(fish) = mob.cast_any().downcast_ref::<PufferfishEntity>() {
            fish.inflate_counter.store(1, Ordering::Relaxed);
            fish.deflate_timer.store(0, Ordering::Relaxed);
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(fish) = mob.cast_any().downcast_ref::<PufferfishEntity>() {
            fish.inflate_counter.store(0, Ordering::Relaxed);
        }
    }
}
