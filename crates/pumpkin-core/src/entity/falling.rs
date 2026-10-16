use crossbeam::atomic::AtomicCell;
use pumpkin_data::Block;
use pumpkin_data::BlockState;
use pumpkin_data::BlockStateId;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::bedrock::client::CUpdateBlock;
use pumpkin_protocol::java::client::play::CBlockUpdate;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use crate::{
    block::{blocks::falling::FallingBlock, registry::can_replace_with_other_block},
    entity::{Entity, EntityBase, living::LivingEntity},
    server::Server,
    world::World,
};

pub struct FallingEntity {
    entity: Entity,
    block_state_id: AtomicCell<BlockStateId>,
    /// Vanilla `FallingBlockEntity.time`: ticks spent falling.
    time: AtomicI32,
    drop_item: AtomicBool,
    cancel_drop: AtomicBool,
    hurt_entities: AtomicBool,
    fall_damage_per_distance: AtomicCell<f32>,
    fall_damage_max: AtomicI32,
    /// Vanilla `FallingBlockEntity.blockData`: merged into the block entity it lands as.
    block_data: Mutex<Option<NbtCompound>>,
}

impl FallingEntity {
    pub const fn new(entity: Entity, block_state_id: BlockStateId) -> Self {
        Self {
            entity,
            block_state_id: AtomicCell::new(block_state_id),
            time: AtomicI32::new(0),
            drop_item: AtomicBool::new(true),
            cancel_drop: AtomicBool::new(false),
            hurt_entities: AtomicBool::new(false),
            fall_damage_per_distance: AtomicCell::new(0.0),
            fall_damage_max: AtomicI32::new(40),
            block_data: Mutex::new(None),
        }
    }

    /// Vanilla `FallingBlockEntity.spawnAtLocation(level, block)` when `dropItem` and the
    /// `entity_drops` rule allow it.
    fn drop_block_item(&self, world: &World, block: &Block) {
        if self.drop_item.load(Ordering::Relaxed)
            && world.level_info.load().game_rules.entity_drops
            && let Some(item) = Item::from_id(block.item_id)
        {
            self.entity.spawn_at_location(ItemStack::new(1, item));
        }
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
        world.spawn_entity_non_save(entity);
    }
}

impl EntityBase for FallingEntity {
    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        let entity = &self.entity;
        if BlockState::from_id(self.block_state_id.load()).is_air() {
            entity.remove();
            return;
        }
        let time = self.time.fetch_add(1, Ordering::Relaxed) + 1;
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
                self.entity.remove();
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
                    if world.get_fluid(&landing_pos) == &pumpkin_data::fluid::Fluid::WATER
                        && let Some(waterlogged) =
                            Block::from_state_id(state_id).set_waterlogged(state_id, true)
                    {
                        state_id = waterlogged;
                    }
                    world.set_block_state(&landing_pos, state_id, BlockFlags::NOTIFY_ALL);
                    // block updates to watchers before the despawn, else a invisible block gap until the tick flush.
                    let placed = world.get_block_state_id(&landing_pos);
                    world.send_to_tracking_players_editioned(
                        entity,
                        &CBlockUpdate::new(landing_pos, i32::from(placed.as_u16()).into()),
                        &CUpdateBlock::new(landing_pos, BlockState::to_be_network_id(placed)),
                    );
                    let block_data = self
                        .block_data
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone();
                    if let Some(block_data) = block_data {
                        crate::command::commands::setblock::load_block_entity_nbt(
                            &world,
                            &landing_pos,
                            &block_data,
                        );
                    }
                } else {
                    self.drop_block_item(&world, block);
                }
                self.entity.remove();
            }
        } else {
            // Vanilla discards a block that falls too long or leaves the world.
            let world = entity.world.load();
            let y = entity.block_pos.load().0.y;
            let max_y = world.min_y + world.dimension.height - 1;
            if (time > 100 && (y <= world.min_y || y > max_y)) || time > 600 {
                self.drop_block_item(&world, Block::from_state_id(self.block_state_id.load()));
                entity.remove();
            }
        }
        entity.velocity.store(velo.multiply(0.98, 0.98, 0.98));
    }

    /// Vanilla `FallingBlockEntity.addAdditionalSaveData`.
    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put(
            "BlockState",
            Block::state_to_nbt(self.block_state_id.load()),
        );
        nbt.put_int("Time", self.time.load(Ordering::Relaxed));
        nbt.put_bool("DropItem", self.drop_item.load(Ordering::Relaxed));
        nbt.put_bool("HurtEntities", self.hurt_entities.load(Ordering::Relaxed));
        nbt.put_float("FallHurtAmount", self.fall_damage_per_distance.load());
        nbt.put_int("FallHurtMax", self.fall_damage_max.load(Ordering::Relaxed));
        if let Some(block_data) = self
            .block_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            nbt.put_compound("TileEntityData", block_data.clone());
        }
        nbt.put_bool("CancelDrop", self.cancel_drop.load(Ordering::Relaxed));
    }

    /// Vanilla `FallingBlockEntity.readAdditionalSaveData`.
    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        let state = nbt
            .get("BlockState")
            .and_then(Block::state_from_nbt)
            .unwrap_or(Block::SAND.default_state.id);
        self.block_state_id.store(state);
        self.entity
            .data
            .store(i32::from(state.as_u16()), Ordering::Relaxed);
        self.time
            .store(nbt.get_int("Time").unwrap_or(0), Ordering::Relaxed);
        let anvil = Block::from_state_id(state).has_tag(&tag::Block::MINECRAFT_ANVIL);
        self.hurt_entities.store(
            nbt.get_bool("HurtEntities").unwrap_or(anvil),
            Ordering::Relaxed,
        );
        self.fall_damage_per_distance
            .store(nbt.get_float("FallHurtAmount").unwrap_or(0.0));
        self.fall_damage_max
            .store(nbt.get_int("FallHurtMax").unwrap_or(40), Ordering::Relaxed);
        self.drop_item
            .store(nbt.get_bool("DropItem").unwrap_or(true), Ordering::Relaxed);
        *self
            .block_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            nbt.get_compound("TileEntityData").cloned();
        self.cancel_drop.store(
            nbt.get_bool("CancelDrop").unwrap_or(false),
            Ordering::Relaxed,
        );
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
