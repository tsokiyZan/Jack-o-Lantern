use crate::block::UseWithItemArgs;
use crate::block::registry::BlockActionResult;
use crate::entity::Entity;
use crate::entity::item::ItemEntity;
use crate::world::loot::LootContextParameters;
use pumpkin_data::block_properties::WallTorchLikeProperties;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, HorizontalFacingExt};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

/// Vanilla `BuiltInLootTables.CARVE_PUMPKIN`.
const CARVE_PUMPKIN_LOOT_TABLE: &str = "minecraft:carve/pumpkin";

#[pumpkin_block("minecraft:pumpkin")]
pub struct PumpkinBlock;

impl crate::block::BlockBehaviour for PumpkinBlock {
    /// Vanilla `PumpkinBlock.useItemOn`: shears carve the pumpkin toward the clicked side (or
    /// back at the player from the top or bottom), dropping the seeds out of that side.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        if args.item_stack.item != &Item::SHEARS {
            return BlockActionResult::PassToDefaultBlockAction;
        }
        let direction = args.hit.face.to_horizontal_facing().unwrap_or_else(|| {
            args.player
                .living_entity
                .entity
                .get_horizontal_facing()
                .opposite()
        });
        let step = direction.to_block_direction().to_offset();

        // `dropFromBlockInteractLootTable(CARVE_PUMPKIN, ..)`.
        if let Some(loot_table) = args.world.get_loot_table(CARVE_PUMPKIN_LOOT_TABLE) {
            let state = args.world.get_block_state(args.position);
            let params = LootContextParameters {
                block_state: Some(state),
                position: Some(args.position.to_centered_f64()),
                tool: Some(args.item_stack.clone()),
                ..Default::default()
            };
            for seeds in
                crate::world::loot::generate_loot_from_handle(&loot_table, args.world, 0, &params)
            {
                let position = Vector3::new(
                    f64::from(args.position.0.x) + 0.5 + f64::from(step.x) * 0.65,
                    f64::from(args.position.0.y) + 0.1,
                    f64::from(args.position.0.z) + 0.5 + f64::from(step.z) * 0.65,
                );
                // `new ItemEntity(level, x, y, z, stack)` keeps a pickup delay of 0.
                let entity = ItemEntity::new(
                    Entity::new(args.world.clone(), position, &EntityType::ITEM),
                    seeds,
                );
                entity.set_pickup_delay(0);
                let velocity = {
                    let mut random = args
                        .world
                        .random
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let x = 0.05 * f64::from(step.x) + random.next_f64() * 0.02;
                    let z = 0.05 * f64::from(step.z) + random.next_f64() * 0.02;
                    Vector3::new(x, 0.05, z)
                };
                entity.get_entity().velocity.store(velocity);
                args.world.spawn_entity(Arc::new(entity));
            }
        }

        args.world.play_sound(
            Sound::BlockPumpkinCarve,
            SoundCategory::Blocks,
            &args.position.to_centered_f64(),
        );
        let mut props = WallTorchLikeProperties::default(&Block::CARVED_PUMPKIN);
        props.facing = direction;
        args.world.set_block_state(
            args.position,
            props.to_state_id(&Block::CARVED_PUMPKIN),
            BlockFlags::NOTIFY_ALL,
        );
        args.player
            .hurt_and_break_stack(args.item_stack, args.equipment_slot, 1);
        args.world.game_event_from(
            Some(args.player.as_ref()),
            GameEvent::Shear,
            args.position.to_centered_f64(),
        );
        args.player.award_item_used(&Item::SHEARS);
        BlockActionResult::Success
    }
}
