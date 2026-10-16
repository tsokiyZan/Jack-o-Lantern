use std::sync::Arc;

use pumpkin_data::enchantment::EnchantmentEntityEffect;
use pumpkin_util::math::vector3::Vector3;

use super::{
    AllOf, ApplyEntityImpulse, ApplyExhaustion, ApplyMobEffect, ChangeItemDamage, DamageEntity,
    ExplodeEffect, Ignite, PlaySound, ReplaceBlock, ReplaceDisk, RunFunction, SetBlockProperties,
    SpawnParticlesEffect, SummonEntityEffect,
};
use crate::enchantment::EnchantedItemInUse;
use crate::entity::EntityBase;
use crate::world::{ExplosionInteraction, World};

/// Extension trait for enchantment entity effects.
pub trait EnchantmentEntityEffectExt {
    /// Applies this entity effect at the given position in the world.
    fn apply(
        &self,
        world: &Arc<World>,
        enchantment_level: i32,
        item: &EnchantedItemInUse<'_>,
        entity: &dyn EntityBase,
        position: Vector3<f64>,
    );
}

impl EnchantmentEntityEffectExt for EnchantmentEntityEffect {
    #[allow(clippy::too_many_lines)]
    fn apply(
        &self,
        world: &Arc<World>,
        enchantment_level: i32,
        item: &EnchantedItemInUse<'_>,
        entity: &dyn EntityBase,
        position: Vector3<f64>,
    ) {
        match self {
            Self::Ignite { duration } => {
                EnchantmentEntityEffectExt::apply(
                    &Ignite::new(duration.clone()),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::DamageEntity {
                min_damage,
                max_damage,
                damage_type,
            } => {
                EnchantmentEntityEffectExt::apply(
                    &DamageEntity::new(min_damage.clone(), max_damage.clone(), *damage_type),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::ChangeItemDamage { amount } => {
                EnchantmentEntityEffectExt::apply(
                    &ChangeItemDamage::new(amount.clone()),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::PlaySound {
                sounds,
                volume,
                pitch,
            } => {
                EnchantmentEntityEffectExt::apply(
                    &PlaySound::new(sounds, volume.clone(), pitch.clone()),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::ReplaceBlock {
                offset_x,
                offset_y,
                offset_z,
                trigger_game_event,
            } => {
                let offset = Vector3::new(*offset_x, *offset_y, *offset_z);
                EnchantmentEntityEffectExt::apply(
                    &ReplaceBlock::new(offset, *trigger_game_event),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::SetBlockProperties {
                properties,
                offset_x,
                offset_y,
                offset_z,
                trigger_game_event,
            } => {
                let offset = Vector3::new(*offset_x, *offset_y, *offset_z);
                let props = properties
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect();
                EnchantmentEntityEffectExt::apply(
                    &SetBlockProperties::new(props, offset, *trigger_game_event),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::ReplaceDisk {
                radius,
                height,
                offset_x,
                offset_y,
                offset_z,
                predicate,
                block_state,
                block_state_properties,
                trigger_game_event,
            } => {
                let offset = Vector3::new(*offset_x, *offset_y, *offset_z);
                EnchantmentEntityEffectExt::apply(
                    &ReplaceDisk {
                        radius: radius.clone(),
                        height: height.clone(),
                        offset,
                        predicate: predicate.clone(),
                        block_state,
                        block_state_properties,
                        trigger_game_event: *trigger_game_event,
                    },
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::SummonEntity {
                entity_types,
                join_team,
            } => {
                EnchantmentEntityEffectExt::apply(
                    &SummonEntityEffect::new(entity_types.to_vec(), *join_team),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::SpawnParticles {
                particle,
                horizontal_position,
                vertical_position,
                horizontal_velocity,
                vertical_velocity,
                speed,
            } => {
                let spawn_particles = SpawnParticlesEffect::new(
                    *particle,
                    *horizontal_position,
                    *vertical_position,
                    horizontal_velocity.clone(),
                    vertical_velocity.clone(),
                    speed.clone(),
                );
                EnchantmentEntityEffectExt::apply(
                    &spawn_particles,
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::RunFunction { function } => {
                EnchantmentEntityEffectExt::apply(
                    &RunFunction::new(*function),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::ApplyExhaustion { amount } => {
                EnchantmentEntityEffectExt::apply(
                    &ApplyExhaustion::new(amount.clone()),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::ApplyImpulse {
                direction,
                coordinate_scale,
                magnitude,
            } => {
                EnchantmentEntityEffectExt::apply(
                    &ApplyEntityImpulse::new(*direction, *coordinate_scale, magnitude.clone()),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::ApplyMobEffect {
                to_apply,
                min_duration,
                max_duration,
                min_amplifier,
                max_amplifier,
            } => {
                EnchantmentEntityEffectExt::apply(
                    &ApplyMobEffect::new(
                        to_apply.to_vec(),
                        min_duration.clone(),
                        max_duration.clone(),
                        min_amplifier.clone(),
                        max_amplifier.clone(),
                    ),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::Explode {
                attribute_to_user,
                damage_type,
                knockback_multiplier,
                immune_blocks,
                offset_x,
                offset_y,
                offset_z,
                radius,
                create_fire,
                block_interaction,
                small_particle,
                large_particle,
                sound,
            } => {
                let Some(block_interaction) = ExplosionInteraction::from_name(block_interaction)
                else {
                    tracing::warn!("Unknown explode block_interaction {block_interaction}");
                    return;
                };
                EnchantmentEntityEffectExt::apply(
                    &ExplodeEffect::new(
                        *attribute_to_user,
                        *damage_type,
                        knockback_multiplier.clone(),
                        *immune_blocks,
                        Vector3::new(*offset_x, *offset_y, *offset_z),
                        radius.clone(),
                        *create_fire,
                        block_interaction,
                        *small_particle,
                        *large_particle,
                        *sound,
                    ),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            Self::AllOf(effects) => {
                EnchantmentEntityEffectExt::apply(
                    &AllOf::new(effects),
                    world,
                    enchantment_level,
                    item,
                    entity,
                    position,
                );
            }
            // Only a location-based effect: `EnchantmentHelper::run_location_changed_effects`.
            Self::Attribute { .. } | Self::Other => {}
        }
    }
}
