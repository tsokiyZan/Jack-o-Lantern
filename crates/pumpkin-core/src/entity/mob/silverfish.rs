use crate::block::blocks::infested::InfestedBlock;
use crate::entity::ai::goal::climb_on_top_of_powder_snow::ClimbOnTopOfPowderSnowGoal;
use crate::entity::ai::goal::wander_around::WanderAroundGoal;
use crate::entity::ai::goal::{Controls, Goal, to_goal_ticks};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, PoisonError};

use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{BlockDirection, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::ActiveTargetGoal, melee_attack::MeleeAttackGoal, revenge::RevengeGoal,
        swim::SwimGoal,
    },
    mob::{Mob, MobEntity},
};

pub struct SilverfishEntity {
    pub entity: Arc<MobEntity>,
    /// Vanilla `SilverfishWakeUpFriendsGoal.lookForFriends`, shared with that goal.
    look_for_friends: Arc<AtomicI32>,
}

impl SilverfishEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = Arc::new(MobEntity::new(entity));
        let silverfish = Self {
            entity,
            look_for_friends: Arc::new(AtomicI32::new(0)),
        };
        let mob_arc = Arc::new(silverfish);

        {
            let mut goal_selector = mob_arc
                .entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut target_selector = mob_arc
                .entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(1, SwimGoal::new(&mob_arc.entity));
            goal_selector.add_goal(1, Box::new(ClimbOnTopOfPowderSnowGoal));
            goal_selector.add_goal(
                3,
                Box::new(SilverfishWakeUpFriendsGoal {
                    look_for_friends: mob_arc.look_for_friends.clone(),
                }),
            );
            goal_selector.add_goal(4, Box::new(MeleeAttackGoal::new(1.0, false)));
            goal_selector.add_goal(5, Box::new(SilverfishMergeWithStoneGoal::new()));

            target_selector.add_goal(1, Box::new(RevengeGoal::new(true).alerting_others()));
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.entity, &EntityType::PLAYER, true),
            );
        };

        mob_arc
    }
}

impl Mob for SilverfishEntity {
    /// Vanilla `Silverfish.getWalkTargetValue`: blocks it could infest are the best spots.
    fn get_walk_target_value(&self, pos: &pumpkin_util::math::position::BlockPos) -> f32 {
        let world = self.get_entity().world.load();
        if InfestedBlock::is_compatible_host_block(world.get_block(&pos.down())) {
            10.0
        } else {
            crate::entity::mob::monster_walk_target_value(&world, pos)
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity
    }

    /// The start of vanilla `Silverfish.tick`: the body follows the head-on rotation.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        let entity = &self.entity.living_entity.entity;
        entity.body_yaw.store(entity.yaw.load());
    }

    /// The start of vanilla `Silverfish.hurtServer`: a hit from an entity (or a damage type
    /// that always counts) sends it to wake up its friends.
    fn before_hurt(
        &self,
        _amount: f32,
        damage_type: DamageType,
        _source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) {
        if (cause.is_some()
            || damage_type.has_tag(&tag::DamageType::MINECRAFT_ALWAYS_TRIGGERS_SILVERFISH))
            && self.look_for_friends.load(Ordering::Relaxed) == 0
        {
            // `adjustedTickDelay(20)` for a goal ticked every other tick.
            self.look_for_friends
                .store(to_goal_ticks(20), Ordering::Relaxed);
        }
    }
}

/// Vanilla `Silverfish.SilverfishWakeUpFriendsGoal`: breaks open infested blocks nearby,
/// nearest first, stopping at random after each.
struct SilverfishWakeUpFriendsGoal {
    look_for_friends: Arc<AtomicI32>,
}

/// The `0, 1, -1, 2, -2, ...` order vanilla walks each axis in, out to `limit`.
fn nearest_first(limit: i32) -> impl Iterator<Item = i32> {
    std::iter::successors(Some(0), |&off: &i32| Some(i32::from(off <= 0) - off))
        .take_while(move |off| off.abs() <= limit)
}

impl Goal for SilverfishWakeUpFriendsGoal {
    fn can_start(&mut self, _mob: &dyn Mob) -> bool {
        self.look_for_friends.load(Ordering::Relaxed) > 0
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let remaining = self.look_for_friends.fetch_sub(1, Ordering::Relaxed) - 1;
        if remaining > 0 {
            return;
        }
        let entity = mob.get_entity();
        let world = entity.world.load_full();
        let base_pos = entity.block_pos.load();
        let mut random = mob.get_random();
        for y_off in nearest_first(5) {
            for x_off in nearest_first(10) {
                for z_off in nearest_first(10) {
                    let test_pos = base_pos.offset(Vector3::new(x_off, y_off, z_off));
                    let (block, state) = world.get_block_and_state_id(&test_pos);
                    if !InfestedBlock::is_infested(block) {
                        continue;
                    }
                    if world.level_info.load().game_rules.mob_griefing {
                        // `destroyBlock(pos, true, silverfish)`, whose `spawnAfterBreak` lets
                        // the silverfish out.
                        world.break_block(&test_pos, None, BlockFlags::NOTIFY_ALL);
                    } else if let Some(host) = InfestedBlock::host_state_by_infested(block, state) {
                        world.set_block_state(&test_pos, host, BlockFlags::NOTIFY_ALL);
                    }
                    if random.random::<bool>() {
                        return;
                    }
                }
            }
        }
    }
}

/// Vanilla `Silverfish.SilverfishMergeWithStoneGoal`: a random stroll that, now and then,
/// hides the silverfish in a neighbouring stone block instead.
struct SilverfishMergeWithStoneGoal {
    stroll: WanderAroundGoal,
    selected_direction: Option<BlockDirection>,
    do_merge: bool,
}

impl SilverfishMergeWithStoneGoal {
    const fn new() -> Self {
        Self {
            stroll: WanderAroundGoal::with_interval(1.0, 10),
            selected_direction: None,
            do_merge: false,
        }
    }

    /// `BlockPos.containing(x, y + 0.5, z).relative(selectedDirection)`.
    fn merge_pos(mob: &dyn Mob, direction: BlockDirection) -> BlockPos {
        let pos = mob.get_entity().pos.load();
        BlockPos::new(
            pos.x.floor() as i32,
            (pos.y + 0.5).floor() as i32,
            pos.z.floor() as i32,
        )
        .offset(direction.to_offset())
    }

    fn host_at(mob: &dyn Mob, pos: &BlockPos) -> Option<BlockStateId> {
        let (block, state) = mob.get_entity().world.load().get_block_and_state_id(pos);
        InfestedBlock::is_compatible_host_block(block)
            .then(|| InfestedBlock::infested_state_by_host(block, state))
            .flatten()
    }
}

impl Goal for SilverfishMergeWithStoneGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let mob_entity = mob.get_mob_entity();
        if mob_entity.get_target().is_some()
            || !mob_entity
                .navigator
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_done()
        {
            return false;
        }
        let mut random = mob.get_random();
        if mob
            .get_entity()
            .world
            .load()
            .level_info
            .load()
            .game_rules
            .mob_griefing
            && random.random_range(0..to_goal_ticks(10)) == 0
        {
            // `Direction.getRandom`: down, up, north, south, west, east.
            let direction = BlockDirection::all()[random.random_range(0..6)];
            self.selected_direction = Some(direction);
            if Self::host_at(mob, &Self::merge_pos(mob, direction)).is_some() {
                self.do_merge = true;
                return true;
            }
        }
        self.do_merge = false;
        self.stroll.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        !self.do_merge && self.stroll.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        if !self.do_merge {
            self.stroll.start(mob);
            return;
        }
        let Some(direction) = self.selected_direction else {
            return;
        };
        let pos = Self::merge_pos(mob, direction);
        if let Some(infested) = Self::host_at(mob, &pos) {
            let entity = mob.get_entity();
            let world = entity.world.load_full();
            world.set_block_state(&pos, infested, BlockFlags::NOTIFY_ALL);
            // `spawnAnim`, then `discard`.
            world.send_entity_status(entity, EntityStatus::SilverfishMergeAnim, None);
            entity.remove();
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.stroll.stop(mob);
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}
