use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering, Ordering::Relaxed},
};

use crate::entity::ai::pathfinder::node::PathType;
use pumpkin_data::chicken_sound_variant::ChickenSoundVariant;
use pumpkin_data::chicken_variant::ChickenVariant;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::{entity::EntityType, item::Item};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::random::RandomImpl;

use crate::entity::custom_sound::CustomSound;
use crate::entity::{
    Entity, EntityBase,
    ageable::AgeableMob,
    ai::goal::{
        breed::BreedGoal, escape_danger::EscapeDangerGoal, follow_parent::FollowParentGoal,
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal,
        tempt::TemptGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

const TEMPT_ITEMS: &[&Item] = &[
    &Item::WHEAT_SEEDS,
    &Item::MELON_SEEDS,
    &Item::PUMPKIN_SEEDS,
    &Item::BEETROOT_SEEDS,
    &Item::TORCHFLOWER_SEEDS,
    &Item::PITCHER_POD,
];

/// Experience a chicken jockey is worth.
const CHICKEN_JOCKEY_EXPERIENCE: u32 = 10;

/// Represents a Chicken, a passive mob that lays eggs and is immune to fall damage.
///
/// Wiki: <https://minecraft.wiki/w/Chicken>
pub struct ChickenEntity {
    pub mob_entity: MobEntity,
    pub variant: AtomicU8,
    pub sound_variant: AtomicU8,
    egg_lay_time: AtomicI32,
    /// Vanilla `isChickenJockey`: carried a baby zombie from its spawn.
    is_chicken_jockey: AtomicBool,
    pub ageable_data: crate::entity::ageable::AgeableData,
}

impl ChickenEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let world = entity.world.load();
        let biome = world.get_zoomed_biome(&entity.block_pos.load());
        let variant = ChickenVariant::select_for_biome(biome.registry_id);
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        // Vanilla `Chicken`'s constructor.
        // `this.eggTime = this.random.nextInt(6000) + 6000`, before the malus.
        let egg_lay_time = mob_entity
            .living_entity
            .entity
            .random()
            .next_bounded_i32(6000)
            + 6000;
        mob_entity.set_pathfinding_malus(PathType::Water, 0.0);
        let chicken = Self {
            mob_entity,
            variant: AtomicU8::new(variant.id()),
            sound_variant: AtomicU8::new(ChickenSoundVariant::Classic as u8),
            egg_lay_time: AtomicI32::new(egg_lay_time),
            is_chicken_jockey: AtomicBool::new(false),
            ageable_data: crate::entity::ageable::AgeableData::default(),
        };
        let mob_arc = Arc::new(chicken);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(1, EscapeDangerGoal::new(1.4));
            goal_selector.add_goal(2, BreedGoal::new(1.0));
            goal_selector.add_goal(3, Box::new(TemptGoal::new(1.0, TEMPT_ITEMS, false)));
            goal_selector.add_goal(4, Box::new(FollowParentGoal::new(1.1)));
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::avoiding_water(1.0)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(7, Box::new(RandomLookAroundGoal::default()));
        };

        mob_arc
    }

    #[must_use]
    pub fn is_chicken_jockey(&self) -> bool {
        self.is_chicken_jockey.load(Ordering::Relaxed)
    }

    pub fn set_chicken_jockey(&self, is_chicken_jockey: bool) {
        self.is_chicken_jockey
            .store(is_chicken_jockey, Ordering::Relaxed);
    }

    pub fn set_variant(&self, variant: ChickenVariant) {
        self.variant.store(variant.id(), Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::chicken::DATA_VARIANT_ID,
            VarInt(variant.id() as i32),
        );
    }

    pub fn set_sound_variant(&self, sound_variant: ChickenSoundVariant) {
        self.sound_variant
            .store(sound_variant as u8, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::chicken::DATA_SOUND_VARIANT_ID,
            VarInt(sound_variant as u8 as i32),
        );
    }
}

impl CustomSound for ChickenEntity {
    fn death_sound(&self) -> Option<Sound> {
        let is_baby = self.is_baby();
        let sound_variant =
            ChickenSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
                .unwrap_or_default();
        Some(sound_variant.death_sound(is_baby))
    }

    fn hurt_sound(&self) -> Option<Sound> {
        let is_baby = self.is_baby();
        let sound_variant =
            ChickenSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
                .unwrap_or_default();
        Some(sound_variant.hurt_sound(is_baby))
    }
}

impl AgeableMob for ChickenEntity {
    fn get_ageable_data(&self) -> &crate::entity::ageable::AgeableData {
        &self.ageable_data
    }
}

impl Animal for ChickenEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        use pumpkin_data::tag::Taggable;
        item_stack
            .item
            .has_tag(&pumpkin_data::tag::Item::MINECRAFT_CHICKEN_FOOD)
            || TEMPT_ITEMS.iter().any(|i| i.id == item_stack.item.id)
    }
}

/// Vanilla `Chicken.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::CHICKEN_BABY_DIMENSIONS;

impl Mob for ChickenEntity {
    /// Vanilla `Chicken.positionRider`: a living passenger's body turns with the chicken.
    fn mob_after_position_rider(&self, passenger: &dyn EntityBase) {
        if passenger.get_living_entity().is_some() {
            passenger
                .get_entity()
                .body_yaw
                .store(self.get_entity().body_yaw.load());
        }
    }

    /// Vanilla `Chicken.playStepSound`.
    fn play_step_sound(
        &self,
        caller: &dyn crate::entity::EntityBase,
        _pos: &pumpkin_util::math::position::BlockPos,
        _block: &'static pumpkin_data::Block,
        _state: &'static pumpkin_data::BlockState,
    ) {
        let sound = ChickenSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
            .unwrap_or_default()
            .step_sound(self.get_entity().is_baby());
        caller.play_sound_at_self(sound, 0.15, 1.0);
    }

    /// Vanilla `Chicken.getAmbientSound`.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        let sounds = ChickenSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
            .unwrap_or_default();
        Some(sounds.ambient_sound(self.get_entity().is_baby()))
    }

    /// Vanilla `Chicken.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    /// Vanilla `AgeableMob.aiStep`, which runs inside this mob's `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("IsChickenJockey", self.is_chicken_jockey());
        nbt.put_int("EggLayTime", self.egg_lay_time.load(Ordering::Relaxed));
        let variant =
            ChickenVariant::from_id(self.variant.load(Ordering::Relaxed)).unwrap_or_default();
        nbt.put_string("variant", format!("minecraft:{}", variant.to_name()));
        let sound_variant =
            ChickenSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
                .unwrap_or_default();
        nbt.put_string(
            "sound_variant",
            format!("minecraft:{}", sound_variant.to_name()),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.set_chicken_jockey(nbt.get_bool("IsChickenJockey").unwrap_or(false));
        self.egg_lay_time
            .store(nbt.get_int("EggLayTime").unwrap_or(6000), Ordering::Relaxed);
        if let Some(variant_str) = nbt.get_string("variant")
            && let Some(variant) = ChickenVariant::from_name(variant_str)
        {
            self.variant.store(variant.id(), Ordering::Relaxed);
        }
        if let Some(sound_str) = nbt.get_string("sound_variant")
            && let Some(sound_variant) = ChickenSoundVariant::from_name(sound_str)
        {
            self.sound_variant
                .store(sound_variant as u8, Ordering::Relaxed);
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `Chicken.removeWhenFarAway`.
    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        self.is_chicken_jockey()
    }

    /// Vanilla `Chicken.getBaseExperienceReward`.
    fn get_base_experience_reward(&self) -> u32 {
        if self.is_chicken_jockey() {
            CHICKEN_JOCKEY_EXPERIENCE
        } else {
            self.animal_base_experience_reward()
        }
    }

    fn mob_set_variant_name(&self, name: &str) {
        if let Some(v) = ChickenVariant::from_name(name) {
            self.set_variant(v);
        }
    }

    fn mob_set_sound_variant_name(&self, name: &str) {
        if let Some(v) = ChickenSoundVariant::from_name(name) {
            self.set_sound_variant(v);
        }
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::chicken::BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::chicken::DATA_VARIANT_ID,
            VarInt(self.variant.load(Ordering::Relaxed) as i32),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::chicken::DATA_SOUND_VARIANT_ID,
            VarInt(self.sound_variant.load(Ordering::Relaxed) as i32),
        );
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        if self.mob_entity.living_entity.dead.load(Relaxed) {
            return;
        }
        let entity = &self.mob_entity.living_entity.entity;
        let current_velocity = entity.velocity.load();
        let on_ground = entity.on_ground.load(Ordering::Relaxed);

        // TODO: move velocity logic to physics tick when implemented
        if (!on_ground) && current_velocity.y < 0.0 {
            entity.set_velocity(current_velocity.multiply(1.0, 0.6, 1.0));
        }
        // Vanilla `aiStep`: babies and jockeys lay no eggs, and their timer stands still.
        if !self.is_baby()
            && !self.is_chicken_jockey()
            && self.egg_lay_time.fetch_sub(1, Ordering::Relaxed) <= 1
        {
            let world = entity.world.load_full();
            let entity_id = entity.entity_id;
            let mut drop_event =
                crate::plugin::api::events::entity::entity_drop_item::EntityDropItemEvent::new(
                    entity_id,
                    "minecraft:egg".to_string(),
                    1,
                );
            if let Some(server) = world.server.upgrade() {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut drop_event);
            }
            if !drop_event.cancelled {
                self.get_entity()
                    .spawn_at_location(ItemStack::new(1, &Item::EGG));
                // `playSound(CHICKEN_EGG, 1.0F, (nextFloat() - nextFloat()) * 0.2F + 1.0F)`.
                let pitch = {
                    let mut random = entity.random();
                    (random.next_f32() - random.next_f32()) * 0.2 + 1.0
                };
                self.play_sound_at_self(Sound::EntityChickenEgg, 1.0, pitch);
            }
            let next_time = entity.random().next_bounded_i32(6000) + 6000;
            self.egg_lay_time.store(next_time, Ordering::Relaxed);
        }
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let sound_variant =
            ChickenSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
                .unwrap_or_default();
        self.animal_interact(
            player,
            item_stack,
            sound_variant.ambient_sound(self.is_baby()),
        )
    }
}
