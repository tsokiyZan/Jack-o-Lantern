use crate::entity::EntityBase;
use crate::entity::effect::MobEffectInstance;
use crate::entity::living::LivingEntity;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Effect;

/// Utilities for reading potion contents from an `ItemStack` and applying effects.
pub struct PotionContents;

/// How a potion's effects reach an entity: each vanilla caller treats instantaneous effects
/// and durations its own way.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PotionApplicationSource {
    /// `PotionContents.applyToLivingEntity`: drinking. Instant effects apply at full strength.
    Drink,
    /// `ThrownSplashPotion.onHitAsPotion`, with the distance `scale`.
    Splash(f64),
    /// `AreaEffectCloud.serverTick`: instant effects at half strength.
    AreaEffectCloud,
    /// `Arrow.doPostHurtEffects` and `Witch.aiStep`: every effect goes through `addEffect`,
    /// instant ones included.
    AddEffect,
}

/// Vanilla `AreaEffectCloud`'s scale for instantaneous effects.
const AREA_EFFECT_CLOUD_INSTANT_SCALE: f64 = 0.5;
/// Vanilla `ThrownSplashPotion`: a splashed effect shorter than this is dropped.
const SPLASH_MIN_DURATION: i32 = 20;

impl PotionContents {
    /// Vanilla `PotionContents.getColor`: the custom color, else the visible effects' colors
    /// weighted by amplifier + 1 (`getColorOptional`, opaque), else -13083194.
    #[must_use]
    pub fn color(stack: &ItemStack) -> i32 {
        if let Some(color) = stack
            .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
            .and_then(|contents| contents.custom_color)
        {
            return color;
        }
        let (mut red, mut green, mut blue, mut total_weight) = (0i32, 0i32, 0i32, 0i32);
        for (effect, _, amplifier, _, visible, _) in Self::read_potion_effects(stack) {
            if visible {
                let color = effect.color;
                let amplifier = i32::from(amplifier) + 1;
                red += amplifier * ((color >> 16) & 0xFF);
                green += amplifier * ((color >> 8) & 0xFF);
                blue += amplifier * (color & 0xFF);
                total_weight += amplifier;
            }
        }
        if total_weight == 0 {
            -13_083_194
        } else {
            // `ARGB.color(r, g, b)`: alpha 255.
            (0xFF << 24)
                | ((red / total_weight) << 16)
                | ((green / total_weight) << 8)
                | (blue / total_weight)
        }
    }

    /// Vanilla `PotionContents.hasEffects`: custom effects, or a potion with effects.
    #[must_use]
    pub fn has_effects(stack: &ItemStack) -> bool {
        !Self::read_potion_effects(stack).is_empty()
    }

    /// Vanilla `potion().isPresent() && potion().get().value().hasInstantEffects()`: only the
    /// base potion's effects, not the custom ones.
    #[must_use]
    pub fn potion_has_instant_effects(stack: &ItemStack) -> bool {
        stack
            .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
            .and_then(|contents| contents.potion_id)
            .and_then(|id| u8::try_from(id).ok())
            .and_then(pumpkin_data::potion::Potion::from_id)
            .is_some_and(|potion| {
                potion
                    .effects
                    .iter()
                    .any(|effect| crate::entity::effect::is_instantaneous(effect.effect_type))
            })
    }

    /// Read effects from an `ItemStack`'s `PotionContents` data component.
    #[must_use]
    pub fn read_potion_effects(
        stack: &ItemStack,
    ) -> Vec<(&'static StatusEffect, i32, u8, bool, bool, bool)> {
        // Prefer generated potion id if present, otherwise use custom_effects
        if let Some(pc) =
            stack.get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
        {
            // Custom effects present
            let mut out = Vec::new();
            if let Some(potion_id) = pc.potion_id {
                // Map potion id to generated Potion if possible
                macro_rules! try_push_potion {
                    ($p:expr) => {
                        if $p.id as i32 == potion_id {
                            for e in $p.effects {
                                out.push((
                                    e.effect_type,
                                    e.duration,
                                    e.amplifier,
                                    e.ambient,
                                    e.show_particles,
                                    e.show_icon,
                                ));
                            }
                        }
                    };
                }
                try_push_potion!(pumpkin_data::potion::Potion::AWKWARD);
                try_push_potion!(pumpkin_data::potion::Potion::FIRE_RESISTANCE);
                try_push_potion!(pumpkin_data::potion::Potion::HARMING);
                try_push_potion!(pumpkin_data::potion::Potion::HEALING);
                try_push_potion!(pumpkin_data::potion::Potion::INFESTED);
                try_push_potion!(pumpkin_data::potion::Potion::INVISIBILITY);
                try_push_potion!(pumpkin_data::potion::Potion::LEAPING);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_FIRE_RESISTANCE);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_INVISIBILITY);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_LEAPING);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_NIGHT_VISION);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_POISON);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_REGENERATION);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_SLOW_FALLING);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_SLOWNESS);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_STRENGTH);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_SWIFTNESS);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_TURTLE_MASTER);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_WATER_BREATHING);
                try_push_potion!(pumpkin_data::potion::Potion::LONG_WEAKNESS);
                try_push_potion!(pumpkin_data::potion::Potion::LUCK);
                try_push_potion!(pumpkin_data::potion::Potion::MUNDANE);
                try_push_potion!(pumpkin_data::potion::Potion::NIGHT_VISION);
                try_push_potion!(pumpkin_data::potion::Potion::OOZING);
                try_push_potion!(pumpkin_data::potion::Potion::POISON);
                try_push_potion!(pumpkin_data::potion::Potion::REGENERATION);
                try_push_potion!(pumpkin_data::potion::Potion::SLOW_FALLING);
                try_push_potion!(pumpkin_data::potion::Potion::SLOWNESS);
                try_push_potion!(pumpkin_data::potion::Potion::STRENGTH);
                try_push_potion!(pumpkin_data::potion::Potion::STRONG_HARMING);
                try_push_potion!(pumpkin_data::potion::Potion::STRONG_HEALING);
                try_push_potion!(pumpkin_data::potion::Potion::STRONG_LEAPING);
                try_push_potion!(pumpkin_data::potion::Potion::STRONG_POISON);
                try_push_potion!(pumpkin_data::potion::Potion::STRONG_REGENERATION);
                try_push_potion!(pumpkin_data::potion::Potion::STRONG_SLOWNESS);
                try_push_potion!(pumpkin_data::potion::Potion::STRONG_STRENGTH);
                try_push_potion!(pumpkin_data::potion::Potion::STRONG_SWIFTNESS);
                try_push_potion!(pumpkin_data::potion::Potion::STRONG_TURTLE_MASTER);
                try_push_potion!(pumpkin_data::potion::Potion::SWIFTNESS);
                try_push_potion!(pumpkin_data::potion::Potion::THICK);
                try_push_potion!(pumpkin_data::potion::Potion::TURTLE_MASTER);
                try_push_potion!(pumpkin_data::potion::Potion::WATER);
                try_push_potion!(pumpkin_data::potion::Potion::WATER_BREATHING);
                try_push_potion!(pumpkin_data::potion::Potion::WEAKNESS);
                try_push_potion!(pumpkin_data::potion::Potion::WEAVING);
                try_push_potion!(pumpkin_data::potion::Potion::WIND_CHARGED);
            }

            // Custom effects appended
            for ce in &pc.custom_effects {
                if let Some(se) = StatusEffect::from_minecraft_name(&ce.effect_id) {
                    out.push((
                        se,
                        ce.duration,
                        ce.amplifier as u8,
                        ce.ambient,
                        ce.show_particles,
                        ce.show_icon,
                    ));
                }
            }

            return out;
        }

        Vec::new()
    }

    /// Applies `effects` to `target` the way `source` does. `duration_scale` is the item's or
    /// cloud's `potion_duration_scale`; `potion_entity` and `owner` make the indirect magic of
    /// instant damage.
    pub fn apply_effects_to(
        target: &LivingEntity,
        effects: Vec<(&'static StatusEffect, i32, u8, bool, bool, bool)>,
        duration_scale: f32,
        source: PotionApplicationSource,
        potion_entity: Option<&dyn EntityBase>,
        owner: Option<&dyn EntityBase>,
    ) {
        for (effect_type, duration, amplifier, ambient, show_particles, show_icon) in effects {
            let effect = MobEffectInstance::from(Effect {
                effect_type,
                duration,
                amplifier,
                ambient,
                show_particles,
                show_icon,
                blend: false,
            });
            let instant = crate::entity::effect::get_mob_effect(effect_type)
                .filter(|mob_effect| mob_effect.is_instantaneous());
            match source {
                PotionApplicationSource::Drink => {
                    let effect = effect.with_scaled_duration(duration_scale);
                    if let Some(instant) = instant {
                        // The drinker is both source and owner when it's a player.
                        let player = target.is_player().then(|| {
                            target
                                .entity
                                .world
                                .load()
                                .get_entity_by_id(target.entity.entity_id)
                        });
                        let player = player.flatten();
                        instant.apply_instantaneous_effect(
                            player.as_deref(),
                            player.as_deref(),
                            target,
                            amplifier,
                            1.0,
                        );
                    } else {
                        target.add_effect(effect.effect);
                    }
                }
                PotionApplicationSource::Splash(scale) => {
                    if let Some(instant) = instant {
                        instant.apply_instantaneous_effect(
                            potion_entity,
                            owner,
                            target,
                            amplifier,
                            scale,
                        );
                    } else {
                        let duration = effect.map_duration(|duration| {
                            (scale * f64::from(duration) * f64::from(duration_scale) + 0.5) as i32
                        });
                        // `new MobEffectInstance(effect, duration, amplifier, ambient, visible)`.
                        let new_effect = MobEffectInstance::from(Effect {
                            effect_type,
                            duration,
                            amplifier,
                            ambient,
                            show_particles,
                            show_icon: show_particles,
                            blend: false,
                        });
                        if !new_effect.ends_within(SPLASH_MIN_DURATION) {
                            target.add_effect(new_effect.effect);
                        }
                    }
                }
                PotionApplicationSource::AreaEffectCloud => {
                    if let Some(instant) = instant {
                        instant.apply_instantaneous_effect(
                            potion_entity,
                            owner,
                            target,
                            amplifier,
                            AREA_EFFECT_CLOUD_INSTANT_SCALE,
                        );
                    } else {
                        target.add_effect(effect.with_scaled_duration(duration_scale).effect);
                    }
                }
                PotionApplicationSource::AddEffect => {
                    target.add_effect(effect.with_scaled_duration(duration_scale).effect);
                }
            }
        }
    }
}
