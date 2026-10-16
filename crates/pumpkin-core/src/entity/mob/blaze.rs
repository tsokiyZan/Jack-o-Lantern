use crate::entity::ai::pathfinder::node::PathType;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;

use crate::entity::ai::util::RandomExt;

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::ActiveTargetGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, move_towards_restriction::MoveTowardsRestrictionGoal,
        revenge::RevengeGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
};

pub struct BlazeEntity {
    pub entity: Arc<MobEntity>,
    pub is_charged: AtomicBool,
    allowed_height_offset: AtomicCell<f32>,
    next_height_offset_change_tick: AtomicI32,
}

impl BlazeEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = Arc::new(MobEntity::new(entity));
        // Vanilla `Blaze`'s constructor.
        entity.set_pathfinding_malus(PathType::Water, -1.0);
        entity.set_pathfinding_malus(PathType::Lava, 8.0);
        entity.set_pathfinding_malus(PathType::DangerFire, 0.0);
        entity.set_pathfinding_malus(PathType::DamageFire, 0.0);
        let blaze = Self {
            entity,
            is_charged: AtomicBool::new(false),
            allowed_height_offset: AtomicCell::new(0.5),
            next_height_offset_change_tick: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(blaze);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };
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

            goal_selector.add_goal(
                4,
                Box::new(
                    crate::entity::ai::goal::blaze_attack::BlazeShootFireballGoal::new(
                        Arc::downgrade(&mob_arc),
                    ),
                ),
            );

            goal_selector.add_goal(5, Box::new(MoveTowardsRestrictionGoal::new(1.0)));
            goal_selector.add_goal(
                7,
                Box::new(WanderAroundGoal::avoiding_water_with_probability(1.0, 0.0)),
            );
            goal_selector.add_goal(
                8,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(1, Box::new(RevengeGoal::new(true).alerting_others()));
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.entity, &EntityType::PLAYER, true),
            );
        };

        mob_arc
    }

    pub fn is_charged(&self) -> bool {
        self.is_charged.load(Ordering::Relaxed)
    }

    pub fn set_charged(&self, charged: bool) {
        self.is_charged.store(charged, Ordering::Relaxed);
        let flags = i8::from(charged);
        self.entity
            .living_entity
            .entity
            .set_synced_data(pumpkin_data::tracked_data::blaze::DATA_FLAGS_ID, flags);
    }
}

impl Mob for BlazeEntity {
    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity
    }

    /// Vanilla `Blaze.aiStep` before `super.aiStep()`: falling slows to 0.6 per tick.
    fn ai_step(&self, _caller: &dyn EntityBase) {
        let base_entity = &self.entity.living_entity.entity;
        let vel = base_entity.velocity.load();
        if !base_entity.on_ground.load(Ordering::Relaxed) && vel.y < 0.0 {
            base_entity
                .velocity
                .store(Vector3::new(vel.x, vel.y * 0.6, vel.z));
        }
    }

    /// Vanilla `Blaze.customServerAiStep`: rise towards a target above the allowed height offset.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        if self
            .next_height_offset_change_tick
            .fetch_sub(1, Ordering::Relaxed)
            - 1
            <= 0
        {
            self.next_height_offset_change_tick
                .store(100, Ordering::Relaxed);
            self.allowed_height_offset
                .store(self.get_random().triangle(0.5, 6.891) as f32);
        }

        let Some(target) = self.entity.get_target() else {
            return;
        };
        let base_entity = &self.entity.living_entity.entity;
        if target.get_entity().get_eye_y()
            > base_entity.get_eye_y() + f64::from(self.allowed_height_offset.load())
            && self.can_attack(target.as_ref())
        {
            let vel = base_entity.velocity.load();
            let up = (f64::from(0.3f32) - vel.y) * f64::from(0.3f32);
            base_entity
                .velocity
                .store(Vector3::new(vel.x, vel.y + up, vel.z));
            // `needsSync`.
            base_entity.velocity_dirty.store(true, Ordering::SeqCst);
        }
    }

    fn is_sensitive_to_water(&self) -> bool {
        true
    }
}
