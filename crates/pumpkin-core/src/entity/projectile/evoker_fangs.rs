use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::sound::Sound;

use crate::enchantment::EnchantmentHelper;
use crate::entity::damage_source::DamageSource;
use crate::entity::entity_reference::EntityReference;
use crate::world::World;
use crate::{
    entity::{Entity, EntityBase},
    server::Server,
};

pub struct EvokerFangsEntity {
    pub entity: Entity,
    pub warmup_delay_ticks: AtomicI32,
    pub life_ticks: AtomicI32,
    /// Vanilla `EvokerFangs.owner`.
    pub owner: EntityReference,
    pub sent_spike_event: AtomicBool,
}

impl EvokerFangsEntity {
    /// Vanilla `EvokerFangs.dealDamageTo`.
    fn deal_damage_to(&self, world: &Arc<World>, target: &dyn EntityBase) {
        let target_entity = target.get_entity();
        if !target_entity.is_alive() || target_entity.invulnerable.load(Ordering::Relaxed) {
            return;
        }
        // `getOwner`, an `EntityReference<LivingEntity>`.
        let owner = self
            .owner
            .get(world)
            .filter(|owner| owner.get_living_entity().is_some());
        if owner
            .as_ref()
            .is_some_and(|owner| owner.get_entity().entity_id == target_entity.entity_id)
        {
            return;
        }
        let Some(owner) = owner else {
            target.damage(target, 6.0, DamageType::MAGIC);
            return;
        };
        if owner.as_ref().is_allied_to(target) {
            return;
        }
        let damage_source = DamageSource {
            damage_type: DamageType::INDIRECT_MAGIC,
            direct: Some(&self.entity),
            causing: Some(owner.as_ref()),
        };
        if target.damage_with_context(
            target,
            6.0,
            damage_source.damage_type,
            None,
            damage_source.direct,
            damage_source.causing,
        ) {
            EnchantmentHelper::do_post_attack_effects(world, target, &damage_source);
        }
    }

    #[must_use]
    pub fn new(entity: Entity, warmup_ticks: u32, yaw: f32, owner: Option<&Entity>) -> Self {
        entity.set_rotation(yaw, 0.0);
        Self {
            entity,
            warmup_delay_ticks: AtomicI32::new(warmup_ticks as i32),
            life_ticks: AtomicI32::new(22),
            owner: EntityReference::of(owner),
            sent_spike_event: AtomicBool::new(false),
        }
    }
}

impl EntityBase for EvokerFangsEntity {
    fn write_custom_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        nbt.put_int("Warmup", self.warmup_delay_ticks.load(Ordering::Relaxed));
        if let Some(owner) = self.owner.uuid() {
            nbt.put_uuid("Owner", owner);
        }
    }

    fn read_custom_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.warmup_delay_ticks
            .store(nbt.get_int("Warmup").unwrap_or(0), Ordering::Relaxed);
        self.owner.set_uuid(nbt.get_uuid("Owner"));
    }

    fn tick(&self, _caller: &dyn EntityBase, _server: &Server) {
        let entity = &self.entity;
        let world = entity.world.load();

        let warmup = self.warmup_delay_ticks.fetch_sub(1, Ordering::Relaxed) - 1;
        if warmup < 0 {
            if warmup == -8 {
                let bb = entity.bounding_box.load().expand(0.2, 0.0, 0.2);
                for target in world.get_all_at_box(&bb) {
                    if target.get_living_entity().is_some() {
                        self.deal_damage_to(&world, target.as_ref());
                    }
                }
            }

            if !self.sent_spike_event.swap(true, Ordering::SeqCst) {
                world.send_entity_status(entity, EntityStatus::StartAttacking, None);
                if !entity.silent.load(Ordering::Relaxed) {
                    entity.play_sound(Sound::EntityEvokerFangsAttack);
                }
            }

            let life = self.life_ticks.fetch_sub(1, Ordering::Relaxed) - 1;
            if life < 0 {
                entity.remove();
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
