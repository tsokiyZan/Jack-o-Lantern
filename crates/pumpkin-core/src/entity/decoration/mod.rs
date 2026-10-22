pub mod armor_stand;
pub mod cushion;
pub mod display;
pub mod end_crystal;
pub mod item_frame;
pub mod leash_knot;
pub mod painting;

use crate::entity::EntityBase;

/// Vanilla `BlockAttachedEntity.skipAttackInteraction`: a player who may not interact at the
/// entity's block is stopped; else the entity takes a 0 damage player attack, and a hit ends
/// the player's attack there.
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
