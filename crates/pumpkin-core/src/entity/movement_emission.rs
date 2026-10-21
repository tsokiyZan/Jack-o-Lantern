//! Vanilla movement emission: step, swim and splash sounds and the `step`, `swim` and `flap`
//! game events (`Entity.applyMovementEmissionAndPlaySound` and its helpers).

use std::sync::atomic::Ordering::Relaxed;

use pumpkin_data::block_sound_type::SoundType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, BlockState};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use super::{Entity, EntityBase};
use crate::world::game_event::GameEventContext;

/// Vanilla `Entity.MovementEmission`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementEmission {
    None,
    Sounds,
    Events,
    All,
}

impl MovementEmission {
    #[must_use]
    pub const fn emits_anything(self) -> bool {
        !matches!(self, Self::None)
    }

    #[must_use]
    pub const fn emits_events(self) -> bool {
        matches!(self, Self::Events | Self::All)
    }

    #[must_use]
    pub const fn emits_sounds(self) -> bool {
        matches!(self, Self::Sounds | Self::All)
    }
}

/// Vanilla `Entity.isStateClimbable`.
fn is_state_climbable(block: &Block) -> bool {
    block.has_tag(&tag::Block::MINECRAFT_CLIMBABLE) || block == &Block::POWDER_SNOW
}

/// `(rand - rand) * 0.4 + 1`, vanilla's swim and splash pitch.
fn swim_pitch(entity: &Entity) -> f32 {
    let mut random = entity.random();
    1.0 + (random.next_f32() - random.next_f32()) * 0.4
}

impl Entity {
    /// Vanilla `Entity.applyMovementEmissionAndPlaySound`, called at the end of `Entity.move`
    /// with the collided movement.
    /// `effect_pos` and `effect_state` are the block 0.2 under the feet, read before the
    /// move's fall damage could change it.
    pub fn apply_movement_emission_and_play_sound(
        &self,
        caller: &dyn EntityBase,
        clipped_movement: Vector3<f64>,
        effect_pos: BlockPos,
        effect_state: &'static BlockState,
    ) {
        let emission = caller.movement_emission();
        if !emission.emits_anything() || caller.is_passenger() {
            return;
        }
        let world = self.world.load();
        let effect_block = Block::from_state_id(effect_state.id);

        let move_dist_scale = f64::from(0.6f32);
        let moved_distance = (clipped_movement.length() * move_dist_scale) as f32;
        let horizontal_moved_distance =
            (clipped_movement.horizontal_length() * move_dist_scale) as f32;
        let (supporting_pos, supporting_block, supporting_state) =
            self.get_block_with_y_offset(f64::from(1.0e-5f32));
        let climbing = is_state_climbable(supporting_block);
        let move_dist = self.move_dist.load()
            + if climbing {
                moved_distance
            } else {
                horizontal_moved_distance
            };
        self.move_dist.store(move_dist);
        self.fly_dist.store(self.fly_dist.load() + moved_distance);

        if move_dist > self.next_step.load() && !supporting_state.is_air() {
            let only_effect_state_emissions = supporting_pos == effect_pos;
            let mut produced_side_effects = self.vibration_and_sound_effects_from_block(
                caller,
                &effect_pos,
                effect_block,
                effect_state,
                emission.emits_sounds(),
                only_effect_state_emissions,
                clipped_movement,
            );
            if !only_effect_state_emissions {
                produced_side_effects |= self.vibration_and_sound_effects_from_block(
                    caller,
                    &supporting_pos,
                    supporting_block,
                    supporting_state,
                    false,
                    emission.emits_events(),
                    clipped_movement,
                );
            }

            if produced_side_effects {
                self.next_step.store(caller.next_step());
            } else if self.is_in_water() {
                self.next_step.store(caller.next_step());
                if emission.emits_sounds() {
                    caller.water_swim_sound(caller);
                }
                if emission.emits_events() {
                    world.game_event_from(Some(caller), GameEvent::Swim, self.pos.load());
                }
            }
        } else if supporting_state.is_air() && caller.is_flapping() {
            // Vanilla `Entity.processFlappingMovement`.
            caller.on_flap();
            if caller.movement_emission().emits_events() {
                world.game_event_from(Some(caller), GameEvent::Flap, self.pos.load());
            }
        }
    }

    /// Vanilla `Entity.vibrationAndSoundEffectsFromBlock`.
    #[expect(clippy::too_many_arguments)]
    fn vibration_and_sound_effects_from_block(
        &self,
        caller: &dyn EntityBase,
        pos: &BlockPos,
        block: &'static Block,
        state: &'static BlockState,
        should_sound: bool,
        should_vibrate: bool,
        clipped_movement: Vector3<f64>,
    ) -> bool {
        if state.is_air() {
            return false;
        }
        let is_climbable = is_state_climbable(block);
        // Pumpkin has no minecarts on rails moving entities yet (`isOnRails`).
        if (self.on_ground.load(Relaxed)
            || is_climbable
            || self.is_sneaking() && clipped_movement.y == 0.0)
            && !self.is_swimming()
        {
            if should_sound {
                // Vanilla `Entity.walkingStepSound`.
                caller.play_step_sound(caller, pos, block, state);
                if block.has_tag(&tag::Block::MINECRAFT_CRYSTAL_SOUND_BLOCKS)
                    && self.tick_count.load(Relaxed)
                        >= self.last_crystal_sound_play_tick.load(Relaxed) + 20
                {
                    self.play_amethyst_step_sound(caller);
                }
            }
            if should_vibrate {
                self.world.load().game_event(
                    GameEvent::Step,
                    self.pos.load(),
                    GameEventContext::of(Some(caller), Some(state)),
                );
            }
            return true;
        }
        false
    }

    /// Vanilla `Entity.playStepSound`, the default for [`EntityBase::play_step_sound`].
    pub fn play_step_sound_default(&self, caller: &dyn EntityBase, state: &BlockState) {
        let sound_type = SoundType::from_state_id(state.id.as_u16());
        caller.play_sound_at_self(
            sound_type.step_sound,
            sound_type.volume * 0.15,
            sound_type.pitch,
        );
    }

    /// Vanilla `Entity.playMuffledStepSound`.
    pub fn play_muffled_step_sound(&self, caller: &dyn EntityBase, state: &BlockState) {
        let sound_type = SoundType::from_state_id(state.id.as_u16());
        caller.play_sound_at_self(
            sound_type.step_sound,
            sound_type.volume * 0.05,
            sound_type.pitch * 0.8,
        );
    }

    /// Vanilla `Entity.playCombinationStepSounds`.
    pub fn play_combination_step_sounds(
        &self,
        caller: &dyn EntityBase,
        primary: &BlockState,
        secondary: &BlockState,
    ) {
        let sound_type = SoundType::from_state_id(primary.id.as_u16());
        caller.play_sound_at_self(
            sound_type.step_sound,
            sound_type.volume * 0.15,
            sound_type.pitch,
        );
        self.play_muffled_step_sound(caller, secondary);
    }

    /// Vanilla `Entity.getPrimaryStepSoundBlockPos`.
    #[must_use]
    pub fn primary_step_sound_block_pos(&self, affecting_pos: &BlockPos) -> BlockPos {
        let above_pos = affecting_pos.up();
        let above = self.world.load().get_block(&above_pos);
        if above.has_tag(&tag::Block::MINECRAFT_INSIDE_STEP_SOUND_BLOCKS)
            || above.has_tag(&tag::Block::MINECRAFT_COMBINATION_STEP_SOUND_BLOCKS)
        {
            above_pos
        } else {
            *affecting_pos
        }
    }

    /// Vanilla `Entity.playAmethystStepSound`.
    fn play_amethyst_step_sound(&self, caller: &dyn EntityBase) {
        let tick_count = self.tick_count.load(Relaxed);
        let elapsed = tick_count - self.last_crystal_sound_play_tick.load(Relaxed);
        let mut intensity =
            self.crystal_sound_intensity.load() * 0.997f64.powf(f64::from(elapsed)) as f32;
        intensity = (intensity + 0.07).min(1.0);
        self.crystal_sound_intensity.store(intensity);
        let pitch = 0.5 + intensity * self.random().next_f32() * 1.2;
        let volume = 0.1 + intensity * 1.2;
        caller.play_sound_at_self(Sound::BlockAmethystBlockChime, volume, pitch);
        self.last_crystal_sound_play_tick.store(tick_count, Relaxed);
    }

    /// Vanilla `Entity.waterSwimSound`, the default for [`EntityBase::water_swim_sound`].
    pub fn water_swim_sound_default(&self, caller: &dyn EntityBase) {
        let speed = self.swim_speed(caller, 0.35, 0.4);
        caller.play_swim_sound(caller, speed);
    }

    /// Vanilla `Entity.playSwimSound`, the default for [`EntityBase::play_swim_sound`].
    pub fn play_swim_sound_default(&self, caller: &dyn EntityBase, volume: f32) {
        caller.play_sound_at_self(caller.swim_sound(), volume, swim_pitch(self));
    }

    /// Vanilla `Entity.doWaterSplashEffect`.
    pub fn do_water_splash_effect(&self, caller: &dyn EntityBase) {
        if caller.is_spectator() {
            // Vanilla `Player.doWaterSplashEffect`.
            return;
        }
        let speed = self.swim_speed(caller, 0.2, 0.9);
        let sound = if speed < 0.25 {
            caller.swim_splash_sound()
        } else {
            caller.swim_high_speed_splash_sound()
        };
        caller.play_sound_at_self(sound, speed, swim_pitch(self));
        // The bubble and splash particles: `addParticle` does nothing on the server, but
        // vanilla still evaluates their arguments, three draws per bubble and two per splash.
        let particle_limit = 1.0 + self.entity_dimension.load().width * 20.0;
        {
            let mut random = self.random();
            let mut i = 0;
            while (i as f32) < particle_limit {
                random.next_f64();
                random.next_f64();
                random.next_f64();
                i += 1;
            }
            let mut i = 0;
            while (i as f32) < particle_limit {
                random.next_f64();
                random.next_f64();
                i += 1;
            }
        }
        self.world
            .load()
            .game_event_from(Some(caller), GameEvent::Splash, self.pos.load());
    }

    /// The speed term shared by `waterSwimSound` and `doWaterSplashEffect`: the controlling
    /// passenger's movement when there is one.
    fn swim_speed(
        &self,
        caller: &dyn EntityBase,
        own_modifier: f32,
        passenger_modifier: f32,
    ) -> f32 {
        let controller = caller.controlling_passenger();
        let (volume_modifier, movement) = controller.as_ref().map_or_else(
            || (own_modifier, self.velocity.load()),
            |c| (passenger_modifier, c.get_entity().velocity.load()),
        );
        let mx = movement.x * movement.x * f64::from(0.2f32);
        let my = movement.y * movement.y;
        let mz = movement.z * movement.z * f64::from(0.2f32);
        ((mx + my + mz).sqrt() as f32 * volume_modifier).min(1.0)
    }
}
