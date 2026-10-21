use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use crate::entity::ai::goal::avoid_entity::AvoidEntityGoal;
use crate::entity::ai::goal::bow_attack::BowAttackGoal;
use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::mob::raider::{add_raider_goals, is_abstract_villager, raider_hurt_by_target};
use crate::entity::mob::spellcaster::{
    IllagerSpell, Spell, SpellcasterCastingSpellGoal, SpellcasterData, SpellcasterIllager,
    UseSpellGoal, base_can_use,
};
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::potion::Effect;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::ActiveTargetGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{
        Mob, MobEntity,
        patrol::{PatrolData, PatrollingMonster},
        raider::{Raider, RaiderData},
    },
};

pub struct IllusionerEntity {
    pub mob_entity: MobEntity,
    pub raider_data: RaiderData,
    pub spellcaster: SpellcasterData,
}

impl IllusionerEntity {
    #[must_use]
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let illusioner = Self {
            mob_entity,
            raider_data: RaiderData::default(),
            spellcaster: SpellcasterData::default(),
        };
        let mob_arc = Arc::new(illusioner);
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

            add_raider_goals(&mut goal_selector);
            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(1, Box::new(SpellcasterCastingSpellGoal));
            goal_selector.add_goal(
                3,
                Box::new(AvoidEntityGoal::new(&EntityType::CREAKING, 8.0, 1.0, 1.2)),
            );
            goal_selector.add_goal(4, Box::new(UseSpellGoal::new(IllusionerMirrorSpell)));
            goal_selector.add_goal(
                5,
                Box::new(UseSpellGoal::new(IllusionerBlindnessSpell {
                    last_target_id: AtomicI32::new(0),
                })),
            );
            goal_selector.add_goal(6, Box::new(BowAttackGoal::new(0.5, 20, 15.0)));
            goal_selector.add_goal(8, Box::new(WanderAroundGoal::new(0.6)));
            goal_selector.add_goal(
                9,
                Box::new(LookAtEntityGoal::new(
                    mob_weak,
                    &EntityType::PLAYER,
                    3.0,
                    1.0,
                    false,
                )),
            );
            goal_selector.add_goal(10, LookAtEntityGoal::at_mobs(8.0));
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            target_selector.add_goal(1, raider_hurt_by_target());
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true)
                    .with_unseen_memory_ticks(300),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::predicated(&mob_arc.mob_entity, 10, false, is_abstract_villager)
                    .with_unseen_memory_ticks(300),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::IRON_GOLEM, false)
                    .with_unseen_memory_ticks(300),
            );
        };

        mob_arc
    }
}

impl Mob for IllusionerEntity {
    /// Vanilla `Illusioner.finalizeSpawn`: a bow, then `super`.
    fn finalize_spawn(
        &self,
        _world: &Arc<crate::world::World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        self.mob_entity.set_item_slot(
            &pumpkin_data::data_component_impl::EquipmentSlot::MAIN_HAND,
            pumpkin_data::item_stack::ItemStack::new(1, &pumpkin_data::item::Item::BOW),
        );
        self.mob_entity.finalize_spawn_base();
        group_data
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_patrolling_monster(&self) -> Option<&dyn PatrollingMonster> {
        Some(self)
    }

    fn as_raider(&self) -> Option<&dyn Raider> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_raider_nbt(nbt);
        self.spellcaster.write_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_raider_nbt(nbt);
        self.spellcaster.read_nbt(nbt);
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        self.spellcaster.custom_server_ai_step();
    }
}

impl SpellcasterIllager for IllusionerEntity {
    fn spellcaster(&self) -> &SpellcasterData {
        &self.spellcaster
    }

    fn casting_sound(&self) -> Sound {
        Sound::EntityIllusionerCastSpell
    }
}

/// Vanilla `Illusioner.IllusionerMirrorSpellGoal`: turns invisible for a minute.
struct IllusionerMirrorSpell;

impl Spell for IllusionerMirrorSpell {
    fn spell(&self) -> IllagerSpell {
        IllagerSpell::Disappear
    }

    fn casting_time(&self) -> i32 {
        20
    }

    fn casting_interval(&self) -> i32 {
        340
    }

    fn prepare_sound(&self) -> Option<Sound> {
        Some(Sound::EntityIllusionerPrepareMirror)
    }

    fn can_use(&mut self, mob: &dyn Mob, next_attack_tick: i32) -> bool {
        base_can_use(mob, next_attack_tick)
            && !mob
                .get_mob_entity()
                .living_entity
                .has_effect(&StatusEffect::INVISIBILITY)
    }

    fn perform(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity().living_entity.add_effect(Effect {
            effect_type: &StatusEffect::INVISIBILITY,
            duration: 1200,
            amplifier: 0,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        });
    }
}

/// Vanilla `Illusioner.IllusionerBlindnessSpellGoal`: blinds each new target once, when the
/// local difficulty is above normal.
struct IllusionerBlindnessSpell {
    last_target_id: AtomicI32,
}

impl Spell for IllusionerBlindnessSpell {
    fn spell(&self) -> IllagerSpell {
        IllagerSpell::Blindness
    }

    fn casting_time(&self) -> i32 {
        20
    }

    fn casting_interval(&self) -> i32 {
        180
    }

    fn prepare_sound(&self) -> Option<Sound> {
        Some(Sound::EntityIllusionerPrepareBlindness)
    }

    fn can_use(&mut self, mob: &dyn Mob, next_attack_tick: i32) -> bool {
        if !base_can_use(mob, next_attack_tick) {
            return false;
        }
        let Some(target) = mob.get_mob_entity().get_target() else {
            return false;
        };
        if target.get_entity().entity_id == self.last_target_id.load(Ordering::Relaxed) {
            return false;
        }
        // `getCurrentDifficultyAt(blockPosition()).isHarderThan(Difficulty.NORMAL.ordinal())`.
        let entity = mob.get_entity();
        let world = entity.world.load_full();
        RegionalDifficulty::at(&world, entity.pos.load()).effective_difficulty > 2.0
    }

    fn on_start(&mut self, mob: &dyn Mob) {
        if let Some(target) = mob.get_mob_entity().get_target() {
            self.last_target_id
                .store(target.get_entity().entity_id, Ordering::Relaxed);
        }
    }

    fn perform(&mut self, mob: &dyn Mob) {
        if let Some(target) = mob.get_mob_entity().get_target()
            && let Some(living) = target.get_living_entity()
        {
            living.add_effect(Effect {
                effect_type: &StatusEffect::BLINDNESS,
                duration: 400,
                amplifier: 0,
                ambient: false,
                show_particles: true,
                show_icon: true,
                blend: false,
            });
        }
    }
}

impl PatrollingMonster for IllusionerEntity {
    fn get_patrol_data(&self) -> &PatrolData {
        &self.raider_data.patrol_data
    }
}

impl Raider for IllusionerEntity {
    fn get_raider_data(&self) -> &RaiderData {
        &self.raider_data
    }

    /// Vanilla `Illusioner.getCelebrateSound`.
    fn get_celebrate_sound(&self) -> Sound {
        Sound::EntityIllusionerAmbient
    }
}
