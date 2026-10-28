use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::enchantment::EnchantmentHelper;
use crate::entity::damage_source::DamageSource;
use crate::{
    entity::{
        Entity, EntityBase, boss::ender_dragon::projectile_targets, living::LivingEntity,
        player::Player,
    },
    server::Server,
};
use pumpkin_data::damage::DamageType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_protocol::IdOr;
use pumpkin_protocol::java::client::play::{CEntityVelocity, CSoundEffect};
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use super::arrow::ArrowPickup;
use super::{ProjectileHit, calculate_ray_intersection};

pub struct TridentEntity {
    pub entity: Entity,
    pub projectile: super::Projectile,
    pub item_stack: Arc<Mutex<ItemStack>>,
    pub pickup: ArrowPickup,
    pub in_ground: AtomicBool,
    pub in_ground_time: AtomicU32,
    pub life: AtomicU32,
    pub shake_time: AtomicU8,
    pub has_hit: AtomicBool,
    /// Vanilla `Projectile.lastDeflectedBy`.
    pub last_deflected_by: crossbeam::atomic::AtomicCell<Option<i32>>,
    /// Vanilla `ThrownTrident.dealtDamage`: once it has hurt something, it hits no more entities.
    pub dealt_damage: AtomicBool,
    pub last_block_pos: Arc<std::sync::RwLock<Option<BlockPos>>>,
}

impl TridentEntity {
    const BASE_DAMAGE: f32 = 8.0;
    const AIR_INERTIA: f64 = 0.99;
    const WATER_INERTIA: f64 = 0.9;
    const GRAVITY: f64 = 0.05;
    const DESPAWN_TIME: u32 = 1200;

    /// A trident without an owner.
    pub fn new(entity: Entity) -> Self {
        Self {
            entity,
            projectile: super::Projectile::new(),
            item_stack: Arc::new(Mutex::new(ItemStack::new(1, &Item::TRIDENT))),
            pickup: ArrowPickup::Disallowed,
            in_ground: AtomicBool::new(false),
            in_ground_time: AtomicU32::new(0),
            life: AtomicU32::new(0),
            shake_time: AtomicU8::new(0),
            has_hit: AtomicBool::new(false),
            last_deflected_by: crossbeam::atomic::AtomicCell::new(None),
            dealt_damage: AtomicBool::new(false),
            last_block_pos: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    pub fn new_shot(
        entity: Entity,
        shooter: &Entity,
        item_stack: ItemStack,
        pickup: ArrowPickup,
    ) -> Self {
        let mut owner_pos = shooter.pos.load();
        // Vanilla `AbstractArrow`'s owner constructor: `getEyeY() - 0.1F`.
        owner_pos.y =
            owner_pos.y + f64::from(shooter.entity_dimension.load().eye_height) - f64::from(0.1f32);
        entity.pos.store(owner_pos);
        entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));

        Self {
            entity,
            projectile: super::Projectile::owned_by(shooter),
            item_stack: Arc::new(Mutex::new(item_stack)),
            pickup,
            in_ground: AtomicBool::new(false),
            in_ground_time: AtomicU32::new(0),
            life: AtomicU32::new(0),
            shake_time: AtomicU8::new(0),
            has_hit: AtomicBool::new(false),
            last_deflected_by: crossbeam::atomic::AtomicCell::new(None),
            dealt_damage: AtomicBool::new(false),
            last_block_pos: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// Applies projectile-spawned enchantment effects matching vanilla `Projectile::applyOnProjectileSpawned`.
    pub fn apply_on_projectile_spawned(&self, pickup_item_stack: &ItemStack) {
        super::apply_on_projectile_spawned(self.get_entity(), pickup_item_stack, None, None);
    }

    /// Vanilla `Projectile.shootFromRotation`.
    pub fn set_velocity_from_rotation(
        &self,
        source: &dyn EntityBase,
        pitch: f32,
        yaw: f32,
        roll: f32,
        speed: f32,
        divergence: f32,
    ) {
        super::shoot_from_rotation(&self.entity, source, pitch, yaw, roll, speed, divergence);
    }

    /// Vanilla `Projectile.shoot`.
    pub fn set_velocity(&self, x: f64, y: f64, z: f64, power: f64, uncertainty: f64) {
        super::shoot(&self.entity, x, y, z, power, uncertainty);
    }

    fn should_skip_collision(&self, self_ent: &Entity, other: &Arc<dyn EntityBase>) -> bool {
        let other_ent = other.get_entity();

        // Don't collide with self
        if other_ent.entity_id == self_ent.entity_id {
            return true;
        }

        // Vanilla `Projectile.canHitEntity`: not the owner's stack until it has left it.
        if !self.projectile.can_hit_owner_stack(self_ent, other_ent) {
            return true;
        }

        // Vanilla `canHitEntity`'s `canBeHitByProjectile`: alive and pickable, which leaves out
        // spectators, clouds, the dragon's body, dying mobs and most projectiles.
        if !other.can_be_hit_by_projectile() {
            return true;
        }

        false
    }
}

impl EntityBase for TridentEntity {
    /// Vanilla `AbstractArrow.isAttackable`: only a `redirectable_projectile`, which no arrow
    /// type is.
    fn is_attackable(&self) -> bool {
        pumpkin_data::tag::Taggable::has_tag(
            self.get_entity().entity_type,
            &pumpkin_data::tag::EntityType::MINECRAFT_REDIRECTABLE_PROJECTILE,
        )
    }

    /// Vanilla `ThrownTrident.getWeaponItem`: its own item.
    fn get_weapon_item(&self) -> Option<ItemStack> {
        Some(
            self.item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        )
    }

    fn get_projectile(&self) -> Option<&super::Projectile> {
        Some(&self.projectile)
    }

    #[expect(clippy::too_many_lines)]
    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        let entity = self.get_entity();
        let world = entity.world.load();

        // Handle shake time
        let shake = self.shake_time.load(Ordering::Relaxed);
        if shake > 0 {
            self.shake_time.store(shake - 1, Ordering::Relaxed);
        }

        if self.in_ground.load(Ordering::Relaxed) {
            let _in_ground_time = self.in_ground_time.fetch_add(1, Ordering::Relaxed);
            let life = self.life.fetch_add(1, Ordering::Relaxed);

            // Despawn after enough time
            if life >= Self::DESPAWN_TIME {
                entity.remove();
            }
            return;
        }

        // Trident is flying
        let start_pos = entity.pos.load();
        let velocity = entity.velocity.load();

        // Vanilla `AbstractArrow.tick`: move by the current velocity first, then apply drag,
        // then gravity.
        let new_pos = start_pos.add(&velocity);
        entity.set_pos(new_pos);

        // Update rotation based on pre-gravity velocity
        let len = velocity.horizontal_length();
        entity.set_rotation(
            velocity.x.atan2(velocity.z) as f32 * 57.295_776,
            velocity.y.atan2(len) as f32 * 57.295_776,
        );
        // Vanilla `AbstractArrow.tick`'s `checkLeftOwner`, with `Projectile.tick`'s shoot event.
        self.projectile.tick(entity);

        // Broadcast velocity update (before drag/gravity — matches vanilla ThrownTrident which
        // calls super.tick() = AbstractArrow.tick; the velocity packet uses the pre-gravity
        // movement since that's what the move was based on).
        let packet = CEntityVelocity::new(entity.entity_id.into(), velocity);
        let chunk_pos = entity.chunk_pos.load();
        world.broadcast_to_chunk(chunk_pos, &packet);

        // Check for collisions using raycasting (pre-gravity velocity)
        let search_box = BoundingBox::new(
            Vector3::new(
                start_pos.x.min(new_pos.x),
                start_pos.y.min(new_pos.y),
                start_pos.z.min(new_pos.z),
            ),
            Vector3::new(
                start_pos.x.max(new_pos.x),
                start_pos.y.max(new_pos.y),
                start_pos.z.max(new_pos.z),
            ),
        )
        .expand(0.3, 0.3, 0.3);

        let mut closest_t = 1.0f64;
        let mut hit = None;

        // Block collisions
        let (block_cols, block_positions) =
            world.get_block_collisions(search_box, self.get_entity());
        for (idx, bb) in block_cols.iter().enumerate() {
            if let Some(t) = calculate_ray_intersection(&start_pos, &velocity, bb)
                && t < closest_t
            {
                closest_t = t;

                // Map back to block pos
                let mut curr = 0;
                for (len, pos) in &block_positions {
                    curr += len;
                    if idx < curr {
                        let hit_pos = start_pos.add(&velocity.multiply(t, t, t));
                        hit = Some(ProjectileHit::Block {
                            pos: *pos,
                            face: get_hit_face(hit_pos, *pos),
                            hit_pos,
                            normal: velocity.normalize().multiply(-1.0, -1.0, -1.0),
                        });
                        break;
                    }
                }
            }
        }

        // Entity collisions
        let candidates = if self.dealt_damage.load(Ordering::Relaxed) {
            Vec::new()
        } else {
            projectile_targets(&world, &search_box, world.get_entities_at_box(&search_box))
        };
        for cand in candidates {
            if self.should_skip_collision(entity, &cand) {
                continue;
            }

            let ebb = cand.get_entity().bounding_box.load().expand(0.3, 0.3, 0.3);
            if let Some(t) = calculate_ray_intersection(&start_pos, &velocity, &ebb)
                && t < closest_t
            {
                closest_t = t;
                let hit_pos = start_pos.add(&velocity.multiply(t, t, t));
                hit = Some(ProjectileHit::Entity {
                    entity: cand.clone(),
                    hit_pos,
                    normal: velocity.normalize().multiply(-1.0, -1.0, -1.0),
                });
            }
        }

        // Handle hit
        if let Some(h) = hit
            && !super::is_deflected(caller, &self.last_deflected_by, &h)
            && !self.has_hit.swap(true, Ordering::SeqCst)
        {
            super::on_hit(caller, h);
        }

        // Apply drag and gravity after move and hit handling, matching vanilla order.
        let mut velocity = velocity;
        let inertia = if entity.touching_water.load(Ordering::Relaxed) {
            Self::WATER_INERTIA
        } else {
            Self::AIR_INERTIA
        };
        velocity = velocity.multiply(inertia, inertia, inertia);
        velocity.y -= Self::GRAVITY;
        entity.velocity.store(velocity);

        // `Entity.tick` → `baseTick` → `checkBelowWorld`: discard if below world.
        entity.check_out_of_world(caller);
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        let entity = self.get_entity();
        let world = entity.world.load();

        match hit {
            ProjectileHit::Block { pos, hit_pos, .. } => {
                self.in_ground.store(true, Ordering::Relaxed);
                self.shake_time.store(7, Ordering::Relaxed);
                *self
                    .last_block_pos
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pos);

                let block = world.get_block(&pos);
                let state = world.get_block_state(&pos);
                if let Some(server) = world.server.upgrade() {
                    world
                        .block_registry
                        .on_projectile_hit(block, &world, self, &pos, state, &hit_pos, &server);
                }

                // Stop the trident
                entity.velocity.store(Vector3::new(0.0, 0.0, 0.0));
                entity.set_pos(hit_pos);

                // Play sound
                let sound_packet = CSoundEffect::new(
                    IdOr::Id(Sound::ItemTridentHitGround as u16),
                    SoundCategory::Neutral,
                    &hit_pos,
                    1.0,
                    1.0,
                    0,
                );
                let chunk_pos = entity.chunk_pos.load();
                world.broadcast_to_chunk(chunk_pos, &sound_packet);
            }
            ProjectileHit::Entity {
                entity: target,
                hit_pos,
                ..
            } => {
                let owner = self.projectile.get_owner(&self.entity);
                // Vanilla `trident(this, owner)`, the trident standing in for a missing owner.
                let damage_source = DamageSource {
                    damage_type: DamageType::TRIDENT,
                    direct: Some(self),
                    causing: owner.as_deref().or(Some(self)),
                };
                // The trident's enchantments (Impaling) against this target.
                let weapon = self
                    .item_stack
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                let damage_val = EnchantmentHelper::modify_damage(
                    &world,
                    &weapon,
                    target.as_ref(),
                    &damage_source,
                    Self::BASE_DAMAGE,
                );
                self.dealt_damage.store(true, Ordering::Relaxed);
                if target.damage_with_context(
                    target.as_ref(),
                    damage_val,
                    damage_source.damage_type,
                    Some(hit_pos),
                    damage_source.direct,
                    damage_source.causing,
                ) {
                    let kill = || self.kill(self);
                    EnchantmentHelper::do_post_attack_effects_with_item_source_on_break(
                        &world,
                        target.as_ref(),
                        &damage_source,
                        Some((&weapon, Some(&self.item_stack))),
                        Some(&kill),
                    );
                }

                // Play hit sound
                let sound_packet = CSoundEffect::new(
                    IdOr::Id(Sound::ItemTridentHit as u16),
                    SoundCategory::Neutral,
                    &hit_pos,
                    1.0,
                    1.0,
                    0,
                );
                let chunk_pos = entity.chunk_pos.load();
                world.broadcast_to_chunk(chunk_pos, &sound_packet);

                // Standard bounce/fall-back behavior
                entity.velocity.store(Vector3::new(0.0, -0.1, 0.0));
                self.has_hit.store(false, Ordering::Relaxed); // Let it hit the ground
            }
        }
    }

    fn on_player_collision(&self, player: &Arc<Player>) {
        // Can only pick up when on the ground
        if !self.in_ground.load(Ordering::Relaxed) {
            return;
        }

        if player.living_entity.health.load() <= 0.0 {
            return;
        }

        match self.pickup {
            ArrowPickup::Disallowed => return,
            ArrowPickup::CreativeOnly if !player.is_creative() => return,
            _ => {}
        }

        let mut stack = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        // Pumpkin's pickup event, before the inventory changes, when the arrow could go in.
        if !player.is_creative() && !player.inventory.has_room_for(&stack)
            || !player.living_entity.fire_pickup_event(&self.entity, 1)
        {
            return;
        }
        if player.is_creative() || player.inventory.insert_stack_anywhere(&mut stack) {
            player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::PickedUp,
                stack.item.id as i32,
                1,
            );
            player.take(&self.entity, 1);
            self.get_entity().remove();
        }
    }
}

/// Get the face of the block that was hit
fn get_hit_face(hit_pos: Vector3<f64>, block_pos: BlockPos) -> pumpkin_data::BlockDirection {
    let local = hit_pos.sub(&block_pos.0.to_f64());
    let eps = 1.0e-4;

    if local.x <= eps {
        pumpkin_data::BlockDirection::West
    } else if local.x >= 1.0 - eps {
        pumpkin_data::BlockDirection::East
    } else if local.y <= eps {
        pumpkin_data::BlockDirection::Down
    } else if local.y >= 1.0 - eps {
        pumpkin_data::BlockDirection::Up
    } else if local.z <= eps {
        pumpkin_data::BlockDirection::North
    } else {
        pumpkin_data::BlockDirection::South
    }
}
