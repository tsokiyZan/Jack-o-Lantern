use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::{
    entity::{Entity, EntityBase},
    server::Server,
};
use pumpkin_data::effect::StatusEffect;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::vector3::Vector3;

type EffectEntry = (&'static StatusEffect, i32, u8, bool, bool, bool);
use pumpkin_data::item_stack::ItemStack;

#[derive(Clone)]
struct ParticleMeta {
    particle_id: pumpkin_protocol::codec::var_int::VarInt,
    data: Box<[u8]>,
}

impl pumpkin_protocol::java::client::play::MetadataSerializer for ParticleMeta {
    fn write_metadata(
        &self,
        writer: &mut impl std::io::Write,
        _version: &pumpkin_util::version::JavaMinecraftVersion,
    ) -> Result<(), pumpkin_protocol::ser::WritingError> {
        use pumpkin_protocol::ser::NetworkWriteExt;
        writer.write_var_int(&self.particle_id)?;
        writer.write_slice(&self.data)
    }
}

/// The effect cloud entity that is spawned where a lingering potion lands.
pub struct AreaEffectCloudEntity {
    pub entity: Entity,
    /// Stored potion item stack (may be empty) to read effects from.
    pub item_stack: Mutex<ItemStack>,
    /// Active potion effects as tuples: (`StatusEffect`, `duration_ticks`, `amplifier`, `ambient`, `show_particles`, `show_icon`)
    pub effects: Mutex<Vec<EffectEntry>>,
    pub radius: Mutex<f32>,
    pub duration: Mutex<i32>,
    pub age: Mutex<i32>,
    /// ticks between reapplications to the same entity
    pub reapplication_delay: Mutex<i32>,
    /// map of `entity_id` -> ticks remaining until that entity can be affected again
    pub reapplication_map: Mutex<HashMap<i32, i32>>,
    /// linear radius change per tick
    pub radius_on_tick: Mutex<f32>,
    /// radius change when an entity is affected
    pub radius_on_use: Mutex<f32>,
    /// duration change (ticks) when an entity is affected
    pub duration_on_use: Mutex<i32>,
    /// ticks to wait before the cloud becomes active and applies effects (grace period)
    pub wait_time: Mutex<i32>,
    /// Vanilla `customParticle`, shown instead of the potion-coloured `entity_effect`.
    custom_particle: Mutex<Option<ParticleMeta>>,
}

impl AreaEffectCloudEntity {
    /// Vanilla `setRadius`, synced to clients.
    pub fn set_radius(&self, radius: f32) {
        *self
            .radius
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = radius;
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::area_effect_cloud::RADIUS,
            radius,
        );
    }

    #[allow(clippy::new_ret_no_self)]
    pub fn new(entity: Entity) -> Arc<dyn EntityBase> {
        entity
            .no_physics
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let cloud = Self {
            entity,
            item_stack: Mutex::new(ItemStack::new(0, &pumpkin_data::item::Item::GLASS_BOTTLE)),
            effects: Mutex::new(Vec::new()),
            radius: Mutex::new(3.0),
            duration: Mutex::new(600), // default for lingering potions
            age: Mutex::new(0),
            reapplication_delay: Mutex::new(20),
            reapplication_map: Mutex::new(HashMap::new()),
            radius_on_tick: Mutex::new(-3.0 / 600.0), // default for lingering potions
            radius_on_use: Mutex::new(0.0),
            duration_on_use: Mutex::new(0),
            wait_time: Mutex::new(20),
            custom_particle: Mutex::new(None),
        };

        Arc::new(cloud)
    }

    /// Vanilla `new AreaEffectCloud(level, x, y, z)`: no effects, a 3 block radius, and no
    /// duration until one is set.
    #[must_use]
    pub fn with_vanilla_defaults(entity: Entity) -> Self {
        entity
            .no_physics
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Self {
            entity,
            item_stack: Mutex::new(ItemStack::EMPTY.clone()),
            effects: Mutex::new(Vec::new()),
            radius: Mutex::new(3.0),
            duration: Mutex::new(-1),
            age: Mutex::new(0),
            reapplication_delay: Mutex::new(20),
            reapplication_map: Mutex::new(HashMap::new()),
            radius_on_tick: Mutex::new(0.0),
            radius_on_use: Mutex::new(0.0),
            duration_on_use: Mutex::new(0),
            wait_time: Mutex::new(20),
            custom_particle: Mutex::new(None),
        }
    }

    /// Vanilla `setCustomParticle` for a particle whose options are `data`.
    pub fn set_custom_particle(&self, particle: pumpkin_data::particle::Particle, data: Box<[u8]>) {
        *self
            .custom_particle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ParticleMeta {
            particle_id: pumpkin_protocol::codec::var_int::VarInt(particle as i32),
            data,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        entity: Entity,
        item_stack: ItemStack,
        effects_in: Vec<EffectEntry>,
        duration_in: i32,
        radius_in: f32,
        reapplication_delay_in: i32,
        wait_time_in: i32,
        radius_on_use_in: f32,
        duration_on_use_in: i32,
    ) -> Arc<dyn EntityBase> {
        entity
            .no_physics
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let cloud = Self {
            entity,
            item_stack: Mutex::new(item_stack),
            effects: Mutex::new(effects_in),
            radius: Mutex::new(radius_in),
            duration: Mutex::new(duration_in),
            age: Mutex::new(0),
            reapplication_delay: Mutex::new(reapplication_delay_in),
            reapplication_map: Mutex::new(HashMap::new()),
            radius_on_tick: Mutex::new(-radius_in / (duration_in as f32).max(1.0)),
            radius_on_use: Mutex::new(radius_on_use_in),
            duration_on_use: Mutex::new(duration_on_use_in),
            wait_time: Mutex::new(wait_time_in),
            custom_particle: Mutex::new(None),
        };

        Arc::new(cloud)
    }
}

impl EntityBase for AreaEffectCloudEntity {
    fn init_data_tracker(&self) {
        // Send initial radius and particle (color) so clients render correctly
        let radius = *self
            .radius
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Compute particle color
        let stack = self
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let effects = self
            .effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();

        // Use ARGB format
        let mut color: i32 = (0xFFi32 << 24) | 0x385dc6; // default water-like color

        if let Some(pc) =
            stack.get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
        {
            if let Some(c) = pc.custom_color {
                color = c | (0xFFi32 << 24);
            } else if !effects.is_empty() {
                let mut r_sum = 0.0f32;
                let mut g_sum = 0.0f32;
                let mut b_sum = 0.0f32;
                let count = effects.len() as f32;
                for (eff, _, _, _, _, _) in &effects {
                    let c = eff.color;
                    r_sum += ((c >> 16) & 0xFF) as f32;
                    g_sum += ((c >> 8) & 0xFF) as f32;
                    b_sum += (c & 0xFF) as f32;
                }
                let r = (r_sum / count) as i32;
                let g = (g_sum / count) as i32;
                let b = (b_sum / count) as i32;
                color = (0xFFi32 << 24) | (r << 16) | (g << 8) | b;
            }
        } else if !effects.is_empty() {
            let mut r_sum = 0.0f32;
            let mut g_sum = 0.0f32;
            let mut b_sum = 0.0f32;
            let count = effects.len() as f32;
            for (eff, _, _, _, _, _) in &effects {
                let c = eff.color;
                r_sum += ((c >> 16) & 0xFF) as f32;
                g_sum += ((c >> 8) & 0xFF) as f32;
                b_sum += (c & 0xFF) as f32;
            }
            let r = (r_sum / count) as i32;
            let g = (g_sum / count) as i32;
            let b = (b_sum / count) as i32;
            color = (0xFFi32 << 24) | (r << 16) | (g << 8) | b;
        }

        // Build raw particle option bytes for ENTITY_EFFECT
        let data_bytes = color.to_be_bytes();

        let meta = self
            .custom_particle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| ParticleMeta {
                particle_id: pumpkin_protocol::codec::var_int::VarInt(
                    pumpkin_data::particle::Particle::EntityEffect as i32,
                ),
                data: Box::new(data_bytes),
            });

        self.entity.set_synced_data(
            pumpkin_data::tracked_data::area_effect_cloud::PARTICLE,
            meta,
        );

        self.entity.set_synced_data(
            pumpkin_data::tracked_data::area_effect_cloud::RADIUS,
            radius,
        );

        // Initial waiting flag
        let wait_time = *self
            .wait_time
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let is_waiting = 0 < wait_time;
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::area_effect_cloud::WAITING,
            is_waiting,
        );
    }

    /// Vanilla `AreaEffectCloud.serverTick`.
    #[expect(clippy::too_many_lines)]
    fn tick(&self, _caller: &dyn EntityBase, _server: &Server) {
        use std::sync::PoisonError;
        let tick_count = self
            .entity
            .tick_count
            .load(std::sync::atomic::Ordering::Relaxed);
        let wait_time = *self
            .wait_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let duration = *self.duration.lock().unwrap_or_else(PoisonError::into_inner);
        if duration != -1 && tick_count - wait_time >= duration {
            self.entity.remove();
            return;
        }

        let should_wait = tick_count < wait_time;
        if !should_wait && tick_count == wait_time {
            self.entity.set_synced_data(
                pumpkin_data::tracked_data::area_effect_cloud::WAITING,
                false,
            );
        }
        if should_wait {
            return;
        }

        let mut radius = *self.radius.lock().unwrap_or_else(PoisonError::into_inner);
        let radius_per_tick = *self
            .radius_on_tick
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if radius_per_tick != 0.0 {
            radius += radius_per_tick;
            if radius < 0.5 {
                self.entity.remove();
                return;
            }
            self.set_radius(radius);
        }

        if tick_count % 5 != 0 {
            return;
        }
        // `victims`: entity id -> the tick it may be affected again.
        self.reapplication_map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, until| tick_count < *until);
        let effects = self
            .effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if effects.is_empty() {
            self.reapplication_map
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            return;
        }

        // `getEntitiesOfClass(LivingEntity.class, getBoundingBox())`: radius * 2 wide, 0.5 tall.
        let pos = self.entity.pos.load();
        let r = f64::from(radius);
        let area = BoundingBox::new(
            Vector3::new(pos.x - r, pos.y, pos.z - r),
            Vector3::new(pos.x + r, pos.y + 0.5, pos.z + r),
        );
        let world = self.entity.world.load();
        let reapplication_delay = *self
            .reapplication_delay
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for candidate in world.get_all_at_box(&area) {
            let Some(living) = candidate.get_living_entity() else {
                continue;
            };
            let id = candidate.get_entity().entity_id;
            if self
                .reapplication_map
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains_key(&id)
            {
                continue;
            }
            let entity_pos = candidate.get_entity().pos.load();
            let xd = entity_pos.x - pos.x;
            let zd = entity_pos.z - pos.z;
            if xd * xd + zd * zd > f64::from(radius * radius) {
                continue;
            }
            self.reapplication_map
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id, tick_count + reapplication_delay);
            // Every vanilla cloud source (lingering potions, creepers, dragon breath) sets a
            // `potion_duration_scale` of 0.25. Clouds don't keep an owner here.
            crate::item::potion::PotionContents::apply_effects_to(
                living,
                effects.clone(),
                0.25,
                crate::item::potion::PotionApplicationSource::AreaEffectCloud,
                Some(self),
                None,
            );

            let radius_on_use = *self
                .radius_on_use
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if radius_on_use != 0.0 {
                radius += radius_on_use;
                if radius < 0.5 {
                    self.entity.remove();
                    return;
                }
                self.set_radius(radius);
            }
            let duration_on_use = *self
                .duration_on_use
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let mut duration = self.duration.lock().unwrap_or_else(PoisonError::into_inner);
            if duration_on_use != 0 && *duration != -1 {
                *duration += duration_on_use;
                if *duration <= 0 {
                    drop(duration);
                    self.entity.remove();
                    return;
                }
            }
        }
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}
