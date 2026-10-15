use std::sync::Arc;

use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::{Block, BlockStateId, Enchantment};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::GameMode;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use uuid::Uuid;

use crate::block::BlockBehaviour;
use crate::block::BrokenArgs;
use crate::entity::r#type::from_type;
use crate::world::World;

#[pumpkin_block(
    "minecraft:infested_stone",
    "minecraft:infested_cobblestone",
    "minecraft:infested_stone_bricks",
    "minecraft:infested_mossy_stone_bricks",
    "minecraft:infested_cracked_stone_bricks",
    "minecraft:infested_chiseled_stone_bricks",
    "minecraft:infested_deepslate"
)]
pub struct InfestedBlock;

/// Vanilla `InfestedBlock.BLOCK_BY_HOST_BLOCK`: each host block and its infested variant.
const INFESTED_BY_HOST: [(&Block, &Block); 7] = [
    (&Block::STONE, &Block::INFESTED_STONE),
    (&Block::COBBLESTONE, &Block::INFESTED_COBBLESTONE),
    (&Block::STONE_BRICKS, &Block::INFESTED_STONE_BRICKS),
    (
        &Block::MOSSY_STONE_BRICKS,
        &Block::INFESTED_MOSSY_STONE_BRICKS,
    ),
    (
        &Block::CRACKED_STONE_BRICKS,
        &Block::INFESTED_CRACKED_STONE_BRICKS,
    ),
    (
        &Block::CHISELED_STONE_BRICKS,
        &Block::INFESTED_CHISELED_STONE_BRICKS,
    ),
    (&Block::DEEPSLATE, &Block::INFESTED_DEEPSLATE),
];

/// `state` of `from` moved onto `to`, keeping its properties (the deepslate axis).
fn state_with_properties(from: &Block, state: BlockStateId, to: &Block) -> BlockStateId {
    from.properties(state)
        .map_or(to.default_state.id, |properties| properties.to_state_id(to))
}

impl InfestedBlock {
    /// Vanilla `isCompatibleHostBlock`.
    #[must_use]
    pub fn is_compatible_host_block(block: &Block) -> bool {
        INFESTED_BY_HOST.iter().any(|(host, _)| host.id == block.id)
    }

    /// Whether `block` is one of the infested blocks.
    #[must_use]
    pub fn is_infested(block: &Block) -> bool {
        INFESTED_BY_HOST
            .iter()
            .any(|(_, infested)| infested.id == block.id)
    }

    /// Vanilla `infestedStateByHost`.
    #[must_use]
    pub fn infested_state_by_host(host: &Block, state: BlockStateId) -> Option<BlockStateId> {
        INFESTED_BY_HOST
            .iter()
            .find(|(host_block, _)| host_block.id == host.id)
            .map(|(host_block, infested)| state_with_properties(host_block, state, infested))
    }

    /// Vanilla `hostStateByInfested`.
    #[must_use]
    pub fn host_state_by_infested(infested: &Block, state: BlockStateId) -> Option<BlockStateId> {
        INFESTED_BY_HOST
            .iter()
            .find(|(_, infested_block)| infested_block.id == infested.id)
            .map(|(host, infested_block)| state_with_properties(infested_block, state, host))
    }

    /// Vanilla `spawnInfestation`: a silverfish pops out of the block.
    pub fn spawn_infestation(world: &Arc<World>, pos: &BlockPos) {
        let silverfish = from_type(
            &EntityType::SILVERFISH,
            Vector3::new(
                f64::from(pos.0.x) + 0.5,
                f64::from(pos.0.y),
                f64::from(pos.0.z) + 0.5,
            ),
            world,
            Uuid::new_v4(),
        );
        world.spawn_entity(silverfish.clone());
        // `spawnAnim`.
        world.send_entity_status(
            silverfish.get_entity(),
            EntityStatus::SilverfishMergeAnim,
            None,
        );
    }
}

impl BlockBehaviour for InfestedBlock {
    /// Vanilla `spawnAfterBreak` for a player's break: creative breaks drop nothing, and
    /// silk touch (`PREVENTS_INFESTED_SPAWNS`) keeps the silverfish in.
    fn broken(&self, args: BrokenArgs<'_>) {
        if args.player.gamemode.load() == GameMode::Creative
            || !args.world.level_info.load().game_rules.block_drops
        {
            return;
        }
        let held_item = args.player.inventory().held_item();
        if held_item.get_enchantment_level(&Enchantment::SILK_TOUCH) > 0 {
            return;
        }
        Self::spawn_infestation(args.world, args.position);
    }
}
