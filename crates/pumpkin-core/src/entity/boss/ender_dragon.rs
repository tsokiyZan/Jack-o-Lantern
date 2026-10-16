//! Vanilla `EnderDragon`.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, game_event::GameEvent};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::math::{atan2, cos, sin, wrap_degrees, wrap_degrees_f64};
use pumpkin_world::chunk::ChunkHeightmapType;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;
use uuid::Uuid;

use crate::enchantment::EnchantmentHelper;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::damage_source::DamageSource;
use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::living::can_be_seen_as_enemy;
use crate::entity::mob::{Mob, MobEntity, mob_hurt_server};
use crate::entity::{Entity, EntityBase, do_teams_allow_damage};
use crate::world::World;
use crate::world::dragon_fight::DragonFight;

pub mod flight_history;
pub mod part;
pub mod path;
pub mod phase;

use flight_history::DragonFlightHistory;
use part::EnderDragonPart;
use path::{DragonPath, NODE_COUNT, Node};
use phase::{DragonPhaseInstance, EnderDragonPhase, EnderDragonPhaseManager};

const SITTING_ALLOWED_DAMAGE_PERCENTAGE: f32 = 0.25;
const DRAGON_DEATH_TIME_KEY: &str = "DragonDeathTime";
const DRAGON_PHASE_KEY: &str = "DragonPhase";
const SITTING_DAMAGE_RECEIVED_KEY: &str = "sitting_damage_received";
/// Vanilla `CRYSTAL_DESTROY_TARGETING`: `forCombat().range(64)`.
const CRYSTAL_DESTROY_RANGE: f64 = 64.0;
const DEG_TO_RAD: f32 = (std::f64::consts::PI / 180.0) as f32;

const HEAD: usize = 0;
const NECK: usize = 1;
const BODY: usize = 2;
const TAIL1: usize = 3;
const WING1: usize = 6;
const WING2: usize = 7;
/// The sub-entities in vanilla's order (their ids follow the dragon's), with their sizes.
const PARTS: [(&str, f32, f32); 8] = [
    ("head", 1.0, 1.0),
    ("neck", 3.0, 3.0),
    ("body", 5.0, 3.0),
    ("tail", 2.0, 2.0),
    ("tail", 2.0, 2.0),
    ("tail", 2.0, 2.0),
    ("wing", 4.0, 2.0),
    ("wing", 4.0, 2.0),
];

/// Vanilla `Entity.getY(scale)`: the height `scale` of the way up the entity.
#[must_use]
pub fn get_y(entity: &Entity, scale: f64) -> f64 {
    entity.pos.load().y + f64::from(entity.height()) * scale
}

/// Vanilla `ServerLevel.getEntityOrPart` for an id that isn't a level entity: the dragon
/// part with that id, if any.
#[must_use]
pub fn find_part(world: &World, id: i32) -> Option<Arc<dyn EntityBase>> {
    world.entities.load().iter().find_map(|entity| {
        let dragon = entity.cast_any().downcast_ref::<EnderDragonEntity>()?;
        dragon
            .sub_entities
            .iter()
            .find(|part| part.get_entity().entity_id == id)
            .map(|part| part.clone() as Arc<dyn EntityBase>)
    })
}

/// The dragon parts `Level.getEntities` lists next to the level entities: those whose box
/// meets `area`.
#[must_use]
pub fn parts_at_box(world: &World, area: &BoundingBox) -> Vec<Arc<dyn EntityBase>> {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.cast_any().downcast_ref::<EnderDragonEntity>())
        .flat_map(|dragon| dragon.sub_entities.iter())
        .filter(|part| part.get_entity().bounding_box.load().intersects(area))
        .map(|part| part.clone() as Arc<dyn EntityBase>)
        .collect()
}

/// What a projectile flying through `area` may hit among `candidates`: the dragon itself isn't
/// pickable (vanilla `canBeHitByProjectile`), so only its parts are.
#[must_use]
pub fn projectile_targets(
    world: &World,
    area: &BoundingBox,
    candidates: impl IntoIterator<Item = Arc<dyn EntityBase>>,
) -> Vec<Arc<dyn EntityBase>> {
    candidates
        .into_iter()
        .filter(|candidate| !candidate.cast_any().is::<EnderDragonEntity>())
        .chain(parts_at_box(world, area))
        .collect()
}

pub struct EnderDragonEntity {
    pub mob_entity: MobEntity,
    sub_entities: [Arc<EnderDragonPart>; 8],
    pub flight_history: Mutex<DragonFlightHistory>,
    pub o_flap_time: AtomicCell<f32>,
    pub flap_time: AtomicCell<f32>,
    pub in_wall: AtomicBool,
    pub dragon_death_time: AtomicI32,
    pub y_rot_a: AtomicCell<f32>,
    nearest_crystal: Mutex<Option<Arc<dyn EntityBase>>>,
    /// Vanilla `dragonFight != null`: this is the level's fight's dragon.
    has_dragon_fight: AtomicBool,
    fight_origin: AtomicCell<BlockPos>,
    pub phase_manager: EnderDragonPhaseManager,
    sitting_damage_received: AtomicCell<f32>,
    /// Vanilla `nodes`, built on the first `findClosestNode`.
    nodes: Mutex<Option<[Node; NODE_COUNT]>>,
}

impl EnderDragonEntity {
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the dragon takes a fresh id block, so `entity` is replaced"
    )]
    pub fn new(entity: Entity) -> Arc<Self> {
        // The client numbers the parts after the dragon, so their ids must follow its own.
        let base_id = Entity::reserve_ids(PARTS.len() as i32 + 1);
        let world = entity.world.load_full();
        let pos = entity.pos.load();
        let entity = Entity::from_uuid_with_id(
            base_id,
            entity.entity_uuid,
            world.clone(),
            pos,
            entity.entity_type,
        );
        entity.no_physics.store(true, Relaxed);
        Arc::new_cyclic(|dragon| Self {
            mob_entity: MobEntity::new(entity),
            sub_entities: std::array::from_fn(|i| {
                let (name, width, height) = PARTS[i];
                let part_entity = Entity::from_uuid_with_id(
                    base_id + 1 + i as i32,
                    Uuid::new_v4(),
                    world.clone(),
                    pos,
                    &EntityType::ENDER_DRAGON,
                );
                Arc::new(EnderDragonPart::new(
                    part_entity,
                    dragon.clone(),
                    i,
                    name,
                    width,
                    height,
                ))
            }),
            flight_history: Mutex::new(DragonFlightHistory::default()),
            o_flap_time: AtomicCell::new(0.0),
            flap_time: AtomicCell::new(0.0),
            in_wall: AtomicBool::new(false),
            dragon_death_time: AtomicI32::new(0),
            y_rot_a: AtomicCell::new(0.0),
            nearest_crystal: Mutex::new(None),
            has_dragon_fight: AtomicBool::new(false),
            fight_origin: AtomicCell::new(BlockPos::ZERO),
            phase_manager: EnderDragonPhaseManager::new(),
            sitting_damage_received: AtomicCell::new(0.0),
            nodes: Mutex::new(None),
        })
    }

    /// Vanilla `setDragonFight`.
    pub fn set_dragon_fight(&self) {
        self.has_dragon_fight.store(true, Relaxed);
    }

    pub fn set_fight_origin(&self, fight_origin: BlockPos) {
        self.fight_origin.store(fight_origin);
    }

    #[must_use]
    pub fn get_fight_origin(&self) -> BlockPos {
        self.fight_origin.load()
    }

    #[must_use]
    pub const fn get_sub_entities(&self) -> &[Arc<EnderDragonPart>; 8] {
        &self.sub_entities
    }

    #[must_use]
    pub fn head(&self) -> &EnderDragonPart {
        &self.sub_entities[HEAD]
    }

    pub fn set_phase(&self, phase: EnderDragonPhase) {
        self.phase_manager.set_phase(self, phase);
    }

    /// Runs `f` on the fight when this is its dragon (vanilla `dragonFight`).
    fn with_dragon_fight<R>(
        &self,
        f: impl FnOnce(&mut DragonFight, &Arc<World>) -> R,
    ) -> Option<R> {
        if !self.has_dragon_fight.load(Relaxed) {
            return None;
        }
        let world = self.get_entity().world.load_full();
        let fight = world.dragon_fight.as_ref()?;
        let mut fight = fight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Some(f(&mut fight, &world))
    }

    /// Vanilla `getDragonFight().aliveCrystals()`, `None` without a fight.
    #[must_use]
    pub fn alive_crystals(&self) -> Option<i32> {
        self.with_dragon_fight(|fight, _| fight.alive_crystals())
    }

    fn update_dragon_fight(&self) {
        let living = &self.mob_entity.living_entity;
        let custom_name = living.entity.custom_name.load();
        self.with_dragon_fight(|fight, world| {
            fight.update_dragon(
                world,
                living.entity.entity_uuid,
                living.health.load(),
                living.get_max_health(),
                custom_name.as_ref().as_ref(),
            );
        });
    }

    /// Vanilla `horizontalCollision || verticalCollision`. Moving without physics clears
    /// both, and Pumpkin tracks no vertical flag.
    #[must_use]
    pub fn has_collided(&self) -> bool {
        self.get_entity().horizontal_collision.load(Relaxed)
    }

    /// Vanilla `level.getHeightmapPos(heightmap, EnderDragonFight.getPodiumLocation(fightOrigin))`.
    #[must_use]
    pub fn podium_heightmap_pos(&self, heightmap: ChunkHeightmapType) -> BlockPos {
        let origin = self.fight_origin.load();
        let world = self.get_entity().world.load();
        // Pumpkin's height is the top block, vanilla's the first free one above it.
        let y = world.get_heightmap_height(heightmap, origin.0.x, origin.0.z) + 1;
        BlockPos::new(origin.0.x, y, origin.0.z)
    }

    /// Vanilla `getHeadLookVector(1.0F)`.
    #[must_use]
    pub fn get_head_look_vector(&self) -> Vector3<f64> {
        let entity = self.get_entity();
        let current_phase = self.phase_manager.get_current_phase();
        let phase = current_phase.get_phase();
        let x_rot = if phase == EnderDragonPhase::Landing || phase == EnderDragonPhase::Takeoff {
            let egg = self.podium_heightmap_pos(ChunkHeightmapType::MotionBlockingNoLeaves);
            let dist = ((egg
                .to_centered_f64()
                .squared_distance_to_vec(&entity.pos.load()))
            .sqrt() as f32
                / 4.0)
                .max(1.0);
            let y_offset = 6.0 / dist;
            -y_offset * 1.5 * 5.0
        } else if current_phase.is_sitting() {
            -45.0
        } else {
            entity.pitch.load()
        };
        Entity::calculate_view_vector(x_rot, entity.yaw.load())
    }

    fn get_head_y_offset(&self, history: &DragonFlightHistory) -> f32 {
        if self.phase_manager.get_current_phase().is_sitting() {
            return -1.0;
        }
        (history.get(5).y - history.get(0).y) as f32
    }

    /// Vanilla `findClosestNode()`, which first lays out the 24 nodes.
    pub fn find_closest_node(&self) -> usize {
        {
            let mut nodes = self
                .nodes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if nodes.is_none() {
                let world = self.get_entity().world.load();
                *nodes = Some(std::array::from_fn(|i| {
                    let mut y_adjustment = 5;
                    let (radius, step, multiplier) = if i < 12 {
                        (60.0, std::f64::consts::PI / 12.0, i)
                    } else if i < 20 {
                        y_adjustment += 10;
                        (40.0, std::f64::consts::PI / 8.0, i - 12)
                    } else {
                        (20.0, std::f64::consts::PI / 4.0, i - 20)
                    };
                    let angle =
                        2.0 * (-std::f64::consts::PI as f32 + step as f32 * multiplier as f32);
                    let node_x = (radius * cos(angle)).floor() as i32;
                    let node_z = (radius * sin(angle)).floor() as i32;
                    // Pumpkin's height is the top block, vanilla's the first free one above it.
                    let height = world.get_heightmap_height(
                        ChunkHeightmapType::MotionBlockingNoLeaves,
                        node_x,
                        node_z,
                    ) + 1;
                    Node::new(node_x, 73.max(height + y_adjustment), node_z)
                }));
            }
        }
        let pos = self.get_entity().pos.load();
        self.find_closest_node_at(pos.x, pos.y, pos.z)
    }

    /// Vanilla `findClosestNode(x, y, z)`: the outer ring only counts while crystals stand,
    /// and nothing within 100 blocks gives node 0.
    #[must_use]
    pub fn find_closest_node_at(&self, x: f64, y: f64, z: f64) -> usize {
        let mut closest_dist = 10000.0f32;
        let mut closest_index = 0;
        let current_pos = Node::new(x.floor() as i32, y.floor() as i32, z.floor() as i32);
        let start_index = if self.alive_crystals().is_none_or(|crystals| crystals == 0) {
            12
        } else {
            0
        };
        let nodes = self
            .nodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(nodes) = nodes.as_ref() {
            for (i, node) in nodes.iter().enumerate().skip(start_index) {
                let dist = node.distance_to_sqr(&current_pos);
                if dist < closest_dist {
                    closest_dist = dist;
                    closest_index = i;
                }
            }
        }
        closest_index
    }

    /// Vanilla `findPath`.
    #[must_use]
    pub fn find_path(
        &self,
        start_index: usize,
        end_index: usize,
        final_node: Option<Node>,
    ) -> Option<DragonPath> {
        let minimum_node_index = if self.alive_crystals().is_none_or(|crystals| crystals == 0) {
            12
        } else {
            0
        };
        let nodes = self
            .nodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        path::find_path(
            nodes.as_ref()?,
            start_index,
            end_index,
            final_node,
            minimum_node_index,
        )
    }

    /// Vanilla `aiStep`, which replaces `LivingEntity.aiStep` entirely: no travel, no
    /// `serverAiStep` and no entity pushing.
    fn dragon_ai_step(&self) {
        let entity = self.get_entity();
        let world = entity.world.load_full();
        // `processFlappingMovement`; the flap sound is the client's.
        if Mob::is_flapping(self) && Mob::movement_emission(self).emits_events() {
            world.emit_game_event(GameEvent::Flap.name(), entity.pos.load());
        }

        if !self.has_dragon_fight.load(Relaxed)
            && let Some(fight) = &world.dragon_fight
            && fight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .dragon_uuid()
                == Some(entity.entity_uuid)
        {
            self.set_dragon_fight();
        }

        self.o_flap_time.store(self.flap_time.load());
        // The explosion particles of a dying dragon are the client's.
        if self.mob_entity.living_entity.health.load() <= 0.0 {
            return;
        }

        self.check_crystals(&world);
        let movement = entity.velocity.load();
        let mut flap_speed = 0.2 / (movement.horizontal_length() as f32 * 10.0 + 1.0);
        flap_speed *= 2.0f64.powf(movement.y) as f32;
        let flap_step = if self.phase_manager.get_current_phase().is_sitting() {
            0.1
        } else if self.in_wall.load(Relaxed) {
            flap_speed * 0.5
        } else {
            flap_speed
        };
        self.flap_time.store(self.flap_time.load() + flap_step);

        entity.yaw.store(wrap_degrees(entity.yaw.load()));
        if self.mob_entity.is_no_ai() {
            self.flap_time.store(0.5);
            return;
        }

        self.flight_history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record(entity.pos.load().y, entity.yaw.load());
        let mut current_phase = self.phase_manager.get_current_phase();
        current_phase.do_server_tick(self);
        if self.phase_manager.get_current_phase().get_phase() != current_phase.get_phase() {
            current_phase = self.phase_manager.get_current_phase();
            current_phase.do_server_tick(self);
        }

        if let Some(target_location) = current_phase.get_fly_target_location() {
            self.fly_towards(current_phase, target_location);
        }

        // `applyEffectsFromBlocks` does nothing without physics.
        entity.body_yaw.store(entity.yaw.load());
        self.tick_parts(&world);
    }

    /// The steering half of vanilla `aiStep`, towards the phase's fly target.
    fn fly_towards(&self, current_phase: &dyn DragonPhaseInstance, target_location: Vector3<f64>) {
        let entity = self.get_entity();
        let pos = entity.pos.load();
        let xdd = target_location.x - pos.x;
        let mut ydd = target_location.y - pos.y;
        let zdd = target_location.z - pos.z;
        let dist_to_target = xdd * xdd + ydd * ydd + zdd * zdd;
        let max = f64::from(current_phase.get_fly_speed());
        #[expect(
            clippy::imprecise_flops,
            reason = "vanilla rounds Math.sqrt(x * x + z * z), not a hypot"
        )]
        let horizontal_dist = (xdd * xdd + zdd * zdd).sqrt();
        if horizontal_dist > 0.0 {
            ydd = (ydd / horizontal_dist).clamp(-max, max);
        }

        entity
            .velocity
            .store(entity.velocity.load() + Vector3::new(0.0, ydd * 0.01, 0.0));
        entity.yaw.store(wrap_degrees(entity.yaw.load()));
        let aim = (target_location - pos).normalize();
        let yaw = entity.yaw.load();
        let dir = Vector3::new(
            f64::from(sin(yaw * DEG_TO_RAD)),
            entity.velocity.load().y,
            f64::from(-cos(yaw * DEG_TO_RAD)),
        )
        .normalize();
        let dot = ((dir.dot(&aim) as f32 + 0.5) / 1.5).max(0.0);
        if xdd.abs() > f64::from(1.0e-5f32) || zdd.abs() > f64::from(1.0e-5f32) {
            let y_rot_d =
                wrap_degrees(180.0 - atan2(xdd, zdd) as f32 * (180.0 / std::f32::consts::PI) - yaw)
                    .clamp(-50.0, 50.0);
            let y_rot_a = self.y_rot_a.load() * 0.8 + y_rot_d * current_phase.get_turn_speed(self);
            self.y_rot_a.store(y_rot_a);
            entity.yaw.store(yaw + y_rot_a * 0.1);
        }

        let span = (2.0 / (dist_to_target + 1.0)) as f32;
        let speed = 0.06f32;
        entity.update_velocity_from_input(
            Vector3::new(0.0, 0.0, -1.0),
            f64::from(speed * (dot * span + (1.0 - span))),
        );
        let delta = entity.velocity.load();
        if self.in_wall.load(Relaxed) {
            entity.move_entity(self, delta * f64::from(0.8f32));
        } else {
            entity.move_entity(self, delta);
        }

        let velocity = entity.velocity.load();
        let actual = velocity.normalize();
        let slide = 0.8 + 0.15 * (actual.dot(&dir) + 1.0) / 2.0;
        entity.velocity.store(Vector3::new(
            velocity.x * slide,
            velocity.y * f64::from(0.91f32),
            velocity.z * slide,
        ));
    }

    /// Vanilla `tickPart`.
    fn tick_part(&self, part: usize, x: f64, y: f64, z: f64) {
        let pos = self.get_entity().pos.load();
        self.sub_entities[part].set_pos(Vector3::new(pos.x + x, pos.y + y, pos.z + z));
    }

    fn part_bounding_box(&self, part: usize) -> BoundingBox {
        self.sub_entities[part].get_entity().bounding_box.load()
    }

    /// The body half of vanilla `aiStep`: placing the parts along the flight history, the wing
    /// and head attacks, and breaking through blocks.
    fn tick_parts(&self, world: &Arc<World>) {
        let entity = self.get_entity();
        let history = *self
            .flight_history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tilt = (history.get(5).y - history.get(10).y) as f32 * 10.0 * DEG_TO_RAD;
        let cc_tilt = cos(tilt);
        let ss_tilt = sin(tilt);
        let rot1 = entity.yaw.load() * DEG_TO_RAD;
        let ss1 = sin(rot1);
        let cc1 = cos(rot1);
        self.tick_part(BODY, f64::from(ss1 * 0.5), 0.0, f64::from(-cc1 * 0.5));
        self.tick_part(WING1, f64::from(cc1 * 4.5), 2.0, f64::from(ss1 * 4.5));
        self.tick_part(WING2, f64::from(cc1 * -4.5), 2.0, f64::from(ss1 * -4.5));
        if !self.mob_entity.living_entity.was_hurt_recently() {
            for wing in [WING1, WING2] {
                let area = self
                    .part_bounding_box(wing)
                    .expand(4.0, 2.0, 4.0)
                    .shift(Vector3::new(0.0, -2.0, 0.0));
                self.knock_back(Self::get_entities(world, entity, &area));
            }
            for part in [HEAD, NECK] {
                let area = self.part_bounding_box(part).expand_all(1.0);
                self.hurt_entities(Self::get_entities(world, entity, &area));
            }
        }

        let y_rot_a = self.y_rot_a.load();
        let rot2 = entity.yaw.load() * DEG_TO_RAD - y_rot_a * 0.01;
        let ss2 = sin(rot2);
        let cc2 = cos(rot2);
        let y_offset = self.get_head_y_offset(&history);
        self.tick_part(
            HEAD,
            f64::from(ss2 * 6.5 * cc_tilt),
            f64::from(y_offset + ss_tilt * 6.5),
            f64::from(-cc2 * 6.5 * cc_tilt),
        );
        self.tick_part(
            NECK,
            f64::from(ss2 * 5.5 * cc_tilt),
            f64::from(y_offset + ss_tilt * 5.5),
            f64::from(-cc2 * 5.5 * cc_tilt),
        );
        let p1 = history.get(5);
        for i in 0..3 {
            let p0 = history.get(12 + i * 2);
            let rot = entity.yaw.load() * DEG_TO_RAD
                + wrap_degrees_f64(f64::from(p0.y_rot - p1.y_rot)) as f32 * DEG_TO_RAD;
            let ss = sin(rot);
            let cc = cos(rot);
            let dd1 = 1.5f32;
            let dd = (i + 1) as f32 * 2.0;
            self.tick_part(
                TAIL1 + i as usize,
                f64::from(-(ss1 * dd1 + ss * dd) * cc_tilt),
                p0.y - p1.y - f64::from((dd + dd1) * ss_tilt) + 1.5,
                f64::from((cc1 * dd1 + cc * dd) * cc_tilt),
            );
        }

        let in_wall = self.check_walls(world, &self.part_bounding_box(HEAD))
            | self.check_walls(world, &self.part_bounding_box(NECK))
            | self.check_walls(world, &self.part_bounding_box(BODY));
        self.in_wall.store(in_wall, Relaxed);
        self.update_dragon_fight();
    }

    /// Vanilla `getEntities(this, area, EntitySelector.NO_CREATIVE_OR_SPECTATOR)`.
    fn get_entities(
        world: &World,
        dragon: &Entity,
        area: &BoundingBox,
    ) -> Vec<Arc<dyn EntityBase>> {
        world
            .get_all_at_box(area)
            .into_iter()
            .filter(|entity| {
                entity.get_entity().entity_id != dragon.entity_id
                    && entity
                        .get_player()
                        .is_none_or(|player| !player.is_spectator() && !player.is_creative())
            })
            .collect()
    }

    /// Vanilla `checkCrystals`: heals 1 every 10 ticks while a crystal is linked, and looks
    /// for the nearest one about every 10 ticks.
    fn check_crystals(&self, world: &World) {
        let entity = self.get_entity();
        let living = &self.mob_entity.living_entity;
        let mut nearest_crystal = self
            .nearest_crystal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(crystal) = nearest_crystal.as_ref() {
            if crystal.get_entity().is_removed() {
                *nearest_crystal = None;
            } else if entity.tick_count.load(Relaxed) % 10 == 0
                && living.health.load() < living.get_max_health()
            {
                living.set_health(living.health.load() + 1.0);
            }
        }

        if self.get_random().random_range(0..10) == 0 {
            let pos = entity.pos.load();
            let area = entity.bounding_box.load().expand_all(32.0);
            let mut nearest = None;
            let mut distance = f64::MAX;
            for crystal in world.get_entities_at_box(&area) {
                if crystal.get_entity().entity_type != &EntityType::END_CRYSTAL {
                    continue;
                }
                let dist = crystal
                    .get_entity()
                    .pos
                    .load()
                    .squared_distance_to_vec(&pos);
                if dist < distance {
                    distance = dist;
                    nearest = Some(crystal);
                }
            }
            *nearest_crystal = nearest;
        }
    }

    /// Vanilla `knockBack`: the wings shove what they pass and hurt it unless sitting.
    fn knock_back(&self, entities: Vec<Arc<dyn EntityBase>>) {
        let body = self.part_bounding_box(BODY);
        let xm = f64::midpoint(body.min.x, body.max.x);
        let zm = f64::midpoint(body.min.z, body.max.z);
        for entity in entities {
            let Some(living_target) = entity.get_living_entity() else {
                continue;
            };
            let target = entity.get_entity();
            let pos = target.pos.load();
            let xd = pos.x - xm;
            let zd = pos.z - zm;
            let dd = (xd * xd + zd * zd).max(0.1);
            target.push_impulse(Vector3::new(
                xd / dd * 4.0,
                f64::from(0.2f32),
                zd / dd * 4.0,
            ));
            if !self.phase_manager.get_current_phase().is_sitting()
                && living_target.last_attacked_time.load(Relaxed)
                    < target.tick_count.load(Relaxed) - 2
                && do_teams_allow_damage(self, entity.as_ref())
            {
                let damage_source = DamageSource {
                    damage_type: DamageType::MOB_ATTACK,
                    direct: Some(self),
                    causing: Some(self),
                };
                entity.damage_with_context(
                    entity.as_ref(),
                    5.0,
                    damage_source.damage_type,
                    None,
                    damage_source.direct,
                    damage_source.causing,
                );
                // Unconditional in vanilla, even when the hit did no damage.
                EnchantmentHelper::do_post_attack_effects(
                    &self.get_entity().world.load_full(),
                    entity.as_ref(),
                    &damage_source,
                );
            }
        }
    }

    /// Vanilla `hurt(level, entities)`: the head and neck bite for 10.
    fn hurt_entities(&self, entities: Vec<Arc<dyn EntityBase>>) {
        for target in entities {
            if target.get_living_entity().is_some() && do_teams_allow_damage(self, target.as_ref())
            {
                let damage_source = DamageSource {
                    damage_type: DamageType::MOB_ATTACK,
                    direct: Some(self),
                    causing: Some(self),
                };
                target.damage_with_context(
                    target.as_ref(),
                    10.0,
                    damage_source.damage_type,
                    None,
                    damage_source.direct,
                    damage_source.causing,
                );
                EnchantmentHelper::do_post_attack_effects(
                    &self.get_entity().world.load_full(),
                    target.as_ref(),
                    &damage_source,
                );
            }
        }
    }

    /// Vanilla `checkWalls`: with mob griefing the dragon breaks what it flies through,
    /// otherwise the blocks slow it down.
    fn check_walls(&self, world: &Arc<World>, bb: &BoundingBox) -> bool {
        let x0 = bb.min.x.floor() as i32;
        let y0 = bb.min.y.floor() as i32;
        let z0 = bb.min.z.floor() as i32;
        let x1 = bb.max.x.floor() as i32;
        let y1 = bb.max.y.floor() as i32;
        let z1 = bb.max.z.floor() as i32;
        let mut hit_wall = false;
        let mut destroyed_block = false;

        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    let block_pos = BlockPos::new(x, y, z);
                    let (block, state) = world.get_block_and_state(&block_pos);
                    if !state.is_air() && !block.has_tag(&tag::Block::MINECRAFT_DRAGON_TRANSPARENT)
                    {
                        if world.level_info.load().game_rules.mob_griefing
                            && !block.has_tag(&tag::Block::MINECRAFT_DRAGON_IMMUNE)
                        {
                            destroyed_block =
                                Self::remove_block(world, &block_pos, block, state.id)
                                    || destroyed_block;
                        } else {
                            hit_wall = true;
                        }
                    }
                }
            }
        }

        if destroyed_block {
            let mut random = self.get_random();
            let random_pos = BlockPos::new(
                x0 + random.random_range(0..x1 - x0 + 1),
                y0 + random.random_range(0..y1 - y0 + 1),
                z0 + random.random_range(0..z1 - z0 + 1),
            );
            world.sync_world_event(WorldEvent::ParticlesDragonBlockBreak, random_pos, 0);
        }

        hit_wall
    }

    /// Vanilla `Level.removeBlock(pos, false)`: the block's fluid is left behind, so a fluid
    /// block itself stays as it is.
    fn remove_block(
        world: &Arc<World>,
        pos: &BlockPos,
        block: &Block,
        state_id: pumpkin_data::BlockStateId,
    ) -> bool {
        if block == &Block::WATER || block == &Block::LAVA {
            return false;
        }
        let new_state = if block.is_waterlogged(state_id) {
            Block::WATER.default_state.id
        } else {
            Block::AIR.default_state.id
        };
        world.set_block_state(pos, new_state, BlockFlags::NOTIFY_ALL) != new_state
    }

    /// Vanilla `hurt(level, part, source, damage)`: the head and neck take full damage, the
    /// rest a quarter plus up to 1, and only players and a few damage types get through.
    pub fn hurt(
        &self,
        part: usize,
        damage_type: DamageType,
        damage: f32,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        let current_phase = self.phase_manager.get_current_phase();
        if current_phase.get_phase() == EnderDragonPhase::Dying {
            return false;
        }

        let mut damage = current_phase.on_hurt(damage_type, source, damage);
        if part != HEAD && part != NECK {
            damage = damage / 4.0 + damage.min(1.0);
        }
        if damage < 0.01 {
            return false;
        }

        if cause.is_some_and(|cause| cause.get_player().is_some())
            || damage_type.has_tag(&tag::DamageType::MINECRAFT_ALWAYS_HURTS_ENDER_DRAGONS)
        {
            let living = &self.mob_entity.living_entity;
            let health_before = living.health.load();
            // `reallyHurt`: `LivingEntity.hurtServer`.
            mob_hurt_server(self, self, damage, damage_type, position, source, cause);
            if self.phase_manager.get_current_phase().is_sitting() {
                let received =
                    self.sitting_damage_received.load() + health_before - living.health.load();
                if received > SITTING_ALLOWED_DAMAGE_PERCENTAGE * living.get_max_health() {
                    self.sitting_damage_received.store(0.0);
                    self.set_phase(EnderDragonPhase::Takeoff);
                } else {
                    self.sitting_damage_received.store(received);
                }
            }
        }
        true
    }

    /// Vanilla `onCrystalDestroyed`: the crystal linked to the dragon blasts its head, and the
    /// phase may turn on whoever broke it.
    pub fn on_crystal_destroyed(&self, crystal: &dyn EntityBase, cause: Option<&dyn EntityBase>) {
        let entity = self.get_entity();
        let world = entity.world.load_full();
        let crystal_pos = crystal.get_entity().block_pos.load();
        let player = cause
            .filter(|cause| cause.get_player().is_some())
            .map_or_else(
                || {
                    TargetPredicate::create_attackable()
                        .set_base_max_distance(CRYSTAL_DESTROY_RANGE)
                        .get_nearest_player(&world, None, crystal_pos.to_f64())
                },
                |cause| world.get_entity_by_id(cause.get_entity().entity_id),
            );

        let is_nearest = self
            .nearest_crystal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|nearest| {
                nearest.get_entity().entity_id == crystal.get_entity().entity_id
            });
        if is_nearest {
            // `damageSources().explosion(crystal, player)`.
            let damage_type = if player.is_some() {
                DamageType::PLAYER_EXPLOSION
            } else {
                DamageType::EXPLOSION
            };
            self.hurt(
                HEAD,
                damage_type,
                10.0,
                None,
                Some(crystal),
                player.as_deref(),
            );
        }

        self.phase_manager
            .get_current_phase()
            .on_crystal_destroyed(self, player);
    }

    /// Vanilla `tickDeath`: rising for 10 seconds while dropping experience, then gone.
    fn tick_dragon_death(&self) {
        self.update_dragon_fight();
        let death_time = self.dragon_death_time.fetch_add(1, Relaxed) + 1;
        // The explosion particles from tick 180 are the client's.
        let xp_count = if self
            .with_dragon_fight(|fight, _| fight.has_previously_killed_dragon())
            .is_some_and(|killed| !killed)
        {
            12000
        } else {
            500
        };

        let entity = self.get_entity();
        let world = entity.world.load_full();
        let mob_drops = world.level_info.load().game_rules.mob_drops;
        if death_time > 150 && death_time % 5 == 0 && mob_drops {
            ExperienceOrbEntity::spawn(&world, entity.pos.load(), (xp_count as f32 * 0.08) as u32);
        }
        if death_time == 1 && !entity.is_silent() {
            world.sync_global_world_event(WorldEvent::SoundDragonDeath, entity.block_pos.load(), 0);
        }

        let death_move = Vector3::new(0.0, f64::from(0.1f32), 0.0);
        entity.move_entity(self, death_move);
        for part in &self.sub_entities {
            part.set_pos(part.get_entity().pos.load() + death_move);
        }

        if death_time >= 200 {
            if mob_drops {
                ExperienceOrbEntity::spawn(
                    &world,
                    entity.pos.load(),
                    (xp_count as f32 * 0.2) as u32,
                );
            }
            self.with_dragon_fight(|fight, world| {
                fight.set_dragon_killed(world, entity.entity_uuid);
            });
            entity.remove();
            world.emit_game_event(GameEvent::EntityDie.name(), entity.pos.load());
        }
    }
}

impl Mob for EnderDragonEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn custom_ai_step(&self, _caller: &dyn EntityBase) -> bool {
        self.dragon_ai_step();
        true
    }

    fn tick_death(&self, _caller: &dyn EntityBase) -> bool {
        if self.mob_entity.living_entity.health.load() <= 0.0 {
            self.tick_dragon_death();
        }
        true
    }

    /// Vanilla `hurtServer`: a hit on the dragon itself counts as one on its body.
    fn hurt_server(
        &self,
        _caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        self.hurt(BODY, damage_type, amount, position, source, cause)
    }

    /// Vanilla `handleKillingBlow`: a flying dragon goes on with 1 health to die over the
    /// podium, a sitting one dies where it is. Neither is marked dead.
    fn handle_killing_blow(&self) -> bool {
        if !self.phase_manager.get_current_phase().is_sitting() {
            self.mob_entity.living_entity.set_health(1.0);
            self.set_phase(EnderDragonPhase::Dying);
        }
        true
    }

    fn takes_knockback(&self) -> bool {
        !self.phase_manager.get_current_phase().is_sitting()
    }

    /// Vanilla `kill`: gone at once, without the death sequence.
    fn mob_kill(&self, _caller: &dyn EntityBase) {
        let entity = self.get_entity();
        entity.remove();
        entity
            .world
            .load()
            .emit_game_event(GameEvent::EntityDie.name(), entity.pos.load());
        self.update_dragon_fight();
        self.with_dragon_fight(|fight, world| fight.set_dragon_killed(world, entity.entity_uuid));
    }

    fn can_attack(&self, target: &dyn EntityBase) -> bool {
        can_be_seen_as_enemy(target)
    }

    /// Vanilla `checkDespawn` does nothing for the dragon.
    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        false
    }

    fn is_flapping(&self) -> bool {
        let tau = (std::f64::consts::PI * 2.0) as f32;
        let flap = cos(self.flap_time.load() * tau);
        let old_flap = cos(self.o_flap_time.load() * tau);
        old_flap <= -0.3 && flap >= -0.3
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int(
            DRAGON_PHASE_KEY,
            self.phase_manager.get_current_phase().get_phase().get_id(),
        );
        nbt.put_int(DRAGON_DEATH_TIME_KEY, self.dragon_death_time.load(Relaxed));
        nbt.put_float(
            SITTING_DAMAGE_RECEIVED_KEY,
            self.sitting_damage_received.load(),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if let Some(phase_id) = nbt.get_int(DRAGON_PHASE_KEY) {
            self.set_phase(EnderDragonPhase::get_by_id(phase_id));
        }
        self.dragon_death_time
            .store(nbt.get_int(DRAGON_DEATH_TIME_KEY).unwrap_or(0), Relaxed);
        self.sitting_damage_received
            .store(nbt.get_float(SITTING_DAMAGE_RECEIVED_KEY).unwrap_or(0.0));
    }
}
