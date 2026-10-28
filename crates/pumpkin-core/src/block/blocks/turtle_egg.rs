use std::sync::Arc;

use pumpkin_data::block_properties::TurtleEggLikeProperties;
use pumpkin_data::entity::{EntityPose, EntityType};
use pumpkin_data::environment_attribute::EnvironmentAttribute;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::Taggable;
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockState, BlockStateId, tag};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::GameMode;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use pumpkin_world::world::{BlockAccessor, BlockFlags};
use uuid::Uuid;

use crate::block::{
    BlockBehaviour, BlockIsReplacing, BrokenArgs, CanUpdateAtArgs, FallOnArgs, OnEntityStepArgs,
    OnPlaceArgs, PlacedArgs, RandomTickArgs,
};
use crate::entity::EntityBase;
use crate::entity::passive::turtle::TurtleEntity;
use crate::entity::r#type::from_type;
use crate::world::World;
use crate::world::game_event::GameEventContext;

type TurtleEggProperties = TurtleEggLikeProperties;

#[pumpkin_block("minecraft:turtle_egg")]
pub struct TurtleEggBlock;

impl TurtleEggBlock {
    #[must_use]
    pub fn is_sand(world: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        world.get_block(pos).has_tag(&tag::Block::MINECRAFT_SAND)
    }

    #[must_use]
    pub fn on_sand(world: &dyn BlockAccessor, pos: &BlockPos) -> bool {
        Self::is_sand(world, &pos.down())
    }

    /// Vanilla `TurtleEggBlock.canDestroyEgg`.
    #[must_use]
    pub fn can_destroy_egg(world: &World, pos: &BlockPos, entity: &dyn EntityBase) -> bool {
        let entity_type = entity.get_entity().entity_type;
        if entity_type == &EntityType::TURTLE || entity_type == &EntityType::BAT {
            return false;
        }
        if let Some(player) = entity.get_player() {
            return !world.is_in_spawn_protection(player, pos);
        }
        entity.get_living_entity().is_some() && world.level_info.load().game_rules.mob_griefing
    }

    /// `0.9F + random.nextFloat() * 0.2F` from the level random: the pitch of every egg sound.
    fn egg_sound_pitch(world: &World) -> f32 {
        0.9 + world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_f32()
            * 0.2
    }

    /// Vanilla `TurtleEggBlock.decreaseEggs`: one egg breaks; the last one destroys the block.
    pub fn decrease_eggs(
        world: &Arc<World>,
        pos: &BlockPos,
        state_id: BlockStateId,
        block: &Block,
    ) {
        world.play_sound_fine(
            Sound::EntityTurtleEggBreak,
            SoundCategory::Blocks,
            &pos.to_centered_f64(),
            0.7,
            Self::egg_sound_pitch(world),
        );

        let props = TurtleEggProperties::from_state_id(state_id);
        if props.eggs <= 1 {
            // `destroyBlock(pos, false)`.
            world.break_block(pos, None, BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS);
        } else {
            let mut new_props = props;
            new_props.eggs -= 1;
            world.set_block_state(
                pos,
                new_props.to_state_id(block),
                BlockFlags::NOTIFY_LISTENERS,
            );
            world.game_event(
                GameEvent::BlockDestroy,
                pos.to_centered_f64(),
                GameEventContext::of_state(BlockState::from_id(state_id)),
            );
            world.sync_world_event(
                WorldEvent::ParticlesAndSoundDestroyBlock,
                *pos,
                i32::from(state_id.as_u16()),
            );
        }
    }

    /// Vanilla `TurtleEggBlock.destroyEgg`.
    pub fn destroy_egg(
        world: &Arc<World>,
        pos: &BlockPos,
        state_id: BlockStateId,
        block: &Block,
        entity: &dyn EntityBase,
        randomness: i32,
    ) {
        if block != &Block::TURTLE_EGG || !Self::can_destroy_egg(world, pos, entity) {
            return;
        }
        let roll = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_bounded_i32(randomness);
        if roll == 0 {
            Self::decrease_eggs(world, pos, state_id, block);
        }
    }

    /// Vanilla `TurtleEggBlock.shouldUpdateHatchLevel`: the `TURTLE_EGG_HATCH_CHANCE`
    /// environment attribute against the level random.
    fn should_update_hatch_level(world: &World, pos: &BlockPos) -> bool {
        let chance = world
            .environment_attributes()
            .get_value_f32(EnvironmentAttribute::GameplayTurtleEggHatchChance, pos);
        chance > 0.0
            && world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .next_f32()
                < chance
    }
}

impl BlockBehaviour for TurtleEggBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        if args.player.get_entity().pose.load() != EntityPose::Crouching
            && let BlockIsReplacing::Itself(state_id) = args.replacing
        {
            let mut properties = TurtleEggProperties::from_state_id(state_id);
            if properties.eggs < 4 {
                properties.eggs += 1;
            }
            return properties.to_state_id(args.block);
        }

        let properties = TurtleEggProperties::default(args.block);
        properties.to_state_id(args.block)
    }

    /// Vanilla `TurtleEggBlock.onPlace`: every state change on sand (an added egg, a crack)
    /// sends the placement particles.
    fn placed(&self, args: PlacedArgs<'_>) {
        if Self::on_sand(args.world.as_ref(), args.position) {
            args.world.sync_world_event(
                WorldEvent::ParticlesTurtleEggPlacement,
                *args.position,
                15,
            );
        }
    }

    fn can_update_at(&self, args: CanUpdateAtArgs<'_>) -> bool {
        let b = BlockAccessor::get_block(args.world, args.position);
        args.player.get_entity().pose.load() != EntityPose::Crouching
            && TurtleEggProperties::from_state_id(args.state_id).eggs < 4
            && args.block.id == b.id
    }

    fn on_entity_step(&self, args: OnEntityStepArgs<'_>) {
        if !args.entity.is_stepping_carefully() {
            Self::destroy_egg(
                args.world,
                args.position,
                args.state.id,
                args.block,
                args.entity,
                100,
            );
        }
    }

    /// Vanilla `TurtleEggBlock.fallOn`: anything but a zombie may crush an egg on landing.
    fn fall_on(&self, args: FallOnArgs<'_>) {
        let entity_type = args.entity.get_entity().entity_type;
        // `entity instanceof Zombie`.
        let is_zombie = [
            &EntityType::ZOMBIE,
            &EntityType::HUSK,
            &EntityType::DROWNED,
            &EntityType::ZOMBIE_VILLAGER,
            &EntityType::ZOMBIFIED_PIGLIN,
        ]
        .contains(&entity_type);
        if !is_zombie {
            Self::destroy_egg(
                args.world,
                args.position,
                args.state.id,
                args.block,
                args.entity,
                3,
            );
        }
        crate::block::default_fall_on(&args);
    }

    /// Vanilla `TurtleEggBlock.randomTick`.
    fn random_tick(&self, args: RandomTickArgs<'_>) {
        if !Self::should_update_hatch_level(args.world, args.position)
            || !Self::on_sand(args.world.as_ref(), args.position)
        {
            return;
        }

        let state_id = args.world.get_block_state_id(args.position);
        let mut props = TurtleEggProperties::from_state_id(state_id);
        let old_state = BlockState::from_id(state_id);

        if props.hatch < 2 {
            args.world.play_sound_fine(
                Sound::EntityTurtleEggCrack,
                SoundCategory::Blocks,
                &args.position.to_centered_f64(),
                0.7,
                Self::egg_sound_pitch(args.world),
            );
            props.hatch += 1;
            args.world.set_block_state(
                args.position,
                props.to_state_id(args.block),
                BlockFlags::NOTIFY_LISTENERS,
            );
            args.world.game_event(
                GameEvent::BlockChange,
                args.position.to_centered_f64(),
                GameEventContext::of_state(old_state),
            );
            return;
        }

        args.world.play_sound_fine(
            Sound::EntityTurtleEggHatch,
            SoundCategory::Blocks,
            &args.position.to_centered_f64(),
            0.7,
            Self::egg_sound_pitch(args.world),
        );
        // `removeBlock(pos, false)`.
        args.world
            .set_block_state(args.position, BlockStateId::AIR, BlockFlags::NOTIFY_ALL);
        args.world.game_event(
            GameEvent::BlockDestroy,
            args.position.to_centered_f64(),
            GameEventContext::of_state(old_state),
        );

        for i in 0..props.eggs {
            args.world.sync_world_event(
                WorldEvent::ParticlesAndSoundDestroyBlock,
                *args.position,
                i32::from(state_id.as_u16()),
            );
            let spawn_pos = Vector3::new(
                f64::from(args.position.0.x) + 0.3 + f64::from(i) * 0.2,
                f64::from(args.position.0.y),
                f64::from(args.position.0.z) + 0.3,
            );
            let turtle = from_type(&EntityType::TURTLE, spawn_pos, args.world, Uuid::new_v4());
            turtle.get_entity().set_age(-24000);
            turtle.refresh_dimensions();
            set_turtle_home(turtle.as_ref(), *args.position);
            args.world.spawn_entity(turtle);
        }
    }

    /// Vanilla `TurtleEggBlock.playerDestroy` (survival only): the eggs after the broken one
    /// stay, one fewer.
    fn broken(&self, args: BrokenArgs<'_>) {
        if args.player.gamemode.load() == GameMode::Creative {
            return;
        }
        Self::decrease_eggs(args.world, args.position, args.state.id, args.block);
    }
}

/// `Turtle.setHomePos` on a turtle made by `from_type`.
fn set_turtle_home(turtle: &dyn EntityBase, home: BlockPos) {
    if let Some(turtle) = turtle.cast_any().downcast_ref::<TurtleEntity>() {
        turtle.set_home_pos(home);
    }
}
