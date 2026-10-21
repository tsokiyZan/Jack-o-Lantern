use pumpkin_data::BlockState;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;

use crate::block::{BlockBehaviour, FallOnArgs, OnEntityCollisionArgs, PathComputationType};
use crate::entity::EntityBase;
use crate::entity::inside_block_effect::InsideBlockEffectType;
use pumpkin_data::Block;
use pumpkin_util::random::RandomImpl;

#[pumpkin_block("minecraft:powder_snow")]
pub struct PowderSnowBlock;

const FALLING_COLLISION_SHAPE: BoundingBox =
    BoundingBox::new_array([0.0, 0.0, 0.0], [1.0, 0.9, 1.0]);

pub(crate) fn can_entity_walk_on_powder_snow(entity: &dyn EntityBase) -> bool {
    let base = entity.get_entity();
    if base
        .entity_type
        .has_tag(&tag::EntityType::MINECRAFT_POWDER_SNOW_WALKABLE_MOBS)
    {
        return true;
    }

    let Some(living) = entity.get_living_entity() else {
        return false;
    };

    living.entity_equipment.try_lock().is_ok_and(|equipment| {
        equipment
            .equipment
            .get(&EquipmentSlot::FEET)
            .is_some_and(|boots| boots.item == &Item::LEATHER_BOOTS)
    })
}

fn is_entity_above_block(entity: &crate::entity::Entity, position: &BlockPos) -> bool {
    crate::entity::spawn_util::is_above(entity, position, 1.0)
}

/// Vanilla `Entity.isDescending`: the shift key, not the motion.
pub(crate) fn is_entity_descending(entity: &crate::entity::Entity) -> bool {
    entity.is_sneaking()
}

pub(crate) fn collision_shape_for_entity(
    entity: &dyn EntityBase,
    position: &BlockPos,
) -> Option<BoundingBox> {
    if entity.get_entity().fall_distance.load() > 2.5 {
        return Some(FALLING_COLLISION_SHAPE);
    }

    let base = entity.get_entity();
    if base.entity_type == &EntityType::FALLING_BLOCK {
        return Some(BoundingBox::full_block());
    }

    if can_entity_walk_on_powder_snow(entity)
        && is_entity_above_block(base, position)
        && !is_entity_descending(base)
    {
        return Some(BoundingBox::full_block());
    }

    None
}

pub(crate) fn inside_collision_shape_for_entity(
    entity: &dyn EntityBase,
    position: &BlockPos,
) -> BoundingBox {
    collision_shape_for_entity(entity, position).unwrap_or_else(BoundingBox::full_block)
}

impl BlockBehaviour for PowderSnowBlock {
    /// Vanilla `PowderSnowBlock.fallOn`: no fall damage, only the entity's fall sound.
    fn fall_on(&self, args: FallOnArgs<'_>) {
        if args.fall_distance >= 4.0
            && let Some(living) = args.entity.get_living_entity()
        {
            let (small, big) = living.get_fall_sounds();
            let sound = if args.fall_distance < 7.0 { small } else { big };
            args.entity.play_sound_at_self(sound, 1.0, 1.0);
        }
    }

    /// Vanilla `PowderSnowBlock.entityInside`.
    fn on_entity_collision(&self, args: OnEntityCollisionArgs<'_>) {
        let entity = args.entity.get_entity();
        // `getInBlockState().is(this)`: a living entity only sinks in once its feet are in.
        if args.entity.get_living_entity().is_none()
            || args.world.get_block(&entity.block_pos.load()) == &Block::POWDER_SNOW
        {
            entity.slow_movement(
                args.state,
                Vector3::new(f64::from(0.9f32), 1.5, f64::from(0.9f32)),
            );
            let known_movement = args.entity.get_known_movement();
            let is_moving = known_movement.x != 0.0 || known_movement.z != 0.0;
            if is_moving {
                let mut random = args
                    .world
                    .random
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if random.next_bool() {
                    // The snowflake's speeds: `Mth.randomBetween(random, -1, 1) / 12`, twice.
                    // The particle itself needs per-axis speeds the packet can't carry yet.
                    random.next_f32();
                    random.next_f32();
                }
            }
        }

        let world = args.world.clone();
        let position = *args.position;
        args.effect_applier
            .run_before(InsideBlockEffectType::Extinguish, move |entity| {
                if entity.get_entity().is_on_fire()
                    && (world.level_info.load().game_rules.mob_griefing
                        || entity.get_player().is_some())
                {
                    let _ = world.break_block(
                        &position,
                        None,
                        BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
                    );
                }
            });
        args.effect_applier.apply(InsideBlockEffectType::Freeze);
        args.effect_applier.apply(InsideBlockEffectType::Extinguish);
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        true
    }
}
