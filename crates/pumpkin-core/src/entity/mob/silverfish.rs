use crate::entity::ai::goal::climb_on_top_of_powder_snow::ClimbOnTopOfPowderSnowGoal;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityType;

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
    wake_up_friends_ticks: AtomicI32,
}

impl SilverfishEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = Arc::new(MobEntity::new(entity));
        let silverfish = Self {
            entity,
            wake_up_friends_ticks: AtomicI32::new(0),
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
            // TODO: 3 SilverfishWakeUpFriendsGoal and 5 SilverfishMergeWithStoneGoal are not
            // ported.
            goal_selector.add_goal(4, Box::new(MeleeAttackGoal::new(1.0, false)));

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
        if is_compatible_host_block(world.get_block(&pos.down())) {
            10.0
        } else {
            crate::entity::mob::monster_walk_target_value(&world, pos)
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        let entity = &self.entity.living_entity.entity;
        if !entity.is_alive() {
            return;
        }

        let yaw = entity.yaw.load();
        entity.body_yaw.store(yaw);
        entity.head_yaw.store(yaw);

        let ticks = self.wake_up_friends_ticks.load(Ordering::Relaxed);
        if ticks > 0 {
            self.wake_up_friends_ticks
                .store(ticks - 1, Ordering::Relaxed);
        }
    }

    fn on_damage(&self, _damage_type: DamageType, _source: Option<&dyn EntityBase>) {
        if self.wake_up_friends_ticks.load(Ordering::Relaxed) == 0 {
            self.wake_up_friends_ticks.store(20, Ordering::Relaxed);
        }
    }
}

/// Vanilla `InfestedBlock.isCompatibleHostBlock`: the blocks with an infested variant.
fn is_compatible_host_block(block: &pumpkin_data::Block) -> bool {
    use pumpkin_data::Block;
    [
        &Block::STONE,
        &Block::COBBLESTONE,
        &Block::STONE_BRICKS,
        &Block::MOSSY_STONE_BRICKS,
        &Block::CRACKED_STONE_BRICKS,
        &Block::CHISELED_STONE_BRICKS,
        &Block::DEEPSLATE,
    ]
    .contains(&block)
}
