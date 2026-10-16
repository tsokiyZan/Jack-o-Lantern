use std::sync::atomic::Ordering;

use pumpkin_data::enchantment::{
    ContextFloatProvider, DamageSourcePredicate, EntityPredicate, ItemPredicate, LocationPredicate,
    LootContextEntityTarget, LootItemCondition,
};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::Taggable;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use crate::entity::EntityBase;
use crate::entity::damage_source::DamageSource;
use crate::entity::player::Player;
use crate::world::World;

/// The loot context vanilla builds for an enchantment effect's requirements.
///
/// The constructors mirror `Enchantment.damageContext`, `itemContext`, `locationContext` and
/// `entityContext`; a parameter a context doesn't have is `None`.
pub struct EnchantmentLootContext<'a> {
    pub world: &'a World,
    pub enchantment_level: i32,
    pub this_entity: Option<&'a dyn EntityBase>,
    pub origin: Option<Vector3<f64>>,
    pub damage_source: Option<&'a DamageSource<'a>>,
    pub tool: Option<&'a ItemStack>,
    pub enchantment_active: Option<bool>,
}

impl<'a> EnchantmentLootContext<'a> {
    /// Vanilla `Enchantment.damageContext`.
    #[must_use]
    pub fn damage(
        world: &'a World,
        enchantment_level: i32,
        victim: &'a dyn EntityBase,
        damage_source: &'a DamageSource<'a>,
    ) -> Self {
        Self {
            world,
            enchantment_level,
            this_entity: Some(victim),
            origin: Some(victim.get_entity().pos.load()),
            damage_source: Some(damage_source),
            tool: None,
            enchantment_active: None,
        }
    }

    /// Vanilla `Enchantment.itemContext`.
    #[must_use]
    pub const fn item(world: &'a World, enchantment_level: i32, item: &'a ItemStack) -> Self {
        Self {
            world,
            enchantment_level,
            this_entity: None,
            origin: None,
            damage_source: None,
            tool: Some(item),
            enchantment_active: None,
        }
    }

    /// Vanilla `Enchantment.locationContext`.
    #[must_use]
    pub fn location(
        world: &'a World,
        enchantment_level: i32,
        entity: &'a dyn EntityBase,
        active: bool,
    ) -> Self {
        Self {
            world,
            enchantment_level,
            this_entity: Some(entity),
            origin: Some(entity.get_entity().pos.load()),
            damage_source: None,
            tool: None,
            enchantment_active: Some(active),
        }
    }

    /// Vanilla `Enchantment.entityContext`, and `blockHitContext` (whose block state no
    /// requirement reads).
    #[must_use]
    pub const fn entity(
        world: &'a World,
        enchantment_level: i32,
        entity: &'a dyn EntityBase,
        position: Vector3<f64>,
    ) -> Self {
        Self {
            world,
            enchantment_level,
            this_entity: Some(entity),
            origin: Some(position),
            damage_source: None,
            tool: None,
            enchantment_active: None,
        }
    }

    fn get_entity(&self, target: LootContextEntityTarget) -> Option<&'a dyn EntityBase> {
        match target {
            LootContextEntityTarget::This => self.this_entity,
            LootContextEntityTarget::Attacker => self.damage_source.and_then(|s| s.causing),
            LootContextEntityTarget::DirectAttacker => self.damage_source.and_then(|s| s.direct),
        }
    }
}

/// Vanilla `LootItemCondition.test` for the conditions enchantment effects use.
pub trait LootItemConditionExt {
    fn test(&self, context: &EnchantmentLootContext<'_>) -> bool;
}

impl LootItemConditionExt for LootItemCondition {
    fn test(&self, context: &EnchantmentLootContext<'_>) -> bool {
        match self {
            Self::AllOf(terms) => terms.iter().all(|term| term.test(context)),
            Self::AnyOf(terms) => terms.iter().any(|term| term.test(context)),
            Self::Inverted(term) => !term.test(context),
            // `LootItemRandomChanceCondition`: the context's random is the level's.
            Self::RandomChance { chance } => {
                let probability = match chance {
                    ContextFloatProvider::Constant(value) => *value,
                    // `EnchantmentLevelProvider`: level 0 without the parameter.
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
            // `DamageSourceCondition`: no match without a damage source and an origin.
            Self::DamageSourceProperties { predicate } => {
                match (context.damage_source, context.origin) {
                    (Some(source), Some(_)) => predicate
                        .as_ref()
                        .is_none_or(|p| damage_source_predicate_matches(p, source)),
                    _ => false,
                }
            }
            Self::WeatherCheck { thundering } => {
                thundering.is_none_or(|t| t == context.world.is_thundering())
            }
            // `MatchTool`.
            Self::MatchTool { predicate } => predicate.as_ref().is_none_or(|p| {
                context
                    .tool
                    .is_some_and(|tool| item_predicate_matches(p, tool))
            }),
            // `LocationCheck`, without an offset: the origin itself.
            Self::LocationCheck { predicate } => context
                .origin
                .is_some_and(|pos| location_predicate_matches(predicate, context.world, pos)),
            // `EnchantmentActiveCheck`.
            Self::EnchantmentActiveCheck { active } => context.enchantment_active == Some(*active),
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
        && !location_predicate_matches(location, world, base.pos.load())
    {
        return false;
    }
    if let Some(flags) = &predicate.flags {
        if flags
            .is_on_ground
            .is_some_and(|on_ground| on_ground != base.on_ground.load(Ordering::Relaxed))
        {
            return false;
        }
        // `isFlying`: fall flying, or a player flying with its abilities.
        if let Some(is_flying) = flags.is_flying {
            let entity_is_flying = entity.get_living_entity().is_some_and(|_| {
                base.is_fall_flying() || entity.get_player().is_some_and(Player::is_flying)
            });
            if entity_is_flying != is_flying {
                return false;
            }
        }
        if flags
            .is_in_water
            .is_some_and(|in_water| in_water != base.is_in_water())
        {
            return false;
        }
        if let Some(is_fall_flying) = flags.is_fall_flying
            && entity.get_living_entity().is_some()
            && base.is_fall_flying() != is_fall_flying
        {
            return false;
        }
    }
    // `MovementPredicate`: `getKnownMovement` scaled to blocks per second.
    if let Some(movement) = &predicate.movement {
        let velocity = base.velocity.load() * 20.0;
        let fall_distance = base.fall_distance.load();
        if !movement
            .horizontal_speed
            .matches(velocity.x.hypot(velocity.z))
            || !movement.fall_distance.matches(fall_distance)
        {
            return false;
        }
    }
    // `EntitySubPredicates.VEHICLE`: no match without a vehicle.
    if let Some(vehicle) = predicate.vehicle
        && !base
            .get_vehicle()
            .is_some_and(|v| entity_predicate_matches(vehicle, world, v.as_ref()))
    {
        return false;
    }
    // `PlayerPredicate`: only players match.
    if let Some(player_predicate) = &predicate.player {
        let Some(player) = entity.get_player() else {
            return false;
        };
        if player_predicate
            .gamemodes
            .is_some_and(|modes| !modes.contains(&player.gamemode.load()))
            || !player_predicate
                .food_level
                .matches(i32::from(player.get_food_level()))
        {
            return false;
        }
    }
    // `MovementAffectedByPredicate`.
    if let Some(location) = &predicate.movement_affected_by {
        let pos = base.get_block_pos_below_that_affects_my_movement();
        if !location_predicate_matches(location, world, pos.to_f64()) {
            return false;
        }
    }
    // `PeriodicTickPredicate`.
    if let Some(period) = predicate.periodic_tick
        && base.tick_count.load(Ordering::Relaxed) % period != 0
    {
        return false;
    }
    true
}

/// Vanilla `LocationPredicate.matches`, for its block and sky parts.
fn location_predicate_matches(
    predicate: &LocationPredicate,
    world: &World,
    pos: Vector3<f64>,
) -> bool {
    let block_pos = BlockPos::floored_v(pos);
    if let Some(block) = &predicate.block
        && !world.get_block(&block_pos).has_tag(block.blocks)
    {
        return false;
    }
    predicate
        .can_see_sky
        .is_none_or(|can_see_sky| can_see_sky == world.can_see_sky(&block_pos))
}

/// Vanilla `ItemPredicate.test`, for its item set.
fn item_predicate_matches(predicate: &ItemPredicate, item: &ItemStack) -> bool {
    predicate.items.contains(item.get_item())
}

/// Vanilla `DamageSourcePredicate.matches`.
fn damage_source_predicate_matches(
    predicate: &DamageSourcePredicate,
    source: &DamageSource<'_>,
) -> bool {
    predicate
        .tags
        .iter()
        .all(|tag| source.damage_type.has_tag(tag.tag) == tag.expected)
        && predicate.is_direct.is_none_or(|d| d == source.is_direct())
}
