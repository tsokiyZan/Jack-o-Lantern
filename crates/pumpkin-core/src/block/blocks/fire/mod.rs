use std::sync::Arc;

use pumpkin_data::dimension::Dimension;
use pumpkin_data::tag::Taggable;
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockDirection, BlockStateId, tag};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::random::xoroshiro128::Xoroshiro;
use pumpkin_util::random::{RandomGenerator, RandomImpl};
use rand::RngExt;
use soul_fire::SoulFireBlock;

use crate::block::blocks::fire::fire::FireBlock;
use crate::block::{BlockBehaviour, CanPlaceAtArgs, OnEntityCollisionArgs, PlacedArgs};
use crate::entity::EntityBase;
use crate::entity::inside_block_effect::InsideBlockEffectType;
use crate::world::World;
use crate::world::portal::nether::NetherPortal;
use pumpkin_data::damage::DamageType;
use std::sync::atomic::Ordering;

#[expect(clippy::module_inception)]
pub mod fire;
pub mod soul_fire;

pub struct FireBlockBase;

impl FireBlockBase {
    /// Vanilla `BaseFireBlock.onPlace`: light a nether portal, else go out at once where the
    /// fire can't survive.
    pub fn on_place(args: &PlacedArgs<'_>, can_survive: impl FnOnce() -> bool) {
        if Block::from_state_id(args.old_state_id) == args.block {
            return;
        }
        let dimension = &args.world.dimension;
        // `inPortalDimension`.
        if (dimension == &Dimension::OVERWORLD || dimension == &Dimension::THE_NETHER)
            && let Some(portal) = NetherPortal::get_new_portal(
                args.world,
                args.position,
                pumpkin_data::block_properties::HorizontalAxis::X,
            )
        {
            portal.create(args.world);
            return;
        }
        if !can_survive() {
            // `removeBlock(pos, false)`: fire holds no fluid, so this leaves air.
            args.world.set_block_state(
                args.position,
                Block::AIR.default_state.id,
                pumpkin_world::world::BlockFlags::NOTIFY_ALL,
            );
        }
    }

    pub fn get_fire_type(world: &World, pos: &BlockPos) -> Block {
        let block = world.get_block(&pos.down());
        if SoulFireBlock::is_soul_base(block) {
            return Block::SOUL_FIRE;
        }
        Block::FIRE
    }

    /// Vanilla `BaseFireBlock.getState`.
    pub fn get_state(world: &World, pos: &BlockPos) -> BlockStateId {
        if SoulFireBlock::is_soul_base(world.get_block(&pos.down())) {
            Block::SOUL_FIRE.default_state.id
        } else {
            fire::FireBlock.get_state_for_position(world, &Block::FIRE, pos)
        }
    }

    #[must_use]
    pub fn can_place_on(block: &Block) -> bool {
        // Make sure the block below is not a fire block or fluid block
        block != &Block::SOUL_FIRE
            && block != &Block::FIRE
            && block != &Block::WATER
            && block != &Block::LAVA
    }

    pub fn is_soul_fire(world: &Arc<World>, block_pos: &BlockPos) -> bool {
        let block = world.get_block(&block_pos.down());
        block.has_tag(&tag::Block::MINECRAFT_SOUL_FIRE_BASE_BLOCKS)
    }

    pub fn can_place_at(world: &Arc<World>, block_pos: &BlockPos) -> bool {
        let block_state = world.get_block_state(block_pos);
        if !block_state.is_air() {
            return false;
        }
        if Self::is_soul_fire(world, block_pos) {
            SoulFireBlock.can_place_at(CanPlaceAtArgs {
                server: None,
                world: Some(world),
                block_accessor: world.as_ref(),
                block: &Block::SOUL_FIRE,
                state: Block::SOUL_FIRE.default_state,
                position: block_pos,
                direction: None,
                player: None,
                use_item_on: None,
            })
        } else {
            FireBlock.can_place_at(CanPlaceAtArgs {
                server: None,
                world: Some(world),
                block_accessor: world.as_ref(),
                block: &Block::FIRE,
                state: Block::FIRE.default_state,
                position: block_pos,
                direction: None,
                player: None,
                use_item_on: None,
            }) || Self::should_light_portal_at(world, block_pos, BlockDirection::Up)
        }
    }

    pub fn should_light_portal_at(
        world: &Arc<World>,
        block_pos: &BlockPos,
        direction: BlockDirection,
    ) -> bool {
        let dimension = &world.dimension;
        if dimension != &Dimension::OVERWORLD && dimension != &Dimension::THE_NETHER {
            return false;
        }
        let mut found = false;

        for dir in BlockDirection::all() {
            if world.get_block(&block_pos.offset(dir.to_offset())) == &Block::OBSIDIAN {
                found = true;
                break;
            }
        }

        if !found {
            return false;
        }

        let dir = if direction.is_horizontal() {
            direction
                .rotate_counter_clockwise()
                .to_horizontal_axis()
                .unwrap_or(pumpkin_data::block_properties::HorizontalAxis::X)
        } else {
            BlockDirection::random_horizontal(&mut RandomGenerator::Xoroshiro(
                Xoroshiro::from_seed(rand::rng().random()),
            ))
            .to_axis()
        };
        NetherPortal::get_new_portal(world, block_pos, dir).is_some()
    }

    /// Vanilla `BaseFireBlock.entityInside`, `fire_damage` being the block's own damage.
    pub fn entity_inside(args: &OnEntityCollisionArgs<'_>, fire_damage: f32) {
        args.effect_applier
            .apply(InsideBlockEffectType::ClearFreeze);
        args.effect_applier.apply(InsideBlockEffectType::FireIgnite);
        args.effect_applier
            .run_after(InsideBlockEffectType::FireIgnite, move |entity| {
                entity.damage(entity, fire_damage, DamageType::IN_FIRE);
            });
    }

    /// Vanilla `BaseFireBlock.fireIgnite`.
    pub fn fire_ignite(caller: &dyn EntityBase) {
        let entity = caller.get_entity();
        if entity.is_fire_immune() {
            return;
        }
        let ticks = entity.fire_ticks.load(Ordering::Relaxed);
        if ticks < 0 {
            caller.set_remaining_fire_ticks(ticks + 1);
        } else if caller.get_player().is_some() {
            let added_fire_ticks = entity
                .world
                .load()
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .next_int_range(1, 3);
            caller.set_remaining_fire_ticks(ticks + added_fire_ticks);
        }

        if entity.fire_ticks.load(Ordering::Relaxed) >= 0 {
            caller.set_on_fire_for(8.0);
        }
    }

    fn broken(world: &World, block_pos: BlockPos) {
        world.sync_world_event(WorldEvent::SoundExtinguishFire, block_pos, 0);
    }
}
