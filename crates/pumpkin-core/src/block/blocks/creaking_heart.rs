use std::sync::Arc;

use pumpkin_data::block_properties::{
    Axis, CreakingHeartLikeProperties, CreakingHeartState, PaleOakWoodLikeProperties,
};
use pumpkin_data::damage::DamageType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{BlockDirection, BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::{BlockAccessor, BlockFlags};
use rand::RngExt;

use crate::block::entities::creaking_heart::CreakingHeartBlockEntity;
use crate::block::{
    BlockBehaviour, ExplodeArgs, GetComparatorOutputArgs, GetStateForNeighborUpdateArgs,
    OnPlaceArgs, OnScheduledTickArgs, OnStateReplacedArgs, PlayerWillDestroyArgs,
};
use crate::entity::EntityBase;
use crate::entity::damage_source::DamageSource;
use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::player::Player;
use crate::plugin::api::events::block::block_exp::BlockExpEvent;
use crate::world::World;

/// The experience vanilla `tryAwardExperience` pops for a natural heart.
const NATURAL_HEART_EXPERIENCE: (i32, i32) = (20, 24);

#[pumpkin_block("minecraft:creaking_heart")]
pub struct CreakingHeartBlock;

impl CreakingHeartBlock {
    /// Vanilla `CreakingHeartBlock.updateState`: a heart between its logs takes root.
    fn update_state(state: BlockStateId, world: &World, pos: &BlockPos) -> BlockStateId {
        let mut props = CreakingHeartLikeProperties::from_state_id(state);
        let has_logs = Self::has_required_logs(world, pos, props.axis);
        let disabled = props.creaking_heart_state == CreakingHeartState::Uprooted;
        if !(has_logs && disabled) {
            return state;
        }
        props.creaking_heart_state = if world.creaking_active(pos) {
            CreakingHeartState::Awake
        } else {
            CreakingHeartState::Dormant
        };
        props.to_state_id(&pumpkin_data::Block::CREAKING_HEART)
    }

    /// Vanilla `CreakingHeartBlock.hasRequiredLogs`: pale oak logs along `axis` on both sides.
    #[must_use]
    pub fn has_required_logs(world: &dyn BlockAccessor, pos: &BlockPos, axis: Axis) -> bool {
        let directions = match axis {
            Axis::X => [BlockDirection::West, BlockDirection::East],
            Axis::Y => [BlockDirection::Down, BlockDirection::Up],
            Axis::Z => [BlockDirection::North, BlockDirection::South],
        };
        directions.iter().all(|dir| {
            let (block, state) = world.get_block_and_state(&pos.offset(dir.to_offset()));
            block.has_tag(&tag::Block::MINECRAFT_PALE_OAK_LOGS)
                && PaleOakWoodLikeProperties::from_state_id(state.id).axis == axis
        })
    }

    /// Vanilla `CreakingHeartBlock.tryAwardExperience`.
    fn try_award_experience(
        player: &Player,
        state: &BlockState,
        world: &Arc<World>,
        pos: &BlockPos,
    ) {
        // Vanilla `Player.preventsBlockDrops`: the `instabuild` ability.
        if player
            .abilities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .creative
            || player.is_spectator()
            || !CreakingHeartLikeProperties::from_state_id(state.id).natural
        {
            return;
        }
        let amount =
            rand::rng().random_range(NATURAL_HEART_EXPERIENCE.0..=NATURAL_HEART_EXPERIENCE.1);
        // Vanilla `Block.popExperience`.
        if !world.level_info.load().game_rules.block_drops {
            return;
        }
        let mut event = BlockExpEvent::new(*pos, world.clone(), amount);
        if let Some(server) = world.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.exp > 0 {
            ExperienceOrbEntity::spawn(world, pos.to_centered_f64(), event.exp as u32);
        }
    }
}

impl BlockBehaviour for CreakingHeartBlock {
    /// Vanilla `CreakingHeartBlock.getStateForPlacement`.
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = CreakingHeartLikeProperties::from_state_id(args.block.default_state.id);
        props.axis = args.direction.to_axis();
        Self::update_state(props.to_state_id(args.block), args.world, args.position)
    }

    /// Vanilla `CreakingHeartBlock.updateShape`.
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        args.world
            .schedule_block_tick(args.block, *args.position, 1, TickPriority::Normal);
        args.state_id
    }

    /// Vanilla `CreakingHeartBlock.tick`.
    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let state = args.world.get_block_state_id(args.position);
        let new_state = Self::update_state(state, args.world, args.position);
        if new_state != state {
            args.world
                .set_block_state(args.position, new_state, BlockFlags::NOTIFY_ALL);
        }
    }

    /// Vanilla `CreakingHeartBlock.onExplosionHit`, before `super.onExplosionHit`.
    fn on_explosion_hit(&self, args: ExplodeArgs<'_>) {
        let Some(block_entity) = args.world.get_block_entity(args.position) else {
            return;
        };
        let Some(heart) = block_entity
            .as_any()
            .downcast_ref::<CreakingHeartBlockEntity>()
        else {
            return;
        };
        let damage_source = args.explosion.damage_source();
        heart.remove_protector(
            args.world,
            Some(DamageSource {
                damage_type: damage_source.damage_type,
                direct: damage_source.direct.as_deref(),
                causing: damage_source.causing.as_deref(),
            }),
        );
        if let Some(indirect_source) = args.explosion.indirect_source_entity()
            && let Some(player) = indirect_source.get_player()
        {
            let state = args.world.get_block_state(args.position);
            Self::try_award_experience(player, state, args.world, args.position);
        }
    }

    /// Vanilla `CreakingHeartBlock.playerWillDestroy`.
    fn player_will_destroy(&self, args: PlayerWillDestroyArgs<'_>) {
        let Some(block_entity) = args.world.get_block_entity(args.position) else {
            return;
        };
        let Some(heart) = block_entity
            .as_any()
            .downcast_ref::<CreakingHeartBlockEntity>()
        else {
            return;
        };
        let player: &dyn EntityBase = args.player.as_ref();
        heart.remove_protector(
            args.world,
            Some(DamageSource {
                damage_type: DamageType::PLAYER_ATTACK,
                direct: Some(player),
                causing: Some(player),
            }),
        );
        Self::try_award_experience(args.player, args.state, args.world, args.position);
    }

    /// Vanilla `CreakingHeartBlock.affectNeighborsAfterRemoval`.
    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        args.world
            .update_neighbour_for_output_signal(args.position, args.block);
    }

    /// Vanilla `CreakingHeartBlock.getAnalogOutputSignal`.
    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        if CreakingHeartLikeProperties::from_state_id(args.state.id).creaking_heart_state
            == CreakingHeartState::Uprooted
        {
            return Some(0);
        }
        Some(
            args.world
                .get_block_entity(args.position)
                .and_then(|entity| {
                    entity
                        .as_any()
                        .downcast_ref::<CreakingHeartBlockEntity>()
                        .map(CreakingHeartBlockEntity::get_analog_output_signal)
                })
                .unwrap_or(0),
        )
    }
}
