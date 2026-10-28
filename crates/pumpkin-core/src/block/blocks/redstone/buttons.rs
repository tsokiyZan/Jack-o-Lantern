use std::sync::Arc;

use pumpkin_data::Block;
use pumpkin_data::BlockDirection;
use pumpkin_data::BlockState;
use pumpkin_data::BlockStateId;
use pumpkin_data::HorizontalFacingExt;
use pumpkin_data::block_properties::AttachFace;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

type ButtonLikeProperties = pumpkin_data::block_properties::LeverLikeProperties;

use crate::block::CanPlaceAtArgs;
use crate::block::EmitsRedstonePowerArgs;
use crate::block::GetRedstonePowerArgs;
use crate::block::GetStateForNeighborUpdateArgs;
use crate::block::OnEntityCollisionArgs;
use crate::block::OnPlaceArgs;
use crate::block::OnScheduledTickArgs;
use crate::block::OnStateReplacedArgs;
use crate::block::blocks::abstract_wall_mounting::WallMountedBlock;
use crate::block::blocks::redstone::lever::LeverLikePropertiesExt;
use crate::block::registry::BlockActionResult;
use crate::block::{BlockBehaviour, NormalUseArgs};
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::world::World;

/// Stone and polished blackstone buttons: `BlockSetType.STONE` / `POLISHED_BLACKSTONE`.
fn is_stone_button(block: &Block) -> bool {
    block == &Block::STONE_BUTTON || block == &Block::POLISHED_BLACKSTONE_BUTTON
}

/// The `ticksToStayPressed` each button is registered with in vanilla `Blocks`.
fn ticks_to_stay_pressed(block: &Block) -> i32 {
    if is_stone_button(block) { 20 } else { 30 }
}

/// Vanilla `BlockSetType.canButtonBeActivatedByArrows`: every wooden set.
fn can_be_activated_by_arrows(block: &Block) -> bool {
    !is_stone_button(block)
}

/// Vanilla `BlockSetType.buttonClickOn` / `buttonClickOff`.
fn get_sound(block: &Block, on: bool) -> Sound {
    let (click_on, click_off) = if is_stone_button(block) {
        (
            Sound::BlockStoneButtonClickOn,
            Sound::BlockStoneButtonClickOff,
        )
    } else if block == &Block::CHERRY_BUTTON {
        (
            Sound::BlockCherryWoodButtonClickOn,
            Sound::BlockCherryWoodButtonClickOff,
        )
    } else if block == &Block::CRIMSON_BUTTON || block == &Block::WARPED_BUTTON {
        (
            Sound::BlockNetherWoodButtonClickOn,
            Sound::BlockNetherWoodButtonClickOff,
        )
    } else if block == &Block::BAMBOO_BUTTON {
        (
            Sound::BlockBambooWoodButtonClickOn,
            Sound::BlockBambooWoodButtonClickOff,
        )
    } else {
        (
            Sound::BlockWoodenButtonClickOn,
            Sound::BlockWoodenButtonClickOff,
        )
    };
    if on { click_on } else { click_off }
}

/// Vanilla `entity instanceof AbstractArrow`.
fn is_abstract_arrow(entity_type: &EntityType) -> bool {
    entity_type == &EntityType::ARROW
        || entity_type == &EntityType::SPECTRAL_ARROW
        || entity_type == &EntityType::TRIDENT
}

/// The bounds of the button's shape (`state.getShape(level, pos).bounds()`) at `pos`.
fn shape_bounds(state: &BlockState, pos: &BlockPos) -> BoundingBox {
    let mut min = Vector3::new(f64::MAX, f64::MAX, f64::MAX);
    let mut max = Vector3::new(f64::MIN, f64::MIN, f64::MIN);
    for shape in state.get_block_outline_shapes() {
        min = Vector3::new(
            min.x.min(shape.min.x),
            min.y.min(shape.min.y),
            min.z.min(shape.min.z),
        );
        max = Vector3::new(
            max.x.max(shape.max.x),
            max.y.max(shape.max.y),
            max.z.max(shape.max.z),
        );
    }
    BoundingBox::new(min, max).at_pos(*pos)
}

#[pumpkin_block_from_tag("minecraft:buttons")]
pub struct ButtonBlock;

impl BlockBehaviour for ButtonBlock {
    /// Vanilla `ButtonBlock.useWithoutItem`.
    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        if ButtonLikeProperties::from_state_id(state_id).powered {
            return BlockActionResult::Consume;
        }
        Self::press(
            args.world,
            args.position,
            args.block,
            state_id,
            Some(args.player),
        );
        BlockActionResult::Success
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        if ButtonLikeProperties::from_state_id(state_id).powered {
            Self::check_pressed(args.world, args.position, args.block, state_id);
        }
    }

    /// Vanilla `ButtonBlock.entityInside`: an arrow keeps a wooden button pressed.
    fn on_entity_collision(&self, args: OnEntityCollisionArgs<'_>) {
        if can_be_activated_by_arrows(args.block)
            && !ButtonLikeProperties::from_state_id(args.state.id).powered
        {
            Self::check_pressed(args.world, args.position, args.block, args.state.id);
        }
    }

    fn emits_redstone_power(&self, _args: EmitsRedstonePowerArgs<'_>) -> bool {
        true
    }

    fn get_weak_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        let button_props = ButtonLikeProperties::from_state_id(args.state.id);
        if button_props.powered { 15 } else { 0 }
    }

    fn get_strong_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        let button_props = ButtonLikeProperties::from_state_id(args.state.id);
        if button_props.powered && button_props.get_direction() == args.direction {
            15
        } else {
            0
        }
    }

    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        if !args.moved {
            let button_props = ButtonLikeProperties::from_state_id(args.old_state_id);
            if button_props.powered {
                Self::update_neighbors(args.world, args.position, args.block, button_props);
            }
        }
    }

    /// Vanilla `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement`.
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = ButtonLikeProperties::from_state_id(args.block.default_state.id);
        (props.face, props.facing) = WallMountedBlock::get_placement_face_for_args(self, &args);

        props.to_state_id(args.block)
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        WallMountedBlock::can_place_at_args(self, &args)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        WallMountedBlock::get_state_for_neighbor_update(self, args)
    }
}

impl WallMountedBlock for ButtonBlock {
    fn get_direction(&self, state_id: BlockStateId, _block: &Block) -> BlockDirection {
        let props = ButtonLikeProperties::from_state_id(state_id);
        match props.face {
            AttachFace::Floor => BlockDirection::Up,
            AttachFace::Ceiling => BlockDirection::Down,
            AttachFace::Wall => props.facing.to_block_direction(),
        }
    }
}

impl ButtonBlock {
    /// Vanilla `ButtonBlock.press`. The pressing player predicts the click itself.
    fn press(
        world: &Arc<World>,
        pos: &BlockPos,
        block: &Block,
        state_id: BlockStateId,
        player: Option<&Arc<Player>>,
    ) {
        let mut props = ButtonLikeProperties::from_state_id(state_id);
        props.powered = true;
        world.set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_ALL);
        Self::update_neighbors(world, pos, block, props);
        world.schedule_block_tick(
            block,
            *pos,
            ticks_to_stay_pressed(block),
            TickPriority::Normal,
        );
        let sound = get_sound(block, true);
        if let Some(player) = player {
            world.play_block_sound_expect(player, sound, SoundCategory::Blocks, *pos);
        } else {
            world.play_block_sound(sound, SoundCategory::Blocks, *pos);
        }
        world.game_event_from(
            player.map(|player| player.as_ref() as &dyn EntityBase),
            GameEvent::BlockActivate,
            pos.to_centered_f64(),
        );
    }

    /// Vanilla `ButtonBlock.checkPressed`: a wooden button stays pressed while an arrow is in
    /// its shape.
    fn check_pressed(world: &Arc<World>, pos: &BlockPos, block: &Block, state_id: BlockStateId) {
        let state = BlockState::from_id(state_id);
        let first_arrow = if can_be_activated_by_arrows(block) {
            world
                .get_entities_at_box(&shape_bounds(state, pos))
                .into_iter()
                .find(|entity| is_abstract_arrow(entity.get_entity().entity_type))
        } else {
            None
        };
        let should_be_pressed = first_arrow.is_some();
        let mut props = ButtonLikeProperties::from_state_id(state_id);
        if should_be_pressed != props.powered {
            props.powered = should_be_pressed;
            world.set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_ALL);
            Self::update_neighbors(world, pos, block, props);
            world.play_block_sound(
                get_sound(block, should_be_pressed),
                SoundCategory::Blocks,
                *pos,
            );
            let event = if should_be_pressed {
                GameEvent::BlockActivate
            } else {
                GameEvent::BlockDeactivate
            };
            world.game_event_from(first_arrow.as_deref(), event, pos.to_centered_f64());
        }

        if should_be_pressed {
            world.schedule_block_tick(
                block,
                *pos,
                ticks_to_stay_pressed(block),
                TickPriority::Normal,
            );
        }
    }

    /// Vanilla `ButtonBlock.updateNeighbours`.
    fn update_neighbors(
        world: &Arc<World>,
        block_pos: &BlockPos,
        block: &Block,
        props: ButtonLikeProperties,
    ) {
        let direction = props.get_direction().opposite();
        world.update_neighbors_at(block_pos, block, None);
        world.update_neighbors_at(&block_pos.offset(direction.to_offset()), block, None);
    }
}
