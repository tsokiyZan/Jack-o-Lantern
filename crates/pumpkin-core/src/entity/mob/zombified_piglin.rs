use crate::entity::ai::goal::spear_use::SpearUseGoal;
use crate::entity::ai::goal::zombie_attack::ZombieAttackGoal;
use pumpkin_data::attributes::Attributes;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::random::RandomImpl;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::ai::goal::goal_selector::GoalSelector;
use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::ai::util::goal_utils;
use crate::entity::attributes::{Modifier, ModifierOperation, send_attribute_updates_for_living};
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::zombie::ZombieEntityBase;
use crate::entity::{
    Entity, EntityBase,
    ai::behavior::neutral::apply_targets,
    ai::goal::{revenge::RevengeGoal, wander_around::WanderAroundGoal},
    mob::{
        Mob, MobEntity,
        equipment::RegionalDifficulty,
        neutral::{NeutralData, NeutralMob},
        set_mob_target_base,
    },
};
use crate::world::World;

/// Vanilla `ALERT_INTERVAL`: 4 to 6 seconds.
const ALERT_INTERVAL: std::ops::RangeInclusive<i32> = 80..=120;
/// Vanilla `FIRST_ANGER_SOUND_DELAY`: 0 to 1 second.
const FIRST_ANGER_SOUND_DELAY: std::ops::RangeInclusive<i32> = 0..=20;
/// Vanilla `SPEED_MODIFIER_ATTACKING_ID`.
const SPEED_MODIFIER_ATTACKING_ID: &str = "minecraft:attacking";

pub struct ZombifiedPiglinEntity {
    zombie: Arc<ZombieEntityBase>,
    neutral_data: NeutralData,
    play_first_anger_sound_in: AtomicI32,
    ticks_until_next_alert: AtomicI32,
}

impl ZombifiedPiglinEntity {
    pub const XP_REWARD: u32 = 5;

    pub fn new(entity: Entity) -> Arc<Self> {
        let zombie = ZombieEntityBase::with_behaviour_goals(
            entity,
            false,
            BABY_DIMENSIONS,
            Self::add_behaviour_goals,
        );
        // Vanilla `ZombifiedPiglin`'s constructor.
        zombie.mob_entity.set_pathfinding_malus(PathType::Lava, 8.0);
        Arc::new(Self {
            zombie,
            neutral_data: NeutralData::default(),
            play_first_anger_sound_in: AtomicI32::new(0),
            ticks_until_next_alert: AtomicI32::new(0),
        })
    }

    /// Vanilla `ZombifiedPiglin.addBehaviourGoals`.
    fn add_behaviour_goals(
        mob: &MobEntity,
        goal_selector: &mut GoalSelector,
        target_selector: &mut GoalSelector,
    ) {
        goal_selector.add_goal(1, SpearUseGoal::new(1.0, 1.0, 10.0, 2.0));
        goal_selector.add_goal(2, ZombieAttackGoal::new(1.0, false));
        goal_selector.add_goal(7, Box::new(WanderAroundGoal::avoiding_water(1.0)));

        target_selector.add_goal(1, Box::new(RevengeGoal::new(true).alerting_others()));
        apply_targets(target_selector, mob, 2, 3, true);
    }

    /// Vanilla `maybePlayFirstAngerSound`.
    fn maybe_play_first_anger_sound(&self) {
        let remaining = self.play_first_anger_sound_in.load(Ordering::Relaxed);
        if remaining > 0 {
            self.play_first_anger_sound_in
                .store(remaining - 1, Ordering::Relaxed);
            if remaining == 1 {
                self.play_anger_sound();
            }
        }
    }

    /// Vanilla `playAngerSound`.
    fn play_anger_sound(&self) {
        let entity = self.get_entity();
        if entity.is_silent() {
            return;
        }
        entity.world.load().play_sound_fine(
            Sound::EntityZombifiedPiglinAngry,
            self.sound_source(),
            &entity.pos.load(),
            self.sound_volume() * 2.0,
            self.voice_pitch() * 1.8,
        );
    }

    /// Vanilla `maybeAlertOthers`: every few seconds, hands the target to nearby
    /// piglins that have none of their own.
    fn maybe_alert_others(&self, target: &Arc<dyn EntityBase>) {
        let remaining = self.ticks_until_next_alert.load(Ordering::Relaxed);
        if remaining > 0 {
            self.ticks_until_next_alert
                .store(remaining - 1, Ordering::Relaxed);
            return;
        }

        if self.has_line_of_sight(target.get_entity()) {
            for other in goal_utils::nearby_same_type(self) {
                let Some(other_mob) = other.get_mob() else {
                    continue;
                };
                if other_mob.get_mob_entity().get_target().is_some()
                    || other.is_allied_to(target.as_ref())
                {
                    continue;
                }
                other_mob.set_mob_target(Some(target.clone()));
            }
        }

        let alert_in = sample(&ALERT_INTERVAL, &mut *self.get_entity().random());
        self.ticks_until_next_alert
            .store(alert_in, Ordering::Relaxed);
    }
}

/// Vanilla `UniformInt.sample`: `Mth.randomBetweenInclusive`.
fn sample(range: &std::ops::RangeInclusive<i32>, random: &mut impl RandomImpl) -> i32 {
    random.next_int_between_inclusive(*range.start(), *range.end())
}

crate::impl_neutral_mob!(ZombifiedPiglinEntity, neutral_data);

/// Vanilla `ZombifiedPiglin.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    pumpkin_data::entity_dimensions::ZOMBIFIED_PIGLIN_BABY_DIMENSIONS;

impl Mob for ZombifiedPiglinEntity {
    /// Vanilla `ZombifiedPiglin.checkSpawnObstruction`.
    fn check_spawn_obstruction(&self, world: &crate::world::World) -> bool {
        let entity = self.get_entity();
        let bounding_box = entity.bounding_box.load();
        world.is_unobstructed(Some(entity), &bounding_box)
            && !world.contains_any_liquid(bounding_box)
    }

    crate::impl_zombie_mob!(zombie);

    fn get_mob_entity(&self) -> &MobEntity {
        &self.zombie.mob_entity
    }

    /// Vanilla `ZombifiedPiglin.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        Some(if self.is_angry() {
            Sound::EntityZombifiedPiglinAngry
        } else {
            Sound::EntityZombifiedPiglinAmbient
        })
    }

    fn as_neutral(&self) -> Option<&dyn NeutralMob> {
        Some(self)
    }

    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.zombie
            .finalize_zombie_spawn(self, world, group_data, false)
    }

    /// Vanilla `ZombifiedPiglin.populateDefaultEquipmentSlots`.
    fn populate_default_equipment_slots(
        &self,
        world: &Arc<World>,
        _difficulty: &RegionalDifficulty,
    ) {
        // `level.getRandom()`, passed down from `finalizeSpawn`.
        let roll = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_bounded_i32(20);
        let weapon = if roll == 0 {
            &Item::GOLDEN_SPEAR
        } else {
            &Item::GOLDEN_SWORD
        };
        self.zombie
            .mob_entity
            .set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, weapon));
    }

    fn on_attack(&self, target: &dyn EntityBase) {
        self.zombie.on_attack(self, target);
    }

    /// Vanilla `ZombifiedPiglin.setTarget`: a fresh target restarts the anger sound and
    /// the alert countdowns.
    fn set_mob_target(&self, target: Option<Arc<dyn EntityBase>>) {
        if self.zombie.mob_entity.get_target().is_none() && target.is_some() {
            let (first_anger_sound_in, alert_in) = {
                let mut random = self.get_entity().random();
                (
                    sample(&FIRST_ANGER_SOUND_DELAY, &mut *random),
                    sample(&ALERT_INTERVAL, &mut *random),
                )
            };
            self.play_first_anger_sound_in
                .store(first_anger_sound_in, Ordering::Relaxed);
            self.ticks_until_next_alert
                .store(alert_in, Ordering::Relaxed);
        }
        set_mob_target_base(self, target);
    }

    /// Vanilla `ZombifiedPiglin.customServerAiStep`.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        let living = &self.zombie.mob_entity.living_entity;
        let has_attacking_modifier = || {
            living
                .attributes
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&Attributes::MOVEMENT_SPEED.id)
                .is_some_and(|instance| {
                    instance.get_modifier(SPEED_MODIFIER_ATTACKING_ID).is_some()
                })
        };
        if self.is_angry() {
            if !self.zombie.is_baby() && !has_attacking_modifier() {
                living.update_attribute(&Attributes::MOVEMENT_SPEED, |instance| {
                    instance.add_or_replace_modifier(Modifier {
                        id: SPEED_MODIFIER_ATTACKING_ID.to_string(),
                        amount: 0.05,
                        operation: ModifierOperation::Add,
                    });
                });
                send_attribute_updates_for_living(living, vec![Attributes::MOVEMENT_SPEED]);
            }
            self.maybe_play_first_anger_sound();
        } else if has_attacking_modifier() {
            living.update_attribute(&Attributes::MOVEMENT_SPEED, |instance| {
                instance.remove_modifier(SPEED_MODIFIER_ATTACKING_ID);
            });
            send_attribute_updates_for_living(living, vec![Attributes::MOVEMENT_SPEED]);
        }

        self.update_persistent_anger();
        if let Some(target) = self.zombie.mob_entity.get_target() {
            self.maybe_alert_others(&target);
        }
    }

    fn updates_anger_in_ai_step(&self) -> bool {
        true
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.zombie.write_zombie_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.zombie.read_zombie_nbt(self, nbt);
    }
}
