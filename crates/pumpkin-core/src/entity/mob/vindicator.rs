use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use crate::entity::ai::goal::avoid_entity::AvoidEntityGoal;
use crate::entity::ai::goal::break_door::BreakDoorGoal;
use crate::entity::ai::goal::open_door::OpenDoorGoal;
use crate::entity::ai::goal::{Controls, Goal, to_goal_ticks};
use crate::entity::mob::raider::{add_raider_goals, is_abstract_villager, raider_hurt_by_target};
use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::Difficulty;
use pumpkin_util::random::RandomImpl;

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::ActiveTargetGoal, look_at_entity::LookAtEntityGoal,
        melee_attack::MeleeAttackGoal, swim::SwimGoal, wander_around::WanderAroundGoal,
    },
    mob::{
        Mob, MobEntity,
        patrol::{PatrolData, PatrollingMonster},
        raider::{HoldGroundAttackGoal, Raider, RaiderData},
    },
};

pub struct VindicatorEntity {
    pub mob_entity: MobEntity,
    pub raider_data: RaiderData,
    /// Vanilla `isJohnny`: named Johnny once, it attacks everything.
    is_johnny: AtomicBool,
    /// The custom name last seen, so only a rename counts as vanilla `setCustomName`.
    seen_name: Mutex<Option<String>>,
}

impl VindicatorEntity {
    #[must_use]
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let vindicator = Self {
            mob_entity,
            raider_data: RaiderData::default(),
            is_johnny: AtomicBool::new(false),
            seen_name: Mutex::new(None),
        };
        let mob_arc = Arc::new(vindicator);
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
            goal_selector.add_goal(
                1,
                Box::new(AvoidEntityGoal::new(&EntityType::CREAKING, 8.0, 1.0, 1.2)),
            );
            goal_selector.add_goal(2, Box::new(VindicatorBreakDoorGoal::new()));
            goal_selector.add_goal(3, Box::new(RaiderOpenDoorGoal::new()));
            goal_selector.add_goal(4, Box::new(HoldGroundAttackGoal::new(10.0)));
            goal_selector.add_goal(5, Box::new(MeleeAttackGoal::new(1.0, false)));
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            target_selector.add_goal(1, raider_hurt_by_target());
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::predicated(&mob_arc.mob_entity, 10, true, is_abstract_villager),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::IRON_GOLEM, true),
            );
            target_selector.add_goal(
                4,
                Box::new(VindicatorJohnnyAttackGoal(
                    ActiveTargetGoal::predicated(&mob_arc.mob_entity, 0, true, |target, _| {
                        target.entity.entity_type.attackable != Some(false)
                    })
                    .must_reach(),
                )),
            );
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
        };

        mob_arc
    }
}

impl Mob for VindicatorEntity {
    /// Vanilla `Vindicator.finalizeSpawn`: `super`, then the axe and its enchantments.
    fn finalize_spawn(
        &self,
        world: &Arc<crate::world::World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        self.mob_entity.finalize_spawn_base();
        let difficulty = crate::entity::mob::equipment::RegionalDifficulty::at(
            world,
            self.get_entity().pos.load(),
        );
        self.populate_default_equipment_slots(world, &difficulty);
        self.populate_default_equipment_enchantments(world, &difficulty);
        group_data
    }

    /// Vanilla `Vindicator.populateDefaultEquipmentSlots`: an iron axe, outside a raid.
    fn populate_default_equipment_slots(
        &self,
        _world: &Arc<crate::world::World>,
        _difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
    ) {
        if self.get_raider_data().raid_id.load().is_none() {
            self.mob_entity.set_item_slot(
                &pumpkin_data::data_component_impl::EquipmentSlot::MAIN_HAND,
                pumpkin_data::item_stack::ItemStack::new(1, &pumpkin_data::item::Item::IRON_AXE),
            );
        }
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
        if self.is_johnny.load(Ordering::Relaxed) {
            nbt.put_bool("Johnny", true);
        }
    }

    /// Vanilla reads `Johnny` after `Entity.load` has set the custom name, so a loaded or
    /// summoned name alone never makes a Johnny.
    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_raider_nbt(nbt);
        self.is_johnny
            .store(nbt.get_bool("Johnny").unwrap_or(false), Ordering::Relaxed);
        *self
            .seen_name
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = self.custom_name_text();
    }

    /// Vanilla `Vindicator.setCustomName`: renamed Johnny, it stays Johnny.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        let name = self.custom_name_text();
        let mut seen = self
            .seen_name
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *seen != name {
            if name.as_deref() == Some("Johnny") {
                self.is_johnny.store(true, Ordering::Relaxed);
            }
            *seen = name;
        }
    }
}

/// Vanilla `Vindicator.VindicatorJohnnyAttackGoal`: Johnny goes for anything attackable it
/// can see and reach.
struct VindicatorJohnnyAttackGoal(Box<ActiveTargetGoal>);

impl Goal for VindicatorJohnnyAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.cast_any()
            .downcast_ref::<VindicatorEntity>()
            .is_some_and(|vindicator| vindicator.is_johnny.load(Ordering::Relaxed))
            && self.0.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.0.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.0.start(mob);
        mob.get_mob_entity()
            .living_entity
            .no_action_time
            .store(0, Ordering::Relaxed);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.0.stop(mob);
    }

    fn controls(&self) -> Controls {
        self.0.controls()
    }
}

impl PatrollingMonster for VindicatorEntity {
    fn get_patrol_data(&self) -> &PatrolData {
        &self.raider_data.patrol_data
    }
}

impl Raider for VindicatorEntity {
    fn get_raider_data(&self) -> &RaiderData {
        &self.raider_data
    }

    fn get_celebrate_sound(&self) -> Sound {
        Sound::EntityVindicatorCelebrate
    }
}

impl VindicatorEntity {
    fn custom_name_text(&self) -> Option<String> {
        self.get_entity()
            .custom_name
            .load()
            .as_ref()
            .as_ref()
            .map(|name| name.clone().get_text())
    }
}

fn has_active_raid(mob: &dyn Mob) -> bool {
    mob.cast_any()
        .downcast_ref::<VindicatorEntity>()
        .is_some_and(Raider::has_active_raid)
}

/// Vanilla `Vindicator.VindicatorBreakDoorGoal`: breaks doors on normal or hard, only in a raid.
struct VindicatorBreakDoorGoal {
    inner: BreakDoorGoal,
}

impl VindicatorBreakDoorGoal {
    fn new() -> Self {
        Self {
            // A break time of 6 ticks, which `getDoorBreakTime` raises to the 240 minimum.
            inner: BreakDoorGoal::with_door_break_time(
                6,
                Arc::new(|difficulty| {
                    difficulty == Difficulty::Normal || difficulty == Difficulty::Hard
                }),
            ),
        }
    }
}

impl Goal for VindicatorBreakDoorGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        has_active_raid(mob)
            && mob
                .get_entity()
                .random()
                .next_bounded_i32(to_goal_ticks(10))
                == 0
            && self.inner.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        has_active_raid(mob) && self.inner.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.inner.start(mob);
        mob.get_mob_entity()
            .living_entity
            .no_action_time
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.inner.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.inner.tick(mob);
    }

    fn should_run_every_tick(&self) -> bool {
        self.inner.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `AbstractIllager.RaiderOpenDoorGoal`: opens doors only during a raid.
struct RaiderOpenDoorGoal {
    inner: OpenDoorGoal,
}

impl RaiderOpenDoorGoal {
    const fn new() -> Self {
        Self {
            inner: OpenDoorGoal::new(false),
        }
    }
}

impl Goal for RaiderOpenDoorGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.inner.can_start(mob) && has_active_raid(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.inner.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.inner.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.inner.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.inner.tick(mob);
    }

    fn should_run_every_tick(&self) -> bool {
        self.inner.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        self.inner.controls()
    }
}
