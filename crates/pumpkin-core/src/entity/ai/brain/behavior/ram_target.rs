use std::sync::Arc;

use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::{BlocksAttacksImpl, DamageTypeSet};
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::super::BrainTick;
use super::super::memory::walk_target::WalkTarget;
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::timed::Behavior;
use crate::enchantment::EnchantmentHelper;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::combat::knockback_after_resistance;
use crate::entity::damage_source::DamageSource;
use crate::entity::living::LivingEntity;
use crate::entity::mob::Mob;
use crate::entity::mob::piglin_ai::sample;
use crate::entity::passive::goat::GoatEntity;
use crate::entity::{Entity, EntityBase};

/// Vanilla `RamTarget.TIME_OUT_DURATION`.
const TIME_OUT_DURATION: i32 = 200;
/// Vanilla `RamTarget.RAM_SPEED_FORCE_FACTOR`.
const RAM_SPEED_FORCE_FACTOR: f32 = 1.65;
/// The registry name of vanilla `damageSources().mobAttack`, the knockback's source.
const MOB_ATTACK: &str = "mob_attack";

/// Vanilla `RamTarget`: charges at the `RAM_TARGET` spot, knocking back whatever it
/// meets, or snapping a horn on a block that breaks it.
pub struct RamTarget {
    get_time_between_rams: fn(&dyn Mob) -> (i32, i32),
    ram_targeting: TargetPredicate,
    speed: f32,
    get_knockback_force: fn(&dyn Mob) -> f64,
    ram_direction: Vector3<f64>,
    get_impact_sound: fn(&dyn Mob) -> Sound,
    get_horn_break_sound: fn(&dyn Mob) -> Sound,
    conditions: [(MemoryModuleId, MemoryStatus); 2],
}

impl RamTarget {
    #[must_use]
    pub const fn new(
        get_time_between_rams: fn(&dyn Mob) -> (i32, i32),
        ram_targeting: TargetPredicate,
        speed: f32,
        get_knockback_force: fn(&dyn Mob) -> f64,
        get_impact_sound: fn(&dyn Mob) -> Sound,
        get_horn_break_sound: fn(&dyn Mob) -> Sound,
    ) -> Self {
        Self {
            get_time_between_rams,
            ram_targeting,
            speed,
            get_knockback_force,
            ram_direction: Vector3::new(0.0, 0.0, 0.0),
            get_impact_sound,
            get_horn_break_sound,
            conditions: [
                (types::RAM_COOLDOWN_TICKS.id(), MemoryStatus::ValueAbsent),
                (types::RAM_TARGET.id(), MemoryStatus::ValuePresent),
            ],
        }
    }

    /// Vanilla `level.getNearbyEntities(LivingEntity.class, ramTargeting, body,
    /// body.getBoundingBox())`.
    fn nearby_entities(&self, tick: &BrainTick<'_>) -> Vec<Arc<dyn EntityBase>> {
        let bounding_box = tick.mob.get_entity().bounding_box.load();
        tick.world
            .get_all_at_box(&bounding_box)
            .into_iter()
            .filter(|entity| {
                entity.get_living_entity().is_some()
                    && self
                        .ram_targeting
                        .test(tick.world, Some(tick.mob), entity.as_ref())
            })
            .collect()
    }

    /// Vanilla `RamTarget.hasRammedHornBreakingBlock`.
    fn has_rammed_horn_breaking_block(tick: &BrainTick<'_>) -> bool {
        let entity = tick.mob.get_entity();
        let horizontal_movement_normalized =
            entity.velocity.load().multiply(1.0, 0.0, 1.0).normalize();
        let facing_block_position =
            BlockPos::floored_v(entity.pos.load() + horizontal_movement_normalized);
        tick.world
            .get_block(&facing_block_position)
            .has_tag(&tag::Block::MINECRAFT_SNAPS_GOAT_HORN)
            || tick
                .world
                .get_block(&facing_block_position.up())
                .has_tag(&tag::Block::MINECRAFT_SNAPS_GOAT_HORN)
    }

    /// Vanilla `RamTarget.finishRam`.
    fn finish_ram(&self, tick: &mut BrainTick<'_>) {
        tick.world
            .send_entity_status(tick.mob.get_entity(), EntityStatus::EndRam, None);
        let cooldown = sample((self.get_time_between_rams)(tick.mob), &mut rand::rng());
        tick.brain.set(types::RAM_COOLDOWN_TICKS, cooldown);
        tick.brain.erase(types::RAM_TARGET.id());
    }

    fn play_sound(tick: &BrainTick<'_>, sound: Sound) {
        tick.world.play_sound_fine(
            sound,
            SoundCategory::Neutral,
            &tick.mob.get_entity().pos.load(),
            1.0,
            1.0,
        );
    }
}

impl Behavior for RamTarget {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn min_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn max_duration(&self) -> i32 {
        TIME_OUT_DURATION
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        tick.brain.has_memory_value(types::RAM_TARGET.id())
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        tick.brain.has_memory_value(types::RAM_TARGET.id())
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        let Some(ram_target_pos) = tick.brain.get(types::RAM_TARGET).copied() else {
            return;
        };
        let cur_pos = tick.mob.get_entity().block_pos.load();
        self.ram_direction = Vector3::new(
            f64::from(cur_pos.0.x) - ram_target_pos.x,
            0.0,
            f64::from(cur_pos.0.z) - ram_target_pos.z,
        )
        .normalize();
        tick.brain.set(
            types::WALK_TARGET,
            WalkTarget::from_vec(ram_target_pos, self.speed, 0),
        );
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        if let Some(ram_target) = self.nearby_entities(tick).into_iter().next() {
            let living = &tick.mob.get_mob_entity().living_entity;
            let damage = living.get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32;
            let attacker: &dyn EntityBase = tick.mob;
            let damage_source = DamageSource {
                damage_type: DamageType::MOB_ATTACK_NO_AGGRO,
                direct: Some(attacker),
                causing: Some(attacker),
            };
            if ram_target.damage_with_context(
                ram_target.as_ref(),
                damage,
                damage_source.damage_type,
                None,
                damage_source.direct,
                damage_source.causing,
            ) {
                EnchantmentHelper::do_post_attack_effects(
                    tick.world,
                    ram_target.as_ref(),
                    &damage_source,
                );
            }

            let level_of = |effect: &'static StatusEffect| {
                living
                    .get_effect(effect)
                    .map_or(0, |effect| i32::from(effect.amplifier) + 1)
            };
            let movement_speed_level = level_of(&StatusEffect::SPEED);
            let movement_slowdown_level = level_of(&StatusEffect::SLOWNESS);
            let speed_boost_power = 0.25 * (movement_speed_level - movement_slowdown_level) as f32;
            let speed_factor =
                (living.speed.load() * RAM_SPEED_FORCE_FACTOR).clamp(0.2, 3.0) + speed_boost_power;
            if let Some(target_living) = ram_target.get_living_entity() {
                let blocked_damage =
                    apply_item_blocking(target_living, tick.mob.get_entity(), damage);
                let blocking_factor: f32 = if blocked_damage > 0.0 { 0.5 } else { 1.0 };
                let power = f64::from(blocking_factor * speed_factor)
                    * (self.get_knockback_force)(tick.mob);
                let resistance =
                    target_living.get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE);
                target_living.entity.apply_knockback(
                    knockback_after_resistance(power, resistance),
                    self.ram_direction.x,
                    self.ram_direction.z,
                );
            }
            self.finish_ram(tick);
            Self::play_sound(tick, (self.get_impact_sound)(tick.mob));
        } else if Self::has_rammed_horn_breaking_block(tick) {
            Self::play_sound(tick, (self.get_impact_sound)(tick.mob));
            let dropped = tick
                .mob
                .cast_any()
                .downcast_ref::<GoatEntity>()
                .is_some_and(GoatEntity::drop_horn);
            if dropped {
                Self::play_sound(tick, (self.get_horn_break_sound)(tick.mob));
            }
            self.finish_ram(tick);
        } else {
            let walk_target = tick
                .brain
                .get(types::WALK_TARGET)
                .map(|walk_target| walk_target.target().current_position());
            let ram_target = tick.brain.get(types::RAM_TARGET).copied();
            let lost_or_reached_target = match (walk_target, ram_target) {
                (Some(walk_target), Some(ram_target)) => {
                    walk_target.squared_distance_to_vec(&ram_target) < 0.25 * 0.25
                }
                _ => true,
            };
            if lost_or_reached_target {
                self.finish_ram(tick);
            }
        }
    }

    fn debug_name(&self) -> &'static str {
        "RamTarget"
    }
}

/// The damage vanilla `LivingEntity.applyItemBlocking` blocks of a `mob_attack` from
/// `attacker`.
// TODO: the rest of `applyItemBlocking`: damaging the blocking item, its block sound and
// `blockUsingItem`.
fn apply_item_blocking(target: &LivingEntity, attacker: &Entity, damage: f32) -> f32 {
    if damage <= 0.0 || !target.is_blocking() {
        return 0.0;
    }
    let blocks_attacks = target
        .item_in_use
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|item| item.get_data_component::<BlocksAttacksImpl>().cloned());
    let Some(blocks_attacks) = blocks_attacks else {
        return 0.0;
    };
    if blocks_attacks
        .bypassed_by
        .as_ref()
        .is_some_and(contains_mob_attack)
    {
        return 0.0;
    }
    let view_vector = Entity::calculate_view_vector(0.0, target.entity.head_yaw.load());
    let vector_to = attacker.pos.load() - target.entity.pos.load();
    let vector_to = Vector3::new(vector_to.x, 0.0, vector_to.z).normalize();
    let angle = vector_to.dot(&view_vector).acos();
    // Vanilla `BlocksAttacks.resolveBlockedDamage`.
    let blocked_damage: f32 = blocks_attacks
        .damage_reductions
        .iter()
        .map(|reduction| {
            let max_angle =
                (std::f64::consts::PI / 180.0) as f32 * reduction.horizontal_blocking_angle;
            if angle > f64::from(max_angle)
                || reduction
                    .r#type
                    .as_ref()
                    .is_some_and(|set| !contains_mob_attack(set))
            {
                0.0
            } else {
                (reduction.base + reduction.factor * damage).clamp(0.0, damage)
            }
        })
        .sum();
    blocked_damage.clamp(0.0, damage)
}

fn contains_mob_attack(set: &DamageTypeSet) -> bool {
    match set {
        DamageTypeSet::Tag(tag) => <DamageType as Taggable>::get_tag_values(tag)
            .is_some_and(|names| names.contains(&MOB_ATTACK)),
        DamageTypeSet::Entries(entries) => entries
            .iter()
            .any(|entry| entry.strip_prefix("minecraft:").unwrap_or(entry) == MOB_ATTACK),
    }
}
