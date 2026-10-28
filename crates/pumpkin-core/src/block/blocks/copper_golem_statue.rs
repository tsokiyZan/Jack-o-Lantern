use std::sync::Arc;

use pumpkin_data::block_properties::{
    CopperGolemStatueLikeProperties, EnumVariants, HorizontalFacing,
};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockId, BlockState};
use pumpkin_util::GameMode;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;
use pumpkin_world::world::BlockFlags;

use crate::block::entities::copper_golem_statue::CopperGolemStatueBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::block::{BlockBehaviour, BlockMetadata, UseWithItemArgs};
use crate::entity::Entity;
use crate::entity::passive::copper_golem::CopperGolemEntity;
use crate::world::World;

/// The waxed statues, plain vanilla `CopperGolemStatueBlock`s; the weathering ones share
/// `WeatheringCopperBlock` and call into `weathering_use_item_on`.
pub struct CopperGolemStatueBlock;

impl BlockMetadata for CopperGolemStatueBlock {
    fn ids() -> Box<[BlockId]> {
        [
            BlockId::WAXED_COPPER_GOLEM_STATUE,
            BlockId::WAXED_EXPOSED_COPPER_GOLEM_STATUE,
            BlockId::WAXED_WEATHERED_COPPER_GOLEM_STATUE,
            BlockId::WAXED_OXIDIZED_COPPER_GOLEM_STATUE,
        ]
        .into()
    }
}

impl BlockBehaviour for CopperGolemStatueBlock {
    /// Vanilla `CopperGolemStatueBlock.useItemOn`.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        if args.item_stack.item.has_tag(&tag::Item::MINECRAFT_AXES) {
            return BlockActionResult::Pass;
        }
        update_pose(args.world, args.block, args.position);
        BlockActionResult::Success
    }
}

/// Vanilla `WeatheringCopperGolemStatueBlock.useItemOn`.
pub fn weathering_use_item_on(args: &mut UseWithItemArgs<'_>) -> BlockActionResult {
    let Some(block_entity) = args.world.get_block_entity(args.position) else {
        return BlockActionResult::Pass;
    };
    let Some(statue) = block_entity
        .as_any()
        .downcast_ref::<CopperGolemStatueBlockEntity>()
    else {
        return BlockActionResult::Pass;
    };
    if !args.item_stack.item.has_tag(&tag::Item::MINECRAFT_AXES) {
        if args.item_stack.item == &Item::HONEYCOMB {
            return BlockActionResult::Pass;
        }
        update_pose(args.world, args.block, args.position);
        return BlockActionResult::Success;
    }
    if args.block == &Block::COPPER_GOLEM_STATUE {
        let state = args.world.get_block_state(args.position);
        let copper_golem = remove_statue(args.world, statue, state);
        if args.player.gamemode.load() != GameMode::Creative {
            let _ = args.item_stack.damage_item(1);
        }
        args.world.spawn_entity(copper_golem);
        // `Level.removeBlock(pos, false)`: what the statue's fluid leaves behind.
        let remaining = if state.is_waterlogged() {
            Block::WATER.default_state.id
        } else {
            Block::AIR.default_state.id
        };
        args.world
            .set_block_state(args.position, remaining, BlockFlags::NOTIFY_ALL);
        return BlockActionResult::Success;
    }
    BlockActionResult::Pass
}

/// Vanilla `CopperGolemStatueBlockEntity.removeStatue` and `initCopperGolem`.
fn remove_statue(
    world: &Arc<World>,
    statue: &CopperGolemStatueBlockEntity,
    state: &BlockState,
) -> Arc<CopperGolemEntity> {
    let pos = statue.position;
    let facing = CopperGolemStatueLikeProperties::from_state_id(state.id).facing;
    let copper_golem = CopperGolemEntity::new(Entity::new(
        world.clone(),
        Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y),
            f64::from(pos.0.z) + 0.5,
        ),
        &EntityType::COPPER_GOLEM,
    ));
    let name = statue
        .custom_name
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let entity = &copper_golem.mob_entity.living_entity.entity;
    if let Some(name) = name {
        entity.set_custom_name(TextComponent::text(name));
    }
    let y_rot = to_y_rot(facing);
    entity.set_rotation(y_rot, 0.0);
    entity.body_yaw.store(y_rot);
    copper_golem.play_spawn_sound();
    copper_golem
}

/// Vanilla `Direction.toYRot`.
const fn to_y_rot(facing: HorizontalFacing) -> f32 {
    match facing {
        HorizontalFacing::South => 0.0,
        HorizontalFacing::West => 90.0,
        HorizontalFacing::North => 180.0,
        HorizontalFacing::East => 270.0,
    }
}

/// Vanilla `CopperGolemStatueBlock.updatePose`.
// TODO: `level.gameEvent(player, GameEvent.BLOCK_CHANGE, pos)`.
fn update_pose(world: &Arc<World>, block: &Block, pos: &BlockPos) {
    world.play_sound_fine(
        Sound::EntityCopperGolemBecomeStatue,
        SoundCategory::Blocks,
        &pos.to_centered_f64(),
        1.0,
        1.0,
    );
    let mut props = CopperGolemStatueLikeProperties::from_state_id(world.get_block_state_id(pos));
    let pose_count = pumpkin_data::block_properties::CopperGolemPose::variant_count();
    props.copper_golem_pose =
        EnumVariants::from_index((props.copper_golem_pose.to_index() + 1) % pose_count);
    world.set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_ALL);
}
