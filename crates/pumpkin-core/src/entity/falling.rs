use crossbeam::atomic::AtomicCell;
use pumpkin_data::Block;
use pumpkin_data::BlockState;
use pumpkin_data::BlockStateId;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::world::WorldEvent;
use pumpkin_protocol::bedrock::client::CUpdateBlock;
use pumpkin_protocol::java::client::play::CBlockUpdate;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use crate::{
    block::{
        blocks::{anvil::AnvilBlock, falling::FallingBlock},
        registry::can_replace_with_other_block,
    },
    entity::{Entity, EntityBase, living::LivingEntity},
    server::Server,
    world::World,
};

pub struct FallingEntity {
    entity: Entity,
    block_state_id: AtomicCell<BlockStateId>,
    /// Vanilla `FallingBlockEntity.cancelDrop`: the block breaks on landing.
    cancel_drop: AtomicBool,
    /// Vanilla `hurtEntities`, `fallDamagePerDistance` and `fallDamageMax`.
    hurt_entities: AtomicBool,
    fall_damage_per_distance: AtomicCell<f32>,
    fall_damage_max: AtomicI32,
}

impl FallingEntity {
    pub const fn new(entity: Entity, block_state_id: BlockStateId) -> Self {
        Self {
            entity,
            block_state_id: AtomicCell::new(block_state_id),
            cancel_drop: AtomicBool::new(false),
            hurt_entities: AtomicBool::new(false),
            fall_damage_per_distance: AtomicCell::new(0.0),
            fall_damage_max: AtomicI32::new(40),
        }
    }

    /// Vanilla `FallingBlockEntity.setHurtsEntities`.
    pub fn set_hurts_entities(&self, damage_per_distance: f32, damage_max: i32) {
        self.hurt_entities.store(true, Ordering::Relaxed);
        self.fall_damage_per_distance.store(damage_per_distance);
        self.fall_damage_max.store(damage_max, Ordering::Relaxed);
    }

    /// Replaced the current Block and Spawns a new Falling one (synchronous)
    pub fn replace_spawn(world: &Arc<World>, position: BlockPos, block_state: BlockStateId) {
        // Replace the original block, TODO: use fluid state
        world.set_block_state(
            &position,
            Block::AIR.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );

        let position = position.0.to_f64().add_raw(0.5, 0.0, 0.5);
        let entity = Entity::new(world.clone(), position, &EntityType::FALLING_BLOCK);
        entity
            .data
            .store(i32::from(block_state.as_u16()), Ordering::Relaxed);
        let entity = Arc::new(Self::new(entity, block_state));
        // Vanilla `FallingBlock.falling`: an anvil hurts what it lands on.
        if Block::from_state_id(block_state).has_tag(&tag::Block::MINECRAFT_ANVIL) {
            entity.set_hurts_entities(2.0, 40);
        }
        world.spawn_entity_non_save(entity);
    }
}

impl EntityBase for FallingEntity {
    /// Vanilla `isPickable`: not removed.
    fn can_hit(&self) -> bool {
        !self.get_entity().is_removed()
    }

    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        let entity = &self.entity;
        let mut velo = entity.velocity.load();
        velo.y -= self.current_gravity();

        entity.velocity.store(velo);

        entity.move_entity(caller, velo);
        entity.tick_block_collisions(caller);
        if entity.on_ground.load(Ordering::Relaxed) {
            entity.velocity.store(velo.multiply(0.7, -0.5, 0.7));
            let world = entity.world.load();
            let landing_pos = self.entity.block_pos.load();
            let (current_block, current_state) = world.get_block_and_state(&landing_pos);
            // Vanilla waits until a piston has finished moving a block through the landing cell.
            if current_block != &Block::MOVING_PISTON && self.cancel_drop.load(Ordering::Relaxed) {
                // `callOnBrokenAfterFall`.
                let block = Block::from_state_id(self.block_state_id.load());
                self.entity.remove();
                if block.has_tag(&tag::Block::MINECRAFT_ANVIL) && !self.entity.is_silent() {
                    world.sync_world_event(WorldEvent::SoundAnvilBroken, landing_pos, 0);
                }
            } else if current_block != &Block::MOVING_PISTON {
                let mut state_id = self.block_state_id.load();
                let block = Block::from_state_id(state_id);
                let (below_block, below_state) = world.get_block_and_state(&landing_pos.down());
                let may_replace = can_replace_with_other_block(current_block, current_state);
                let would_continue_falling =
                    FallingBlock::can_fall_through(below_state, below_block);
                let would_survive = world.block_registry.can_place_at(
                    None,
                    Some(&**world),
                    &**world,
                    None,
                    block,
                    state_id.to_state(),
                    &landing_pos,
                    None,
                    None,
                ) && !would_continue_falling;
                if may_replace && would_survive {
                    if block.has_tag(&tag::Block::MINECRAFT_CONCRETE_POWDERS)
                        && FallingBlock::should_solidify(&**world, &landing_pos)
                        && let Some(name) = block.name.strip_suffix("_powder")
                        && let Some(concrete) = Block::from_name(name)
                    {
                        state_id = concrete.default_state.id;
                    }
                    world.set_block_state(&landing_pos, state_id, BlockFlags::NOTIFY_ALL);
                    // Vanilla `AnvilBlock.onLand`.
                    if block.has_tag(&tag::Block::MINECRAFT_ANVIL) && !entity.is_silent() {
                        world.sync_world_event(WorldEvent::SoundAnvilLand, landing_pos, 0);
                    }
                    // block updates to watchers before the despawn, else a invisible block gap until the tick flush.
                    let placed = world.get_block_state_id(&landing_pos);
                    world.send_to_tracking_players_editioned(
                        entity,
                        &CBlockUpdate::new(landing_pos, i32::from(placed.as_u16()).into()),
                        &CUpdateBlock::new(landing_pos, BlockState::to_be_network_id(placed)),
                    );
                } else if world.level_info.load().game_rules.entity_drops
                    && let Some(item) = Item::from_id(block.item_id)
                {
                    entity.spawn_at_location(ItemStack::new(1, item));
                }
                self.entity.remove();
            }
        }

        entity.velocity.store(velo.multiply(0.98, 0.98, 0.98));
    }

    fn init_data_tracker(&self) {
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::falling_block::START_POS,
            self.entity.block_pos.load(),
        );
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    /// Vanilla `FallingBlockEntity.causeFallDamage`: a block that hurts entities hurts what
    /// it lands in, and an anvil may get damaged doing so.
    fn cause_fall_damage(
        &self,
        caller: &dyn EntityBase,
        fall_distance: f64,
        _damage_modifier: f32,
        _damage_type: DamageType,
    ) -> bool {
        if !self.hurt_entities.load(Ordering::Relaxed) {
            return false;
        }
        let fall_distance_int = (fall_distance - 1.0).ceil() as i32;
        if fall_distance_int < 0 {
            return false;
        }
        let state_id = self.block_state_id.load();
        let block = Block::from_state_id(state_id);
        let is_anvil = block.has_tag(&tag::Block::MINECRAFT_ANVIL);
        // `Fallable.getFallDamageSource`.
        let damage_type = if is_anvil {
            DamageType::FALLING_ANVIL
        } else {
            DamageType::FALLING_BLOCK
        };
        let damage = ((fall_distance_int as f32 * self.fall_damage_per_distance.load()).floor()
            as i32)
            .min(self.fall_damage_max.load(Ordering::Relaxed)) as f32;
        let world = self.entity.world.load();
        let bounding_box = self.entity.bounding_box.load();
        let mut targets = world.get_entities_at_box(&bounding_box);
        targets.extend(
            world
                .get_players_at_box(&bounding_box)
                .into_iter()
                .map(|player| player as Arc<dyn EntityBase>),
        );
        for target in targets {
            // `NO_CREATIVE_OR_SPECTATOR` and `LIVING_ENTITY_STILL_ALIVE`.
            let target_entity = target.get_entity();
            if target_entity.entity_id == self.entity.entity_id
                || target.is_spectator()
                || target.get_player().is_some_and(|player| {
                    player.gamemode.load() == pumpkin_util::GameMode::Creative
                })
                || target.get_living_entity().is_none_or(|living| {
                    !target_entity.is_alive() || living.dead.load(Ordering::Relaxed)
                })
            {
                continue;
            }
            target.damage_with_context(
                target.as_ref(),
                damage,
                damage_type,
                None,
                Some(caller),
                None,
            );
        }
        if is_anvil
            && damage > 0.0
            && rand::random::<f32>() < 0.05 + fall_distance_int as f32 * 0.05
        {
            match AnvilBlock::damage(state_id) {
                Some(damaged) => self.block_state_id.store(damaged),
                None => self.cancel_drop.store(true, Ordering::Relaxed),
            }
        }
        false
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    fn damage(&self, _caller: &dyn EntityBase, _amount: f32, _damage_type: DamageType) -> bool {
        false
    }

    fn get_gravity(&self) -> f64 {
        0.04
    }

    // TODO: Bedrock spawn metadata lacks the block variant (renders grey while falling)
    fn bedrock_y_offset(&self) -> f64 {
        0.49
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn movement_emission(&self) -> crate::entity::movement_emission::MovementEmission {
        crate::entity::movement_emission::MovementEmission::None
    }
}
