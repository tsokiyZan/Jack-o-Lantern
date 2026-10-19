use std::sync::atomic::{AtomicI32, Ordering::Relaxed};
use std::sync::{Arc, Weak};

use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;

use crate::entity::mob::raider::{add_raider_goals, is_abstract_villager, raider_hurt_by_target};
use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::Sound;
use pumpkin_nbt::compound::NbtCompound;

use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::ActiveTargetGoal, look_at_entity::LookAtEntityGoal,
        melee_attack::MeleeAttackGoal, swim::SwimGoal, wander_around::WanderAroundGoal,
    },
    mob::{
        Mob, MobEntity,
        patrol::{PatrolData, PatrollingMonster},
        raider::{Raider, RaiderData},
    },
};

/// Vanilla `Ravager.ATTACK_DURATION`.
const ATTACK_DURATION: i32 = 10;
/// Vanilla `Ravager.STUN_DURATION`.
const STUN_DURATION: i32 = 40;
/// Vanilla `Ravager.BASE_MOVEMENT_SPEED`.
const BASE_MOVEMENT_SPEED: f64 = 0.3;
/// Vanilla `Ravager.ATTACK_MOVEMENT_SPEED`.
const ATTACK_MOVEMENT_SPEED: f64 = 0.35;

pub struct RavagerEntity {
    pub mob_entity: MobEntity,
    pub raider_data: RaiderData,
    attack_tick: AtomicI32,
    stunned_tick: AtomicI32,
    roar_tick: AtomicI32,
}

impl RavagerEntity {
    #[must_use]
    pub fn new(entity: Entity) -> Arc<Self> {
        let mut mob_entity = MobEntity::new(entity);
        // Vanilla `Ravager`'s constructor.
        mob_entity.set_pathfinding_malus(PathType::Leaves, 0.0);
        // `getAttackBoundingBox` deflates the default box by 0.05 sideways.
        mob_entity.attack_box_deflate = 0.05;
        let ravager = Self {
            mob_entity,
            raider_data: RaiderData::default(),
            attack_tick: AtomicI32::new(0),
            stunned_tick: AtomicI32::new(0),
            roar_tick: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(ravager);
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
                4,
                Box::new(MeleeAttackGoal::new(1.0, true).attacking_with(Self::do_hurt_target)),
            );
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::avoiding_water(0.4)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(10, LookAtEntityGoal::at_mobs(8.0));
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            target_selector.add_goal(2, raider_hurt_by_target());
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true),
            );
            target_selector.add_goal(
                4,
                ActiveTargetGoal::predicated(&mob_arc.mob_entity, 10, true, |target, world| {
                    is_abstract_villager(target, world) && !target.entity.is_baby()
                }),
            );
            target_selector.add_goal(
                4,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::IRON_GOLEM, true),
            );
        };

        mob_arc
    }

    /// `MeleeAttackGoal`'s swing, then vanilla `Ravager.doHurtTarget`.
    fn do_hurt_target(mob: &dyn Mob, target: &dyn EntityBase) {
        let mob_entity = mob.get_mob_entity();
        let attacker: &dyn EntityBase = mob;
        mob_entity.living_entity.swing_hand();
        if let Some(ravager) = attacker.cast_any().downcast_ref::<Self>() {
            ravager.attack_tick.store(ATTACK_DURATION, Relaxed);
            let entity = ravager.get_entity();
            entity
                .world
                .load()
                .send_entity_status(entity, EntityStatus::StartAttacking, None);
            ravager.play_sound(Sound::EntityRavagerAttack);
        }
        if mob_entity.try_attack(attacker, target) {
            mob.on_attack(target);
        }
    }

    fn play_sound(&self, sound: Sound) {
        let entity = self.get_entity();
        if !entity.silent.load(Relaxed) {
            entity.world.load().play_sound_fine(
                sound,
                pumpkin_data::sound::SoundCategory::Hostile,
                &entity.pos.load(),
                1.0,
                1.0,
            );
        }
    }

    /// Vanilla `stunEffect`: its particle is the client's, but the server draws the numbers.
    fn stun_effect(&self) {
        let mut rng = self.get_random();
        if rng.random_range(0..6) == 0 {
            rng.random::<f64>();
            rng.random::<f64>();
        }
    }

    /// Vanilla `roar`.
    fn roar(&self) {
        let entity = self.get_entity();
        if !entity.is_alive() {
            return;
        }
        let world = entity.world.load_full();
        let griefing = world.level_info.load().game_rules.mob_griefing;
        let area = entity.bounding_box.load().expand_all(4.0);
        let mut targets = world.get_entities_at_box(&area);
        targets.extend(
            world
                .get_players_at_box(&area)
                .into_iter()
                .map(|player| player as Arc<dyn EntityBase>),
        );
        for target in targets {
            let target_entity = target.get_entity();
            // `ROAR_TARGET_WITH_GRIEFING` / `ROAR_TARGET_WITHOUT_GRIEFING`.
            if target.get_living_entity().is_none()
                || target_entity.entity_type == &EntityType::RAVAGER
                || !target_entity.is_alive()
                || (!griefing && target_entity.entity_type == &EntityType::ARMOR_STAND)
            {
                continue;
            }
            if !target_entity
                .entity_type
                .has_tag(&tag::EntityType::MINECRAFT_ILLAGER)
            {
                target.damage_with_context(
                    target.as_ref(),
                    6.0,
                    DamageType::MOB_ATTACK,
                    None,
                    Some(self),
                    Some(self),
                );
            }
            if target.get_player().is_none() {
                self.strong_knockback(target.as_ref());
            }
        }
        world.send_entity_status(entity, EntityStatus::RavagerRoared, None);
    }

    /// Vanilla `strongKnockback`.
    fn strong_knockback(&self, target: &dyn EntityBase) {
        let pos = self.get_entity().pos.load();
        let target_pos = target.get_entity().pos.load();
        let xd = target_pos.x - pos.x;
        let zd = target_pos.z - pos.z;
        let dd = (xd * xd + zd * zd).max(0.001);
        target.push_impulse(Vector3::new(xd / dd * 4.0, 0.2, zd / dd * 4.0));
    }
}

impl Mob for RavagerEntity {
    /// Vanilla `Ravager.aiStep` after `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        if !entity.is_alive() {
            return;
        }
        if self.mob_is_immobile() {
            living.set_attribute_base(&Attributes::MOVEMENT_SPEED, 0.0);
        } else {
            let max_speed = if self.mob_entity.get_target().is_some() {
                ATTACK_MOVEMENT_SPEED
            } else {
                BASE_MOVEMENT_SPEED
            };
            let base = living.get_attribute_base(&Attributes::MOVEMENT_SPEED);
            living.set_attribute_base(&Attributes::MOVEMENT_SPEED, base + 0.1 * (max_speed - base));
        }
        let world = entity.world.load_full();
        if entity.horizontal_collision.load(Relaxed)
            && world.level_info.load().game_rules.mob_griefing
        {
            let mut destroyed_block = false;
            let area = entity.bounding_box.load().expand_all(0.2);
            for x in area.min.x.floor() as i32..=area.max.x.floor() as i32 {
                for y in area.min.y.floor() as i32..=area.max.y.floor() as i32 {
                    for z in area.min.z.floor() as i32..=area.max.z.floor() as i32 {
                        let pos = BlockPos::new(x, y, z);
                        if world.get_block(&pos).has_tag(&tag::Block::MINECRAFT_LEAVES) {
                            // `destroyBlock(pos, true, this)`.
                            destroyed_block |= world
                                .break_block(&pos, None, BlockFlags::NOTIFY_ALL)
                                .is_some();
                        }
                    }
                }
            }
            if !destroyed_block && entity.on_ground.load(Relaxed) {
                self.jump_from_ground();
            }
        }
        if self.roar_tick.load(Relaxed) > 0 && self.roar_tick.fetch_sub(1, Relaxed) - 1 == 10 {
            self.roar();
        }
        if self.attack_tick.load(Relaxed) > 0 {
            self.attack_tick.fetch_sub(1, Relaxed);
        }
        if self.stunned_tick.load(Relaxed) > 0 {
            let stunned = self.stunned_tick.fetch_sub(1, Relaxed) - 1;
            self.stun_effect();
            if stunned == 0 {
                self.play_sound(Sound::EntityRavagerRoar);
                self.roar_tick.store(20, Relaxed);
            }
        }
    }

    /// Vanilla `Ravager.isImmobile`.
    fn mob_is_immobile(&self) -> bool {
        self.attack_tick.load(Relaxed) > 0
            || self.stunned_tick.load(Relaxed) > 0
            || self.roar_tick.load(Relaxed) > 0
    }

    /// Vanilla `Ravager.hasLineOfSight`: blind while stunned or roaring.
    fn has_line_of_sight(&self, target: &Entity) -> bool {
        if self.stunned_tick.load(Relaxed) > 0 || self.roar_tick.load(Relaxed) > 0 {
            return false;
        }
        self.mob_entity
            .sensing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .has_line_of_sight(&self.mob_entity.living_entity.entity, target)
    }

    /// Vanilla `Ravager.blockedByItem`: a shield stuns it half the time, otherwise throws the
    /// defender back.
    fn blocked_by_item(
        &self,
        defender: &dyn EntityBase,
        _damage_type: DamageType,
        _damage: f32,
        _fully_blocked: bool,
    ) {
        if self.roar_tick.load(Relaxed) != 0 {
            return;
        }
        let stun = self.get_random().random::<f64>() < 0.5;
        let defender_entity = defender.get_entity();
        if stun {
            self.stunned_tick.store(STUN_DURATION, Relaxed);
            self.play_sound(Sound::EntityRavagerStunned);
            let entity = self.get_entity();
            entity
                .world
                .load()
                .send_entity_status(entity, EntityStatus::RavagerStunned, None);
            defender.push(self);
        } else {
            self.strong_knockback(defender);
        }
        defender_entity.send_velocity();
    }

    /// Vanilla `Ravager.getMaxHeadYRot`.
    fn get_max_head_rotation(&self) -> f32 {
        45.0
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
        nbt.put_int("AttackTick", self.attack_tick.load(Relaxed));
        nbt.put_int("StunTick", self.stunned_tick.load(Relaxed));
        nbt.put_int("RoarTick", self.roar_tick.load(Relaxed));
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_raider_nbt(nbt);
        self.attack_tick
            .store(nbt.get_int("AttackTick").unwrap_or(0), Relaxed);
        self.stunned_tick
            .store(nbt.get_int("StunTick").unwrap_or(0), Relaxed);
        self.roar_tick
            .store(nbt.get_int("RoarTick").unwrap_or(0), Relaxed);
    }
}

impl PatrollingMonster for RavagerEntity {
    fn get_patrol_data(&self) -> &PatrolData {
        &self.raider_data.patrol_data
    }

    fn can_be_leader(&self) -> bool {
        false
    }
}

impl Raider for RavagerEntity {
    fn get_raider_data(&self) -> &RaiderData {
        &self.raider_data
    }

    fn get_celebrate_sound(&self) -> Sound {
        Sound::EntityRavagerCelebrate
    }
}
