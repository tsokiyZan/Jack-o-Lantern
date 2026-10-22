pub mod armor_stand;
pub mod cushion;
pub mod display;
pub mod end_crystal;
pub mod item_frame;
pub mod leash_knot;
pub mod painting;

use crate::entity::EntityBase;

/// Vanilla `BlockAttachedEntity.skipAttackInteraction`.
///
/// A player who may not interact at the entity's block is stopped; else the entity takes a 0
/// damage player attack, and a hit ends the player's attack there.
pub fn block_attached_skip_attack_interaction(
    this: &dyn EntityBase,
    source: &dyn EntityBase,
) -> bool {
    if source.get_player().is_none() {
        return false;
    }
    let entity = this.get_entity();
    if !entity.world.load().may_interact(&entity.block_pos.load()) {
        return true;
    }
    this.damage_with_context(
        this,
        0.0,
        pumpkin_data::damage::DamageType::PLAYER_ATTACK,
        None,
        Some(source),
        Some(source),
    )
}

/// Vanilla `BlockAttachedEntity.kill(level, attributedTo)`: removed, then `ENTITY_DIE` at the
/// entity from `attributed_to` (else the entity itself). `onKilled` is the caller's.
pub fn block_attached_kill(this: &dyn EntityBase, attributed_to: Option<&dyn EntityBase>) {
    let entity = this.get_entity();
    entity.remove();
    entity.world.load().game_event_from(
        Some(attributed_to.unwrap_or(this)),
        pumpkin_data::game_event::GameEvent::EntityDie,
        entity.pos.load(),
    );
}

/// Vanilla `BlockAttachedEntity.hurtServer`.
///
/// Nothing for damage it is invulnerable to, or from a mob without `mob_griefing`; else, unless
/// already removed, killed (attributed to `cause`, vanilla `source.getEntity()`), `markHurt` and
/// `drop_item(cause)`.
pub fn block_attached_hurt_server(
    this: &dyn EntityBase,
    damage_type: &pumpkin_data::damage::DamageType,
    cause: Option<&dyn EntityBase>,
    drop_item: impl FnOnce(Option<&dyn EntityBase>),
) -> bool {
    let entity = this.get_entity();
    if entity.is_invulnerable_to(damage_type, cause) {
        return false;
    }
    if !entity
        .world
        .load()
        .level_info
        .load()
        .game_rules
        .mob_griefing
        && cause.is_some_and(|cause| cause.get_mob().is_some())
    {
        return false;
    }
    if !entity.is_removed() {
        block_attached_kill(this, cause);
        // `markHurt`.
        entity
            .velocity_dirty
            .store(true, std::sync::atomic::Ordering::SeqCst);
        drop_item(cause);
    }
    true
}

/// Vanilla `BlockAttachedEntity.move` and `push(x, y, z)`: any movement of a block-attached
/// entity (`delta`'s squared length above 0) kills it and drops its item.
pub fn block_attached_moved(
    this: &dyn EntityBase,
    delta: pumpkin_util::math::vector3::Vector3<f64>,
    drop_item: impl FnOnce(Option<&dyn EntityBase>),
) {
    if !this.get_entity().is_removed()
        && delta.x * delta.x + delta.y * delta.y + delta.z * delta.z > 0.0
    {
        block_attached_kill(this, None);
        drop_item(None);
    }
}

/// Vanilla `BlockAttachedEntity.thunderHit`: a lightning bolt does nothing.
pub const fn block_attached_thunder_hit() {}

/// Vanilla `BlockAttachedEntity.ignoreExplosion`: an explosion whose direct source is in water,
/// or one that doesn't affect block-like entities (`shouldAffectBlocklikeEntities`), is ignored.
pub fn block_attached_ignore_explosion(
    this: &dyn EntityBase,
    explosion: &crate::world::explosion::Explosion,
) -> bool {
    if explosion
        .direct_source_entity()
        .is_some_and(|direct| direct.get_entity().is_in_water())
    {
        return true;
    }
    // `super.ignoreExplosion` is false.
    !explosion.should_affect_blocklike_entities(&this.get_entity().world.load())
}
