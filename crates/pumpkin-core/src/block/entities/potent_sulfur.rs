use std::sync::{
    Arc,
    atomic::{AtomicI32, AtomicI64, Ordering},
};

use pumpkin_data::{
    Block, BlockState,
    block_properties::{PotentSulfurLikeProperties, PotentSulfurState},
    effect::StatusEffect,
    game_event::GameEvent,
    potion::Effect,
    tag::{self, Taggable},
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{
    math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3},
    random::{RandomImpl, xoroshiro128::Xoroshiro},
};
use pumpkin_world::{
    clip::{ClipBlock, ClipContext, ClipFluid},
    world::BlockFlags,
};

use super::BlockEntity;
use crate::{
    block::blocks::potent_sulfur::PotentSulfurBlock,
    entity::{EntityBase, inside_block_effect::is_alive},
    world::{World, game_event::GameEventContext},
};

/// Vanilla `PotentSulfurBlock.ALLOWED_WATER_BLOCKS_ABOVE`.
const ALLOWED_WATER_BLOCKS_ABOVE: i32 = 4;
/// Vanilla `EFFECT_APPLICATION_FREQUENCY_TICKS`.
const EFFECT_APPLICATION_FREQUENCY_TICKS: i64 = 10;
/// Vanilla `EFFECT_DURATION_IN_TICKS`.
const EFFECT_DURATION_IN_TICKS: i32 = 80;
/// Vanilla `GEYSER_BASE_LAUNCH_SPEED`.
const GEYSER_BASE_LAUNCH_SPEED: f32 = 0.3;
/// Vanilla `GEYSER_LAUNCH_FORCE`.
const GEYSER_LAUNCH_FORCE: f32 = 0.2;
/// Vanilla `GEYSER_SALT`.
const GEYSER_SALT: i64 = -904_011_478;

pub struct PotentSulfurBlockEntity {
    pub position: BlockPos,
    /// Vanilla `waitingCountdown`.
    pub waiting_countdown: AtomicI32,
    /// Vanilla `eruptionTick`: only the client's plume ticker reads it.
    eruption_tick: AtomicI64,
}

impl BlockEntity for PotentSulfurBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let entity = Self::new(position);
        if let Some(countdown) = nbt.get_int("countdown") {
            entity.waiting_countdown.store(countdown, Ordering::Relaxed);
        }
        entity
    }

    /// Vanilla `saveAdditional`: the countdown is always written.
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int("countdown", self.waiting_countdown.load(Ordering::Relaxed));
    }

    /// The server tickers vanilla `PotentSulfurBlock.getTicker` picks for the state.
    fn tick(&self, world: &Arc<World>) {
        let (block, state_id) = world.get_block_and_state_id(&self.position);
        if block != &Block::POTENT_SULFUR {
            return;
        }
        let state = BlockState::from_id(state_id);
        match PotentSulfurLikeProperties::from_state_id(state_id).potent_sulfur_state {
            PotentSulfurState::Dry => {}
            PotentSulfurState::Wet => Self::nausea_effect_tick(world, self.position),
            PotentSulfurState::Dormant => {
                self.waiting_countdown_tick(world, state);
                Self::nausea_effect_tick(world, self.position);
            }
            PotentSulfurState::Erupting => {
                Self::launch_entity_tick(world, self.position);
                self.waiting_countdown_tick(world, state);
            }
            PotentSulfurState::Continuous => Self::launch_entity_tick(world, self.position),
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl PotentSulfurBlockEntity {
    pub const ID: &'static str = "minecraft:potent_sulfur";

    #[must_use]
    pub const fn new(position: BlockPos) -> Self {
        Self {
            position,
            waiting_countdown: AtomicI32::new(-1),
            eruption_tick: AtomicI64::new(-1),
        }
    }

    /// Vanilla `resetCountdown`.
    pub fn reset_countdown(&self) {
        self.waiting_countdown.store(-1, Ordering::Relaxed);
    }

    pub fn set_eruption_tick(&self, tick: i64) {
        self.eruption_tick.store(tick, Ordering::Relaxed);
    }

    /// Vanilla `SERVER_NAUSEA_EFFECT_TICKER`.
    fn nausea_effect_tick(world: &Arc<World>, pos: BlockPos) {
        if world.get_world_age() % EFFECT_APPLICATION_FREQUENCY_TICKS != 0 {
            return;
        }
        let Some(source) = find_noxious_gas_source_block(world, pos) else {
            return;
        };
        // `getNearbyLivingEntities`: `new AABB(pos).inflate(2.5, 0.0, 2.5)`.
        let min = source.0.to_f64();
        let area = BoundingBox::new(
            Vector3::new(min.x - 2.5, min.y, min.z - 2.5),
            Vector3::new(min.x + 3.5, min.y + 1.0, min.z + 3.5),
        );
        for entity in entities_in(world, &area) {
            let Some(living) = entity.get_living_entity() else {
                continue;
            };
            if can_be_reached_by_noxious_gas(world, source, entity.get_entity().get_eye_pos()) {
                // `new MobEffectInstance(NAUSEA, 80, 0, true, true)`.
                living.add_effect(Effect {
                    effect_type: &StatusEffect::NAUSEA,
                    duration: EFFECT_DURATION_IN_TICKS,
                    amplifier: 0,
                    ambient: true,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                });
            }
        }
    }

    /// Vanilla `SERVER_WAITING_COUNTDOWN_TICKER`.
    fn waiting_countdown_tick(&self, world: &Arc<World>, state: &'static BlockState) {
        if world.get_world_age() % 20 != 0 {
            return;
        }
        let Some(source) = find_noxious_gas_source_block(world, self.position) else {
            return;
        };
        let pos = self.position;
        let sulfur_state = PotentSulfurLikeProperties::from_state_id(state.id).potent_sulfur_state;
        let mut countdown = self.waiting_countdown.load(Ordering::Relaxed);
        if countdown <= 0 {
            let water_blocks = source.0.y - pos.0.y - 1;
            let mut positional = geyser_positional(world, pos);
            countdown = if sulfur_state == PotentSulfurState::Dormant {
                10 * (water_blocks - 1) + positional.next_inbetween_i32(15, 30)
            } else {
                positional.next_i32();
                water_blocks - 1 + positional.next_inbetween_i32(1, 2)
            };
        }
        if countdown > 0 {
            countdown -= 1;
        }
        self.waiting_countdown.store(countdown, Ordering::Relaxed);
        if countdown == 0 {
            let to_set = if sulfur_state == PotentSulfurState::Dormant {
                PotentSulfurState::Erupting
            } else {
                PotentSulfurState::Dormant
            };
            let mut props = PotentSulfurLikeProperties::from_state_id(state.id);
            props.potent_sulfur_state = to_set;
            // `setBlockAndUpdate`: `onPlace` starts an eruption.
            world.set_block_state(
                &pos,
                props.to_state_id(&Block::POTENT_SULFUR),
                BlockFlags::NOTIFY_ALL,
            );
            if to_set == PotentSulfurState::Dormant {
                world.game_event(
                    GameEvent::BlockDeactivate,
                    pos.to_centered_f64(),
                    GameEventContext::of_state(state),
                );
            }
        }
    }

    /// Vanilla `LAUNCH_ENTITY_TICKER`.
    fn launch_entity_tick(world: &Arc<World>, pos: BlockPos) {
        let Some(source) = find_noxious_gas_source_block(world, pos) else {
            return;
        };
        let water_blocks = source.0.y - pos.0.y - 1;
        let force_height = get_unobstructed_block_count(world, pos.up(), water_blocks);
        // `new AABB(pos.above()).expandTowards(0.0, geyserForceHeight - 1, 0.0)`.
        let min = pos.up().0.to_f64();
        let mut area = BoundingBox::new(min, Vector3::new(min.x + 1.0, min.y + 1.0, min.z + 1.0));
        let expand = f64::from(force_height - 1);
        if expand < 0.0 {
            area.min.y += expand;
        } else {
            area.max.y += expand;
        }
        let max_speed = f64::from(GEYSER_BASE_LAUNCH_SPEED) + f64::from(water_blocks) * 0.1;
        for entity in entities_in(world, &area) {
            let base = entity.get_entity();
            let velocity = base.velocity.load();
            base.check_fall_distance_accumulation();
            let flying = entity.get_player().is_some_and(|player| {
                player
                    .abilities
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .flying
            });
            if entity.can_simulate_movement()
                && !flying
                && !base.has_vehicle()
                && !base
                    .entity_type
                    .has_tag(&tag::EntityType::MINECRAFT_NOT_AFFECTED_BY_GEYSERS)
                && velocity.y < max_speed
            {
                // `addDeltaMovement`.
                base.velocity.store(
                    base.velocity.load() + Vector3::new(0.0, f64::from(GEYSER_LAUNCH_FORCE), 0.0),
                );
            }
        }
    }
}

/// Vanilla `getEntitiesOfClass(Entity.class, aabb, EFFECT_PREDICATE)`: no spectators, only
/// entities still alive.
fn entities_in(world: &World, area: &BoundingBox) -> Vec<Arc<dyn EntityBase>> {
    let mut found = world.get_entities_at_box(area);
    found.extend(
        world
            .get_players_at_box(area)
            .into_iter()
            .map(|player| player as Arc<dyn EntityBase>),
    );
    found.retain(|entity| !entity.is_spectator() && is_alive(entity.as_ref()));
    found
}

/// Vanilla `geyserPositional`: `new XoroshiroRandomSource(seed ^ GEYSER_SALT)`, forked
/// positionally at `pos`.
fn geyser_positional(world: &World, pos: BlockPos) -> impl RandomImpl {
    let seed = world.level.seed.0 ^ GEYSER_SALT as u64;
    Xoroshiro::from_seed(seed)
        .next_splitter()
        .split_pos(pos.0.x, pos.0.y, pos.0.z)
}

/// Vanilla `isGeyserPassableBlock`: air and water pass, anything else when its collision shape
/// is empty for `CollisionContext.positionContext` below the block. Every caller's context sits
/// at least a block below the block it tests, so scaffolding (not above it) and powder snow (no
/// entity) are empty too.
fn is_geyser_passable_block(world: &World, pos: &BlockPos) -> bool {
    let (block, state) = world.get_block_and_state(pos);
    if state.is_air() || block == &Block::WATER {
        return true;
    }
    if block == &Block::SCAFFOLDING || block == &Block::POWDER_SNOW {
        return true;
    }
    state.get_block_collision_shapes_at(pos).next().is_none()
}

/// Vanilla `getUnobstructedBlockCount`.
fn get_unobstructed_block_count(world: &World, pos: BlockPos, water_blocks: i32) -> i32 {
    let force_height = 6 * water_blocks;
    for i in 0..force_height {
        if !is_geyser_passable_block(world, &pos.up_height(i)) {
            return i;
        }
    }
    force_height
}

/// Vanilla `findNoxiousGasSourceBlock`: the first block above the water column over `origin`.
fn find_noxious_gas_source_block(world: &World, origin: BlockPos) -> Option<BlockPos> {
    let max_y = origin.0.y + ALLOWED_WATER_BLOCKS_ABOVE + 1;
    let mut pos = origin.up();
    while pos.0.y <= max_y {
        let (block, state) = world.get_block_and_state(&pos);
        let water_logged = PotentSulfurBlock::is_water_source(world, &pos);
        if !water_logged || (block != &Block::WATER && !is_geyser_passable_block(world, &pos)) {
            if state.is_air() || is_geyser_passable_block(world, &pos) {
                return Some(pos);
            }
            break;
        }
        pos = pos.up();
    }
    None
}

/// Vanilla `canBeReachedByNoxiousGas`.
fn can_be_reached_by_noxious_gas(world: &World, source: BlockPos, eye: Vector3<f64>) -> bool {
    let block_pos = BlockPos::floored_v(eye);
    if !is_geyser_passable_block(world, &block_pos) {
        return false;
    }
    if eye.squared_distance_to_vec(&source.to_centered_f64()) > 9.0 {
        return false;
    }
    let below_source = source.down().to_centered_f64();
    let below_eye = Vector3::new(eye.x, eye.y - 1.0, eye.z);
    // `isWater` and `haveLineOfSight`.
    PotentSulfurBlock::is_water_source(world, &BlockPos::floored_v(below_eye))
        && !world
            .clip(&ClipContext::new(
                below_source,
                below_eye,
                ClipBlock::Collider,
                ClipFluid::None,
            ))
            .is_hit()
}
