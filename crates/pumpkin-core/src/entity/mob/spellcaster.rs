//! Vanilla `SpellcasterIllager`: the spell state and goals evokers and illusioners share.

use std::sync::PoisonError;
use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};

use pumpkin_data::sound::Sound;
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::ai::goal::{Controls, Goal};
use crate::entity::mob::Mob;
use crate::entity::mob::evoker::EvokerEntity;
use crate::entity::mob::illusioner::IllusionerEntity;
use crate::entity::{Entity, EntityBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum IllagerSpell {
    None = 0,
    SummonVex = 1,
    Fangs = 2,
    Wololo = 3,
    Disappear = 4,
    Blindness = 5,
}

impl IllagerSpell {
    #[must_use]
    pub const fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::SummonVex,
            2 => Self::Fangs,
            3 => Self::Wololo,
            4 => Self::Disappear,
            5 => Self::Blindness,
            _ => Self::None,
        }
    }
}

/// Vanilla `SpellcasterIllager`'s fields.
#[derive(Default)]
pub struct SpellcasterData {
    spell_casting_tick_count: AtomicI32,
    current_spell: AtomicU8,
}

impl SpellcasterData {
    #[must_use]
    pub fn is_casting_spell(&self) -> bool {
        self.spell_casting_tick_count.load(Ordering::Relaxed) > 0
    }

    pub fn set_is_casting_spell(&self, entity: &Entity, spell: IllagerSpell) {
        self.current_spell.store(spell as u8, Ordering::Relaxed);
        entity.set_synced_data(tracked_data::evoker::SPELL_CASTING_ID, spell as u8 as i8);
    }

    #[must_use]
    pub fn current_spell(&self) -> IllagerSpell {
        IllagerSpell::from_u8(self.current_spell.load(Ordering::Relaxed))
    }

    #[must_use]
    pub fn spell_casting_time(&self) -> i32 {
        self.spell_casting_tick_count.load(Ordering::Relaxed)
    }

    pub fn set_spell_casting_time(&self, ticks: i32) {
        self.spell_casting_tick_count
            .store(ticks, Ordering::Relaxed);
    }

    pub fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_int("SpellTicks", self.spell_casting_time());
    }

    pub fn read_nbt(&self, nbt: &NbtCompound) {
        if let Some(ticks) = nbt.get_int("SpellTicks") {
            self.set_spell_casting_time(ticks);
        }
    }

    /// Vanilla `SpellcasterIllager.customServerAiStep`.
    pub fn custom_server_ai_step(&self) {
        let ticks = self.spell_casting_tick_count.load(Ordering::Relaxed);
        if ticks > 0 {
            self.spell_casting_tick_count
                .store(ticks - 1, Ordering::Relaxed);
        }
    }
}

pub trait SpellcasterIllager: Mob {
    fn spellcaster(&self) -> &SpellcasterData;

    /// Vanilla `getCastingSoundEvent`.
    fn casting_sound(&self) -> Sound;

    /// What the casting goal looks at when there is no target: the evoker's wololo sheep.
    fn casting_look_fallback(&self) -> Option<std::sync::Arc<dyn EntityBase>> {
        None
    }
}

/// The spellcaster behind an evoker or illusioner.
pub fn as_spellcaster(mob: &dyn Mob) -> Option<&dyn SpellcasterIllager> {
    let any = mob.cast_any();
    if let Some(evoker) = any.downcast_ref::<EvokerEntity>() {
        return Some(evoker);
    }
    any.downcast_ref::<IllusionerEntity>()
        .map(|illusioner| illusioner as &dyn SpellcasterIllager)
}

/// Vanilla `Entity.playSound(sound, 1.0F, 1.0F)`.
pub fn play_sound(mob: &dyn Mob, sound: Sound) {
    let entity = mob.get_entity();
    if !entity.is_silent() {
        entity.world.load().play_sound_fine(
            sound,
            mob.sound_source(),
            &entity.pos.load(),
            1.0,
            1.0,
        );
    }
}

/// Vanilla `SpellcasterIllager.SpellcasterCastingSpellGoal`: stands still and faces the
/// target while a spell is being cast.
pub struct SpellcasterCastingSpellGoal;

impl Goal for SpellcasterCastingSpellGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_spellcaster(mob).is_some_and(|caster| caster.spellcaster().spell_casting_time() > 0)
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(caster) = as_spellcaster(mob) {
            caster
                .spellcaster()
                .set_is_casting_spell(mob.get_entity(), IllagerSpell::None);
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let look_at = mob
            .get_target()
            .or_else(|| as_spellcaster(mob).and_then(SpellcasterIllager::casting_look_fallback));
        if let Some(look_at) = look_at {
            mob.get_mob_entity()
                .look_control
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .look_at_entity_with_range(
                    &look_at,
                    mob.get_max_head_rotation(),
                    mob.get_max_look_pitch_change(),
                );
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}

/// One spell for [`UseSpellGoal`]: vanilla's `SpellcasterUseSpellGoal` subclasses.
pub trait Spell: Send + Sync {
    fn spell(&self) -> IllagerSpell;

    /// Vanilla `getCastWarmupTime`.
    fn cast_warmup_time(&self) -> i32 {
        20
    }

    /// Vanilla `getCastingTime`.
    fn casting_time(&self) -> i32;

    /// Vanilla `getCastingInterval`.
    fn casting_interval(&self) -> i32;

    /// Vanilla `getSpellPrepareSound`.
    fn prepare_sound(&self) -> Option<Sound>;

    /// Vanilla `canUse`; most spells add to [`base_can_use`].
    fn can_use(&mut self, mob: &dyn Mob, next_attack_tick: i32) -> bool {
        base_can_use(mob, next_attack_tick)
    }

    /// Vanilla `canContinueToUse`.
    fn can_continue(&mut self, mob: &dyn Mob, attack_warmup_delay: i32) -> bool {
        living_target(mob).is_some() && attack_warmup_delay > 0
    }

    /// Runs after the shared `start`.
    fn on_start(&mut self, _mob: &dyn Mob) {}

    fn on_stop(&mut self, _mob: &dyn Mob) {}

    /// Vanilla `performSpellCasting`.
    fn perform(&mut self, mob: &dyn Mob);
}

/// The mob's target while it is alive.
pub fn living_target(mob: &dyn Mob) -> Option<std::sync::Arc<dyn EntityBase>> {
    mob.get_target().filter(|target| {
        target
            .get_living_entity()
            .is_some_and(crate::entity::living::LivingEntity::is_alive)
    })
}

/// Vanilla `SpellcasterUseSpellGoal.canUse`.
pub fn base_can_use(mob: &dyn Mob, next_attack_tick: i32) -> bool {
    living_target(mob).is_some()
        && as_spellcaster(mob).is_some_and(|caster| !caster.spellcaster().is_casting_spell())
        && mob.get_entity().tick_count.load(Ordering::Relaxed) >= next_attack_tick
}

/// Vanilla `SpellcasterIllager.SpellcasterUseSpellGoal`: prepares, then casts `spell` after
/// its warm-up, at most once per casting interval.
pub struct UseSpellGoal<S: Spell> {
    attack_warmup_delay: i32,
    next_attack_tick_count: i32,
    spell: S,
}

impl<S: Spell> UseSpellGoal<S> {
    pub const fn new(spell: S) -> Self {
        Self {
            attack_warmup_delay: 0,
            next_attack_tick_count: 0,
            spell,
        }
    }
}

impl<S: Spell> Goal for UseSpellGoal<S> {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.spell.can_use(mob, self.next_attack_tick_count)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.spell.can_continue(mob, self.attack_warmup_delay)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.attack_warmup_delay = self.get_tick_count(self.spell.cast_warmup_time());
        if let Some(caster) = as_spellcaster(mob) {
            caster
                .spellcaster()
                .set_spell_casting_time(self.spell.casting_time());
            self.next_attack_tick_count =
                mob.get_entity().tick_count.load(Ordering::Relaxed) + self.spell.casting_interval();
            if let Some(sound) = self.spell.prepare_sound() {
                play_sound(mob, sound);
            }
            caster
                .spellcaster()
                .set_is_casting_spell(mob.get_entity(), self.spell.spell());
        }
        self.spell.on_start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.spell.on_stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.attack_warmup_delay -= 1;
        if self.attack_warmup_delay == 0 {
            self.spell.perform(mob);
            if let Some(caster) = as_spellcaster(mob) {
                play_sound(mob, caster.casting_sound());
            }
        }
    }
}
