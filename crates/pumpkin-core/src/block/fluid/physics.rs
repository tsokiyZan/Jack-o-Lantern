use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, fluid::Fluid, tag};
use pumpkin_data::{BlockDirection, BlockState};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Axis};

/// Check if a specific block can be replaced by fluid (based on block properties)
#[must_use]
pub fn can_be_replaced(block_state: &BlockState, block: &Block, fluid: &Fluid) -> bool {
    // Waterlogged blocks should not be replaced by water
    if block.is_waterlogged(block_state.id) {
        return false;
    }
    // Nor a bubble column, which holds a water source (vanilla `BubbleColumnBlock.getFluidState`;
    // `WaterFluid.canBeReplacedWith` refuses water).
    if block == &Block::BUBBLE_COLUMN
        && (fluid.id == Fluid::WATER.id || fluid.id == Fluid::FLOWING_WATER.id)
    {
        return false;
    }

    // Fluid Logic
    if let Some(other_fluid) = Fluid::from_state_id(block_state.id) {
        if !fluid.matches_type(other_fluid) {
            return true;
        }
        // Replace current fluid if it is a falling source
        if other_fluid.is_source(block_state.id) && other_fluid.is_falling(block_state.id) {
            return true;
        }
    }

    let id = block.id;

    // Blocks that fluid should never replace
    if block.has_tag(&tag::Block::MINECRAFT_DOORS)
        || block.has_tag(&tag::Block::MINECRAFT_BEDS)
        || block.has_tag(&tag::Block::MINECRAFT_LEAVES)
        || block.has_tag(&tag::Block::MINECRAFT_PRESSURE_PLATES)
        || block.has_tag(&tag::Block::C_CLUSTERS)
        || block.has_tag(&tag::Block::MINECRAFT_WALL_CORALS)
        || block.has_tag(&tag::Block::MINECRAFT_SHULKER_BOXES)
        || block.has_tag(&tag::Block::MINECRAFT_PORTALS)
        || id == Block::BELL.id
        || id == Block::BIG_DRIPLEAF.id
        || id == Block::BIG_DRIPLEAF_STEM.id
        || id == Block::SMALL_DRIPLEAF.id
        || id == Block::CAKE.id
        || id == Block::CONDUIT.id
        || id == Block::CAMPFIRE.id
        || id == Block::DRAGON_EGG.id
        || id == Block::KELP.id
        || id == Block::KELP_PLANT.id
        || id == Block::SEAGRASS.id
        || id == Block::TALL_SEAGRASS.id
        || id == Block::LADDER.id
        || id == Block::POINTED_DRIPSTONE.id
        || id == Block::SCAFFOLDING.id
    {
        return false;
    }

    // Only replace air, explicitly replaceable blocks, or carpets
    block_state.replaceable()
        || id == Block::AIR.id
        || block.has_tag(&tag::Block::MINECRAFT_WOOL_CARPETS)
        // Only use PistonBehavior::Destroy if it didn't pass the checks above
        || block_state.piston_behavior == pumpkin_data::block_state::PistonBehavior::Destroy
}

const EPSILON: f64 = 1.0e-7;

fn is_full_block(shapes: &[BoundingBox]) -> bool {
    matches!(shapes, [shape] if shape.min.x <= 0.0 && shape.min.y <= 0.0 && shape.min.z <= 0.0
        && shape.max.x >= 1.0 && shape.max.y >= 1.0 && shape.max.z >= 1.0)
}

/// The two axes across a face on `axis`.
const fn face_axes(axis: Axis) -> (Axis, Axis) {
    match axis {
        Axis::X => (Axis::Y, Axis::Z),
        Axis::Y => (Axis::X, Axis::Z),
        Axis::Z => (Axis::X, Axis::Y),
    }
}

/// Vanilla `Shapes.mergedFaceOccludes`: whether `shape`'s face on `direction` and `occluder`'s
/// opposite face together cover the whole square.
fn merged_face_occludes(
    shape: &[BoundingBox],
    occluder: &[BoundingBox],
    direction: BlockDirection,
) -> bool {
    if is_full_block(shape) && is_full_block(occluder) {
        return true;
    }
    if shape.is_empty() && occluder.is_empty() {
        return false;
    }
    let axis = match direction {
        BlockDirection::West | BlockDirection::East => Axis::X,
        BlockDirection::Down | BlockDirection::Up => Axis::Y,
        BlockDirection::North | BlockDirection::South => Axis::Z,
    };
    let (first, second) = if direction.positive() {
        (shape, occluder)
    } else {
        (occluder, shape)
    };
    // The last layer of `first` and the first layer of `second`, if they reach the face.
    let first_max = first
        .iter()
        .map(|b| b.max.get_axis(axis))
        .fold(f64::NEG_INFINITY, f64::max);
    let second_min = second
        .iter()
        .map(|b| b.min.get_axis(axis))
        .fold(f64::INFINITY, f64::min);
    let (u, v) = face_axes(axis);
    let mut rects: Vec<&BoundingBox> = Vec::new();
    if (first_max - 1.0).abs() < EPSILON {
        rects.extend(
            first
                .iter()
                .filter(|b| (b.max.get_axis(axis) - 1.0).abs() < EPSILON),
        );
    }
    if second_min.abs() < EPSILON {
        rects.extend(
            second
                .iter()
                .filter(|b| b.min.get_axis(axis).abs() < EPSILON),
        );
    }
    let mut us = vec![0.0, 1.0];
    let mut vs = vec![0.0, 1.0];
    for b in &rects {
        us.extend([
            b.min.get_axis(u).clamp(0.0, 1.0),
            b.max.get_axis(u).clamp(0.0, 1.0),
        ]);
        vs.extend([
            b.min.get_axis(v).clamp(0.0, 1.0),
            b.max.get_axis(v).clamp(0.0, 1.0),
        ]);
    }
    us.sort_by(f64::total_cmp);
    us.dedup_by(|a, b| (*a - *b).abs() < EPSILON);
    vs.sort_by(f64::total_cmp);
    vs.dedup_by(|a, b| (*a - *b).abs() < EPSILON);
    us.windows(2).all(|cu| {
        let cu = f64::midpoint(cu[0], cu[1]);
        vs.windows(2).all(|cv| {
            let cv = f64::midpoint(cv[0], cv[1]);
            rects.iter().any(|b| {
                b.min.get_axis(u) < cu
                    && cu < b.max.get_axis(u)
                    && b.min.get_axis(v) < cv
                    && cv < b.max.get_axis(v)
            })
        })
    })
}

/// Vanilla `FlowingFluid.canPassThroughWall`: fluid crosses from `source` to `target` (its
/// neighbour on `direction`) unless either collision shape is a full block or their faces
/// together close the gap.
#[must_use]
pub fn can_pass_through_wall(
    direction: BlockDirection,
    source_pos: &BlockPos,
    source: &BlockState,
    target_pos: &BlockPos,
    target: &BlockState,
) -> bool {
    let target_shape: Vec<_> = target.get_block_collision_shapes_at(target_pos).collect();
    if is_full_block(&target_shape) {
        return false;
    }
    let source_shape: Vec<_> = source.get_block_collision_shapes_at(source_pos).collect();
    if is_full_block(&source_shape) {
        return false;
    }
    if source_shape.is_empty() && target_shape.is_empty() {
        return true;
    }
    !merged_face_occludes(&source_shape, &target_shape, direction)
}
