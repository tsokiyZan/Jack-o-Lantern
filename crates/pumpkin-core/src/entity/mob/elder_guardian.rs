use crate::entity::mob::guardian::{GuardianData, guardian_walk_target_value};
use crate::entity::movement_emission::MovementEmission;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::potion::Effect;
use pumpkin_data::sound::Sound;
use pumpkin_protocol::java::client::play::{CGameEvent, GameEvent};
use pumpkin_util::GameMode;
use pumpkin_util::math::position::BlockPos;

use crate::entity::custom_sound::CustomSound;
use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
};

/// Vanilla `ElderGuardian.customServerAiStep`'s mining fatigue: duration and amplifier.
const MINING_FATIGUE_DURATION: i32 = 6000;
const MINING_FATIGUE_AMPLIFIER: u8 = 2;
/// Vanilla `addEffectToPlayersAround`'s `displayEffectLimit`: players are re-applied the
/// effect once less than this is left.
const MINING_FATIGUE_DISPLAY_LIMIT: i32 = 1200;

pub struct ElderGuardianEntity {
    pub mob_entity: MobEntity,
    pub guardian: GuardianData,
}

impl ElderGuardianEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_arc = Arc::new(Self {
            mob_entity: MobEntity::new(entity),
            guardian: GuardianData::new(true),
        });
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };
        mob_arc.guardian.init(&mob_arc.mob_entity, &mob_weak);
        // Vanilla `ElderGuardian`'s constructor.
        mob_arc
            .mob_entity
            .persistence_required
            .store(true, Ordering::Relaxed);
        mob_arc
    }
}

impl CustomSound for ElderGuardianEntity {
    fn hurt_sound(&self) -> Option<Sound> {
        Some(if self.get_entity().is_in_water() {
            Sound::EntityElderGuardianHurt
        } else {
            Sound::EntityElderGuardianHurtLand
        })
    }

    fn death_sound(&self) -> Option<Sound> {
        Some(if self.get_entity().is_in_water() {
            Sound::EntityElderGuardianDeath
        } else {
            Sound::EntityElderGuardianDeathLand
        })
    }
}

impl Mob for ElderGuardianEntity {
    /// Vanilla `Guardian.checkSpawnObstruction`: water doesn't obstruct it.
    fn check_spawn_obstruction(&self, world: &crate::world::World) -> bool {
        let entity = self.get_entity();
        world.is_unobstructed(Some(entity), &entity.bounding_box.load())
    }

    /// Vanilla `ElderGuardian.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        Some(if self.get_entity().is_in_water() {
            Sound::EntityElderGuardianAmbient
        } else {
            Sound::EntityElderGuardianAmbientLand
        })
    }

    /// Vanilla `Guardian.getAmbientSoundInterval`.
    fn ambient_sound_interval(&self) -> i32 {
        160
    }

    /// Vanilla `Guardian.getMovementEmission`.
    fn movement_emission(&self) -> MovementEmission {
        MovementEmission::Events
    }

    /// Vanilla `Guardian.getMaxHeadXRot`.
    fn get_max_look_pitch_change(&self) -> f32 {
        180.0
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    fn get_walk_target_value(&self, pos: &BlockPos) -> f32 {
        guardian_walk_target_value(self, pos)
    }

    fn ai_step(&self, caller: &dyn EntityBase) {
        self.guardian.ai_step(self, caller);
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        self.guardian.travel(self, caller)
    }

    fn before_hurt(
        &self,
        _amount: f32,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) {
        self.guardian.before_hurt(self, damage_type, source);
    }

    /// Vanilla `ElderGuardian.customServerAiStep`: every minute, gives mining fatigue to
    /// survival players within 50 blocks, and keeps the guardian near where it first was.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        let entity = &self.mob_entity.living_entity.entity;
        let ticks = entity
            .tick_count
            .load(Ordering::Relaxed)
            .wrapping_add(entity.entity_id);
        if ticks % 1200 == 0 {
            let pos = entity.pos.load();
            let world = entity.world.load();
            let packet = CGameEvent::new(
                GameEvent::PlayElderGuardianMobAppearance,
                if entity.is_silent() { 0.0 } else { 1.0 },
            );
            let elder: &dyn EntityBase = self;
            for player in world.players.load().iter() {
                let player_base: &dyn EntityBase = player.as_ref();
                let needs_effect = player
                    .living_entity
                    .get_effect(&StatusEffect::MINING_FATIGUE)
                    .is_none_or(|effect| {
                        // `endsWithin(displayEffectLimit - 1)`; -1 is an infinite effect.
                        effect.amplifier < MINING_FATIGUE_AMPLIFIER
                            || (effect.duration != -1
                                && effect.duration < MINING_FATIGUE_DISPLAY_LIMIT)
                    });
                // `gameMode.isSurvival()` counts adventure too.
                let gamemode = player.gamemode.load();
                if !matches!(gamemode, GameMode::Survival | GameMode::Adventure)
                    || elder.is_allied_to(player_base)
                    || player.get_entity().pos.load().squared_distance_to_vec(&pos) >= 50.0 * 50.0
                    || !needs_effect
                {
                    continue;
                }
                player.living_entity.add_effect(Effect {
                    effect_type: &StatusEffect::MINING_FATIGUE,
                    duration: MINING_FATIGUE_DURATION,
                    amplifier: MINING_FATIGUE_AMPLIFIER,
                    ambient: false,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                });
                player.try_send_client_packet(&packet);
            }
        }

        if !self.mob_entity.has_position_target() {
            self.mob_entity
                .position_target
                .store(entity.block_pos.load());
            self.mob_entity
                .position_target_range
                .store(16, Ordering::Relaxed);
        }
    }
}
