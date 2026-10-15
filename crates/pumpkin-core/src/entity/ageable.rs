use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};

use crate::entity::mob::Mob;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::player::Player;

pub const BABY_START_AGE: i32 = -24000;
pub const FORCED_AGE_PARTICLE_TICKS: i32 = 40;
pub const AGE_LOCK_DOWNWARDS_MOVING_PARTICLE_Y_OFFSET: f32 = 0.2;
/// The baby chance of vanilla's `AgeableMobGroupData(boolean)`.
const DEFAULT_BABY_SPAWN_CHANCE: f32 = 0.05;

pub struct AgeableData {
    pub forced_age: AtomicI32,
    pub forced_age_timer: AtomicI32,
    pub age_locked: AtomicBool,
    pub age_lock_particle_timer: AtomicI32,
}

impl Default for AgeableData {
    fn default() -> Self {
        Self {
            forced_age: AtomicI32::new(0),
            forced_age_timer: AtomicI32::new(0),
            age_locked: AtomicBool::new(false),
            age_lock_particle_timer: AtomicI32::new(0),
        }
    }
}

pub trait AgeableMob: Mob {
    fn get_ageable_data(&self) -> &AgeableData;

    fn get_baby_start_age(&self) -> i32 {
        BABY_START_AGE
    }

    fn is_baby(&self) -> bool {
        self.can_be_a_baby() && self.get_age() < 0
    }

    fn set_baby(&self, baby: bool) {
        if self.can_be_a_baby() {
            self.set_age(if baby { self.get_baby_start_age() } else { 0 });
        }
    }

    fn get_age(&self) -> i32 {
        self.get_mob_entity().living_entity.entity.age.load(Relaxed)
    }

    fn set_age(&self, new_age: i32) {
        let mob = self.get_mob_entity();
        let entity = &mob.living_entity.entity;
        let old_age = entity.age.swap(new_age, Relaxed);

        if (old_age < 0 && new_age >= 0) || (old_age >= 0 && new_age < 0) {
            let is_baby = new_age < 0;
            entity.set_synced_data(tracked_data::ageable_mob::DATA_BABY_ID, is_baby);
            // Vanilla `AgeableMob.onSyncedDataUpdated` on `DATA_BABY_ID`.
            self.refresh_dimensions();
            self.age_boundary_reached();
        }
    }

    /// Vanilla `AgeableMob.ageBoundaryReached`, run when the mob turns into or out of a baby.
    fn age_boundary_reached(&self) {}

    fn is_age_locked(&self) -> bool {
        self.get_ageable_data().age_locked.load(Relaxed)
    }

    fn set_age_locked(&self, locked: bool) {
        self.get_ageable_data().age_locked.store(locked, Relaxed);
        self.get_mob_entity()
            .living_entity
            .entity
            .set_synced_data(tracked_data::ageable_mob::AGE_LOCKED, locked);
    }

    fn can_age_up(&self) -> bool {
        self.is_baby() && !self.is_age_locked()
    }

    fn age_up(&self, seconds: i32, forced: bool) {
        let mut age = self.get_age();
        let old_age = age;
        age += seconds * 20;
        if age > 0 {
            age = 0;
        }

        let delta = age - old_age;
        self.set_age(age);

        let data = self.get_ageable_data();
        if forced {
            data.forced_age.fetch_add(delta, Relaxed);
            if data.forced_age_timer.load(Relaxed) == 0 {
                data.forced_age_timer
                    .store(FORCED_AGE_PARTICLE_TICKS, Relaxed);
            }
        }

        if self.get_age() == 0 {
            self.set_age(data.forced_age.load(Relaxed));
        }
    }

    #[must_use]
    fn get_speed_up_seconds_when_feeding(ticks_until_adult: i32) -> i32
    where
        Self: Sized,
    {
        get_speed_up_seconds_when_feeding(ticks_until_adult)
    }

    fn write_ageable_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        if self.can_be_a_baby() {
            nbt.put_int("Age", self.get_age());
            nbt.put_int(
                "ForcedAge",
                self.get_ageable_data().forced_age.load(Relaxed),
            );
            nbt.put_bool("AgeLocked", self.is_age_locked());
        }
    }

    fn read_ageable_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        if self.can_be_a_baby() {
            self.set_age(nbt.get_int("Age").unwrap_or(0));
            self.get_ageable_data()
                .forced_age
                .store(nbt.get_int("ForcedAge").unwrap_or(0), Relaxed);
            self.set_age_locked(nbt.get_bool("AgeLocked").unwrap_or(false));
        }
    }

    /// Vanilla `AgeableMob.finalizeSpawn`: after the first mob of a group, each may
    /// spawn as a baby.
    fn finalize_ageable_spawn(&self, group_data: Option<SpawnGroupData>) -> SpawnGroupData {
        let mut group_data = match group_data {
            Some(data @ SpawnGroupData::Ageable { .. }) => data,
            // Vanilla's `AgeableMobGroupData(true)`.
            _ => SpawnGroupData::ageable(DEFAULT_BABY_SPAWN_CHANCE),
        };
        if let SpawnGroupData::Ageable {
            group_size,
            should_spawn_baby,
            baby_spawn_chance,
        } = &mut group_data
        {
            if *should_spawn_baby && *group_size > 0 && rand::random::<f32>() <= *baby_spawn_chance
            {
                self.set_age(self.get_baby_start_age());
            }
            *group_size += 1;
        }
        group_data
    }

    fn can_be_a_baby(&self) -> bool {
        true
    }

    /// Vanilla `AgeableMob.aiStep` after its `super.aiStep()`: the forced-growth particles,
    /// one tick of growing up (or of the adult cooldown running down) and the age-lock
    /// particles.
    fn ageable_ai_step(&self) {
        let living = &self.get_mob_entity().living_entity;
        let entity = &living.entity;
        let data = self.get_ageable_data();
        let forced_age_timer = data.forced_age_timer.load(Relaxed);
        if forced_age_timer > 0 {
            if forced_age_timer % 4 == 0 {
                entity.world.load().spawn_particle(
                    Vector3::new(
                        entity.get_random_x(1.0),
                        entity.get_random_y() + 0.5,
                        entity.get_random_z(1.0),
                    ),
                    Vector3::new(0.0, 0.0, 0.0),
                    0.0,
                    1,
                    Particle::HappyVillager,
                );
            }
            data.forced_age_timer.store(forced_age_timer - 1, Relaxed);
        }

        if living.is_alive() {
            let age = self.get_age();
            if self.can_age_up() {
                self.set_age(age + 1);
            } else if age > 0 {
                self.set_age(age - 1);
            }
        }

        make_age_locked_particle(entity, &data.age_lock_particle_timer, self.is_age_locked());
    }
}

/// Vanilla `AgeableMob.getSpeedUpSecondsWhenFeeding`.
#[must_use]
pub fn get_speed_up_seconds_when_feeding(ticks_until_adult: i32) -> i32 {
    ((ticks_until_adult / 20) as f32 * 0.1) as i32
}

/// Vanilla `AgeableMob.makeAgeLockedParticle`.
pub fn make_age_locked_particle(
    entity: &crate::entity::Entity,
    age_lock_particle_timer: &AtomicI32,
    is_age_locked: bool,
) {
    let timer = age_lock_particle_timer.load(Relaxed);
    if timer > 0 {
        if timer % 2 == 0 {
            let y_offset = if is_age_locked {
                AGE_LOCK_DOWNWARDS_MOVING_PARTICLE_Y_OFFSET
            } else {
                0.0
            };
            entity.world.load().spawn_particle(
                Vector3::new(
                    entity.get_random_x(1.0),
                    entity.get_random_y_spread(0.2)
                        + f64::from(entity.height())
                        + f64::from(y_offset),
                    entity.get_random_z(1.0),
                ),
                Vector3::new(0.0, 0.0, 0.0),
                0.0,
                1,
                if is_age_locked {
                    Particle::PauseMobGrowth
                } else {
                    Particle::ResetMobGrowth
                },
            );
        }
        age_lock_particle_timer.store(timer - 1, Relaxed);
    }
}

/// Vanilla `AgeableMob.canUseGoldenDandelion`.
#[must_use]
pub fn can_use_golden_dandelion(
    item_in_hand: &ItemStack,
    is_baby: bool,
    cooldown: i32,
    mob: &dyn Mob,
) -> bool {
    item_in_hand.item == &Item::GOLDEN_DANDELION
        && is_baby
        && cooldown == 0
        && !mob
            .get_entity()
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_CANNOT_BE_AGE_LOCKED)
}

/// Vanilla `AgeableMob.setAgeLocked(mob, isAgeLocked, player, itemInHand, setAgeLockData)`:
/// toggles the lock through `set_age_lock_data`, uses up the golden dandelion and plays
/// its sound.
pub fn set_age_locked_by_player(
    mob: &dyn Mob,
    is_age_locked: impl FnOnce() -> bool,
    player: &Player,
    item_in_hand: &mut ItemStack,
    set_age_lock_data: impl FnOnce(),
) {
    set_age_lock_data();
    item_in_hand.decrement_unless_creative(player.gamemode.load(), 1);
    let is_age_locked = is_age_locked();
    if is_age_locked {
        mob.get_mob_entity()
            .persistence_required
            .store(true, Relaxed);
    }
    let entity = mob.get_entity();
    entity.world.load().play_sound(
        if is_age_locked {
            Sound::ItemGoldenDandelionUse
        } else {
            Sound::ItemGoldenDandelionUnuse
        },
        SoundCategory::Players,
        &entity.block_pos.load().to_centered_f64(),
    );
}
