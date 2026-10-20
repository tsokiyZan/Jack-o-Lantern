use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::Hand;
use pumpkin_util::random::RandomImpl;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::entity::ai::goal::{Controls, Goal};
use crate::entity::mob::Mob;
use crate::entity::projectile::arrow::{ArrowEntity, ArrowPickup};
use crate::entity::{Entity, EntityBase};
use crate::item::items::bow::BowItem;

/// Vanilla `RangedAttackMob.performRangedAttack`, given the draw power.
pub type PerformRangedAttackFn = fn(&dyn Mob, &Arc<dyn EntityBase>, f32);

/// Vanilla `RangedBowAttackGoal`: the mob closes in until it sees its target from within
/// range, strafes around it, and looses an arrow after a full draw.
pub struct BowAttackGoal {
    goal_control: Controls,
    speed_modifier: f64,
    attack_interval_min: Arc<AtomicI32>,
    attack_radius_sqr: f32,
    attack_time: i32,
    see_time: i32,
    strafing_clockwise: bool,
    strafing_backwards: bool,
    strafing_time: i32,
    perform_ranged_attack: PerformRangedAttackFn,
}

impl BowAttackGoal {
    #[must_use]
    pub fn new(speed_modifier: f64, attack_interval_min: i32, attack_radius: f32) -> Self {
        Self {
            goal_control: Controls::MOVE | Controls::LOOK,
            speed_modifier,
            attack_interval_min: Arc::new(AtomicI32::new(attack_interval_min)),
            attack_radius_sqr: attack_radius * attack_radius,
            attack_time: -1,
            see_time: 0,
            strafing_clockwise: false,
            strafing_backwards: false,
            strafing_time: -1,
            perform_ranged_attack: |mob, target, power| {
                perform_bow_attack(mob, target, power, |_| {});
            },
        }
    }

    /// Shoots with `perform` instead of a plain arrow, like an `AbstractSkeleton` subclass.
    #[must_use]
    pub const fn shooting_with(mut self, perform: PerformRangedAttackFn) -> Self {
        self.perform_ranged_attack = perform;
        self
    }

    /// Vanilla `setMinAttackInterval`, through a handle the mob keeps while the goal is
    /// in its selector.
    #[must_use]
    pub fn min_attack_interval(&self) -> Arc<AtomicI32> {
        self.attack_interval_min.clone()
    }

    /// Vanilla `isHoldingBow`: `Mob.isHolding(Items.BOW)` checks both hands.
    fn is_holding_bow(mob: &dyn Mob) -> bool {
        let equipment = mob
            .get_mob_entity()
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        equipment.get(&EquipmentSlot::MAIN_HAND).item == &Item::BOW
            || equipment.get(&EquipmentSlot::OFF_HAND).item == &Item::BOW
    }

    fn is_using_item(mob: &dyn Mob) -> bool {
        mob.get_mob_entity()
            .living_entity
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// Vanilla `getTicksUsingItem` for a bow.
    fn ticks_using_item(mob: &dyn Mob) -> i32 {
        BowItem::USE_DURATION
            - mob
                .get_mob_entity()
                .living_entity
                .item_use_time
                .load(Ordering::Relaxed)
    }

    /// Vanilla `startUsingItem(ProjectileUtil.getWeaponHoldingHand(mob, Items.BOW))`.
    fn start_using_bow(mob: &dyn Mob) {
        let living = &mob.get_mob_entity().living_entity;
        let (hand, stack) = weapon_holding_hand(mob, &Item::BOW);
        if !stack.is_empty() && !Self::is_using_item(mob) {
            living.set_active_hand(hand, stack, BowItem::USE_DURATION);
        }
    }

    fn stop_using_item(mob: &dyn Mob) {
        mob.get_mob_entity().living_entity.clear_active_hand();
    }
}

impl Goal for BowAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        mob.get_mob_entity().get_target().is_some() && Self::is_holding_bow(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        (self.can_start(mob) || !mob.is_navigator_idle()) && Self::is_holding_bow(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity().set_attacking(true);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity().set_attacking(false);
        self.see_time = 0;
        self.attack_time = -1;
        Self::stop_using_item(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(target) = mob.get_mob_entity().get_target().clone() else {
            return;
        };
        let mob_entity = mob.get_mob_entity();
        let living = &mob_entity.living_entity;

        let target_pos = target.get_entity().pos.load();
        let target_dist_sqr = living
            .entity
            .pos
            .load()
            .squared_distance_to_vec(&target_pos);
        let has_line_of_sight = mob.has_line_of_sight(target.get_entity());
        let had_line_of_sight = self.see_time > 0;
        if has_line_of_sight != had_line_of_sight {
            self.see_time = 0;
        }
        if has_line_of_sight {
            self.see_time += 1;
        } else {
            self.see_time -= 1;
        }

        {
            let mut navigator = mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if target_dist_sqr <= f64::from(self.attack_radius_sqr) && self.see_time >= 20 {
                navigator.stop();
                self.strafing_time += 1;
            } else {
                // Vanilla `moveTo(Entity, speed)` keeps the old path when none is found.
                if let Some(path) = navigator.create_path(living, target_pos, 1) {
                    navigator.move_to_path(Some(path), self.speed_modifier, living);
                }
                self.strafing_time = -1;
            }
        }

        if self.strafing_time >= 20 {
            if mob.get_entity().random().next_f32() < 0.3 {
                self.strafing_clockwise = !self.strafing_clockwise;
            }
            if mob.get_entity().random().next_f32() < 0.3 {
                self.strafing_backwards = !self.strafing_backwards;
            }
            self.strafing_time = 0;
        }

        if self.strafing_time > -1 {
            if target_dist_sqr > f64::from(self.attack_radius_sqr * 0.75) {
                self.strafing_backwards = false;
            } else if target_dist_sqr < f64::from(self.attack_radius_sqr * 0.25) {
                self.strafing_backwards = true;
            }
            mob_entity
                .move_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .strafe(
                    if self.strafing_backwards { -0.5 } else { 0.5 },
                    if self.strafing_clockwise { 0.5 } else { -0.5 },
                );
            // `getControlledVehicle`: the vehicle this mob steers.
            if let Some(vehicle) = living.entity.get_vehicle()
                && vehicle
                    .controlling_passenger()
                    .is_some_and(|rider| rider.get_entity().entity_id == living.entity.entity_id)
                && let Some(vehicle) = vehicle.get_mob()
            {
                vehicle
                    .get_mob_entity()
                    .look_at(target.as_ref(), 30.0, 30.0);
            }
            mob_entity.look_at(target.as_ref(), 30.0, 30.0);
        } else {
            mob_entity
                .look_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .look_at_entity_with_range(&target, 30.0, 30.0);
        }

        if Self::is_using_item(mob) {
            if !has_line_of_sight && self.see_time < -60 {
                Self::stop_using_item(mob);
            } else if has_line_of_sight {
                let pull_time = Self::ticks_using_item(mob);
                if pull_time >= 20 {
                    Self::stop_using_item(mob);
                    (self.perform_ranged_attack)(
                        mob,
                        &target,
                        BowItem::get_power_for_time(pull_time),
                    );
                    self.attack_time = self.attack_interval_min.load(Ordering::Relaxed);
                }
            }
        } else {
            self.attack_time -= 1;
            if self.attack_time <= 0 && self.see_time >= -60 {
                Self::start_using_bow(mob);
            }
        }
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}

/// Vanilla `ProjectileUtil.getWeaponHoldingHand`: the main hand if it holds `weapon`,
/// otherwise the off hand, with what that hand holds.
#[must_use]
pub fn weapon_holding_hand(mob: &dyn Mob, weapon: &Item) -> (Hand, ItemStack) {
    let equipment = mob
        .get_mob_entity()
        .living_entity
        .entity_equipment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let main_hand = equipment.get(&EquipmentSlot::MAIN_HAND);
    if main_hand.item == weapon {
        (Hand::Right, main_hand)
    } else {
        (Hand::Left, equipment.get(&EquipmentSlot::OFF_HAND))
    }
}

/// Vanilla `Monster.getProjectile` for a bow: an arrow held in either hand, off hand first,
/// or a plain arrow.
fn bow_projectile(mob: &dyn Mob) -> ItemStack {
    let equipment = mob
        .get_mob_entity()
        .living_entity
        .entity_equipment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    [EquipmentSlot::OFF_HAND, EquipmentSlot::MAIN_HAND]
        .iter()
        .map(|slot| equipment.get(slot))
        .find(|stack| !stack.is_empty() && stack.item.has_tag(&tag::Item::MINECRAFT_ARROWS))
        .unwrap_or_else(|| ItemStack::new(1, &Item::ARROW))
}

/// Vanilla `AbstractSkeleton.performRangedAttack` (and the identical
/// `Illusioner.performRangedAttack`); `get_arrow` is what a subclass's `getArrow` adds to
/// `ProjectileUtil.getMobArrow`.
pub fn perform_bow_attack(
    mob: &dyn Mob,
    target: &Arc<dyn EntityBase>,
    power: f32,
    get_arrow: impl FnOnce(&ArrowEntity),
) {
    let entity = mob.get_entity();
    let world = entity.world.load_full();
    let (_, bow_item) = weapon_holding_hand(mob, &Item::BOW);
    let projectile = bow_projectile(mob);

    // `getMobArrow`: the held arrow's own `createArrow`, then `setBaseDamageFromMob`.
    let arrow_entity = Entity::new(
        world.clone(),
        entity.pos.load(),
        ArrowEntity::entity_type_for_item(projectile.item),
    );
    let arrow = ArrowEntity::new_shot_with_weapon(
        arrow_entity,
        entity,
        &projectile,
        &bow_item,
        ArrowPickup::Disallowed,
    );
    let difficulty = world.level_info.load().difficulty as i32;
    arrow.set_base_damage_from_mob(f64::from(power), difficulty);
    get_arrow(&arrow);

    let mob_pos = entity.pos.load();
    let target_entity = target.get_entity();
    let target_pos = target_entity.pos.load();
    let xd = target_pos.x - mob_pos.x;
    let yd = target_pos.y
        + f64::from(target_entity.entity_dimension.load().height) * 0.333_333_333_333_333_3
        - arrow.entity.pos.load().y;
    let zd = target_pos.z - mob_pos.z;
    #[expect(
        clippy::imprecise_flops,
        reason = "vanilla's Math.sqrt rounds like this, hypot doesn't"
    )]
    let distance_to_target = (xd * xd + zd * zd).sqrt();
    // `RangedAttackMob.rangedAttackUncertainty`.
    let uncertainty = 14 - difficulty * 4;
    arrow.set_velocity(
        xd,
        yd + distance_to_target * f64::from(0.2f32),
        zd,
        f64::from(1.6f32),
        f64::from(uncertainty),
    );

    let mut shoot_event =
        crate::plugin::api::events::entity::entity_shoot_bow::EntityShootBowEvent::new(
            entity.entity_id,
            "minecraft:bow".to_string(),
            power,
        );
    if let Some(server) = world.server.upgrade() {
        server
            .plugin_manager
            .fire_blocking(&server, &mut shoot_event);
    }
    if shoot_event.cancelled {
        return;
    }
    // `Projectile.spawnProjectileUsingShoot`: the projectile-spawned enchantment effects
    // follow the spawn.
    let arrow = Arc::new(arrow);
    world.spawn_entity(arrow.clone());
    arrow.apply_on_projectile_spawned(&projectile);

    let pitch = 1.0 / (mob.get_entity().random().next_f32() * 0.4 + 0.8);
    mob.play_sound_at_self(Sound::EntitySkeletonShoot, 1.0, pitch);
}

/// Whether `arrow` is a vanilla `Arrow`, which `getArrow` overrides add effects to; a
/// spectral arrow is not.
#[must_use]
pub fn is_plain_arrow(arrow: &ArrowEntity) -> bool {
    arrow.entity.entity_type == &EntityType::ARROW
}
