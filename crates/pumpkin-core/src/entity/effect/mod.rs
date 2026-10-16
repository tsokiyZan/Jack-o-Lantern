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

use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use tracing::warn;

use crate::entity::living::LivingEntity;
use crate::entity::{NBTStorage, NBTStorageInit};

pub trait MobEffect: Send + Sync {
    /// Returns true if `apply_effect_tick` should be called for the current tick and duration.
    fn should_apply_effect_tick(&self, _duration: i32, _amplifier: u8) -> bool {
        false
    }

    /// Applies periodic/tick-based effect logic on a living entity.
    fn apply_effect_tick(&self, _living: &LivingEntity, _amplifier: u8) {}

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
    if effect == &StatusEffect::REGENERATION {
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

/// Vanilla `MobEffectInstance.CODEC`: optional fields are left out at their defaults, except
/// `show_icon`, which is always written.
impl NBTStorage for pumpkin_data::potion::Effect {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put("id", self.effect_type.minecraft_name);
        if self.amplifier != 0 {
            // `ExtraCodecs.UNSIGNED_BYTE`.
            nbt.put("amplifier", NbtTag::Byte(self.amplifier as i8));
        }
        if self.duration != 0 {
            nbt.put("duration", NbtTag::Int(self.duration));
        }
        if self.ambient {
            nbt.put("ambient", NbtTag::Byte(1));
        }
        if !self.show_particles {
            nbt.put("show_particles", NbtTag::Byte(0));
        }
        nbt.put("show_icon", NbtTag::Byte(i8::from(self.show_icon)));
    }
}

impl NBTStorageInit for pumpkin_data::potion::Effect {
    fn create_from_nbt(nbt: &mut NbtCompound) -> Option<Self> {
        let Some(effect_id) = nbt.get_string("id") else {
            warn!("Unable to read effect. Effect id is not present");
            return None;
        };
        let Some(effect_type) = StatusEffect::from_minecraft_name(effect_id) else {
            warn!("Unable to read effect. Unknown effect type: {effect_id}");
            return None;
        };
        // NBT numbers of any type decode, as through `NbtOps`.
        let number = |name: &str| nbt.get(name).and_then(NbtTag::as_numeric_int);
        let amplifier = number("amplifier").unwrap_or(0) as u8;
        let duration = number("duration").unwrap_or(0);
        let ambient = number("ambient").is_some_and(|v| v != 0);
        let show_particles = number("show_particles").is_none_or(|v| v != 0);
        // `Details`: a missing `show_icon` follows `show_particles`.
        let show_icon = number("show_icon").map_or(show_particles, |v| v != 0);
        // TODO: vanilla `hidden_effect` (a weaker effect kept under a stronger one).
        Some(Self {
            effect_type,
            duration,
            amplifier,
            ambient,
            show_particles,
            show_icon,
            blend: false,
        })
    }
}
