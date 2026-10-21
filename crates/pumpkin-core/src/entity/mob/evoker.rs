use crate::entity::ai::goal::avoid_entity::AvoidEntityGoal;
use crate::entity::mob::raider::{add_raider_goals, is_abstract_villager, raider_hurt_by_target};
use crate::entity::mob::spellcaster::{
    IllagerSpell, Spell, SpellcasterCastingSpellGoal, SpellcasterData, SpellcasterIllager,
    UseSpellGoal, base_can_use,
};
use crate::entity::mob::vex::VexEntity;
use crate::entity::passive::sheep::SheepEntity;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use uuid::Uuid;

use pumpkin_data::BlockDirection;
use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{self, position::BlockPos, vector3::Vector3};
use pumpkin_util::random::RandomImpl;

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
    projectile::evoker_fangs::EvokerFangsEntity,
    r#type::from_type,
};

/// Vanilla `DyeColor.BLUE` and `DyeColor.RED` ids.
const BLUE: u8 = 11;
const RED: u8 = 14;

pub struct EvokerEntity {
    pub mob_entity: MobEntity,
    pub raider_data: RaiderData,
    pub spellcaster: SpellcasterData,
    wololo_target: Mutex<Option<Arc<dyn EntityBase>>>,
}

impl EvokerEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_arc = Arc::new(Self {
            mob_entity: MobEntity::new(entity),
            raider_data: RaiderData::default(),
            spellcaster: SpellcasterData::default(),
            wololo_target: Mutex::new(None),
        });
        let mob_weak: Weak<dyn Mob> = {
            let mob_dyn: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_dyn)
        };

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            add_raider_goals(&mut goal_selector);
            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(1, Box::new(SpellcasterCastingSpellGoal));
            goal_selector.add_goal(
                2,
                Box::new(AvoidEntityGoal::new(&EntityType::PLAYER, 8.0, 0.6, 1.0)),
            );
            goal_selector.add_goal(
                3,
                Box::new(AvoidEntityGoal::new(&EntityType::CREAKING, 8.0, 0.6, 1.0)),
            );
            goal_selector.add_goal(4, Box::new(UseSpellGoal::new(EvokerSummonSpell)));
            goal_selector.add_goal(5, Box::new(UseSpellGoal::new(EvokerAttackSpell)));
            goal_selector.add_goal(6, Box::new(UseSpellGoal::new(EvokerWololoSpell)));
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
                .unwrap_or_else(PoisonError::into_inner);
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
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::IRON_GOLEM, false),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn is_casting_spell(&self) -> bool {
        self.spellcaster.is_casting_spell()
    }

    fn wololo_target(&self) -> Option<Arc<dyn EntityBase>> {
        self.wololo_target
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_wololo_target(&self, target: Option<Arc<dyn EntityBase>>) {
        *self
            .wololo_target
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = target;
    }
}

fn as_evoker(mob: &dyn Mob) -> Option<&EvokerEntity> {
    mob.cast_any().downcast_ref::<EvokerEntity>()
}

impl SpellcasterIllager for EvokerEntity {
    fn spellcaster(&self) -> &SpellcasterData {
        &self.spellcaster
    }

    fn casting_sound(&self) -> Sound {
        Sound::EntityEvokerCastSpell
    }

    /// Vanilla `EvokerCastingSpellGoal`: with no target it watches the sheep it converts.
    fn casting_look_fallback(&self) -> Option<Arc<dyn EntityBase>> {
        self.wololo_target()
    }
}

impl Mob for EvokerEntity {
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

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        self.spellcaster.custom_server_ai_step();
    }
}

impl PatrollingMonster for EvokerEntity {
    fn get_patrol_data(&self) -> &PatrolData {
        &self.raider_data.patrol_data
    }
}

impl Raider for EvokerEntity {
    fn get_raider_data(&self) -> &RaiderData {
        &self.raider_data
    }

    fn get_celebrate_sound(&self) -> Sound {
        Sound::EntityEvokerCelebrate
    }
}

/// Vanilla `Evoker.EvokerAttackSpellGoal`: a line of fangs towards the target, or two rings
/// around the evoker when the target is within 3 blocks.
struct EvokerAttackSpell;

impl Spell for EvokerAttackSpell {
    fn spell(&self) -> IllagerSpell {
        IllagerSpell::Fangs
    }

    fn casting_time(&self) -> i32 {
        40
    }

    fn casting_interval(&self) -> i32 {
        100
    }

    fn prepare_sound(&self) -> Option<Sound> {
        Some(Sound::EntityEvokerPrepareAttack)
    }

    fn perform(&mut self, mob: &dyn Mob) {
        let Some(target) = mob.get_target() else {
            return;
        };
        let evoker = mob.get_entity();
        let pos = evoker.pos.load();
        let target_pos = target.get_entity().pos.load();
        let min_y = target_pos.y.min(pos.y);
        let max_y = target_pos.y.max(pos.y) + 1.0;
        let angle_towards_target = math::atan2(target_pos.z - pos.z, target_pos.x - pos.x) as f32;
        let pi = std::f64::consts::PI as f32;
        if pos.squared_distance_to_vec(&target_pos) < 9.0 {
            for i in 0..5 {
                let angle = angle_towards_target + i as f32 * pi * 0.4;
                create_spell_entity(
                    mob,
                    pos.x + f64::from(math::cos(angle)) * 1.5,
                    pos.z + f64::from(math::sin(angle)) * 1.5,
                    min_y,
                    max_y,
                    angle,
                    0,
                );
            }
            for i in 0..8 {
                let angle = angle_towards_target
                    + i as f32 * pi * 2.0 / 8.0
                    + (std::f64::consts::PI * 2.0 / 5.0) as f32;
                create_spell_entity(
                    mob,
                    pos.x + f64::from(math::cos(angle)) * 2.5,
                    pos.z + f64::from(math::sin(angle)) * 2.5,
                    min_y,
                    max_y,
                    angle,
                    3,
                );
            }
        } else {
            for i in 0..16 {
                let reach = 1.25 * f64::from(i + 1);
                create_spell_entity(
                    mob,
                    pos.x + f64::from(math::cos(angle_towards_target)) * reach,
                    pos.z + f64::from(math::sin(angle_towards_target)) * reach,
                    min_y,
                    max_y,
                    angle_towards_target,
                    i,
                );
            }
        }
    }
}

/// Vanilla `EvokerAttackSpellGoal.createSpellEntity`: drops from `max_y` to the first block
/// with a sturdy top, no lower than a block under `min_y`, and puts the fangs on it.
fn create_spell_entity(
    mob: &dyn Mob,
    x: f64,
    z: f64,
    min_y: f64,
    max_y: f64,
    angle: f32,
    delay_ticks: u32,
) {
    let evoker = mob.get_entity();
    let world = evoker.world.load_full();
    let mut pos = BlockPos::new(x.floor() as i32, max_y.floor() as i32, z.floor() as i32);
    let mut top_offset = 0.0;
    let mut success = false;
    loop {
        let below = pos.down();
        if world
            .get_block_state(&below)
            .is_side_solid(BlockDirection::Up)
        {
            let state = world.get_block_state(&pos);
            if !state.is_air()
                && let Some(top) = state
                    .get_block_collision_shapes()
                    .map(|shape| shape.max.y)
                    .reduce(f64::max)
            {
                top_offset = top;
            }
            success = true;
            break;
        }
        pos = pos.down();
        if pos.0.y < min_y.floor() as i32 - 1 {
            break;
        }
    }

    if success {
        let entity = Entity::from_uuid(
            Uuid::new_v4(),
            world.clone(),
            Vector3::new(x, f64::from(pos.0.y) + top_offset, z),
            &EntityType::EVOKER_FANGS,
        );
        let yaw = angle * (180.0 / std::f32::consts::PI);
        world.spawn_entity(Arc::new(EvokerFangsEntity::new(
            entity,
            delay_ticks,
            yaw,
            Some(evoker),
        )));
    }
}

/// Vanilla `Evoker.EvokerSummonSpellGoal`: three vexes, while fewer than a random 1-8 are
/// around.
struct EvokerSummonSpell;

impl Spell for EvokerSummonSpell {
    fn spell(&self) -> IllagerSpell {
        IllagerSpell::SummonVex
    }

    fn casting_time(&self) -> i32 {
        100
    }

    fn casting_interval(&self) -> i32 {
        340
    }

    fn prepare_sound(&self) -> Option<Sound> {
        Some(Sound::EntityEvokerPrepareSummon)
    }

    fn can_use(&mut self, mob: &dyn Mob, next_attack_tick: i32) -> bool {
        if !base_can_use(mob, next_attack_tick) {
            return false;
        }
        // `getNearbyEntities(Vex.class, forNonCombat().range(16.0)...)`.
        let entity = mob.get_entity();
        let pos = entity.pos.load();
        let vexes = entity
            .world
            .load()
            .get_entities_at_box(&entity.bounding_box.load().expand(16.0, 16.0, 16.0))
            .iter()
            .filter(|other| {
                let other = other.get_entity();
                other.entity_type == &EntityType::VEX
                    && other.is_alive()
                    && other.pos.load().squared_distance_to_vec(&pos) <= 16.0 * 16.0
            })
            .count();
        mob.get_entity().random().next_bounded_i32(8) as usize + 1 > vexes
    }

    fn perform(&mut self, mob: &dyn Mob) {
        let evoker = mob.get_entity();
        let world = evoker.world.load_full();
        let evoker_block_pos = evoker.block_pos.load();
        for _ in 0..3 {
            let (dx, dz) = {
                let mut random = evoker.random();
                (random.next_bounded_i32(5), random.next_bounded_i32(5))
            };
            let pos = evoker_block_pos.offset(Vector3::new(-2 + dx, 1, -2 + dz));
            let vex = from_type(
                &EntityType::VEX,
                // `snapTo(pos)`: the bottom centre of the block.
                Vector3::new(
                    f64::from(pos.0.x) + 0.5,
                    f64::from(pos.0.y),
                    f64::from(pos.0.z) + 0.5,
                ),
                &world,
                Uuid::new_v4(),
            );
            crate::entity::mob::spawn::finalize_spawn(
                &vex,
                &world,
                crate::entity::mob::spawn::EntitySpawnReason::MobSummoned,
                None,
            );
            if let Some(vex_mob) = vex
                .get_mob()
                .and_then(|vex| vex.cast_any().downcast_ref::<VexEntity>())
            {
                vex_mob.set_owner(evoker.entity_uuid);
                vex_mob.set_bound_origin(Some(pos));
                let life = evoker.random().next_bounded_i32(90);
                vex_mob.set_limited_life(20 * (30 + life));
            }
            world.spawn_entity(vex);
        }
    }
}

/// Vanilla `Evoker.EvokerWololoSpellGoal`: with nothing to fight, turns a nearby blue sheep
/// red.
struct EvokerWololoSpell;

impl Spell for EvokerWololoSpell {
    fn spell(&self) -> IllagerSpell {
        IllagerSpell::Wololo
    }

    fn cast_warmup_time(&self) -> i32 {
        40
    }

    fn casting_time(&self) -> i32 {
        60
    }

    fn casting_interval(&self) -> i32 {
        140
    }

    fn prepare_sound(&self) -> Option<Sound> {
        Some(Sound::EntityEvokerPrepareWololo)
    }

    fn can_use(&mut self, mob: &dyn Mob, next_attack_tick: i32) -> bool {
        let Some(evoker) = as_evoker(mob) else {
            return false;
        };
        let entity = mob.get_entity();
        if mob.get_target().is_some()
            || evoker.is_casting_spell()
            || entity.tick_count.load(std::sync::atomic::Ordering::Relaxed) < next_attack_tick
        {
            return false;
        }
        let world = entity.world.load();
        if !world.level_info.load().game_rules.mob_griefing {
            return false;
        }
        // `getNearbyEntities(Sheep.class, forNonCombat().range(16.0)` with a blue selector.
        let pos = entity.pos.load();
        let sheep: Vec<_> = world
            .get_entities_at_box(&entity.bounding_box.load().expand(16.0, 4.0, 16.0))
            .into_iter()
            .filter(|other| {
                other.get_entity().is_alive()
                    && other.get_entity().pos.load().squared_distance_to_vec(&pos) <= 16.0 * 16.0
                    && other
                        .get_mob()
                        .and_then(|mob| mob.cast_any().downcast_ref::<SheepEntity>())
                        .is_some_and(|sheep| sheep.get_color() == BLUE)
            })
            .collect();
        if sheep.is_empty() {
            return false;
        }
        let index = entity.random().next_bounded_i32(sheep.len() as i32) as usize;
        evoker.set_wololo_target(Some(sheep[index].clone()));
        true
    }

    fn can_continue(&mut self, mob: &dyn Mob, attack_warmup_delay: i32) -> bool {
        as_evoker(mob).is_some_and(|evoker| evoker.wololo_target().is_some())
            && attack_warmup_delay > 0
    }

    fn on_stop(&mut self, mob: &dyn Mob) {
        if let Some(evoker) = as_evoker(mob) {
            evoker.set_wololo_target(None);
        }
    }

    fn perform(&mut self, mob: &dyn Mob) {
        let Some(target) = as_evoker(mob).and_then(EvokerEntity::wololo_target) else {
            return;
        };
        if target.get_entity().is_alive()
            && let Some(sheep) = target
                .get_mob()
                .and_then(|mob| mob.cast_any().downcast_ref::<SheepEntity>())
        {
            sheep.set_color(RED);
        }
    }
}
