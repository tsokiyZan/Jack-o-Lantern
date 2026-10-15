//! Vanilla `Mob.finalizeSpawn`: the step run on freshly created mobs before they enter the world.

use std::sync::Arc;

use pumpkin_data::attributes::Attributes;
use pumpkin_data::effect::StatusEffect;
use rand::RngExt;

use crate::entity::EntityBase;
use crate::entity::attributes::{Modifier, ModifierOperation};
use crate::entity::mob::MobEntity;
use crate::world::World;

/// Vanilla `Mob.RANDOM_SPAWN_BONUS_ID`.
pub(crate) const RANDOM_SPAWN_BONUS_ID: &str = "minecraft:random_spawn_bonus";

/// State shared by every mob of one spawn group (vanilla `SpawnGroupData`).
pub enum SpawnGroupData {
    /// The effect the first spider of a group rolled on hard difficulty.
    SpiderEffects(Option<&'static StatusEffect>),
    /// Vanilla `AgeableMob.AgeableMobGroupData`.
    Ageable {
        group_size: i32,
        should_spawn_baby: bool,
        baby_spawn_chance: f32,
    },
    /// Vanilla `Fox.FoxGroupData`: an `AgeableMobGroupData(false)` carrying the group's
    /// variant.
    Fox {
        variant: crate::entity::passive::fox::FoxVariant,
        group_size: i32,
    },
    /// Vanilla `Zombie.ZombieGroupData`.
    Zombie {
        is_baby: bool,
        can_spawn_jockey: bool,
    },
}

impl SpawnGroupData {
    /// Vanilla `AgeableMobGroupData(babySpawnChance)`.
    #[must_use]
    pub const fn ageable(baby_spawn_chance: f32) -> Self {
        Self::Ageable {
            group_size: 0,
            should_spawn_baby: true,
            baby_spawn_chance,
        }
    }
}

impl MobEntity {
    /// Vanilla `Mob.finalizeSpawn`: random follow range bonus and left-handedness.
    pub fn finalize_spawn_base(&self) {
        let mut rng = rand::rng();
        let follow_range_changed = {
            let mut attributes = self
                .living_entity
                .attributes
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(follow_range) = attributes.get_mut(&Attributes::FOLLOW_RANGE.id)
                && !follow_range
                    .modifiers
                    .iter()
                    .any(|modifier| modifier.id == RANDOM_SPAWN_BONUS_ID)
            {
                follow_range.add_or_replace_modifier(Modifier {
                    id: RANDOM_SPAWN_BONUS_ID.to_string(),
                    // Triangle distribution around 0.
                    amount: 0.114_85 * f64::from(rng.random::<f32>() - rng.random::<f32>()),
                    operation: ModifierOperation::MultiplyBase,
                });
                true
            } else {
                false
            }
        };
        // Vanilla `Mob.onAttributeUpdated`.
        if follow_range_changed {
            self.navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .update_pathfinder_max_visited_nodes(&self.living_entity);
        }
        self.set_left_handed(rng.random::<f32>() < 0.05);

        // Spawn equipment and the loot-pickup roll belong to `finalizeSpawn`
        // (`populateDefaultEquipmentSlots`, `setCanPickUpLoot`), so mobs loaded from disk or
        // summoned with NBT keep what they had.
        let world = self.living_entity.entity.world.load_full();
        super::equipment::equip_mob_on_spawn(&self.living_entity, &world);
        let entity = &self.living_entity.entity;
        if let Some(def) =
            super::equipment::EQUIPMENT_REGISTRY.get(entity.entity_type.resource_name)
            && def.can_pick_up_loot
        {
            let difficulty = super::equipment::RegionalDifficulty::at(&world, entity.pos.load());
            let pickup_chance = 0.55 * difficulty.special_multiplier;
            self.set_can_pick_up_loot(rand::random::<f32>() < pickup_chance);
        }
    }

    /// Queues a mob to be added and mounted onto this one when it enters the world.
    pub fn add_pending_rider(&self, rider: Arc<dyn EntityBase>) {
        self.pending_riders
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(rider);
    }

    pub(crate) fn take_pending_riders(&self) -> Vec<Arc<dyn EntityBase>> {
        std::mem::take(
            &mut *self
                .pending_riders
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Queues an entity already in the world for this mob to mount when it enters the world,
    /// as a vanilla `startRiding` during `finalizeSpawn` does.
    pub fn set_pending_vehicle(&self, vehicle: Arc<dyn EntityBase>) {
        *self
            .pending_vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(vehicle);
    }

    pub(crate) fn take_pending_vehicle(&self) -> Option<Arc<dyn EntityBase>> {
        self.pending_vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// Finalizes `entity` if it is a mob, and returns the group data for the next mob of the group.
pub fn finalize_spawn(
    entity: &Arc<dyn EntityBase>,
    world: &Arc<World>,
    group_data: Option<SpawnGroupData>,
) -> Option<SpawnGroupData> {
    match entity.get_mob() {
        Some(mob) => mob.finalize_spawn(world, group_data),
        None => group_data,
    }
}

impl MobEntity {
    /// Baby state kept only as a negative age and the shared baby flag (hoglins, zoglins and
    /// zombified piglins, which are not `AgeableMob`s here).
    pub fn set_baby_by_age(&self) {
        let entity = &self.living_entity.entity;
        entity
            .age
            .store(-24000, std::sync::atomic::Ordering::Relaxed);
        entity.set_synced_data(pumpkin_data::tracked_data::ageable_mob::DATA_BABY_ID, true);
    }

    /// Baby state kept as a mob's own flag and synced key (vanilla zombies and piglins), so it
    /// never ages up.
    pub fn set_baby_flag(
        &self,
        flag: &std::sync::atomic::AtomicBool,
        tracked: pumpkin_data::tracked_data::TrackedData,
        baby: bool,
    ) {
        flag.store(baby, std::sync::atomic::Ordering::Relaxed);
        let entity = &self.living_entity.entity;
        entity.set_synced_data(tracked, baby);
        // Pumpkin's negative age is what reports the Bedrock baby flag.
        entity.age.store(
            if baby { -24000 } else { 0 },
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}
