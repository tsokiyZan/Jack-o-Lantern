//! Vanilla `ServerExplosion`, `Explosion` and the explosion damage calculators.

use std::sync::{Arc, Mutex, PoisonError};

use pumpkin_data::{
    Block, BlockState, BlockStateId,
    damage::DamageType,
    entity::EntityType,
    fluid::Fluid,
    game_event::GameEvent,
    item_stack::ItemStack,
    tag::{self, Tag, Taggable},
};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use rand::seq::SliceRandom;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    block::{ExplodeArgs, SpawnAfterBreakArgs, block_loot_stacks, drop_block_experience},
    entity::{Entity, EntityBase, boss::ender_dragon::parts_at_box, tnt::TNTEntity},
    world::loot::LootContextParameters,
};

use super::{BlockFlags, World};

/// Defines the type of explosion interaction with the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplosionInteraction {
    None,
    Block,
    Mob,
    Tnt,
    Trigger,
}

impl ExplosionInteraction {
    /// Parses the vanilla `Level.ExplosionInteraction` serialized name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "block" => Some(Self::Block),
            "mob" => Some(Self::Mob),
            "tnt" => Some(Self::Tnt),
            "trigger" => Some(Self::Trigger),
            _ => None,
        }
    }
}

/// Defines how an explosion interacts with blocks in the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockInteraction {
    /// Keeps blocks intact (no block damage, no drops).
    Keep,
    /// Destroys blocks and drops 100% of items without decay.
    Destroy,
    /// Destroys blocks and applies loot decay based on explosion radius.
    DestroyWithDecay,
    /// Triggers block effects without destroying them.
    TriggerBlock,
}

/// Vanilla `DamageSource` of an explosion: its type, the entity that exploded (direct) and
/// the one responsible (causing).
#[derive(Clone)]
pub struct ExplosionDamageSource {
    pub damage_type: DamageType,
    pub direct: Option<Arc<dyn EntityBase>>,
    pub causing: Option<Arc<dyn EntityBase>>,
    pub position: Option<Vector3<f64>>,
}

impl ExplosionDamageSource {
    /// Vanilla `DamageSources.explosion(entity, cause)`: `player_explosion` whenever both are
    /// known, whoever the cause is.
    #[must_use]
    pub fn explosion(
        direct: Option<Arc<dyn EntityBase>>,
        causing: Option<Arc<dyn EntityBase>>,
    ) -> Self {
        let damage_type = if direct.is_some() && causing.is_some() {
            DamageType::PLAYER_EXPLOSION
        } else {
            DamageType::EXPLOSION
        };
        Self {
            damage_type,
            direct,
            causing,
            position: None,
        }
    }

    /// Vanilla `Explosion.getDefaultDamageSource`.
    #[must_use]
    pub fn default_for(source: Option<&Arc<dyn EntityBase>>) -> Self {
        Self::explosion(source.cloned(), source.and_then(indirect_source_entity))
    }

    /// Vanilla `DamageSources.badRespawnPointExplosion`: beds and respawn anchors.
    #[must_use]
    pub const fn bad_respawn_point(position: Vector3<f64>) -> Self {
        Self {
            damage_type: DamageType::BAD_RESPAWN_POINT,
            direct: None,
            causing: None,
            position: Some(position),
        }
    }
}

/// Vanilla `Explosion.getIndirectSourceEntity`: primed TNT credits its owner, a living
/// entity itself, a projectile its living owner.
#[must_use]
pub fn indirect_source_entity(source: &Arc<dyn EntityBase>) -> Option<Arc<dyn EntityBase>> {
    if let Some(tnt) = source.cast_any().downcast_ref::<TNTEntity>() {
        return tnt.owner();
    }
    if source.get_living_entity().is_some() {
        return Some(source.clone());
    }
    source
        .get_owner_id()
        .and_then(|id| source.get_entity().world.load().get_entity_by_id(id))
        .filter(|owner| owner.get_living_entity().is_some())
}

/// One entry of the client's explosion block particles: vanilla `ExplosionParticleInfo` with
/// its weight.
#[derive(Clone, Copy)]
pub struct ExplosionBlockParticle {
    pub particle: pumpkin_data::particle::Particle,
    pub scaling: f32,
    pub speed: f32,
    pub weight: i32,
}

/// The particles and sound a vanilla `ServerLevel.explode` call sends with the explosion.
#[derive(Clone, Copy)]
pub struct ExplosionEffects {
    pub small_particle: pumpkin_data::particle::Particle,
    pub large_particle: pumpkin_data::particle::Particle,
    pub block_particles: &'static [ExplosionBlockParticle],
    pub sound: pumpkin_data::sound::Sound,
}

/// Vanilla `Level.DEFAULT_EXPLOSION_BLOCK_PARTICLES`.
const DEFAULT_EXPLOSION_BLOCK_PARTICLES: [ExplosionBlockParticle; 2] = [
    ExplosionBlockParticle {
        particle: pumpkin_data::particle::Particle::Poof,
        scaling: 0.5,
        speed: 1.0,
        weight: 1,
    },
    ExplosionBlockParticle {
        particle: pumpkin_data::particle::Particle::Smoke,
        scaling: 1.0,
        speed: 1.0,
        weight: 1,
    },
];

impl ExplosionEffects {
    /// The effects of vanilla's short `Level.explode` overloads.
    pub const DEFAULT: Self = Self {
        small_particle: pumpkin_data::particle::Particle::Explosion,
        large_particle: pumpkin_data::particle::Particle::ExplosionEmitter,
        block_particles: &DEFAULT_EXPLOSION_BLOCK_PARTICLES,
        sound: pumpkin_data::sound::Sound::EntityGenericExplode,
    };
}

impl ExplosionEffects {
    /// Vanilla `WindCharge.explode`.
    pub const WIND_CHARGE: Self = Self {
        small_particle: pumpkin_data::particle::Particle::GustEmitterSmall,
        large_particle: pumpkin_data::particle::Particle::GustEmitterLarge,
        block_particles: &[],
        sound: pumpkin_data::sound::Sound::EntityWindChargeWindBurst,
    };

    /// Vanilla `BreezeWindCharge.explode` and `WindChargedMobEffect.onMobRemoved`.
    pub const BREEZE_WIND_CHARGE: Self = Self {
        small_particle: pumpkin_data::particle::Particle::GustEmitterSmall,
        large_particle: pumpkin_data::particle::Particle::GustEmitterLarge,
        block_particles: &[],
        sound: pumpkin_data::sound::Sound::EntityBreezeWindBurst,
    };
}

/// Defines how damage and block destruction are calculated for an explosion.
pub trait ExplosionDamageCalculator: Send + Sync {
    /// Vanilla `getBlockExplosionResistance`: `None` means the ray passes through.
    fn get_block_explosion_resistance(
        &self,
        _explosion: &Explosion,
        _world: &World,
        _pos: &BlockPos,
        block: &Block,
        state: &BlockState,
        fluid: &pumpkin_data::fluid::FluidState,
    ) -> Option<f32> {
        if state.is_air() && fluid.is_empty {
            None
        } else {
            Some(block.blast_resistance.max(fluid.blast_resistance))
        }
    }

    /// Vanilla `shouldBlockExplode`.
    fn should_block_explode(
        &self,
        _explosion: &Explosion,
        _world: &World,
        _pos: &BlockPos,
        _block: &Block,
        _power: f32,
    ) -> bool {
        true
    }

    /// Vanilla `shouldDamageEntity`.
    fn should_damage_entity(&self, _explosion: &Explosion, _entity: &dyn EntityBase) -> bool {
        true
    }

    /// Vanilla `getKnockbackMultiplier`.
    fn get_knockback_multiplier(&self, _entity: &dyn EntityBase) -> f32 {
        1.0
    }

    /// Vanilla `getEntityDamageAmount`.
    #[expect(clippy::manual_midpoint, reason = "vanilla's exact expression")]
    fn get_entity_damage_amount(
        &self,
        explosion: &Explosion,
        entity: &dyn EntityBase,
        exposure: f32,
    ) -> f32 {
        let double_radius = explosion.radius * 2.0;
        let dist = entity
            .get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&explosion.center)
            .sqrt()
            / f64::from(double_radius);
        let pow = (1.0 - dist) * f64::from(exposure);
        ((pow * pow + pow) / 2.0 * 7.0 * f64::from(double_radius) + 1.0) as f32
    }
}

/// Vanilla `ServerExplosion.EXPLOSION_DAMAGE_CALCULATOR`.
pub struct DefaultExplosionDamageCalculator;

impl ExplosionDamageCalculator for DefaultExplosionDamageCalculator {}

/// Vanilla `EntityBasedExplosionDamageCalculator`: the exploding entity adjusts block
/// resistance and which blocks break.
pub struct EntityBasedExplosionDamageCalculator {
    source: Arc<dyn EntityBase>,
}

impl ExplosionDamageCalculator for EntityBasedExplosionDamageCalculator {
    fn get_block_explosion_resistance(
        &self,
        explosion: &Explosion,
        world: &World,
        pos: &BlockPos,
        block: &Block,
        state: &BlockState,
        fluid: &pumpkin_data::fluid::FluidState,
    ) -> Option<f32> {
        DefaultExplosionDamageCalculator
            .get_block_explosion_resistance(explosion, world, pos, block, state, fluid)
            .map(|resistance| {
                self.source
                    .get_block_explosion_resistance(world, pos, block, resistance)
            })
    }

    fn should_block_explode(
        &self,
        _explosion: &Explosion,
        world: &World,
        pos: &BlockPos,
        block: &Block,
        power: f32,
    ) -> bool {
        self.source.should_block_explode(world, pos, block, power)
    }
}

/// Vanilla `SimpleExplosionDamageCalculator` (wind charges and the wind burst enchantment).
pub struct SimpleExplosionDamageCalculator {
    pub explodes_blocks: bool,
    pub damages_entities: bool,
    pub knockback_multiplier: Option<f32>,
    pub immune_blocks: Option<&'static Tag>,
}

impl SimpleExplosionDamageCalculator {
    #[must_use]
    pub const fn new(
        explodes_blocks: bool,
        damages_entities: bool,
        knockback_multiplier: Option<f32>,
        immune_blocks: Option<&'static Tag>,
    ) -> Self {
        Self {
            explodes_blocks,
            damages_entities,
            knockback_multiplier,
            immune_blocks,
        }
    }
}

impl ExplosionDamageCalculator for SimpleExplosionDamageCalculator {
    fn get_block_explosion_resistance(
        &self,
        explosion: &Explosion,
        world: &World,
        pos: &BlockPos,
        block: &Block,
        state: &BlockState,
        fluid: &pumpkin_data::fluid::FluidState,
    ) -> Option<f32> {
        // With an immune set, immune blocks stop the ray and every other block lets it through.
        if let Some(immune) = self.immune_blocks {
            return block.has_tag(immune).then_some(3_600_000.0);
        }
        DefaultExplosionDamageCalculator
            .get_block_explosion_resistance(explosion, world, pos, block, state, fluid)
    }

    fn should_block_explode(
        &self,
        _explosion: &Explosion,
        _world: &World,
        _pos: &BlockPos,
        _block: &Block,
        _power: f32,
    ) -> bool {
        self.explodes_blocks
    }

    fn should_damage_entity(&self, _explosion: &Explosion, _entity: &dyn EntityBase) -> bool {
        self.damages_entities
    }

    fn get_knockback_multiplier(&self, entity: &dyn EntityBase) -> f32 {
        if entity
            .get_player()
            .is_some_and(crate::entity::player::Player::is_flying)
        {
            return 0.0;
        }
        self.knockback_multiplier.unwrap_or(1.0)
    }
}

/// Vanilla `ServerExplosion`.
pub struct Explosion {
    pub(crate) radius: f32,
    pub(crate) center: Vector3<f64>,
    block_interaction: BlockInteraction,
    fire: bool,
    source: Option<Arc<dyn EntityBase>>,
    indirect_source: Option<Arc<dyn EntityBase>>,
    damage_source: ExplosionDamageSource,
    damage_calculator: Arc<dyn ExplosionDamageCalculator>,
    /// Vanilla `hitPlayers`: the knockback each player gets in its explosion packet.
    hit_players: Mutex<FxHashMap<i32, Vector3<f64>>>,
}

impl Explosion {
    #[must_use]
    pub fn new(
        source: Option<Arc<dyn EntityBase>>,
        damage_source: Option<ExplosionDamageSource>,
        damage_calculator: Option<Arc<dyn ExplosionDamageCalculator>>,
        center: Vector3<f64>,
        radius: f32,
        fire: bool,
        block_interaction: BlockInteraction,
    ) -> Self {
        let damage_source =
            damage_source.unwrap_or_else(|| ExplosionDamageSource::default_for(source.as_ref()));
        // Vanilla `makeDamageCalculator`.
        let damage_calculator = damage_calculator.unwrap_or_else(|| match &source {
            Some(source) => Arc::new(EntityBasedExplosionDamageCalculator {
                source: source.clone(),
            }),
            None => Arc::new(DefaultExplosionDamageCalculator),
        });
        let indirect_source = source.as_ref().and_then(indirect_source_entity);
        Self {
            radius,
            center,
            block_interaction,
            fire,
            source,
            indirect_source,
            damage_source,
            damage_calculator,
            hit_players: Mutex::new(FxHashMap::default()),
        }
    }

    /// Vanilla `Explosion.getIndirectSourceEntity`.
    #[must_use]
    pub fn indirect_source_entity(&self) -> Option<Arc<dyn EntityBase>> {
        self.indirect_source.clone()
    }

    /// Vanilla `ServerExplosion.getDamageSource`.
    #[must_use]
    pub const fn damage_source(&self) -> &ExplosionDamageSource {
        &self.damage_source
    }

    /// Vanilla `isSmall`: picks the small particle.
    #[must_use]
    pub fn is_small(&self) -> bool {
        self.radius < 2.0 || !self.interacts_with_blocks()
    }

    fn interacts_with_blocks(&self) -> bool {
        self.block_interaction != BlockInteraction::Keep
    }

    /// The knockback a player gets in its explosion packet.
    #[must_use]
    pub fn player_knockback(&self, entity_id: i32) -> Option<Vector3<f64>> {
        self.hit_players
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&entity_id)
            .copied()
    }

    /// Vanilla `calculateExplodedPositions`, in its float maths.
    fn calculate_exploded_positions(&self, world: &World) -> Vec<BlockPos> {
        let mut to_blow = FxHashSet::default();
        let mut chunk_cache: FxHashMap<
            pumpkin_util::math::vector2::Vector2<i32>,
            Option<Arc<ChunkData>>,
        > = FxHashMap::default();

        for xx in 0..16 {
            for yy in 0..16 {
                for zz in 0..16 {
                    if !(xx == 0 || xx == 15 || yy == 0 || yy == 15 || zz == 0 || zz == 15) {
                        continue;
                    }
                    let mut xd = f64::from(xx as f32 / 15.0 * 2.0 - 1.0);
                    let mut yd = f64::from(yy as f32 / 15.0 * 2.0 - 1.0);
                    let mut zd = f64::from(zz as f32 / 15.0 * 2.0 - 1.0);
                    let d = (xd * xd + yd * yd + zd * zd).sqrt();
                    xd /= d;
                    yd /= d;
                    zd /= d;
                    let mut remaining_power = self.radius * (0.7 + rand::random::<f32>() * 0.6);
                    let mut xp = self.center.x;
                    let mut yp = self.center.y;
                    let mut zp = self.center.z;

                    while remaining_power > 0.0 {
                        let pos = BlockPos::floored(xp, yp, zp);
                        if !world.is_in_build_limit(pos) {
                            break;
                        }
                        let state_id = Self::block_state_at(world, &mut chunk_cache, pos);
                        let (block, state) = BlockState::from_id_with_block(state_id);
                        let fluid = Self::fluid_state(block, state_id);

                        if let Some(resistance) = self
                            .damage_calculator
                            .get_block_explosion_resistance(self, world, &pos, block, state, fluid)
                        {
                            remaining_power -= (resistance + 0.3) * 0.3;
                        }
                        if remaining_power > 0.0
                            && self.damage_calculator.should_block_explode(
                                self,
                                world,
                                &pos,
                                block,
                                remaining_power,
                            )
                        {
                            to_blow.insert(pos);
                        }

                        xp += xd * f64::from(0.3f32);
                        yp += yd * f64::from(0.3f32);
                        zp += zd * f64::from(0.3f32);
                        remaining_power -= 0.225_000_01;
                    }
                }
            }
        }
        to_blow.into_iter().collect()
    }

    fn block_state_at(
        world: &World,
        chunk_cache: &mut FxHashMap<
            pumpkin_util::math::vector2::Vector2<i32>,
            Option<Arc<ChunkData>>,
        >,
        pos: BlockPos,
    ) -> BlockStateId {
        let (chunk_pos, relative) = pos.chunk_and_chunk_relative_position();
        let chunk = chunk_cache.entry(chunk_pos).or_insert_with(|| {
            world
                .level
                .read_chunk_sync(&chunk_pos, std::clone::Clone::clone)
        });
        chunk
            .as_ref()
            .and_then(|chunk| {
                chunk.section.get_block_absolute_y(
                    relative.x as usize,
                    pos.0.y,
                    relative.z as usize,
                )
            })
            .unwrap_or(Block::AIR.default_state.id)
    }

    fn fluid_state(
        block: &Block,
        state_id: BlockStateId,
    ) -> &'static pumpkin_data::fluid::FluidState {
        Fluid::from_state_id(state_id).map_or_else(
            || {
                if block.is_waterlogged(state_id) {
                    &Fluid::FLOWING_WATER.states[0]
                } else {
                    &Fluid::EMPTY.states[0]
                }
            },
            |raw_fluid| &raw_fluid.to_flowing().states[0],
        )
    }

    /// Vanilla `hurtEntities`.
    #[expect(
        clippy::too_many_lines,
        reason = "mirrors vanilla `ServerExplosion.hurtEntities`"
    )]
    fn hurt_entities(&self, world: &Arc<World>) {
        if self.radius < 1.0e-5 {
            return;
        }
        let double_radius = self.radius * 2.0;
        let r = f64::from(double_radius);
        let x0 = (self.center.x - r - 1.0).floor();
        let x1 = (self.center.x + r + 1.0).floor();
        let y0 = (self.center.y - r - 1.0).floor();
        let y1 = (self.center.y + r + 1.0).floor();
        let z0 = (self.center.z - r - 1.0).floor();
        let z1 = (self.center.z + r + 1.0).floor();
        let search_box = BoundingBox::new(Vector3::new(x0, y0, z0), Vector3::new(x1, y1, z1));
        let source_id = self.source.as_ref().map(|s| s.get_entity().entity_id);

        // `getEntities(source, box)`: everything but the source and spectators.
        let in_box = world.get_all_at_box(&search_box).into_iter();
        for entity_base in in_box.chain(parts_at_box(world, &search_box)) {
            let entity = entity_base.get_entity();
            if Some(entity.entity_id) == source_id || entity_base.is_spectator() {
                continue;
            }
            if entity_base.is_immune_to_explosion() {
                continue;
            }
            let dist = entity
                .pos
                .load()
                .squared_distance_to_vec(&self.center)
                .sqrt()
                / r;
            if dist > 1.0 {
                continue;
            }
            let origin = if entity.entity_type == &EntityType::TNT {
                entity.pos.load()
            } else {
                entity.get_eye_pos()
            };
            let direction = (origin - self.center).normalize();
            let should_damage = self
                .damage_calculator
                .should_damage_entity(self, entity_base.as_ref());
            let knockback_multiplier = self
                .damage_calculator
                .get_knockback_multiplier(entity_base.as_ref());
            let exposure = if !should_damage && knockback_multiplier == 0.0 {
                0.0
            } else {
                Self::seen_percent(&self.center, entity, world)
            };
            if should_damage
                && self.source.as_ref().is_none_or(|source| {
                    crate::entity::do_teams_allow_damage(source.as_ref(), entity_base.as_ref())
                })
            {
                let damage = self.damage_calculator.get_entity_damage_amount(
                    self,
                    entity_base.as_ref(),
                    exposure,
                );
                entity_base.damage_with_context(
                    entity_base.as_ref(),
                    damage,
                    self.damage_source.damage_type,
                    self.damage_source.position,
                    self.damage_source.direct.as_deref(),
                    self.damage_source.causing.as_deref(),
                );
            }

            let knockback_resistance = entity_base.get_living_entity().map_or(0.0, |living| {
                living.get_attribute_value(
                    &pumpkin_data::attributes::Attributes::EXPLOSION_KNOCKBACK_RESISTANCE,
                )
            });
            let knockback_power = (1.0 - dist)
                * f64::from(exposure)
                * f64::from(knockback_multiplier)
                * (1.0 - knockback_resistance);
            let knockback = direction * knockback_power;
            // `pushFromExplosion`.
            entity_base.push_impulse(knockback);
            if entity
                .entity_type
                .has_tag(&tag::EntityType::MINECRAFT_REDIRECTABLE_PROJECTILE)
            {
                if let Some(projectile) = entity_base.get_projectile() {
                    projectile.set_owner(
                        self.damage_source
                            .causing
                            .as_ref()
                            .map(|causing| causing.get_entity()),
                    );
                }
            } else if let Some(player) = entity_base.get_player()
                && !player.is_spectator()
                && !(player.is_creative() && player.is_flying())
            {
                self.hit_players
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(entity.entity_id, knockback);
            }
            // `onExplosionHit`, which only `ServerPlayer` overrides.
            if let Some(player) = entity_base.get_player() {
                player.on_explosion_hit(self.source.as_deref());
            }
        }
    }

    /// Vanilla `getSeenPercent`.
    fn seen_percent(center: &Vector3<f64>, entity: &Entity, world: &Arc<World>) -> f32 {
        let bb = entity.bounding_box.load();
        let xs = 1.0 / ((bb.max.x - bb.min.x) * 2.0 + 1.0);
        let ys = 1.0 / ((bb.max.y - bb.min.y) * 2.0 + 1.0);
        let zs = 1.0 / ((bb.max.z - bb.min.z) * 2.0 + 1.0);
        let x_offset = (1.0 - (1.0 / xs).floor() * xs) / 2.0;
        let z_offset = (1.0 - (1.0 / zs).floor() * zs) / 2.0;
        if xs < 0.0 || ys < 0.0 || zs < 0.0 {
            return 0.0;
        }
        let mut hits = 0;
        let mut count = 0;
        let mut xx = 0.0;
        while xx <= 1.0 {
            let mut yy = 0.0;
            while yy <= 1.0 {
                let mut zz = 0.0;
                while zz <= 1.0 {
                    // `Mth.lerp(delta, min, max)`.
                    let x = bb.min.x + xx * (bb.max.x - bb.min.x);
                    let y = bb.min.y + yy * (bb.max.y - bb.min.y);
                    let z = bb.min.z + zz * (bb.max.z - bb.min.z);
                    let from = Vector3::new(x + x_offset, y, z + z_offset);
                    if world
                        .clip(&pumpkin_world::clip::ClipContext::new(
                            from,
                            *center,
                            pumpkin_world::clip::ClipBlock::Collider,
                            pumpkin_world::clip::ClipFluid::None,
                        ))
                        .is_miss
                    {
                        hits += 1;
                    }
                    count += 1;
                    zz += zs;
                }
                yy += ys;
            }
            xx += xs;
        }
        hits as f32 / count as f32
    }

    /// Vanilla `interactWithBlocks`: shuffled, each block's `onExplosionHit`, then the merged
    /// drops pop out.
    fn interact_with_blocks(&self, world: &Arc<World>, target_blocks: &mut [BlockPos]) {
        target_blocks.shuffle(&mut rand::rng());
        let mut stacks: Vec<(BlockPos, ItemStack)> = Vec::new();
        for pos in target_blocks.iter() {
            let (block, state) = world.get_block_and_state(pos);
            self.on_explosion_hit(world, pos, block, state, &mut stacks);
        }
        for (pos, stack) in stacks {
            world.drop_stack(&pos, stack);
        }
    }

    /// Vanilla `BlockBehaviour.onExplosionHit`.
    fn on_explosion_hit(
        &self,
        world: &Arc<World>,
        pos: &BlockPos,
        block: &'static Block,
        state: &'static BlockState,
        stacks: &mut Vec<(BlockPos, ItemStack)>,
    ) {
        if state.is_air() || self.block_interaction == BlockInteraction::TriggerBlock {
            return;
        }
        let pumpkin_block = world.block_registry.get_pumpkin_block(block.id);
        if let Some(pumpkin_block) = pumpkin_block {
            pumpkin_block.on_explosion_hit(ExplodeArgs {
                world,
                block,
                position: pos,
                explosion: self,
            });
        }
        let drop_experience = self
            .indirect_source
            .as_ref()
            .is_some_and(|source| source.get_player().is_some());
        if pumpkin_block.is_none_or(|b| b.should_drop_items_on_explosion()) {
            if let Some(pumpkin_block) = pumpkin_block {
                pumpkin_block.spawn_after_break(SpawnAfterBreakArgs {
                    world,
                    block,
                    position: pos,
                    tool: None,
                });
            }
            if drop_experience {
                drop_block_experience(world, block, pos, None);
            }
            let params = LootContextParameters {
                block_state: Some(state),
                explosion_radius: (self.block_interaction == BlockInteraction::DestroyWithDecay)
                    .then_some(self.radius),
                position: Some(pos.to_centered_f64()),
                this_entity: self.source.as_ref().map(|s| s.get_entity().entity_type),
                world_time: world.get_time_of_day() as u64,
                is_raining: Some(world.is_raining()),
                is_thundering: Some(world.is_thundering()),
                ..Default::default()
            };
            for stack in block_loot_stacks(world, block, pos, &params) {
                add_or_append_stack(stacks, stack, *pos);
            }
        }
        world.set_block_state(pos, BlockStateId::AIR, BlockFlags::NOTIFY_ALL);
        world.close_container_screens_at(pos);
        if let Some(pumpkin_block) = pumpkin_block {
            pumpkin_block.explode(ExplodeArgs {
                world,
                block,
                position: pos,
                explosion: self,
            });
        }
    }

    /// Vanilla `createFire`.
    fn create_fire(world: &Arc<World>, target_blocks: &[BlockPos]) {
        for pos in target_blocks {
            if rand::random_range(0..3) == 0
                && world.get_block_state(pos).is_air()
                && world.get_block_state(&pos.down()).is_solid_render()
            {
                let fire = crate::block::blocks::fire::FireBlockBase::get_state(world, pos);
                world.set_block_state(pos, fire, BlockFlags::NOTIFY_ALL);
            }
        }
    }

    /// Vanilla `ServerExplosion.explode`; returns the exploded block count.
    pub fn explode(&self, world: &Arc<World>) -> usize {
        world.game_event_from(self.source.as_deref(), GameEvent::Explode, self.center);
        let mut to_blow = self.calculate_exploded_positions(world);
        self.hurt_entities(world);
        if self.interacts_with_blocks() {
            let center_pos = BlockPos::floored(self.center.x, self.center.y, self.center.z);
            let mut event =
                crate::plugin::api::events::block::block_explode::BlockExplodeEvent::new(
                    center_pos,
                    if self.radius > 0.0 {
                        1.0 / self.radius
                    } else {
                        1.0
                    },
                );
            if let Some(server) = world.server.upgrade() {
                server.plugin_manager.fire_blocking(&server, &mut event);
            }
            if !event.cancelled {
                self.interact_with_blocks(world, &mut to_blow);
            }
        }
        if self.fire {
            Self::create_fire(world, &to_blow);
        }
        to_blow.len()
    }
}

/// Vanilla `ServerExplosion.addOrAppendStack`.
fn add_or_append_stack(
    stacks: &mut Vec<(BlockPos, ItemStack)>,
    mut stack: ItemStack,
    pos: BlockPos,
) {
    for (_, collected) in stacks.iter_mut() {
        // `StackCollector.tryMerge`: `ItemEntity.areMergable` then `merge(.., 16)`.
        if u32::from(stack.item_count) + u32::from(collected.item_count)
            <= u32::from(stack.get_max_stack_size())
            && collected.are_items_and_components_equal(&stack)
        {
            let delta = (i32::from(collected.get_max_stack_size().min(16))
                - i32::from(collected.item_count))
            .min(i32::from(stack.item_count));
            collected.item_count = (i32::from(collected.item_count) + delta) as u8;
            stack.item_count = (i32::from(stack.item_count) - delta) as u8;
        }
        if stack.is_empty() {
            return;
        }
    }
    stacks.push((pos, stack));
}
