use crate::entity::ai::control::flying_move_control::FlyingMoveControl;
use crate::entity::ai::goal::water_avoiding_random_flying::WaterAvoidingRandomFlyingGoal;
use crate::entity::ai::pathfinder::Navigator;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};
use uuid::Uuid;

use pumpkin_data::{
    Block,
    damage::DamageType,
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    tag::{self, Taggable},
    tracked_data,
    world::WorldEvent,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::{
    Difficulty,
    math::{position::BlockPos, vector3::Vector3},
    text::TextComponent,
};
use pumpkin_world::world::BlockFlags;

use crate::{
    entity::{
        Entity, EntityBase,
        ai::goal::{
            look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal,
            revenge::RevengeGoal,
        },
        mob::{Mob, MobEntity},
        projectile::wither_skull::WitherSkullEntity,
    },
    world::{
        ExplosionInteraction,
        bossbar::{Bossbar, BossbarColor, BossbarDivisions, BossbarFlags},
    },
};

const INVULNERABLE_TICKS: i32 = 220;

pub struct WitherEntity {
    pub mob_entity: MobEntity,
    pub invulnerable_ticks: AtomicI32,
    pub destroy_blocks_tick: AtomicI32,
    pub next_head_update: [AtomicI32; 2],
    pub idle_head_updates: [AtomicI32; 2],
    pub alternative_targets: [AtomicI32; 3],
    pub bossbar_uuid: Uuid,
    pub bossbar_players: Mutex<Vec<Uuid>>,
    pub dropped_loot: AtomicBool,
}

impl WitherEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let wither = Self {
            mob_entity,
            invulnerable_ticks: AtomicI32::new(0),
            destroy_blocks_tick: AtomicI32::new(0),
            next_head_update: [AtomicI32::new(0), AtomicI32::new(0)],
            idle_head_updates: [AtomicI32::new(0), AtomicI32::new(0)],
            alternative_targets: [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)],
            bossbar_uuid: Uuid::new_v4(),
            bossbar_players: Mutex::new(Vec::new()),
            dropped_loot: AtomicBool::new(false),
        };
        // Vanilla `WitherBoss`: FlyingMoveControl(10, false) and flying navigation that
        // doesn't open doors but floats.
        *wither
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(FlyingMoveControl::new(10, false));
        let mut navigator = Navigator::flying();
        navigator.set_can_open_doors(false);
        navigator.set_can_float(true);
        *wither
            .mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = navigator;
        let mob_arc = Arc::new(wither);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(0, Box::new(WitherDoNothingGoal));
            let ranged: Weak<dyn crate::entity::mob::RangedAttackMob> = {
                let ranged: Arc<dyn crate::entity::mob::RangedAttackMob> = mob_arc.clone();
                Arc::downgrade(&ranged)
            };
            goal_selector.add_goal(
                2,
                Box::new(
                    crate::entity::ai::goal::ranged_attack::RangedAttackGoal::new(
                        ranged, 1.0, 40, 20.0,
                    ),
                ),
            );
            goal_selector.add_goal(5, Box::new(WaterAvoidingRandomFlyingGoal::new(1.0)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(7, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(1, Box::new(RevengeGoal::new(true)));
            target_selector.add_goal(
                2,
                crate::entity::ai::goal::active_target::ActiveTargetGoal::predicated(
                    &mob_arc.mob_entity,
                    0,
                    false,
                    living_entity_selector,
                ),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn get_invulnerable_ticks(&self) -> i32 {
        self.invulnerable_ticks.load(Ordering::Relaxed)
    }

    pub fn set_invulnerable_ticks(&self, ticks: i32) {
        self.invulnerable_ticks.store(ticks, Ordering::Relaxed);
        self.mob_entity
            .living_entity
            .entity
            .set_synced_data(tracked_data::wither::DATA_ID_INV, VarInt(ticks));
    }

    #[must_use]
    pub fn get_alternative_target(&self, head: usize) -> i32 {
        if head < 3 {
            self.alternative_targets[head].load(Ordering::Relaxed)
        } else {
            0
        }
    }

    pub fn set_alternative_target(&self, head: usize, entity_id: i32) {
        if head < 3 {
            let old = self.alternative_targets[head].swap(entity_id, Ordering::Relaxed);
            if old != entity_id {
                let tracker_id = match head {
                    0 => tracked_data::wither::DATA_TARGET_A,
                    1 => tracked_data::wither::DATA_TARGET_B,
                    _ => tracked_data::wither::DATA_TARGET_C,
                };
                self.mob_entity
                    .living_entity
                    .entity
                    .set_synced_data(tracker_id, VarInt(entity_id));
            }
        }
    }

    #[must_use]
    pub fn is_powered(&self) -> bool {
        let living = &self.mob_entity.living_entity;
        living.health.load() <= living.get_max_health() / 2.0
    }

    pub fn make_invulnerable(&self) {
        self.set_invulnerable_ticks(INVULNERABLE_TICKS);
        self.mob_entity
            .living_entity
            .set_health(self.mob_entity.living_entity.get_max_health() / 3.0);
    }

    /// Vanilla `WitherBoss.canDestroy`.
    #[must_use]
    pub fn can_destroy(block: &Block) -> bool {
        !block.default_state.is_air() && !block.has_tag(&tag::Block::MINECRAFT_WITHER_IMMUNE)
    }

    fn scale(&self) -> f32 {
        self.mob_entity
            .living_entity
            .get_attribute_value(&pumpkin_data::attributes::Attributes::SCALE) as f32
    }

    /// The side heads' angle: vanilla `(yBodyRot + 180 * (index - 1)) * (PI / 180)`.
    fn head_angle(&self, head: usize) -> f32 {
        (self.mob_entity.living_entity.entity.body_yaw.load() + 180.0 * (head as f32 - 1.0))
            * (std::f32::consts::PI / 180.0)
    }

    /// Vanilla `WitherBoss.getHeadX`.
    #[must_use]
    pub fn get_head_x(&self, head: usize) -> f64 {
        let x = self.mob_entity.living_entity.entity.pos.load().x;
        if head == 0 {
            return x;
        }
        x + f64::from(pumpkin_util::math::cos(self.head_angle(head)))
            * 1.3
            * f64::from(self.scale())
    }

    /// Vanilla `WitherBoss.getHeadY`.
    #[must_use]
    pub fn get_head_y(&self, head: usize) -> f64 {
        let y = self.mob_entity.living_entity.entity.pos.load().y;
        let head_height = f64::from(self.scale());
        if head == 0 {
            y + 3.0 * head_height
        } else {
            y + 2.2 * head_height
        }
    }

    /// Vanilla `WitherBoss.getHeadZ`.
    #[must_use]
    pub fn get_head_z(&self, head: usize) -> f64 {
        let z = self.mob_entity.living_entity.entity.pos.load().z;
        if head == 0 {
            return z;
        }
        z + f64::from(pumpkin_util::math::sin(self.head_angle(head)))
            * 1.3
            * f64::from(self.scale())
    }

    /// Vanilla `performRangedAttack(head, target)`: at half the target's eye height, the main
    /// head's skull rarely blue.
    fn perform_ranged_attack_at(&self, head: usize, target: &dyn EntityBase) {
        let entity = target.get_entity();
        let pos = entity.pos.load();
        let dangerous = head == 0 && rand::random::<f32>() < 0.001;
        self.perform_ranged_attack(
            head,
            pos.x,
            pos.y + entity.get_eye_height() * 0.5,
            pos.z,
            dangerous,
        );
    }

    /// Vanilla `performRangedAttack(head, x, y, z, dangerous)`.
    pub fn perform_ranged_attack(
        &self,
        head: usize,
        target_x: f64,
        target_y: f64,
        target_z: f64,
        dangerous: bool,
    ) {
        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load();

        if !entity.silent.load(Ordering::Relaxed) {
            world.sync_world_event(WorldEvent::SoundWitherBossShoot, entity.block_pos.load(), 0);
        }

        let hx = self.get_head_x(head);
        let hy = self.get_head_y(head);
        let hz = self.get_head_z(head);
        let direction = Vector3::new(target_x - hx, target_y - hy, target_z - hz).normalize();

        let skull_entity = Entity::from_uuid(
            Uuid::new_v4(),
            (*world).clone(),
            Vector3::new(hx, hy, hz),
            &EntityType::WITHER_SKULL,
        );
        let skull = Arc::new(WitherSkullEntity::new_shot(
            skull_entity,
            entity,
            dangerous,
            direction,
        ));
        world.spawn_entity(skull);
    }

    fn make_bossbar(&self) -> Bossbar {
        let title = self
            .mob_entity
            .living_entity
            .entity
            .custom_name
            .load()
            .as_ref()
            .clone()
            .unwrap_or_else(|| {
                TextComponent::translate_cross(
                    "entity.minecraft.wither",
                    "entity.minecraft.wither",
                    [],
                )
            });

        Bossbar {
            uuid: self.bossbar_uuid,
            title,
            health: 1.0,
            color: BossbarColor::Purple,
            division: BossbarDivisions::NoDivision,
            flags: BossbarFlags::DARKEN_SKY,
        }
    }

    fn update_bossbar(&self, world: &Arc<crate::world::World>, progress: f32) {
        let pos = self.mob_entity.living_entity.entity.pos.load();
        let tracking_radius_sq = 50.0 * 50.0;
        let players = world.players.load();

        let current: Vec<Uuid> = players
            .iter()
            .filter(|p| {
                let p_pos = p.living_entity.entity.pos.load();
                (p_pos - pos).length_squared() < tracking_radius_sq
            })
            .map(|p| p.gameprofile.id)
            .collect();

        let mut bossbar_players = self
            .bossbar_players
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        for &uid in &current {
            if !bossbar_players.contains(&uid) {
                if let Some(p) = players.iter().find(|p| p.gameprofile.id == uid) {
                    let mut bar = self.make_bossbar();
                    bar.health = progress;
                    p.send_bossbar(&bar);
                }
                bossbar_players.push(uid);
            }
        }

        let to_remove: Vec<Uuid> = bossbar_players
            .iter()
            .filter(|uid| !current.contains(uid))
            .copied()
            .collect();

        for uid in &to_remove {
            if let Some(p) = players.iter().find(|p| &p.gameprofile.id == uid) {
                p.remove_bossbar(self.bossbar_uuid);
            }
            bossbar_players.retain(|u| u != uid);
        }

        for player in players.iter() {
            if bossbar_players.contains(&player.gameprofile.id) {
                player.update_bossbar_health(&self.bossbar_uuid, progress);
            }
        }
    }

    fn remove_all_bossbar(&self, world: &Arc<crate::world::World>) {
        let mut bossbar_players = self
            .bossbar_players
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let players = world.players.load();
        for player in players.iter() {
            if bossbar_players.contains(&player.gameprofile.id) {
                player.remove_bossbar(self.bossbar_uuid);
            }
        }
        bossbar_players.clear();
    }

    /// The start of vanilla `WitherBoss.aiStep`: fly towards the main target.
    fn steer(&self) {
        let entity = &self.mob_entity.living_entity.entity;
        let mut delta_movement = entity.velocity.load().multiply(1.0, 0.6, 1.0);
        let main_target = self.get_alternative_target(0);
        if main_target > 0
            && let Some(target) = entity.world.load().get_entity_by_id(main_target)
        {
            let target_pos = target.get_entity().pos.load();
            let pos = entity.pos.load();
            let mut yd = delta_movement.y;
            if pos.y < target_pos.y || (!self.is_powered() && pos.y < target_pos.y + 5.0) {
                yd = yd.max(0.0);
                yd += 0.3 - yd * f64::from(0.6f32);
            }
            delta_movement.y = yd;
            let delta = Vector3::new(target_pos.x - pos.x, 0.0, target_pos.z - pos.z);
            if delta.horizontal_length_squared() > 9.0 {
                let scale = delta.normalize();
                delta_movement.x += scale.x * 0.3 - delta_movement.x * 0.6;
                delta_movement.z += scale.z * 0.3 - delta_movement.z * 0.6;
            }
        }
        entity.velocity.store(delta_movement);
        if delta_movement.horizontal_length_squared() > 0.05 {
            let yaw = pumpkin_util::math::atan2(delta_movement.z, delta_movement.x) as f32
                * (180.0 / std::f32::consts::PI)
                - 90.0;
            entity.yaw.store(yaw);
        }
    }

    /// Vanilla `WitherBoss.customServerAiStep`.
    #[expect(clippy::too_many_lines)]
    fn server_ai_step(&self) {
        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load_full();
        let living = &self.mob_entity.living_entity;
        let tick_count = entity.tick_count.load(Ordering::Relaxed);

        let invulnerable = self.get_invulnerable_ticks();
        if invulnerable > 0 {
            let new_count = invulnerable - 1;
            self.update_bossbar(&world, 1.0 - new_count as f32 / 220.0);
            if new_count <= 0 {
                let pos = entity.pos.load();
                world.explode(
                    world.get_entity_by_id(entity.entity_id),
                    Vector3::new(pos.x, entity.get_eye_y(), pos.z),
                    7.0,
                    ExplosionInteraction::Mob,
                );
                if !entity.silent.load(Ordering::Relaxed) {
                    world.sync_global_world_event(
                        WorldEvent::SoundWitherBossSpawn,
                        entity.block_pos.load(),
                        0,
                    );
                }
            }
            self.set_invulnerable_ticks(new_count);
            if tick_count % 10 == 0 {
                living.heal(10.0);
            }
            return;
        }

        let difficulty = world.level_info.load().difficulty;
        for i in 1..3 {
            if tick_count < self.next_head_update[i - 1].load(Ordering::Relaxed) {
                continue;
            }
            self.next_head_update[i - 1].store(
                tick_count + 10 + rand::random_range(0..10),
                Ordering::Relaxed,
            );
            if matches!(difficulty, Difficulty::Normal | Difficulty::Hard)
                && self.idle_head_updates[i - 1].fetch_add(1, Ordering::Relaxed) > 15
            {
                let pos = entity.pos.load();
                // `Mth.nextDouble(random, min, max)`.
                let xt = pos.x - 10.0 + rand::random::<f64>() * 20.0;
                let yt = pos.y - 5.0 + rand::random::<f64>() * 10.0;
                let zt = pos.z - 10.0 + rand::random::<f64>() * 20.0;
                self.perform_ranged_attack(i + 1, xt, yt, zt, true);
                self.idle_head_updates[i - 1].store(0, Ordering::Relaxed);
            }

            let head_target = self.get_alternative_target(i);
            if head_target > 0 {
                let current = world
                    .get_entity_by_id(head_target)
                    .filter(|current| current.get_living_entity().is_some());
                if let Some(current) = current
                    && self.can_attack(current.as_ref())
                    && entity
                        .pos
                        .load()
                        .squared_distance_to_vec(&current.get_entity().pos.load())
                        <= 900.0
                    && self.has_line_of_sight(current.get_entity())
                {
                    self.perform_ranged_attack_at(i + 1, current.as_ref());
                    self.next_head_update[i - 1].store(
                        tick_count + 40 + rand::random_range(0..20),
                        Ordering::Relaxed,
                    );
                    self.idle_head_updates[i - 1].store(0, Ordering::Relaxed);
                } else {
                    self.set_alternative_target(i, 0);
                }
            } else {
                // `getNearbyEntities(LivingEntity.class, TARGETING_CONDITIONS, this, box)`.
                let mut conditions =
                    crate::entity::ai::target_predicate::TargetPredicate::create_attackable()
                        .set_base_max_distance(20.0);
                conditions.set_predicate(living_entity_selector);
                let area = entity.bounding_box.load().expand(20.0, 8.0, 20.0);
                let candidates: Vec<_> = world
                    .get_all_at_box(&area)
                    .into_iter()
                    .filter(|candidate| candidate.get_living_entity().is_some())
                    .filter(|candidate| {
                        conditions.test(&world, Some(self as &dyn EntityBase), candidate.as_ref())
                    })
                    .collect();
                if !candidates.is_empty() {
                    let selected = &candidates[rand::random_range(0..candidates.len())];
                    self.set_alternative_target(i, selected.get_entity().entity_id);
                }
            }
        }

        self.set_alternative_target(
            0,
            self.mob_entity
                .get_target()
                .map_or(0, |target| target.get_entity().entity_id),
        );

        let destroy_tick = self.destroy_blocks_tick.load(Ordering::Relaxed);
        if destroy_tick > 0 {
            self.destroy_blocks_tick
                .store(destroy_tick - 1, Ordering::Relaxed);
            if destroy_tick - 1 == 0 && world.level_info.load().game_rules.mob_griefing {
                let dims = entity.entity_dimension.load();
                let width = (dims.width / 2.0 + 1.0).floor() as i32;
                let height = dims.height.floor() as i32;
                let origin = entity.block_pos.load();
                let mut destroyed = false;
                // `BlockPos.betweenClosed`: x fastest, then y, then z.
                for z in origin.0.z - width..=origin.0.z + width {
                    for y in origin.0.y..=origin.0.y + height {
                        for x in origin.0.x - width..=origin.0.x + width {
                            let pos = BlockPos::new(x, y, z);
                            if Self::can_destroy(world.get_block(&pos)) {
                                // `destroyBlock(pos, true, this)`.
                                destroyed |= world
                                    .break_block(&pos, None, BlockFlags::NOTIFY_ALL)
                                    .is_some();
                            }
                        }
                    }
                }
                if destroyed {
                    world.sync_world_event(
                        WorldEvent::SoundWitherBlockBreak,
                        entity.block_pos.load(),
                        0,
                    );
                }
            }
        }

        if tick_count % 20 == 0 {
            living.heal(1.0);
        }
        let max_health = living.get_max_health();
        self.update_bossbar(&world, living.health.load() / max_health);
    }
}

/// Vanilla `WitherBoss.LIVING_ENTITY_SELECTOR`.
fn living_entity_selector(
    target: &crate::entity::living::LivingEntity,
    _world: &crate::world::World,
) -> bool {
    !target
        .entity
        .entity_type
        .has_tag(&tag::EntityType::MINECRAFT_WITHER_FRIENDS)
        && target.entity.entity_type.attackable.unwrap_or(true)
}

impl Mob for WitherEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_init_data_tracker(&self) {
        let entity = &self.mob_entity.living_entity.entity;
        entity.set_synced_data(
            tracked_data::wither::DATA_TARGET_A,
            VarInt(self.get_alternative_target(0)),
        );
        entity.set_synced_data(
            tracked_data::wither::DATA_TARGET_B,
            VarInt(self.get_alternative_target(1)),
        );
        entity.set_synced_data(
            tracked_data::wither::DATA_TARGET_C,
            VarInt(self.get_alternative_target(2)),
        );
        entity.set_synced_data(
            tracked_data::wither::DATA_ID_INV,
            VarInt(self.get_invulnerable_ticks()),
        );
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load();
        if world.level_info.load().difficulty == Difficulty::Peaceful {
            self.remove_all_bossbar(&world);
            entity.remove();
        }
    }

    fn ai_step(&self, _caller: &dyn EntityBase) {
        self.steer();
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if self.mob_entity.living_entity.entity.is_alive() {
            self.server_ai_step();
        }
    }

    fn pre_damage(&self, damage_type: DamageType, source: Option<&dyn EntityBase>) -> bool {
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_WITHER_IMMUNE_TO) {
            return false;
        }

        if let Some(src) = source {
            let src_type = src.get_entity().entity_type;
            if src_type == &EntityType::WITHER {
                return false;
            }
            if src_type.has_tag(&tag::EntityType::MINECRAFT_WITHER_FRIENDS) {
                return false;
            }
            if self.is_powered()
                && (src_type == &EntityType::ARROW
                    || src_type == &EntityType::SPECTRAL_ARROW
                    || src_type == &EntityType::WIND_CHARGE
                    || src_type == &EntityType::BREEZE_WIND_CHARGE)
            {
                return false;
            }
        }

        if self.get_invulnerable_ticks() > 0
            && !damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_INVULNERABILITY)
        {
            return false;
        }

        true
    }

    fn on_damage(&self, _damage_type: DamageType, _source: Option<&dyn EntityBase>) {
        if self.destroy_blocks_tick.load(Ordering::Relaxed) <= 0 {
            self.destroy_blocks_tick.store(20, Ordering::Relaxed);
        }

        for idle in &self.idle_head_updates {
            idle.fetch_add(3, Ordering::Relaxed);
        }
    }

    fn post_tick(&self) {
        let entity = &self.mob_entity.living_entity.entity;
        if !entity.is_alive() || self.mob_entity.living_entity.health.load() <= 0.0 {
            let world = entity.world.load_full();
            self.remove_all_bossbar(&world);
            if !self.dropped_loot.swap(true, Ordering::SeqCst) {
                entity.spawn_at_location(ItemStack::new(1, &Item::NETHER_STAR));
            }
        }
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int("Invul", self.get_invulnerable_ticks());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if let Some(invul) = nbt.get_int("Invul") {
            self.set_invulnerable_ticks(invul);
        }
    }
}

impl crate::entity::mob::RangedAttackMob for WitherEntity {
    /// Vanilla `WitherBoss.performRangedAttack(target, power)`: the main head fires.
    fn perform_ranged_attack(&self, target: &Arc<dyn EntityBase>, _power: f32) {
        self.perform_ranged_attack_at(0, target.as_ref());
    }
}

/// Vanilla `WitherBoss.WitherDoNothingGoal`: holds still while the wither charges up.
struct WitherDoNothingGoal;

impl crate::entity::ai::goal::Goal for WitherDoNothingGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.cast_any()
            .downcast_ref::<WitherEntity>()
            .is_some_and(|wither| wither.get_invulnerable_ticks() > 0)
    }

    fn controls(&self) -> crate::entity::ai::goal::Controls {
        crate::entity::ai::goal::Controls::MOVE
            | crate::entity::ai::goal::Controls::JUMP
            | crate::entity::ai::goal::Controls::LOOK
    }
}
