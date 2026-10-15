use pumpkin_data::{
    Block, BlockDirection, BlockState,
    block_properties::{HorizontalFacing, ShelfMushroomLikeProperties},
};
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, RandomImpl},
};

use crate::generation::proto_chunk::GenerationCache;

/// Vanilla `ShelfMushroomDecorator`.
pub struct ShelfMushroomTreeDecorator {
    pub probability: f32,
}

impl ShelfMushroomTreeDecorator {
    pub fn generate<T: GenerationCache>(
        &self,
        chunk: &mut T,
        random: &mut RandomGenerator,
        log_positions: &[BlockPos],
        decorations: &mut Vec<BlockPos>,
    ) {
        if random.next_f32() >= self.probability {
            return;
        }
        let (Some(first), Some(last)) = (log_positions.first(), log_positions.last()) else {
            return;
        };
        if first.0.y == last.0.y {
            Self::place_on_fallen_log(chunk, random, log_positions, decorations);
        } else {
            Self::place_on_standing_tree(chunk, random, log_positions, decorations);
        }
    }

    fn place_on_standing_tree<T: GenerationCache>(
        chunk: &mut T,
        random: &mut RandomGenerator,
        logs: &[BlockPos],
        decorations: &mut Vec<BlockPos>,
    ) {
        // `pickTwoPerpendicularDirections`.
        let first = BlockDirection::random_horizontal(random);
        let directions = [first, first.rotate_clockwise()];
        let tree_base_y = logs[0].0.y;

        for &log_pos in logs {
            // `isWithinDecoratableHeight`.
            let dy = log_pos.0.y - tree_base_y;
            if !(1..=4).contains(&dy) {
                continue;
            }
            for facing in directions {
                if random.next_f32() <= 0.25
                    && Self::try_place_on_standing_tree(chunk, random, log_pos, facing, decorations)
                {
                    break;
                }
            }
        }
    }

    fn place_on_fallen_log<T: GenerationCache>(
        chunk: &mut T,
        random: &mut RandomGenerator,
        logs: &[BlockPos],
        decorations: &mut Vec<BlockPos>,
    ) {
        // `perpendicularToFallenLog`.
        let directions = if logs[0].0.x == logs[logs.len() - 1].0.x {
            [HorizontalFacing::East, HorizontalFacing::West]
        } else {
            [HorizontalFacing::North, HorizontalFacing::South]
        };

        for &log_pos in logs {
            for facing in directions {
                if random.next_f32() <= 0.25 {
                    let pos = mushroom_pos_for(log_pos, facing);
                    if is_replaceable_with_shelf_mushroom(chunk, pos)
                        && !has_horizontally_adjacent_shelf_mushroom(chunk, pos)
                        && !has_horizontally_adjacent_shelf_mushroom(chunk, log_pos)
                    {
                        place_mushroom(chunk, random, pos, facing, decorations);
                    }
                }
            }
        }
    }

    fn try_place_on_standing_tree<T: GenerationCache>(
        chunk: &mut T,
        random: &mut RandomGenerator,
        log_pos: BlockPos,
        facing: HorizontalFacing,
        decorations: &mut Vec<BlockPos>,
    ) -> bool {
        let pos = mushroom_pos_for(log_pos, facing);
        if !is_replaceable_with_shelf_mushroom(chunk, pos)
            || has_shelf_mushroom_at(chunk, pos.down())
        {
            return false;
        }
        place_mushroom(chunk, random, pos, facing, decorations);
        true
    }
}

fn mushroom_pos_for(log_pos: BlockPos, facing: HorizontalFacing) -> BlockPos {
    log_pos.offset(facing.to_offset())
}

fn place_mushroom<T: GenerationCache>(
    chunk: &mut T,
    random: &mut RandomGenerator,
    pos: BlockPos,
    facing: HorizontalFacing,
    decorations: &mut Vec<BlockPos>,
) {
    let props = ShelfMushroomLikeProperties {
        age: random.next_bounded_i32(2) as u8,
        facing,
    };
    chunk.set_block_state(
        &pos.0,
        BlockState::from_id(props.to_state_id(&Block::SHELF_MUSHROOM)),
    );
    decorations.push(pos);
}

fn has_shelf_mushroom_at<T: GenerationCache>(chunk: &T, pos: BlockPos) -> bool {
    GenerationCache::get_block_state(chunk, &pos.0).to_block_id() == Block::SHELF_MUSHROOM.id
}

/// `Direction.Plane.HORIZONTAL`: north, east, south, west.
fn has_horizontally_adjacent_shelf_mushroom<T: GenerationCache>(chunk: &T, pos: BlockPos) -> bool {
    BlockDirection::horizontal_worldgen()
        .into_iter()
        .any(|dir| has_shelf_mushroom_at(chunk, pos.offset(dir.to_offset())))
}

/// `isBlockReplaceableWithShelfMushroom`: replaceable, and no water at or beside it.
fn is_replaceable_with_shelf_mushroom<T: GenerationCache>(chunk: &T, pos: BlockPos) -> bool {
    let is_water = |pos: BlockPos| {
        GenerationCache::get_block_state(chunk, &pos.0).to_block_id() == Block::WATER.id
    };
    GenerationCache::get_block_state(chunk, &pos.0)
        .to_state()
        .replaceable()
        && !is_water(pos)
        && !BlockDirection::horizontal_worldgen()
            .into_iter()
            .any(|dir| is_water(pos.offset(dir.to_offset())))
}
