use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::mob::guardian::register_guardian_goals;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::potion::Effect;
use pumpkin_protocol::java::client::play::{CGameEvent, GameEvent};

use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
};

pub struct ElderGuardianEntity {
    pub mob_entity: MobEntity,
    tick_count: AtomicI32,
}

impl ElderGuardianEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `Guardian`'s constructor.
        mob_entity.set_pathfinding_malus(PathType::Water, 0.0);
        let guardian = Self {
            mob_entity,
            tick_count: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(guardian);
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
            register_guardian_goals(
                &mut goal_selector,
                &mut target_selector,
                &mob_arc.mob_entity,
                &mob_weak,
                400,
            );
        };

        mob_arc
    }
}

impl Mob for ElderGuardianEntity {
    /// Vanilla `ElderGuardian.getAmbientSound`.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        Some(if self.get_entity().is_in_water() {
            pumpkin_data::sound::Sound::EntityElderGuardianAmbient
        } else {
            pumpkin_data::sound::Sound::EntityElderGuardianAmbientLand
        })
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        let entity = &self.mob_entity.living_entity.entity;
        if !entity.is_alive() {
            return;
        }

        let ticks = self.tick_count.fetch_add(1, Ordering::Relaxed) + 1;
        if ticks % 1200 == 0 {
            let pos = entity.pos.load();
            let world = entity.world.load();
            let players = world.get_nearby_players(pos, 50.0);
            let packet = CGameEvent::new(GameEvent::PlayElderGuardianMobAppearance, 1.0);

            for player in players {
                player.try_send_client_packet(&packet);
                player.living_entity.add_effect(Effect {
                    effect_type: &StatusEffect::MINING_FATIGUE,
                    duration: 6000,
                    amplifier: 2,
                    ambient: false,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                });
            }
        }
    }

    fn on_damage(&self, _damage_type: DamageType, source: Option<&dyn EntityBase>) {
        if let Some(src) = source
            && let Some(living) = src.get_living_entity()
        {
            let _ = living.damage(src, 2.0, DamageType::THORNS);
        }
    }
}
