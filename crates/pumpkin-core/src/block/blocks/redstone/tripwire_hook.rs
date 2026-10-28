use std::sync::Arc;

use pumpkin_data::{
    Block, BlockDirection, BlockStateId, FacingExt, HorizontalFacingExt,
    block_properties::HorizontalFacing,
    game_event::GameEvent,
    sound::{Sound, SoundCategory},
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::{
    tick::TickPriority,
    world::{BlockAccessor, BlockFlags},
};

use crate::{
    block::{
        BlockBehaviour, BlockIsReplacing, CanPlaceAtArgs, EmitsRedstonePowerArgs,
        GetRedstonePowerArgs, GetStateForNeighborUpdateArgs, OnPlaceArgs, OnScheduledTickArgs,
        OnStateReplacedArgs, PlayerPlacedArgs,
    },
    world::World,
};

type TripwireProperties = pumpkin_data::block_properties::TripwireLikeProperties;
type TripwireHookProperties = pumpkin_data::block_properties::TripwireHookLikeProperties;

#[pumpkin_block("minecraft:tripwire_hook")]
pub struct TripwireHookBlock;

/// Vanilla `TripWireHookBlock.RECHECK_PERIOD`.
const RECHECK_PERIOD: i32 = 10;

impl BlockBehaviour for TripwireHookBlock {
    /// Vanilla `TripWireHookBlock.getStateForPlacement`: the first horizontal looking
    /// direction (the clicked face first) with a wall the hook can hang on.
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = TripwireHookProperties::default(args.block);
        props.powered = false;
        props.attached = false;
        let mut directions = args.player.living_entity.entity.get_entity_facing_order();
        if args.replacing == BlockIsReplacing::None {
            let face = args.direction.to_facing();
            if let Some(i) = directions.iter().position(|&d| d == face)
                && i > 0
            {
                directions.copy_within(0..i, 1);
                directions[0] = face;
            }
        }
        for direction in directions {
            let direction = direction.to_block_direction();
            if let Some(facing) = direction.opposite().to_horizontal_facing()
                && Self::can_survive(args.world, args.position, facing)
            {
                props.facing = facing;
                return props.to_state_id(args.block);
            }
        }
        args.block.default_state.id
    }

    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        if args.player.is_some() {
            return BlockDirection::horizontal()
                .into_iter()
                .any(|facing| Self::can_survive(args.block_accessor, args.position, facing));
        }
        let props = TripwireHookProperties::from_state_id(args.state.id);
        Self::can_survive(args.block_accessor, args.position, props.facing)
    }

    /// Vanilla `TripWireHookBlock.setPlacedBy`.
    fn player_placed(&self, args: PlayerPlacedArgs<'_>) {
        Self::update(
            args.world,
            *args.position,
            args.state_id,
            false,
            false,
            -1,
            None,
        );
    }

    /// Vanilla `TripWireHookBlock.updateShape`: the hook goes with its wall.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let props = TripwireHookProperties::from_state_id(args.state_id);
        if args.direction.opposite() == props.facing.to_block_direction()
            && !Self::can_survive(args.world, args.position, props.facing)
        {
            Block::AIR.default_state.id
        } else {
            args.state_id
        }
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state_id = args.world.get_block_state_id(args.position);
        Self::update(args.world, *args.position, state_id, false, true, -1, None);
    }

    /// Vanilla `TripWireHookBlock.affectNeighborsAfterRemoval`.
    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        if args.moved {
            return;
        }
        Self::on_removed(args.world, *args.position, args.old_state_id);
    }

    #[inline]
    fn emits_redstone_power(&self, _args: EmitsRedstonePowerArgs<'_>) -> bool {
        true
    }

    fn get_weak_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        let props = TripwireHookProperties::from_state_id(args.state.id);
        if props.powered { 15 } else { 0 }
    }

    fn get_strong_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        let props = TripwireHookProperties::from_state_id(args.state.id);
        if props.powered
            && args
                .direction
                .to_horizontal_facing()
                .is_some_and(|facing| props.facing == facing)
        {
            15
        } else {
            0
        }
    }
}

impl TripwireHookBlock {
    /// Vanilla `TripWireHookBlock.canSurvive`: the wall behind a hook facing `facing`.
    pub fn can_survive(
        world: &dyn BlockAccessor,
        block_pos: &BlockPos,
        facing: HorizontalFacing,
    ) -> bool {
        let facing = facing.to_block_direction();
        let wall_pos = block_pos.offset(facing.opposite().to_offset());
        world.get_block_state(&wall_pos).is_side_solid(facing)
    }

    /// Vanilla `TripWireHookBlock.onRemoved`.
    fn on_removed(world: &Arc<World>, pos: BlockPos, state_id: BlockStateId) {
        let props = TripwireHookProperties::from_state_id(state_id);
        if props.attached || props.powered {
            Self::update(world, pos, state_id, true, false, -1, None);
        }
        if props.powered {
            Self::update_neighbors_on_axis(
                &Block::TRIPWIRE_HOOK,
                world,
                pos,
                props.facing.to_block_direction(),
            );
        }
    }

    /// Vanilla `TripWireHookBlock.calculateState`.
    #[expect(clippy::too_many_lines)]
    pub fn update(
        world: &Arc<World>,
        start_hook_pos: BlockPos,
        start_hook_state_id: BlockStateId,
        is_being_destroyed: bool,
        can_update: bool,
        wire_source: i32,
        wire_source_state: Option<BlockStateId>,
    ) {
        let start_hook_props = TripwireHookProperties::from_state_id(start_hook_state_id);
        let direction = start_hook_props.facing;
        let was_attached = start_hook_props.attached;
        let was_powered = start_hook_props.powered;
        let mut attached = !is_being_destroyed;
        let mut powered = false;
        let mut receiver_pos = 0;
        let mut wire_states: Vec<Option<TripwireProperties>> = vec![None; 42];

        for i in 1..42 {
            let test_pos = start_hook_pos.offset_dir(direction.to_offset(), i);
            let (test_block, test_state_id) = world.get_block_and_state_id(&test_pos);
            if test_block == &Block::TRIPWIRE_HOOK {
                if TripwireHookProperties::from_state_id(test_state_id).facing
                    == direction.opposite()
                {
                    receiver_pos = i;
                }
                break;
            }

            if test_block != &Block::TRIPWIRE && i != wire_source {
                wire_states[i as usize] = None;
                attached = false;
            } else {
                let wire_state_id = if i == wire_source {
                    wire_source_state.unwrap_or(test_state_id)
                } else {
                    test_state_id
                };
                let wire = TripwireProperties::from_state_id(wire_state_id);
                let wire_armed = !wire.disarmed;
                powered |= wire_armed && wire.powered;
                wire_states[i as usize] = Some(wire);
                if i == wire_source {
                    world.schedule_block_tick(
                        &Block::TRIPWIRE_HOOK,
                        start_hook_pos,
                        RECHECK_PERIOD,
                        TickPriority::Normal,
                    );
                    attached &= wire_armed;
                }
            }
        }

        attached &= receiver_pos > 1;
        powered &= attached;
        let mut new_state = TripwireHookProperties::default(&Block::TRIPWIRE_HOOK);
        new_state.attached = attached;
        new_state.powered = powered;

        if receiver_pos > 0 {
            let test_pos = start_hook_pos.offset_dir(direction.to_offset(), receiver_pos);
            let opposite = direction.opposite();
            let mut receiver_state = new_state;
            receiver_state.facing = opposite;
            world.set_block_state(
                &test_pos,
                receiver_state.to_state_id(&Block::TRIPWIRE_HOOK),
                BlockFlags::NOTIFY_ALL,
            );
            Self::update_neighbors_on_axis(
                &Block::TRIPWIRE_HOOK,
                world,
                test_pos,
                opposite.to_block_direction(),
            );
            if world.get_block(&start_hook_pos) != &Block::TRIPWIRE_HOOK {
                Self::on_removed(
                    world,
                    start_hook_pos,
                    new_state.to_state_id(&Block::TRIPWIRE_HOOK),
                );
                return;
            }

            Self::play_sound(
                world,
                &test_pos,
                attached,
                powered,
                was_attached,
                was_powered,
            );
        }

        Self::play_sound(
            world,
            &start_hook_pos,
            attached,
            powered,
            was_attached,
            was_powered,
        );

        if !is_being_destroyed {
            let mut start_state = new_state;
            start_state.facing = direction;
            world.set_block_state(
                &start_hook_pos,
                start_state.to_state_id(&Block::TRIPWIRE_HOOK),
                BlockFlags::NOTIFY_ALL,
            );
            if can_update {
                Self::update_neighbors_on_axis(
                    &Block::TRIPWIRE_HOOK,
                    world,
                    start_hook_pos,
                    direction.to_block_direction(),
                );
            }
        }

        if was_attached != attached {
            for i in 1..receiver_pos {
                let test_pos = start_hook_pos.offset_dir(direction.to_offset(), i);
                if let Some(mut wire) = wire_states[i as usize] {
                    let test_block = world.get_block(&test_pos);
                    if test_block == &Block::TRIPWIRE || test_block == &Block::TRIPWIRE_HOOK {
                        wire.attached = attached;
                        world.set_block_state(
                            &test_pos,
                            wire.to_state_id(&Block::TRIPWIRE),
                            BlockFlags::NOTIFY_ALL,
                        );
                    }
                }
            }
        }
    }

    /// Vanilla `TripWireHookBlock.emitState`.
    #[expect(clippy::fn_params_excessive_bools)]
    fn play_sound(
        world: &Arc<World>,
        block_pos: &BlockPos,
        attached: bool,
        powered: bool,
        was_attached: bool,
        was_powered: bool,
    ) {
        let cat = SoundCategory::Blocks;
        let pos = block_pos.to_centered_f64();
        let (sound, pitch, event) = if powered && !was_powered {
            (Sound::BlockTripwireClickOn, 0.6, GameEvent::BlockActivate)
        } else if !powered && was_powered {
            (
                Sound::BlockTripwireClickOff,
                0.5,
                GameEvent::BlockDeactivate,
            )
        } else if attached && !was_attached {
            (Sound::BlockTripwireAttach, 0.7, GameEvent::BlockAttach)
        } else if !attached && was_attached {
            let pitch = 1.2 / world.random_f32().mul_add(0.2, 0.9);
            (Sound::BlockTripwireDetach, pitch, GameEvent::BlockDetach)
        } else {
            return;
        };
        world.play_sound_raw(sound as u16, cat, &pos, 0.4, pitch);
        world.game_event_from(None, event, pos);
    }

    /// Vanilla `TripWireHookBlock.notifyNeighbors`: the hook and the wall behind it.
    pub fn update_neighbors_on_axis(
        block: &Block,
        world: &Arc<World>,
        block_pos: BlockPos,
        direction: BlockDirection,
    ) {
        world.update_neighbors_at(&block_pos, block, None);
        world.update_neighbors_at(
            &block_pos.offset(direction.opposite().to_offset()),
            block,
            None,
        );
    }
}
