use pumpkin_data::enchantment::{
    ContextFloatProvider, DamageSourcePredicate, EntityPredicate, LootContextEntityTarget,
    LootItemCondition,
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use crate::entity::EntityBase;
use crate::entity::damage_source::DamageSource;
use crate::entity::player::Player;
use crate::world::World;

/// The parameters of vanilla's `ENCHANTED_DAMAGE` loot context, built by
/// `Enchantment.damageContext`.
pub struct DamageLootContext<'a> {
    pub world: &'a World,
    pub this_entity: &'a dyn EntityBase,
    pub enchantment_level: i32,
    pub origin: Vector3<f64>,
    pub damage_source: &'a DamageSource<'a>,
}

impl<'a> DamageLootContext<'a> {
    /// Vanilla `Enchantment.damageContext`.
    #[must_use]
    pub fn new(
        world: &'a World,
        enchantment_level: i32,
        victim: &'a dyn EntityBase,
        damage_source: &'a DamageSource<'a>,
    ) -> Self {
        Self {
            world,
            this_entity: victim,
            enchantment_level,
            origin: victim.get_entity().pos.load(),
            damage_source,
        }
    }

    fn get_entity(&self, target: LootContextEntityTarget) -> Option<&'a dyn EntityBase> {
        match target {
            LootContextEntityTarget::This => Some(self.this_entity),
            LootContextEntityTarget::Attacker => self.damage_source.causing,
            LootContextEntityTarget::DirectAttacker => self.damage_source.direct,
        }
    }
}

/// Vanilla `LootItemCondition.test` for the conditions enchantment effects use.
pub trait LootItemConditionExt {
    fn test(&self, context: &DamageLootContext<'_>) -> bool;
}

impl LootItemConditionExt for LootItemCondition {
    fn test(&self, context: &DamageLootContext<'_>) -> bool {
        match self {
            Self::AllOf(terms) => terms.iter().all(|term| term.test(context)),
            // `LootItemRandomChanceCondition`: the context's random is the level's.
            Self::RandomChance { chance } => {
                let probability = match chance {
                    ContextFloatProvider::Constant(value) => *value,
                    ContextFloatProvider::EnchantmentLevel(amount) => {
                        amount.calculate(context.enchantment_level)
                    }
                };
                rand::random::<f32>() < probability
            }
            // `LootItemEntityPropertyCondition`.
            Self::EntityProperties { entity, predicate } => predicate.as_ref().is_none_or(|p| {
                context
                    .get_entity(*entity)
                    .is_some_and(|e| entity_predicate_matches(p, context.world, e))
            }),
            // `DamageSourceCondition`; the damage context always has an origin.
            Self::DamageSourceProperties { predicate } => predicate
                .as_ref()
                .is_none_or(|p| damage_source_predicate_matches(*p, context.damage_source)),
            Self::WeatherCheck { thundering } => {
                thundering.is_none_or(|t| t == context.world.is_thundering())
            }
        }
    }
}

/// Vanilla `EntityPredicate.matches` for the sub-predicates enchantment requirements use.
fn entity_predicate_matches(
    predicate: &EntityPredicate,
    world: &World,
    entity: &dyn EntityBase,
) -> bool {
    let base = entity.get_entity();
    if let Some(entity_type) = &predicate.entity_type
        && !entity_type.matches(base.entity_type)
    {
        return false;
    }
    // `EntityLocationPredicate`: the entity's own position, not the context origin.
    if let Some(location) = &predicate.location
        && let Some(can_see_sky) = location.can_see_sky
        && can_see_sky != world.can_see_sky(&BlockPos::floored_v(base.pos.load()))
    {
        return false;
    }
    // `EntityFlagsPredicate.isFlying`.
    if let Some(flags) = &predicate.flags
        && let Some(is_flying) = flags.is_flying
    {
        let entity_is_flying = entity.get_living_entity().is_some_and(|living| {
            living.entity.is_fall_flying() || entity.get_player().is_some_and(Player::is_flying)
        });
        if entity_is_flying != is_flying {
            return false;
        }
    }
    // `MovementPredicate`: only `fall_distance` is ever bounded here.
    if let Some(movement) = &predicate.movement {
        let fall_distance = entity
            .get_living_entity()
            .map_or(0.0, |living| f64::from(living.fall_distance.load()));
        if !movement.fall_distance.matches(fall_distance) {
            return false;
        }
    }
    true
}

/// Vanilla `DamageSourcePredicate.matches`.
fn damage_source_predicate_matches(
    predicate: DamageSourcePredicate,
    source: &DamageSource<'_>,
) -> bool {
    predicate.is_direct.is_none_or(|d| d == source.is_direct())
}
