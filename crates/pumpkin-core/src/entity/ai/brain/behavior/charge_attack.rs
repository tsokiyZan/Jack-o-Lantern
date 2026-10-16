use std::sync::Arc;

use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::sound::Sound;
use pumpkin_util::math::vector3::Vector3;

use super::super::BrainTick;
use super::super::memory::{MemoryModuleId, MemoryStatus, types};
use super::timed::Behavior;
use crate::enchantment::EnchantmentHelper;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::damage_source::DamageSource;
use crate::entity::passive::tamable::TamableAnimal;
use crate::entity::{EntityBase, mob::Mob};

/// Vanilla `ChargeAttack`: dashes in a straight line at the attack target, hurting and
/// knocking back the first valid target it touches.
pub struct ChargeAttack {
    time_between_attacks: i32,
    charge_targeting: TargetPredicate,
    speed: f32,
    knockback_force: f32,
    max_target_detection_distance: f64,
    max_charge_distance: f64,
    charge_sound: Sound,
    charge_velocity_vector: Vector3<f64>,
    start_position: Vector3<f64>,
    conditions: [(MemoryModuleId, MemoryStatus); 2],
}

impl ChargeAttack {
    #[must_use]
    pub const fn new(
        time_between_attacks: i32,
        charge_targeting: TargetPredicate,
        speed: f32,
        knockback_force: f32,
        max_charge_distance: f64,
        max_target_detection_distance: f64,
        charge_sound: Sound,
    ) -> Self {
        Self {
            time_between_attacks,
            charge_targeting,
            speed,
            knockback_force,
            max_target_detection_distance,
            max_charge_distance,
            charge_sound,
            charge_velocity_vector: Vector3::new(0.0, 0.0, 0.0),
            start_position: Vector3::new(0.0, 0.0, 0.0),
            conditions: [
                (types::CHARGE_COOLDOWN_TICKS.id(), MemoryStatus::ValueAbsent),
                (types::ATTACK_TARGET.id(), MemoryStatus::ValuePresent),
            ],
        }
    }

    /// Vanilla `level.getEntities(LivingEntity, body.getBoundingBox(), chargeTargeting, 1)`.
    fn first_colliding_entity(&self, tick: &BrainTick<'_>) -> Option<Arc<dyn EntityBase>> {
        let bounding_box = tick.mob.get_entity().bounding_box.load();
        tick.world
            .get_all_at_box(&bounding_box)
            .into_iter()
            .find(|entity| {
                entity.get_living_entity().is_some()
                    && self
                        .charge_targeting
                        .test(tick.world, Some(tick.mob), entity.as_ref())
            })
    }

    /// Vanilla `ChargeAttack.dealDamageToTarget`.
    fn deal_damage_to_target(tick: &BrainTick<'_>, target: &dyn EntityBase) {
        let damage = tick
            .mob
            .get_mob_entity()
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32;
        let body: &dyn EntityBase = tick.mob;
        let damage_source = DamageSource {
            damage_type: DamageType::MOB_ATTACK,
            direct: Some(body),
            causing: Some(body),
        };
        if target.damage_with_context(
            target,
            damage,
            damage_source.damage_type,
            None,
            damage_source.direct,
            damage_source.causing,
        ) {
            EnchantmentHelper::do_post_attack_effects(tick.world, target, &damage_source);
        }
    }

    /// Vanilla `ChargeAttack.dealKnockBack`.
    fn deal_knock_back(&self, tick: &BrainTick<'_>, target: &dyn EntityBase) {
        let living = &tick.mob.get_mob_entity().living_entity;
        let level_of = |effect: &'static StatusEffect| {
            living
                .get_effect(effect)
                .map_or(0, |effect| i32::from(effect.amplifier) + 1)
        };
        let movement_speed_level = level_of(&StatusEffect::SPEED);
        let movement_slowdown_level = level_of(&StatusEffect::SLOWNESS);
        let speed_boost_power = 0.25 * (movement_speed_level - movement_slowdown_level) as f32;
        let speed_factor = (self.speed
            * living.get_attribute_value(&Attributes::MOVEMENT_SPEED) as f32)
            .clamp(0.2, 2.0)
            + speed_boost_power;
        let damage = living.get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32;
        cause_extra_knockback(
            tick.mob,
            target,
            speed_factor * self.knockback_force,
            damage,
        );
    }
}

/// Vanilla `LivingEntity.causeExtraKnockback` of a `mob_attack` that dealt `damage`: pushes
/// `target` along the attacker's facing and slows the attacker.
fn cause_extra_knockback(attacker: &dyn Mob, target: &dyn EntityBase, knockback: f32, damage: f32) {
    let Some(target_living) = target.get_living_entity() else {
        return;
    };
    if knockback <= 0.0 {
        return;
    }
    let entity = attacker.get_entity();
    let y_rot = entity.yaw.load() * (std::f64::consts::PI / 180.0) as f32;
    target_living.knockback(
        target,
        f64::from(knockback),
        f64::from(pumpkin_util::math::sin(y_rot)),
        f64::from(-pumpkin_util::math::cos(y_rot)),
        Some(attacker),
        damage,
        false,
    );
    entity
        .velocity
        .store(entity.velocity.load().multiply(0.6, 1.0, 0.6));
}

impl Behavior for ChargeAttack {
    fn entry_conditions(&self) -> &[(MemoryModuleId, MemoryStatus)] {
        &self.conditions
    }

    fn check_extra_start_conditions(&mut self, tick: &mut BrainTick<'_>) -> bool {
        tick.brain.has_memory_value(types::ATTACK_TARGET.id())
    }

    fn can_still_use(&mut self, tick: &BrainTick<'_>) -> bool {
        let Some(attack_target) = tick.brain.get(types::ATTACK_TARGET) else {
            return false;
        };
        if tick.mob.as_tamable().is_some_and(TamableAnimal::is_tame) {
            return false;
        }
        let body = tick.mob.get_entity();
        let body_pos = body.pos.load();
        let target = attack_target.get_entity();
        (body_pos - self.start_position).length_squared()
            < self.max_charge_distance * self.max_charge_distance
            && (target.pos.load() - body_pos).length_squared()
                < self.max_target_detection_distance * self.max_target_detection_distance
            && body.has_line_of_sight(target)
            && !tick
                .brain
                .has_memory_value(types::CHARGE_COOLDOWN_TICKS.id())
    }

    fn start(&mut self, tick: &mut BrainTick<'_>) {
        let Some(attack_target) = tick.brain.get(types::ATTACK_TARGET) else {
            return;
        };
        let body_pos = tick.mob.get_entity().pos.load();
        self.start_position = body_pos;
        let direction = (attack_target.get_entity().pos.load() - body_pos).normalize();
        self.charge_velocity_vector = direction * f64::from(self.speed);
        if self.can_still_use(tick) {
            tick.mob.play_sound_at_self(self.charge_sound, 1.0, 1.0);
        }
    }

    fn tick(&mut self, tick: &mut BrainTick<'_>) {
        let Some(attack_target) = tick.brain.get(types::ATTACK_TARGET).map(Arc::clone) else {
            return;
        };
        let body = tick.mob.get_mob_entity();
        body.look_at(attack_target.as_ref(), 360.0, 360.0);
        body.living_entity
            .entity
            .velocity
            .store(self.charge_velocity_vector);
        if let Some(closest_attack_target) = self.first_colliding_entity(tick) {
            if body
                .living_entity
                .entity
                .has_passenger(closest_attack_target.get_entity().entity_id)
            {
                return;
            }
            Self::deal_damage_to_target(tick, closest_attack_target.as_ref());
            self.deal_knock_back(tick, closest_attack_target.as_ref());
            // Vanilla calls `stop` here; the behaviour stops again once the target is gone.
            self.stop(tick);
        }
    }

    fn stop(&mut self, tick: &mut BrainTick<'_>) {
        tick.brain
            .set(types::CHARGE_COOLDOWN_TICKS, self.time_between_attacks);
        tick.brain.erase(types::ATTACK_TARGET.id());
    }

    fn debug_name(&self) -> &'static str {
        "ChargeAttack"
    }
}
