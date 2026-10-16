use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::{Holder, InstrumentImpl};
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::instrument::Instrument;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::{RandomImpl, legacy_rand::LegacyRand};
use rand::RngExt;
use uuid::Uuid;

use crate::entity::ai::brain::{Brain, BrainTick, memory::PackedMemories};
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::item::ItemEntity;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::r#type::from_type;
use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    custom_sound::CustomSound,
    mob::{Mob, MobEntity},
    passive::{
        animal::{Animal, set_animal_pathfinding_malus},
        goat_ai,
    },
    player::Player,
};
use crate::world::World;

/// Vanilla `Goat.LONG_JUMPING_DIMENSION_SCALE_FACTOR`.
const LONG_JUMPING_DIMENSION_SCALE_FACTOR: f32 = 0.7;
/// Vanilla `Goat.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    EntityDimensions::scalable(0.45, 0.65).with_eye_height(0.593_75);
/// Vanilla `Goat.BABY_SCALE`.
const BABY_SCALE: f32 = 0.55;
/// Vanilla `Goat.ADULT_ATTACK_DAMAGE` and `BABY_ATTACK_DAMAGE`.
const ADULT_ATTACK_DAMAGE: f64 = 2.0;
const BABY_ATTACK_DAMAGE: f64 = 1.0;
/// Vanilla `Goat.GOAT_FALL_DAMAGE_REDUCTION`.
const GOAT_FALL_DAMAGE_REDUCTION: i32 = 10;
/// Vanilla `Goat.GOAT_SCREAMING_CHANCE`.
const GOAT_SCREAMING_CHANCE: f64 = 0.02;
/// Vanilla `Goat.UNIHORN_CHANCE`, which `finalizeSpawn` compares as a float.
const UNIHORN_CHANCE: f32 = 0.1;
/// Vanilla `Goat.getMaxHeadYRot`.
const MAX_HEAD_Y_ROT: f32 = 15.0;

pub struct GoatEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub is_screaming: AtomicBool,
    pub has_left_horn: AtomicBool,
    pub has_right_horn: AtomicBool,
}

impl GoatEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        // Vanilla `Goat`'s constructor.
        mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_can_float(true);
        mob_entity.set_pathfinding_malus(PathType::PowderSnow, -1.0);
        mob_entity.set_pathfinding_malus(PathType::DangerPowderSnow, -1.0);
        let goat = Arc::new(Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            is_screaming: AtomicBool::new(false),
            has_left_horn: AtomicBool::new(true),
            has_right_horn: AtomicBool::new(true),
        });
        goat.mob_entity.init_brain(goat.as_ref());
        goat
    }

    #[must_use]
    pub fn is_screaming(&self) -> bool {
        self.is_screaming.load(Ordering::Relaxed)
    }

    pub fn set_screaming(&self, screaming: bool) {
        self.is_screaming.store(screaming, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::goat::DATA_IS_SCREAMING_GOAT,
            screaming,
        );
    }

    #[must_use]
    pub fn has_left_horn(&self) -> bool {
        self.has_left_horn.load(Ordering::Relaxed)
    }

    pub fn set_has_left_horn(&self, has_horn: bool) {
        self.has_left_horn.store(has_horn, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::goat::DATA_HAS_LEFT_HORN,
            has_horn,
        );
    }

    #[must_use]
    pub fn has_right_horn(&self) -> bool {
        self.has_right_horn.load(Ordering::Relaxed)
    }

    pub fn set_has_right_horn(&self, has_horn: bool) {
        self.has_right_horn.store(has_horn, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::goat::DATA_HAS_RIGHT_HORN,
            has_horn,
        );
    }

    /// Vanilla `Goat.createHorn`: an instrument from the goat's horn tag, picked by a random
    /// seeded with the goat's UUID so one goat always drops the same horn.
    #[must_use]
    pub fn create_horn(&self) -> ItemStack {
        let mut random =
            LegacyRand::from_seed(i64::from(uuid_hash_code(self.get_entity().entity_uuid)) as u64);
        let horns = if self.is_screaming() {
            &tag::Instrument::MINECRAFT_SCREAMING_GOAT_HORNS
        } else {
            &tag::Instrument::MINECRAFT_REGULAR_GOAT_HORNS
        };
        let mut horn = ItemStack::new(1, &Item::GOAT_HORN);
        if !horns.0.is_empty()
            && let Some(instrument) = Instrument::from_name(
                horns.0[random.next_bounded_i32(horns.0.len() as i32) as usize],
            )
        {
            horn.set_data_component(InstrumentImpl {
                instrument: Holder::Reference(instrument),
            });
        }
        horn
    }

    /// Vanilla `Goat.dropHorn`: an adult loses one of its horns, picked at random when it
    /// has both, and drops it as an item.
    pub fn drop_horn(&self) -> bool {
        if self.is_baby() {
            return false;
        }

        let has_left = self.has_left_horn();
        let has_right = self.has_right_horn();
        if !has_left && !has_right {
            return false;
        }

        let mut random = self.get_random();
        let drop_left = if !has_left {
            false
        } else if !has_right {
            true
        } else {
            random.random::<bool>()
        };
        if drop_left {
            self.set_has_left_horn(false);
        } else {
            self.set_has_right_horn(false);
        }

        let entity = self.get_entity();
        let body_position = entity.pos.load();
        let item = self.create_horn();
        let delta_x = f64::from(random_between(&mut random, -0.2, 0.2));
        let delta_y = f64::from(random_between(&mut random, 0.3, 0.7));
        let delta_z = f64::from(random_between(&mut random, -0.2, 0.2));
        let world = entity.world.load_full();
        let item_entity = ItemEntity::new_with_velocity(
            Entity::new(world.clone(), body_position, &EntityType::ITEM),
            item,
            Vector3::new(delta_x, delta_y, delta_z),
            0,
        );
        world.spawn_entity(Arc::new(item_entity));
        true
    }

    /// Vanilla `Goat.getMilkingSound`.
    fn milking_sound(&self) -> Sound {
        if self.is_screaming() {
            Sound::EntityGoatScreamingMilk
        } else {
            Sound::EntityGoatMilk
        }
    }
}

/// Vanilla `Mth.randomBetween`.
fn random_between(random: &mut impl RngExt, min: f32, max: f32) -> f32 {
    random.random::<f32>() * (max - min) + min
}

/// Java's `UUID.hashCode`.
const fn uuid_hash_code(uuid: Uuid) -> i32 {
    let (most, least) = uuid.as_u64_pair();
    let hilo = most ^ least;
    ((hilo >> 32) as i32) ^ (hilo as i32)
}

impl AgeableMob for GoatEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }

    /// Vanilla `Goat.ageBoundaryReached`.
    fn age_boundary_reached(&self) {
        self.mob_entity.living_entity.set_attribute_base(
            &Attributes::ATTACK_DAMAGE,
            if self.is_baby() {
                BABY_ATTACK_DAMAGE
            } else {
                ADULT_ATTACK_DAMAGE
            },
        );
    }
}

impl Animal for GoatEntity {
    /// Vanilla `Goat.isFood`.
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_GOAT_FOOD)
    }

    /// Vanilla `Goat.getBreedOffspring`.
    fn get_breed_offspring(
        &self,
        world: &Arc<World>,
        partner: &dyn EntityBase,
    ) -> Option<Arc<dyn EntityBase>> {
        let baby = from_type(
            &EntityType::GOAT,
            self.get_entity().pos.load(),
            world,
            Uuid::new_v4(),
        );
        if let Some(new_goat) = baby.cast_any().downcast_ref::<Self>() {
            new_goat
                .mob_entity
                .with_brain(new_goat, goat_ai::init_memories);
            let mut random = self.get_random();
            let screaming_parent = if random.random::<bool>() {
                self.is_screaming()
            } else {
                partner
                    .cast_any()
                    .downcast_ref::<Self>()
                    .is_some_and(Self::is_screaming)
            };
            let baby_is_screaming =
                screaming_parent || random.random::<f64>() < GOAT_SCREAMING_CHANCE;
            new_goat.set_screaming(baby_is_screaming);
        }
        Some(baby)
    }

    /// Vanilla `Goat.playEatingSound`.
    fn play_eating_sound(&self, _sound: Sound) {
        let entity = self.get_entity();
        let sound = if self.is_screaming() {
            Sound::EntityGoatScreamingEat
        } else {
            Sound::EntityGoatEat
        };
        let pitch = random_between(&mut self.get_random(), 0.8, 1.2);
        entity.world.load().play_sound_fine(
            sound,
            SoundCategory::Neutral,
            &entity.pos.load(),
            1.0,
            pitch,
        );
    }
}

impl CustomSound for GoatEntity {
    /// Vanilla `Goat.getDeathSound`.
    fn death_sound(&self) -> Option<Sound> {
        Some(if self.is_screaming() {
            Sound::EntityGoatScreamingDeath
        } else {
            Sound::EntityGoatDeath
        })
    }

    /// Vanilla `Goat.getHurtSound`.
    fn hurt_sound(&self) -> Option<Sound> {
        Some(if self.is_screaming() {
            Sound::EntityGoatScreamingHurt
        } else {
            Sound::EntityGoatHurt
        })
    }
}

impl Mob for GoatEntity {
    /// Vanilla `Goat.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        Some(if self.is_screaming() {
            Sound::EntityGoatScreamingAmbient
        } else {
            Sound::EntityGoatAmbient
        })
    }

    /// Vanilla `Goat.getAgeScale`.
    fn age_scale(&self) -> f32 {
        if self.get_entity().is_baby() {
            BABY_SCALE
        } else {
            1.0
        }
    }

    /// Vanilla `Goat.getDefaultDimensions`: long jumps shrink the goat.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        let dimensions = if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        };
        if pose == EntityPose::LongJumping {
            dimensions.scale(LONG_JUMPING_DIMENSION_SCALE_FACTOR)
        } else {
            dimensions
        }
    }

    /// Vanilla `Goat.calculateFallDamage`.
    fn calculate_fall_damage(&self, fall_distance: f64, damage_modifier: f32) -> i32 {
        self.get_mob_entity()
            .living_entity
            .calculate_fall_damage(fall_distance, damage_modifier)
            - GOAT_FALL_DAMAGE_REDUCTION
    }

    /// Vanilla `Goat.getMaxHeadYRot`.
    // TODO: vanilla `Goat.setYHeadRot`'s clamp on teleports, loads and `lookAt`; Pumpkin has
    // no head rotation setter to hook. `LookControl` writes the field directly, so it skips it.
    fn get_max_head_rotation(&self) -> f32 {
        MAX_HEAD_Y_ROT
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `AgeableMob.aiStep`, the end of `Goat`'s `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        if self.get_entity().is_alive() {
            self.ageable_ai_step();
        }
    }

    fn make_brain(&self, packed: &PackedMemories) -> Brain {
        goat_ai::GOAT_PROVIDER.make_brain(self, packed)
    }

    /// The rest of vanilla `Goat.customServerAiStep`: the activity update.
    fn after_brain_tick(&self, tick: &mut BrainTick<'_>) {
        goat_ai::update_activity(tick);
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if !crate::entity::ai::brain::behavior::utils::is_alive(self) {
            self.mob_entity.apply_brain_inbox(self);
            return;
        }
        self.mob_entity.tick_brain(self);
    }

    /// Vanilla `Goat.finalizeSpawn`, then `AgeableMob`'s and `Mob`'s.
    fn finalize_spawn(
        &self,
        _world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.mob_entity.with_brain(self, goat_ai::init_memories);
        let mut random = self.get_random();
        self.set_screaming(random.random::<f64>() < GOAT_SCREAMING_CHANCE);
        self.age_boundary_reached();
        if !self.is_baby() && random.random::<f32>() < UNIHORN_CHANCE {
            if random.random::<bool>() {
                self.set_has_left_horn(false);
            } else {
                self.set_has_right_horn(false);
            }
        }

        let group_data = self.finalize_ageable_spawn(group_data);
        self.mob_entity.finalize_spawn_base();
        Some(group_data)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("IsScreamingGoat", self.is_screaming());
        nbt.put_bool("HasLeftHorn", self.has_left_horn());
        nbt.put_bool("HasRightHorn", self.has_right_horn());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.set_screaming(nbt.get_bool("IsScreamingGoat").unwrap_or(false));
        self.set_has_left_horn(nbt.get_bool("HasLeftHorn").unwrap_or(true));
        self.set_has_right_horn(nbt.get_bool("HasRightHorn").unwrap_or(true));
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::goat::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::goat::DATA_IS_SCREAMING_GOAT,
            self.is_screaming(),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::goat::DATA_HAS_LEFT_HORN,
            self.has_left_horn(),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::goat::DATA_HAS_RIGHT_HORN,
            self.has_right_horn(),
        );
    }

    /// Vanilla `Goat.mobInteract`.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if item_stack.get_item() == &Item::BUCKET && !self.is_baby() {
            // Vanilla `Player.playSound`: everyone but the milking player, who plays it itself.
            let world = self.get_entity().world.load();
            world.play_sound_raw_expect(
                player,
                self.milking_sound() as u16,
                SoundCategory::Players,
                &player.get_entity().pos.load(),
                1.0,
                1.0,
            );
            crate::item::create_filled_result(
                item_stack,
                player,
                ItemStack::new(1, &Item::MILK_BUCKET),
                true,
            );
            return true;
        }

        let eat_sound = if self.is_screaming() {
            Sound::EntityGoatScreamingEat
        } else {
            Sound::EntityGoatEat
        };
        let consumed = self.animal_interact(player, item_stack, eat_sound);
        // The fed stack may be used up by now, and an empty stack is no food.
        if consumed && !item_stack.is_empty() && self.is_food(item_stack) {
            self.play_eating_sound(eat_sound);
        }
        consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_hash_code_matches_java() {
        assert_eq!(uuid_hash_code(Uuid::from_u64_pair(1, 2)), 3);
        assert_eq!(uuid_hash_code(Uuid::from_u64_pair(1 << 32, 0)), 1);
        assert_eq!(
            uuid_hash_code(Uuid::from_u64_pair(
                0x0123_4567_89ab_cdef,
                0xfedc_ba98_7654_3210
            )),
            0
        );
    }
}
