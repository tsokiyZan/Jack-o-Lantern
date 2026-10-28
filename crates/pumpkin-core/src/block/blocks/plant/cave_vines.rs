use pumpkin_data::block_properties::{CaveVinesLikeProperties, CaveVinesPlantLikeProperties};
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, BlockDirection, BlockId, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::{BlockAccessor, BlockFlags};

use std::sync::Arc;

use crate::block::blocks::plant::level_random;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, BlockMetadata, BonemealArgs, CanPlaceAtArgs, GetStateForNeighborUpdateArgs,
    NormalUseArgs, OnPlaceArgs, OnScheduledTickArgs, RandomTickArgs,
};
use crate::world::World;

const GROW_PER_TICK_PROBABILITY: f64 = 0.1;
const CHANCE_OF_BERRIES_ON_GROWTH: f32 = 0.11;

pub struct CaveVinesBlock;

impl BlockMetadata for CaveVinesBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::CAVE_VINES, BlockId::CAVE_VINES_PLANT].into()
    }
}

impl CaveVinesBlock {
    /// Vanilla `CaveVines.hasGlowBerries`.
    #[must_use]
    pub fn has_glow_berries(block: &Block, state_id: BlockStateId) -> bool {
        if block == &Block::CAVE_VINES {
            CaveVinesLikeProperties::from_state_id(state_id).berries
        } else if block == &Block::CAVE_VINES_PLANT {
            CaveVinesPlantLikeProperties::from_state_id(state_id).berries
        } else {
            false
        }
    }

    /// Vanilla `CaveVines.use`: picks the glow berries off a vine, returning whether it had
    /// any.
    pub fn use_berries(
        world: &Arc<World>,
        pos: &BlockPos,
        source_entity: Option<&dyn crate::entity::EntityBase>,
    ) -> bool {
        let (block, state) = world.get_block_and_state_id(pos);
        if !Self::has_glow_berries(block, state) {
            return false;
        }
        // The `harvest_cave_vine` loot table.
        world.drop_stack(pos, ItemStack::new(1, &Item::GLOW_BERRIES));
        // `Mth.randomBetween(level.getRandom(), 0.8F, 1.2F)`.
        let pitch = level_random(world).next_f32() * 0.4 + 0.8;
        world.play_sound_fine(
            Sound::BlockCaveVinesPickBerries,
            SoundCategory::Blocks,
            &pos.to_centered_f64(),
            1.0,
            pitch,
        );
        let new_state = if block == &Block::CAVE_VINES {
            let mut props = CaveVinesLikeProperties::from_state_id(state);
            props.berries = false;
            props.to_state_id(block)
        } else {
            let mut props = CaveVinesPlantLikeProperties::from_state_id(state);
            props.berries = false;
            props.to_state_id(block)
        };
        world.set_block_state(pos, new_state, BlockFlags::NOTIFY_LISTENERS);
        world.game_event(
            GameEvent::BlockChange,
            pos.to_centered_f64(),
            crate::world::game_event::GameEventContext::of(
                source_entity,
                Some(pumpkin_data::BlockState::from_id(new_state)),
            ),
        );
        true
    }

    /// Vanilla `GrowingPlantBlock.canSurvive`, growing down.
    #[must_use]
    pub fn can_survive(world: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        let support_pos = pos.up();
        let (support_block, support_state) = world.get_block_and_state(&support_pos);
        if support_block == &Block::CAVE_VINES || support_block == &Block::CAVE_VINES_PLANT {
            return true;
        }
        support_state.is_side_solid(BlockDirection::Down)
    }
}

impl BlockBehaviour for CaveVinesBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        Self::can_survive(args.block_accessor, args.position)
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        if args.block == &Block::CAVE_VINES {
            let below_block = args.world.get_block(&args.position.down());
            if below_block == &Block::CAVE_VINES || below_block == &Block::CAVE_VINES_PLANT {
                return Block::CAVE_VINES_PLANT.default_state.id;
            }
            let mut props = CaveVinesLikeProperties::default(&Block::CAVE_VINES);
            props.age = level_random(args.world).next_bounded_i32(25) as u8;
            props.to_state_id(&Block::CAVE_VINES)
        } else {
            Block::CAVE_VINES_PLANT.default_state.id
        }
    }

    fn random_tick(&self, args: RandomTickArgs<'_>) {
        if args.block == &Block::CAVE_VINES {
            let state_id = args.world.get_block_state_id(args.position);
            let mut props = CaveVinesLikeProperties::from_state_id(state_id);
            if props.age < 25 && level_random(args.world).next_f64() < GROW_PER_TICK_PROBABILITY {
                let growth_pos = args.position.down();
                if args.world.get_block_state(&growth_pos).is_air() {
                    props.age += 1;
                    props.berries =
                        level_random(args.world).next_f32() < CHANCE_OF_BERRIES_ON_GROWTH;
                    args.world.set_block_state(
                        &growth_pos,
                        props.to_state_id(&Block::CAVE_VINES),
                        BlockFlags::NOTIFY_ALL,
                    );
                }
            }
        }
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        if !Self::can_survive(args.world.as_ref(), args.position) {
            args.world
                .break_block(args.position, None, BlockFlags::NOTIFY_ALL);
        }
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if args.block == &Block::CAVE_VINES {
            if args.direction == BlockDirection::Up {
                if Self::can_survive(args.world, args.position) {
                    let below_block = args.world.get_block(&args.position.down());
                    if below_block == &Block::CAVE_VINES || below_block == &Block::CAVE_VINES_PLANT
                    {
                        let head_props = CaveVinesLikeProperties::from_state_id(args.state_id);
                        let mut body_props =
                            CaveVinesPlantLikeProperties::default(&Block::CAVE_VINES_PLANT);
                        body_props.berries = head_props.berries;
                        return body_props.to_state_id(&Block::CAVE_VINES_PLANT);
                    }
                } else {
                    args.world.schedule_block_tick(
                        args.block,
                        *args.position,
                        1,
                        TickPriority::Normal,
                    );
                }
            }
            if args.direction == BlockDirection::Down {
                let neighbor_block = args.world.get_block(args.neighbor_position);
                if neighbor_block == &Block::CAVE_VINES
                    || neighbor_block == &Block::CAVE_VINES_PLANT
                {
                    let head_props = CaveVinesLikeProperties::from_state_id(args.state_id);
                    let mut body_props =
                        CaveVinesPlantLikeProperties::default(&Block::CAVE_VINES_PLANT);
                    body_props.berries = head_props.berries;
                    return body_props.to_state_id(&Block::CAVE_VINES_PLANT);
                }
            }
        } else {
            if args.direction == BlockDirection::Up && !Self::can_survive(args.world, args.position)
            {
                args.world
                    .schedule_block_tick(args.block, *args.position, 1, TickPriority::Normal);
            }
            if args.direction == BlockDirection::Down {
                let neighbor_block = args.world.get_block(args.neighbor_position);
                if neighbor_block != &Block::CAVE_VINES
                    && neighbor_block != &Block::CAVE_VINES_PLANT
                {
                    let body_props = CaveVinesPlantLikeProperties::from_state_id(args.state_id);
                    let mut head_props = CaveVinesLikeProperties::default(&Block::CAVE_VINES);
                    head_props.age = level_random(args.world).next_bounded_i32(25) as u8;
                    head_props.berries = body_props.berries;
                    return head_props.to_state_id(&Block::CAVE_VINES);
                }
            }
        }
        args.state_id
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        if Self::use_berries(args.world, args.position, Some(args.player.as_ref())) {
            BlockActionResult::SuccessServer
        } else {
            BlockActionResult::Pass
        }
    }

    fn is_valid_bonemeal_target(&self, args: BonemealArgs<'_>) -> bool {
        if args.block == &Block::CAVE_VINES {
            !CaveVinesLikeProperties::from_state_id(args.state_id).berries
        } else if args.block == &Block::CAVE_VINES_PLANT {
            !CaveVinesPlantLikeProperties::from_state_id(args.state_id).berries
        } else {
            false
        }
    }

    fn perform_bonemeal(&self, args: BonemealArgs<'_>) {
        if args.block == &Block::CAVE_VINES {
            let mut props = CaveVinesLikeProperties::from_state_id(args.state_id);
            props.berries = true;
            args.world.set_block_state(
                args.position,
                props.to_state_id(args.block),
                BlockFlags::NOTIFY_LISTENERS,
            );
        } else if args.block == &Block::CAVE_VINES_PLANT {
            let mut props = CaveVinesPlantLikeProperties::from_state_id(args.state_id);
            props.berries = true;
            args.world.set_block_state(
                args.position,
                props.to_state_id(args.block),
                BlockFlags::NOTIFY_LISTENERS,
            );
        }
    }
}
