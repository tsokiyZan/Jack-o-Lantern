pub mod absorption;
pub mod heal_or_harm;
pub mod hunger;
pub mod infested;
pub mod oozing;
pub mod poison;
pub mod raid_omen;
pub mod regeneration;
pub mod saturation;
pub mod weaving;
pub mod wind_charged;
pub mod wither;

use std::ops::{Deref, DerefMut};

use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::particle::Particle;
use pumpkin_data::potion::Effect;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use tracing::warn;

use crate::entity::living::LivingEntity;
use crate::entity::{EntityBase, NBTStorage, NBTStorageInit};

pub trait MobEffect: Send + Sync {
    /// Vanilla `shouldApplyEffectTickThisTick`: `tick_count` is the remaining duration, or the
    /// entity's tick count for an infinite effect.
    fn should_apply_effect_tick(&self, _tick_count: i32, _amplifier: u8) -> bool {
        false
    }

    /// Vanilla `applyEffectTick`: false ends the effect right away.
    fn apply_effect_tick(&self, _living: &LivingEntity, _amplifier: u8) -> bool {
        true
    }

    /// Vanilla `isInstantaneous`.
    fn is_instantaneous(&self) -> bool {
        false
    }

    /// Vanilla `applyInstantaneousEffect`: a splash, cloud or drink applying the effect at
    /// once. `source` is the potion entity and `owner` whoever threw it.
    fn apply_instantaneous_effect(
        &self,
        _source: Option<&dyn EntityBase>,
        _owner: Option<&dyn EntityBase>,
        living: &LivingEntity,
        amplifier: u8,
        _scale: f64,
    ) {
        self.apply_effect_tick(living, amplifier);
    }

    /// Vanilla `onEffectStarted`: runs for every `addEffect`, whether or not it changed anything.
    fn on_effect_started(&self, _living: &LivingEntity, _amplifier: u8) {}

    /// Called when an entity carrying this effect is hurt.
    fn on_mob_hurt(
        &self,
        _living: &LivingEntity,
        _amplifier: u8,
        _damage_type: &DamageType,
        _damage_amount: f32,
    ) {
    }

    /// Vanilla `onMobRemoved` with `RemovalReason.KILLED`: the entity carrying this effect
    /// is removed after dying (a mob when its death animation ends, a player on respawn).
    fn on_mob_death(&self, _living: &LivingEntity, _amplifier: u8) {}
}

pub static ABSORPTION: absorption::AbsorptionMobEffect = absorption::AbsorptionMobEffect;
pub static INSTANT_HEALTH: heal_or_harm::HealOrHarmMobEffect =
    heal_or_harm::HealOrHarmMobEffect { is_harm: false };
pub static INSTANT_DAMAGE: heal_or_harm::HealOrHarmMobEffect =
    heal_or_harm::HealOrHarmMobEffect { is_harm: true };
pub static REGENERATION: regeneration::RegenerationMobEffect = regeneration::RegenerationMobEffect;
pub static POISON: poison::PoisonMobEffect = poison::PoisonMobEffect;
pub static WITHER: wither::WitherMobEffect = wither::WitherMobEffect;
pub static HUNGER: hunger::HungerMobEffect = hunger::HungerMobEffect;
pub static SATURATION: saturation::SaturationMobEffect = saturation::SaturationMobEffect;
pub static RAID_OMEN: raid_omen::RaidOmenMobEffect = raid_omen::RaidOmenMobEffect;
pub static INFESTED: infested::InfestedMobEffect = infested::InfestedMobEffect;
pub static OOZING: oozing::OozingMobEffect = oozing::OozingMobEffect;
pub static WEAVING: weaving::WeavingMobEffect = weaving::WeavingMobEffect;
pub static WIND_CHARGED: wind_charged::WindChargedMobEffect = wind_charged::WindChargedMobEffect;

#[must_use]
pub fn get_mob_effect(effect: &'static StatusEffect) -> Option<&'static dyn MobEffect> {
    if effect == &StatusEffect::ABSORPTION {
        Some(&ABSORPTION)
    } else if effect == &StatusEffect::INSTANT_HEALTH {
        Some(&INSTANT_HEALTH)
    } else if effect == &StatusEffect::INSTANT_DAMAGE {
        Some(&INSTANT_DAMAGE)
    } else if effect == &StatusEffect::REGENERATION {
        Some(&REGENERATION)
    } else if effect == &StatusEffect::POISON {
        Some(&POISON)
    } else if effect == &StatusEffect::WITHER {
        Some(&WITHER)
    } else if effect == &StatusEffect::HUNGER {
        Some(&HUNGER)
    } else if effect == &StatusEffect::SATURATION {
        Some(&SATURATION)
    } else if effect == &StatusEffect::RAID_OMEN {
        Some(&RAID_OMEN)
    } else if effect == &StatusEffect::INFESTED {
        Some(&INFESTED)
    } else if effect == &StatusEffect::OOZING {
        Some(&OOZING)
    } else if effect == &StatusEffect::WEAVING {
        Some(&WEAVING)
    } else if effect == &StatusEffect::WIND_CHARGED {
        Some(&WIND_CHARGED)
    } else {
        None
    }
}

/// The `interval >> amplification` check of poison, wither and regeneration. Java masks the
/// shift count to five bits.
pub(crate) const fn interval_tick(interval: i32, tick_count: i32, amplifier: u8) -> bool {
    let interval = interval.wrapping_shr(amplifier as u32);
    interval <= 0 || tick_count % interval == 0
}

/// Vanilla `MobEffect.isInstantaneous` for any effect type.
#[must_use]
pub fn is_instantaneous(effect: &'static StatusEffect) -> bool {
    get_mob_effect(effect).is_some_and(MobEffect::is_instantaneous)
}

/// The particle vanilla's `MobEffects` registers each effect with: `entity_effect` tinted by
/// the effect colour unless the effect names its own.
#[must_use]
pub fn effect_particle(effect: &'static StatusEffect) -> Option<Particle> {
    if effect == &StatusEffect::INFESTED {
        Some(Particle::Infested)
    } else if effect == &StatusEffect::WEAVING {
        Some(Particle::ItemCobweb)
    } else if effect == &StatusEffect::OOZING {
        Some(Particle::ItemSlime)
    } else if effect == &StatusEffect::WIND_CHARGED {
        Some(Particle::SmallGust)
    } else if effect == &StatusEffect::TRIAL_OMEN {
        Some(Particle::TrialOmen)
    } else if effect == &StatusEffect::RAID_OMEN {
        Some(Particle::RaidOmen)
    } else {
        None
    }
}

/// Vanilla `MobEffect.soundOnAdded`.
#[must_use]
pub fn sound_on_added(effect: &'static StatusEffect) -> Option<Sound> {
    if effect == &StatusEffect::BAD_OMEN {
        Some(Sound::EventMobEffectBadOmen)
    } else if effect == &StatusEffect::TRIAL_OMEN {
        Some(Sound::EventMobEffectTrialOmen)
    } else if effect == &StatusEffect::RAID_OMEN {
        Some(Sound::EventMobEffectRaidOmen)
    } else {
        None
    }
}

/// Vanilla `MobEffectInstance`: an effect plus the weaker one it hides, which takes over when
/// it runs out.
#[derive(Clone)]
pub struct MobEffectInstance {
    pub effect: Effect,
    pub hidden_effect: Option<Box<Self>>,
}

impl Deref for MobEffectInstance {
    type Target = Effect;

    fn deref(&self) -> &Effect {
        &self.effect
    }
}

impl DerefMut for MobEffectInstance {
    fn deref_mut(&mut self) -> &mut Effect {
        &mut self.effect
    }
}

impl From<Effect> for MobEffectInstance {
    fn from(effect: Effect) -> Self {
        Self {
            effect,
            hidden_effect: None,
        }
    }
}

impl MobEffectInstance {
    /// Vanilla `MobEffectInstance.INFINITE_DURATION`.
    pub const INFINITE_DURATION: i32 = -1;

    /// The copy constructor: the details without the hidden effect.
    fn copy(&self) -> Self {
        Self::from(self.effect.clone())
    }

    /// Vanilla `setDetailsFrom`.
    fn set_details_from(&mut self, other: &Self) {
        self.effect.duration = other.duration;
        self.effect.amplifier = other.amplifier;
        self.effect.ambient = other.ambient;
        self.effect.show_particles = other.show_particles;
        self.effect.show_icon = other.show_icon;
    }

    /// Vanilla `update`: merges `take_over` into this effect, keeping a weaker but longer one
    /// as the hidden effect. Returns whether this effect changed.
    pub fn update(&mut self, take_over: &Self) -> bool {
        let mut changed = false;
        if take_over.amplifier > self.amplifier {
            if take_over.is_shorter_duration_than(self) {
                let prev_hidden = self.hidden_effect.take();
                let mut hidden = self.copy();
                hidden.hidden_effect = prev_hidden;
                self.hidden_effect = Some(Box::new(hidden));
            }
            self.effect.amplifier = take_over.amplifier;
            self.effect.duration = take_over.duration;
            changed = true;
        } else if self.is_shorter_duration_than(take_over) {
            if take_over.amplifier == self.amplifier {
                self.effect.duration = take_over.duration;
                changed = true;
            } else if let Some(hidden) = &mut self.hidden_effect {
                hidden.update(take_over);
            } else {
                self.hidden_effect = Some(Box::new(take_over.copy()));
            }
        }

        if !take_over.ambient && self.ambient || changed {
            self.effect.ambient = take_over.ambient;
            changed = true;
        }
        if take_over.show_particles != self.show_particles {
            self.effect.show_particles = take_over.show_particles;
            changed = true;
        }
        if take_over.show_icon != self.show_icon {
            self.effect.show_icon = take_over.show_icon;
            changed = true;
        }
        changed
    }

    fn is_shorter_duration_than(&self, other: &Self) -> bool {
        !self.is_infinite_duration()
            && (self.duration < other.duration || other.is_infinite_duration())
    }

    #[must_use]
    pub const fn is_infinite_duration(&self) -> bool {
        self.effect.duration == Self::INFINITE_DURATION
    }

    /// Vanilla `endsWithin`.
    #[must_use]
    pub const fn ends_within(&self, ticks: i32) -> bool {
        !self.is_infinite_duration() && self.effect.duration <= ticks
    }

    /// Vanilla `mapDuration`: infinite and zero durations are left alone.
    #[must_use]
    pub fn map_duration(&self, mapper: impl FnOnce(i32) -> i32) -> i32 {
        if self.is_infinite_duration() || self.effect.duration == 0 {
            self.effect.duration
        } else {
            mapper(self.effect.duration)
        }
    }

    /// Vanilla `withScaledDuration`.
    #[must_use]
    pub fn with_scaled_duration(&self, scale: f32) -> Self {
        let mut copy = self.copy();
        copy.effect.duration =
            self.map_duration(|duration| ((duration as f32 * scale).floor() as i32).max(1));
        copy
    }

    /// Vanilla `hasRemainingDuration`.
    #[must_use]
    pub const fn has_remaining_duration(&self) -> bool {
        self.is_infinite_duration() || self.effect.duration > 0
    }

    /// Vanilla `tickDownDuration`: the hidden effects run down too.
    pub fn tick_down_duration(&mut self) {
        if let Some(hidden) = &mut self.hidden_effect {
            hidden.tick_down_duration();
        }
        self.effect.duration = self.map_duration(|duration| duration - 1);
    }

    /// Vanilla `downgradeToHiddenEffect`: the hidden effect takes over once this one ends.
    pub fn downgrade_to_hidden_effect(&mut self) -> bool {
        if self.effect.duration == 0
            && let Some(hidden) = self.hidden_effect.take()
        {
            self.set_details_from(&hidden);
            self.hidden_effect = hidden.hidden_effect;
            true
        } else {
            false
        }
    }
}

/// Vanilla `MobEffectInstance.Details` codec fields, shared by the instance and its hidden
/// effects (which leave out `id`).
fn write_details(instance: &MobEffectInstance, nbt: &mut NbtCompound) {
    let effect = &instance.effect;
    if effect.amplifier != 0 {
        // `ExtraCodecs.UNSIGNED_BYTE`.
        nbt.put("amplifier", NbtTag::Byte(effect.amplifier as i8));
    }
    if effect.duration != 0 {
        nbt.put("duration", NbtTag::Int(effect.duration));
    }
    if effect.ambient {
        nbt.put("ambient", NbtTag::Byte(1));
    }
    if !effect.show_particles {
        nbt.put("show_particles", NbtTag::Byte(0));
    }
    nbt.put("show_icon", NbtTag::Byte(i8::from(effect.show_icon)));
    if let Some(hidden) = &instance.hidden_effect {
        let mut hidden_nbt = NbtCompound::new();
        write_details(hidden, &mut hidden_nbt);
        nbt.put("hidden_effect", hidden_nbt);
    }
}

fn read_details(effect_type: &'static StatusEffect, nbt: &NbtCompound) -> MobEffectInstance {
    // NBT numbers of any type decode, as through `NbtOps`.
    let number = |name: &str| nbt.get(name).and_then(NbtTag::as_numeric_int);
    let amplifier = number("amplifier").unwrap_or(0) as u8;
    let duration = number("duration").unwrap_or(0);
    let ambient = number("ambient").is_some_and(|v| v != 0);
    let show_particles = number("show_particles").is_none_or(|v| v != 0);
    // `Details`: a missing `show_icon` follows `show_particles`.
    let show_icon = number("show_icon").map_or(show_particles, |v| v != 0);
    MobEffectInstance {
        effect: Effect {
            effect_type,
            duration,
            amplifier,
            ambient,
            show_particles,
            show_icon,
            blend: false,
        },
        hidden_effect: nbt
            .get_compound("hidden_effect")
            .map(|hidden| Box::new(read_details(effect_type, hidden))),
    }
}

/// Vanilla `MobEffectInstance.CODEC`: optional fields are left out at their defaults, except
/// `show_icon`, which is always written.
impl NBTStorage for MobEffectInstance {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put("id", self.effect.effect_type.minecraft_name);
        write_details(self, nbt);
    }
}

impl NBTStorageInit for MobEffectInstance {
    fn create_from_nbt(nbt: &mut NbtCompound) -> Option<Self> {
        let Some(effect_id) = nbt.get_string("id") else {
            warn!("Unable to read effect. Effect id is not present");
            return None;
        };
        let Some(effect_type) = StatusEffect::from_minecraft_name(effect_id) else {
            warn!("Unable to read effect. Unknown effect type: {effect_id}");
            return None;
        };
        Some(read_details(effect_type, nbt))
    }
}

/// The plain codec for an `Effect` alone, as item components and block entities keep them.
impl NBTStorage for Effect {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        MobEffectInstance::from(self.clone()).write_nbt(nbt);
    }
}

impl NBTStorageInit for Effect {
    fn create_from_nbt(nbt: &mut NbtCompound) -> Option<Self> {
        MobEffectInstance::create_from_nbt(nbt).map(|instance| instance.effect)
    }
}

#[cfg(test)]
mod tests {
    use super::MobEffectInstance;
    use pumpkin_data::effect::StatusEffect;
    use pumpkin_data::potion::Effect;

    fn speed(amplifier: u8, duration: i32) -> MobEffectInstance {
        MobEffectInstance::from(Effect {
            effect_type: &StatusEffect::SPEED,
            duration,
            amplifier,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        })
    }

    #[test]
    fn stronger_shorter_effect_hides_the_weaker_one_until_it_ends() {
        let mut active = speed(0, 600);
        assert!(active.update(&speed(1, 100)));
        assert_eq!((active.amplifier, active.duration), (1, 100));
        let hidden = active
            .hidden_effect
            .as_ref()
            .map(|h| (h.amplifier, h.duration));
        assert_eq!(hidden, Some((0, 600)));

        for _ in 0..100 {
            active.tick_down_duration();
        }
        assert!(active.downgrade_to_hidden_effect());
        assert_eq!((active.amplifier, active.duration), (0, 500));
        assert!(active.hidden_effect.is_none());
    }

    #[test]
    fn weaker_longer_effect_is_hidden_and_does_not_count_as_a_change() {
        let mut active = speed(1, 100);
        assert!(!active.update(&speed(0, 600)));
        assert_eq!((active.amplifier, active.duration), (1, 100));
        assert_eq!(
            active
                .hidden_effect
                .as_ref()
                .map(|h| (h.amplifier, h.duration)),
            Some((0, 600))
        );
        // A weaker, shorter one is dropped.
        let mut active = speed(1, 600);
        assert!(!active.update(&speed(0, 100)));
        assert!(active.hidden_effect.is_none());
    }

    #[test]
    fn same_amplifier_only_extends() {
        let mut active = speed(0, 600);
        assert!(!active.update(&speed(0, 100)));
        assert_eq!(active.duration, 600);
        assert!(active.update(&speed(0, MobEffectInstance::INFINITE_DURATION)));
        assert!(active.is_infinite_duration());
    }

    #[test]
    fn scaled_duration_floors_and_keeps_at_least_one_tick() {
        assert_eq!(speed(0, 160).with_scaled_duration(0.125).duration, 20);
        assert_eq!(speed(0, 3).with_scaled_duration(0.125).duration, 1);
        assert_eq!(
            speed(0, MobEffectInstance::INFINITE_DURATION)
                .with_scaled_duration(0.125)
                .duration,
            MobEffectInstance::INFINITE_DURATION
        );
    }
}
