use pumpkin_data::world::WorldEvent;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, PoisonError, RwLock};

use crate::{
    entity::{Entity, EntityBase, projectile::ThrownItemEntity},
    server::Server,
};
use pumpkin_data::damage::DamageType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::{Block, BlockId};
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;

const GRAVITY: f64 = 0.05;

pub struct SplashPotionEntity {
    pub thrown: ThrownItemEntity,
    pub item_stack: RwLock<ItemStack>,
}

impl SplashPotionEntity {
    pub fn new(entity: Entity) -> Self {
        entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        let thrown = ThrownItemEntity {
            entity,
            owner_id: crossbeam::atomic::AtomicCell::new(None),
            collides_with_projectiles: false,
            has_hit: AtomicBool::new(false),
            last_deflected_by: crossbeam::atomic::AtomicCell::new(None),
            gravity: GRAVITY,
            left_owner: std::sync::atomic::AtomicBool::new(false),
        };

        Self {
            thrown,
            item_stack: RwLock::new(ItemStack::new(1, &pumpkin_data::item::Item::SPLASH_POTION)),
        }
    }

    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        let thrown = ThrownItemEntity::new(entity, shooter, GRAVITY);
        thrown.entity.set_velocity(Vector3::new(0.0, 0.1, 0.0));
        Self {
            thrown,
            item_stack: RwLock::new(ItemStack::new(1, &pumpkin_data::item::Item::SPLASH_POTION)),
        }
    }

    pub fn set_item_stack(&self, item_stack: ItemStack) {
        let mut write = self
            .item_stack
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *write = item_stack;
    }
}

fn is_water_potion(stack: &ItemStack) -> bool {
    stack
        .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
        .and_then(|pc| pc.potion_id)
        == Some(pumpkin_data::potion::Potion::WATER.id as i32)
}

/// Extinguishes fire (including soul fire) at the hit position and its four horizontal neighbors.
fn extinguish_fire(world: &Arc<crate::world::World>, hit_pos: Vector3<f64>) {
    let air_state_id = Block::AIR.default_state.id;

    let neighbors = [
        hit_pos,
        Vector3::new(hit_pos.x + 1.0, hit_pos.y, hit_pos.z),
        Vector3::new(hit_pos.x - 1.0, hit_pos.y, hit_pos.z),
        Vector3::new(hit_pos.x, hit_pos.y, hit_pos.z + 1.0),
        Vector3::new(hit_pos.x, hit_pos.y, hit_pos.z - 1.0),
    ];

    for p in neighbors {
        let pos = BlockPos(Vector3::new(
            p.x.floor() as i32,
            p.y.floor() as i32,
            p.z.floor() as i32,
        ));
        let state_id = world.get_block_state_id(&pos);
        let raw_block_id = state_id.to_block_id();
        if raw_block_id == BlockId::FIRE || raw_block_id == BlockId::SOUL_FIRE {
            world.set_block_state(&pos, air_state_id, BlockFlags::NOTIFY_ALL);
        }
    }
}

pub(crate) fn extinguish_fire_if_water_potion(
    world: &Arc<crate::world::World>,
    hit_pos: Vector3<f64>,
    stack: &ItemStack,
) {
    if is_water_potion(stack) {
        extinguish_fire(world, hit_pos);
    }
}

/// Whether the potion in `stack` is in a potion `tag` (vanilla `PotionContents.is`).
fn potion_is(stack: &ItemStack, tag: &pumpkin_data::tag::Tag) -> bool {
    stack
        .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
        .and_then(|pc| pc.potion_id)
        .is_some_and(|id| tag.1.contains(&(id as u16)))
}

/// Whether `stack` hurts water-sensitive entities (vanilla `Enderman.hurtWithCleanWater`).
#[must_use]
pub fn hurts_water_sensitive_entities(stack: &ItemStack) -> bool {
    potion_is(
        stack,
        &pumpkin_data::tag::Potion::MINECRAFT_HURTS_WATER_SENSITIVE_ENTITIES,
    )
}

/// The item of a thrown splash or lingering potion (vanilla `AbstractThrownPotion.getItem`).
#[must_use]
pub fn thrown_potion_item(entity: &dyn EntityBase) -> Option<ItemStack> {
    let any = entity.cast_any();
    let lock = if let Some(splash) = any.downcast_ref::<SplashPotionEntity>() {
        &splash.item_stack
    } else {
        &any.downcast_ref::<super::lingering_potion::LingeringPotionEntity>()?
            .item_stack
    };
    Some(lock.read().unwrap_or_else(PoisonError::into_inner).clone())
}

/// Vanilla `AbstractThrownPotion.affectEntitiesAround`, run on every hit before the effects.
pub(crate) fn affect_entities_around(potion: &dyn EntityBase, stack: &ItemStack) {
    let hurts = hurts_water_sensitive_entities(stack);
    let extinguishes = potion_is(
        stack,
        &pumpkin_data::tag::Potion::MINECRAFT_EXTINGUISHES_ENTITIES,
    );
    let entity = potion.get_entity();
    let world = entity.world.load_full();
    let pos = entity.pos.load();
    let aabb = entity.bounding_box.load().expand(4.0, 2.0, 4.0);
    let mut nearby = world.get_entities_at_box(&aabb);
    nearby.extend(
        world
            .get_players_at_box(&aabb)
            .into_iter()
            .map(|p| p as Arc<dyn EntityBase>),
    );
    if hurts || extinguishes {
        let owner = potion
            .get_owner_id()
            .and_then(|id| world.get_entity_by_id(id));
        for target in &nearby {
            if target.get_living_entity().is_none() {
                continue;
            }
            let sensitive = target
                .get_mob()
                .is_some_and(crate::entity::mob::Mob::is_sensitive_to_water);
            let target_entity = target.get_entity();
            if !sensitive && !target_entity.is_on_fire() {
                continue;
            }
            if pos.squared_distance_to_vec(&target_entity.pos.load()) < 16.0 {
                if hurts && sensitive {
                    target.damage_with_context(
                        target.as_ref(),
                        1.0,
                        DamageType::INDIRECT_MAGIC,
                        None,
                        Some(potion),
                        owner.as_deref(),
                    );
                }
                if extinguishes && target_entity.is_on_fire() && target_entity.is_alive() {
                    target_entity.extinguish();
                }
            }
        }
    }
    if potion_is(
        stack,
        &pumpkin_data::tag::Potion::MINECRAFT_REHYDRATES_AXOLOTLS,
    ) {
        for target in &nearby {
            if let Some(axolotl) = target
                .cast_any()
                .downcast_ref::<crate::entity::passive::axolotl::AxolotlEntity>()
            {
                axolotl.rehydrate();
            }
        }
    }
}

impl EntityBase for SplashPotionEntity {
    fn get_owner_id(&self) -> Option<i32> {
        self.thrown.owner_id.load()
    }

    fn init_data_tracker(&self) {
        let entity = self.get_entity();
        let stack = self
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Sync the item stack
        entity.set_synced_data(
            pumpkin_data::tracked_data::splash_potion::ITEM_STACK,
            pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer::from(stack.clone()),
        );
    }

    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        self.thrown.process_tick(caller);
    }

    fn get_entity(&self) -> &Entity {
        self.thrown.get_entity()
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    #[allow(clippy::too_many_lines)]
    fn on_hit(&self, hit: crate::entity::projectile::ProjectileHit) {
        let world = self.get_entity().world.load();
        let hit_pos = hit.hit_pos();

        // Extinguish fire if it's a water potion
        let stack = self
            .item_stack
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        extinguish_fire_if_water_potion(&world, hit_pos, &stack);
        affect_entities_around(self, &stack);

        // Send impact entity status (plays break particles)
        world.send_entity_status(
            self.get_entity(),
            pumpkin_data::entity::EntityStatus::Death,
            Some(pumpkin_protocol::bedrock::server::actor_event::ActorEventID::Death),
        );

        let effects = crate::item::potion::PotionContents::read_potion_effects(&stack);

        // Calculate color: custom_color if present, else blend of effects, else default water color
        let mut color = 0x385dc6; // default water-like color
        if let Some(pc) =
            stack.get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
        {
            if let Some(c) = pc.custom_color {
                color = c;
            } else if !effects.is_empty() {
                let mut r_sum = 0.0;
                let mut g_sum = 0.0;
                let mut b_sum = 0.0;
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
                color = (r << 16) | (g << 8) | b;
            }
        } else if !effects.is_empty() {
            let mut r_sum = 0.0;
            let mut g_sum = 0.0;
            let mut b_sum = 0.0;
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
            color = (r << 16) | (g << 8) | b;
        }

        // Play splash particles
        let has_instant = effects.iter().any(|(e, _, _, _, _, _)| {
            e.id == pumpkin_data::effect::StatusEffect::INSTANT_DAMAGE.id
                || e.id == pumpkin_data::effect::StatusEffect::INSTANT_HEALTH.id
        });
        let event = if has_instant {
            WorldEvent::ParticlesInstantPotionSplash
        } else {
            WorldEvent::ParticlesSpellPotionSplash
        };

        // Convert hit_pos to BlockPos
        let block_pos = BlockPos(Vector3::new(
            hit_pos.x.floor() as i32,
            hit_pos.y.floor() as i32,
            hit_pos.z.floor() as i32,
        ));
        world.sync_world_event(event, block_pos, color);

        // If no effects, just splash (like water bottles)
        if effects.is_empty() {
            return;
        }

        let radius = 4.0f64;
        let min = Vector3::new(hit_pos.x - radius, hit_pos.y - radius, hit_pos.z - radius);
        let max = Vector3::new(hit_pos.x + radius, hit_pos.y + radius, hit_pos.z + radius);
        let aabb = BoundingBox::new(min, max);

        // Gather entity and player candidates
        let mut candidates = world.get_entities_at_box(&aabb);
        let players = world.get_players_at_box(&aabb);
        for p in players {
            candidates.push(p.clone() as Arc<dyn EntityBase>);
        }

        let mut affected: Vec<(Arc<dyn EntityBase>, f32)> = Vec::new();
        for cand in candidates {
            if cand.get_living_entity().is_some() {
                let pos = cand.get_entity().pos.load();
                let dx = pos.x - hit_pos.x;
                let dy = pos.y - hit_pos.y;
                let dz = pos.z - hit_pos.z;
                let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                if dist <= radius {
                    let scale = (1.0f32 - (dist as f32 / radius as f32)).max(0.0);
                    affected.push((cand, scale));
                }
            }
        }

        let affected_ids: Vec<i32> = affected
            .iter()
            .map(|(c, _)| c.get_entity().entity_id)
            .collect();
        if let Some(server) = world.server.upgrade() {
            let mut event =
                crate::plugin::api::events::entity::potion_splash::PotionSplashEvent::new(
                    self.get_entity().entity_id,
                    block_pos,
                    stack.item.registry_key.to_string(),
                    affected_ids,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
            affected.retain(|(c, _)| event.affected_entities.contains(&c.get_entity().entity_id));
        }

        for (cand, scale) in affected {
            if let Some(living) = cand.get_living_entity() {
                crate::item::potion::PotionContents::apply_effects_to(
                    living,
                    effects.clone(),
                    scale,
                    crate::item::potion::PotionApplicationSource::Normal,
                );
            }
        }
    }
}
