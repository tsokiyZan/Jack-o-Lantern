use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::enchantment::EnchantmentHelper;
use crate::entity::combat;
use crate::entity::damage_source::DamageSource;
use crate::entity::mob::Mob;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use crate::world::World;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::{
    AttackRangeImpl, AttributeModifiersImpl, EquipmentSlot, KineticWeaponImpl, Operation,
    PiercingWeaponImpl, WeaponImpl,
};
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::tag;
use pumpkin_data::tag::Taggable;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::{GameMode, Hand};

pub struct SpearItem;

impl ItemMetadata for SpearItem {
    fn ids() -> Box<[u16]> {
        tag::Item::MINECRAFT_SPEARS.1.into()
    }
}

impl ItemBehaviour for SpearItem {
    fn on_spear_jab(&self, stack: &ItemStack, player: &Player) {
        let Some(piercing) = stack.get_data_component::<PiercingWeaponImpl>() else {
            return;
        };
        let world = player.world();
        let Some(server) = world.server.upgrade() else {
            return;
        };

        let damage = Self::attack_damage(player, stack) as f32;
        let mut hit_something = false;
        for target in Self::targets_in_range(player, &server, stack) {
            hit_something |= Self::stab_attack(
                player,
                &server,
                Hand::Right,
                stack,
                &target,
                damage,
                StabEffects {
                    damage: true,
                    knockback: piercing.deals_knockback,
                    dismount: piercing.dismounts,
                },
            );
        }
        // `onAttack`.
        player.reset_attack_strength_ticker();
        player.living_entity.post_piercing_attack(player);

        let position = player.position();
        if hit_something && let Some(sound) = piercing.hit_sound.as_ref() {
            world.play_sound_event(sound, SoundCategory::Players, &position);
        }
        if let Some(sound) = piercing.sound.as_ref() {
            world.play_sound_event_expect(player, sound, SoundCategory::Players, &position);
        }
        player.swing_and_reset_attack_strength(Hand::Right, stack.get_attack_animation(), false);
    }

    fn on_use_tick(&self, stack: &ItemStack, player: &Player, remaining_use_ticks: i32) {
        let active_hand = *player
            .living_entity
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(hand) = active_hand else {
            return;
        };
        let held = player.inventory().get_stack_in_hand(hand);
        if held.item.id != stack.item.id {
            player.living_entity.clear_active_hand();
            return;
        }
        Self::kinetic_attack(&held, player, hand, remaining_use_ticks);
    }

    fn get_use_duration(&self, _stack: &ItemStack) -> i32 {
        Self::USE_DURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone, Copy)]
struct StabEffects {
    damage: bool,
    knockback: bool,
    dismount: bool,
}

/// Vanilla `KineticWeapon.damageEntities`'s `actionFactor` for a player.
const PLAYER_ACTION_FACTOR: f64 = 1.0;
/// Vanilla `KineticWeapon.damageEntities`'s `actionFactor` for anything else, `0.2F`.
const MOB_ACTION_FACTOR: f32 = 0.2;

impl SpearItem {
    const USE_DURATION: i32 = 72_000;

    fn stab_attack(
        player: &Player,
        server: &Server,
        hand: Hand,
        stack: &ItemStack,
        target: &Arc<dyn EntityBase>,
        base_damage: f32,
        effects: StabEffects,
    ) -> bool {
        let target_entity = target.get_entity();
        let world = player.world();
        let damage_source = DamageSource {
            damage_type: DamageType::SPEAR,
            direct: Some(player),
            causing: Some(player),
        };
        let mut base_damage = base_damage;
        // Vanilla `getEnchantedDamage(target, baseDamage, source) - baseDamage`.
        let mut magic_boost = EnchantmentHelper::modify_damage(
            &world,
            stack,
            target.as_ref(),
            &damage_source,
            base_damage,
        ) - base_damage;
        if !Self::is_using_hand(player, hand) {
            let charge = player.get_attack_strength_scale(0.5);
            magic_boost *= charge;
            base_damage *= 0.2 + charge * charge * 0.8;
        }

        let total_damage = if effects.damage {
            base_damage + magic_boost
        } else {
            0.0
        };
        let was_hurt = effects.damage
            && target.damage_with_context(
                target.as_ref(),
                total_damage,
                damage_source.damage_type,
                None,
                damage_source.direct,
                damage_source.causing,
            );

        let config = &server.advanced_config.pvp;
        if effects.knockback && config.knockback && target.get_living_entity().is_some() {
            let attacker = player.get_entity();
            combat::handle_knockback(attacker, target.as_ref(), 0.8, total_damage, false);
            let knockback_level = EnchantmentHelper::modify_knockback(
                &world,
                stack,
                target.as_ref(),
                &damage_source,
                0.0,
            );
            if knockback_level > 0.0 {
                combat::handle_knockback(
                    attacker,
                    target.as_ref(),
                    f64::from(knockback_level),
                    total_damage,
                    true,
                );
            }
            target_entity.send_velocity();
        }

        let mut dismounted = false;
        if effects.dismount
            && let Some(vehicle) = target_entity.get_vehicle()
        {
            dismounted = true;
            vehicle
                .get_entity()
                .remove_passenger(target_entity.entity_id);
        }

        if !was_hurt && !effects.knockback && !dismounted {
            return false;
        }

        player
            .living_entity
            .last_attacking_id
            .store(target_entity.entity_id, Ordering::Relaxed);
        player.living_entity.last_attack_time.store(
            player.get_entity().tick_count.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        // `itemAttackInteraction`.
        if was_hurt {
            EnchantmentHelper::do_post_attack_effects_with_item_source(
                &player.world(),
                target.as_ref(),
                &damage_source,
                Some(stack),
            );
        }
        if target.get_living_entity().is_some()
            && let Some(weapon) = stack.get_data_component::<WeaponImpl>()
        {
            let slot = if hand == Hand::Right {
                EquipmentSlot::MAIN_HAND
            } else {
                EquipmentSlot::OFF_HAND
            };
            player.damage_item_in_slot(&slot, weapon.item_damage_per_attack as i32);
        }
        player.add_exhaustion(0.1);
        true
    }

    fn kinetic_attack(stack: &ItemStack, player: &Player, hand: Hand, remaining_use_ticks: i32) {
        let Some(weapon) = stack.get_data_component::<KineticWeaponImpl>() else {
            return;
        };
        let ticks_used = Self::USE_DURATION - remaining_use_ticks - weapon.delay_ticks;
        if ticks_used < 0 {
            return;
        }
        let world = player.world();
        let Some(server) = world.server.upgrade() else {
            return;
        };

        let look = Self::look_vector(player);
        let attacker_speed = look.dot(&Self::known_speed(player.get_entity()));
        let base_damage = player
            .living_entity
            .get_attribute_base(&Attributes::ATTACK_DAMAGE) as f32;
        let now = player.get_entity().tick_count.load(Ordering::Relaxed);
        let mut affected = false;

        for target in Self::targets_in_range(player, &server, stack) {
            let target_entity = target.get_entity();
            if player.living_entity.was_recently_stabbed(
                target_entity.entity_id,
                now,
                weapon.contact_cooldown_ticks,
            ) {
                continue;
            }
            player
                .living_entity
                .remember_stabbed_entity(target_entity.entity_id, now);

            let target_speed = look.dot(&Self::known_speed(target_entity));
            let relative_speed = (attacker_speed - target_speed).max(0.0);
            let effects = StabEffects {
                damage: weapon.damage_conditions.as_ref().is_some_and(|condition| {
                    condition.test(
                        ticks_used,
                        attacker_speed,
                        relative_speed,
                        PLAYER_ACTION_FACTOR,
                    )
                }),
                knockback: weapon
                    .knockback_conditions
                    .as_ref()
                    .is_some_and(|condition| {
                        condition.test(
                            ticks_used,
                            attacker_speed,
                            relative_speed,
                            PLAYER_ACTION_FACTOR,
                        )
                    }),
                dismount: weapon
                    .dismount_conditions
                    .as_ref()
                    .is_some_and(|condition| {
                        condition.test(
                            ticks_used,
                            attacker_speed,
                            relative_speed,
                            PLAYER_ACTION_FACTOR,
                        )
                    }),
            };
            if !effects.damage && !effects.knockback && !effects.dismount {
                continue;
            }

            let damage =
                base_damage + (relative_speed * f64::from(weapon.damage_multiplier)).floor() as f32;
            affected |= Self::stab_attack(player, &server, hand, stack, &target, damage, effects);
        }

        if affected {
            world.send_entity_status(player.get_entity(), EntityStatus::KineticHit, None);
        }
    }

    /// Vanilla `LivingEntity.updatingUsingItem` and `ItemStack.onUseTick` for a mob holding
    /// a kinetic weapon up: the use stops once the hand no longer holds it.
    pub fn mob_use_tick(mob: &dyn Mob) {
        let living = &mob.get_mob_entity().living_entity;
        let remaining_use_ticks = living.item_use_time.load(Ordering::Relaxed);
        if remaining_use_ticks <= 0 {
            return;
        }
        let Some(use_item) = living
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        else {
            return;
        };
        if use_item.get_data_component::<KineticWeaponImpl>().is_none() {
            return;
        }
        let hand = living
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .unwrap_or(Hand::Right);
        let slot = if hand == Hand::Right {
            EquipmentSlot::MAIN_HAND
        } else {
            EquipmentSlot::OFF_HAND
        };
        let held = mob.get_mob_entity().get_item_slot(&slot);
        if held.item.id != use_item.item.id {
            living.clear_active_hand();
            return;
        }
        Self::mob_kinetic_attack(mob, &held, remaining_use_ticks);
    }

    /// Vanilla `KineticWeapon.damageEntities` for a mob.
    fn mob_kinetic_attack(mob: &dyn Mob, stack: &ItemStack, remaining_use_ticks: i32) {
        let Some(weapon) = stack.get_data_component::<KineticWeaponImpl>() else {
            return;
        };
        let ticks_used = Self::USE_DURATION - remaining_use_ticks - weapon.delay_ticks;
        if ticks_used < 0 {
            return;
        }
        let living = &mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        let world = entity.world.load_full();

        let look = entity.get_look_angle();
        let attacker_speed = look.dot(&Self::known_speed(entity));
        let action_factor = f64::from(MOB_ACTION_FACTOR);
        let base_damage = living.get_attribute_base(&Attributes::ATTACK_DAMAGE) as f32;
        let now = entity.tick_count.load(Ordering::Relaxed);
        let mut affected = false;

        for target in Self::mob_targets_in_range(mob, stack) {
            let target_entity = target.get_entity();
            if living.was_recently_stabbed(
                target_entity.entity_id,
                now,
                weapon.contact_cooldown_ticks,
            ) {
                continue;
            }
            living.remember_stabbed_entity(target_entity.entity_id, now);

            let target_speed = look.dot(&Self::known_speed(target_entity));
            let relative_speed = (attacker_speed - target_speed).max(0.0);
            let passes = |condition: &Option<_>| {
                condition.as_ref().is_some_and(
                    |condition: &pumpkin_data::data_component_impl::KineticConditionImpl| {
                        condition.test(ticks_used, attacker_speed, relative_speed, action_factor)
                    },
                )
            };
            let effects = StabEffects {
                damage: passes(&weapon.damage_conditions),
                knockback: passes(&weapon.knockback_conditions),
                dismount: passes(&weapon.dismount_conditions),
            };
            if !effects.damage && !effects.knockback && !effects.dismount {
                continue;
            }

            let damage =
                base_damage + (relative_speed * f64::from(weapon.damage_multiplier)).floor() as f32;
            affected |= Self::mob_stab_attack(mob, stack, &target, damage, effects);
        }

        if affected {
            world.send_entity_status(entity, EntityStatus::KineticHit, None);
        }
    }

    /// Vanilla `ProjectileUtil.getHitEntitiesAlong` through a mob's `AttackRange`, scaled by
    /// its `mob_factor`.
    fn mob_targets_in_range(mob: &dyn Mob, stack: &ItemStack) -> Vec<Arc<dyn EntityBase>> {
        let living = &mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        // Vanilla `AttackRange.defaultFor` when the item has none.
        let (min_reach, max_reach, hitbox_margin) =
            stack.get_data_component::<AttackRangeImpl>().map_or_else(
                || {
                    let interaction_range =
                        living.get_attribute_value(&Attributes::ENTITY_INTERACTION_RANGE) as f32;
                    (0.0, interaction_range, 0.0)
                },
                |range| {
                    (
                        range.min_reach * range.mob_factor,
                        range.max_reach * range.mob_factor,
                        range.hitbox_margin,
                    )
                },
            );
        let world = entity.world.load();
        // `getHeadLookAngle`.
        let look = Entity::calculate_view_vector(entity.pitch.load(), entity.head_yaw.load());
        let eye = entity.get_eye_pos();
        let from = eye.add(&(look * f64::from(min_reach)));
        // A mob's `getKnownMovement` is its velocity.
        let forward = entity.velocity.load().dot(&look).max(0.0);
        let to = eye.add(&(look * (f64::from(max_reach) + forward)));
        Self::entities_along(&world, eye, from, to, hitbox_margin, |target| {
            Self::can_be_stabbed(entity, target)
        })
    }

    /// Vanilla `LivingEntity.stabAttack`, which mobs use. Their weapon does not wear down, as
    /// for any other mob attack in Pumpkin.
    fn mob_stab_attack(
        mob: &dyn Mob,
        stack: &ItemStack,
        target: &Arc<dyn EntityBase>,
        base_damage: f32,
        effects: StabEffects,
    ) -> bool {
        let living = &mob.get_mob_entity().living_entity;
        let attacker = &living.entity;
        let target_entity = target.get_entity();
        let world = attacker.world.load_full();
        let damage_source = DamageSource {
            damage_type: DamageType::SPEAR,
            direct: Some(mob),
            causing: Some(mob),
        };
        let post_enchantment_damage = EnchantmentHelper::modify_damage(
            &world,
            stack,
            target.as_ref(),
            &damage_source,
            base_damage,
        );
        let mut affected = effects.knockback;
        let dealt_damage = effects.damage
            && target.damage_with_context(
                target.as_ref(),
                post_enchantment_damage,
                damage_source.damage_type,
                None,
                damage_source.direct,
                damage_source.causing,
            );
        affected |= dealt_damage;
        if effects.knockback && target.get_living_entity().is_some() {
            combat::handle_knockback(
                attacker,
                target.as_ref(),
                0.8,
                post_enchantment_damage,
                false,
            );
            // `getKnockback`, which `handle_knockback` halves.
            let knockback = f64::from(EnchantmentHelper::modify_knockback(
                &world,
                stack,
                target.as_ref(),
                &damage_source,
                living.get_attribute_value(&Attributes::ATTACK_KNOCKBACK) as f32,
            ));
            if knockback > 0.0 {
                combat::handle_knockback(
                    attacker,
                    target.as_ref(),
                    knockback,
                    post_enchantment_damage,
                    true,
                );
            }
            target_entity.send_velocity();
        }

        if effects.dismount
            && !target_entity
                .entity_type
                .has_tag(&tag::EntityType::MINECRAFT_CANNOT_BE_DISMOUNTED_BY_ITEM_USAGE)
            && let Some(vehicle) = target_entity.get_vehicle()
        {
            affected = true;
            vehicle
                .get_entity()
                .remove_passenger(target_entity.entity_id);
        }

        if dealt_damage {
            EnchantmentHelper::do_post_attack_effects(
                &attacker.world.load_full(),
                target.as_ref(),
                &damage_source,
            );
        }
        if !affected {
            return false;
        }

        // `setLastHurtMob`.
        living
            .last_attacking_id
            .store(target_entity.entity_id, Ordering::Relaxed);
        living.last_attack_time.store(
            attacker.tick_count.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        true
    }

    fn look_vector(player: &Player) -> Vector3<f64> {
        let (yaw, pitch) = player.rotation();
        Vector3::rotation_vector(f64::from(pitch), f64::from(yaw))
    }

    fn known_speed(entity: &Entity) -> Vector3<f64> {
        let mut movement = entity.movement.load();
        if entity.entity_type != &EntityType::PLAYER {
            let mut vehicle = entity.get_vehicle();
            while let Some(current) = vehicle {
                movement = current.get_entity().movement.load();
                vehicle = current.get_entity().get_vehicle();
            }
        }
        movement * 20.0
    }

    fn is_using_hand(player: &Player, hand: Hand) -> bool {
        let living = &player.living_entity;
        living
            .item_in_use
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
            && *living
                .active_hand
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                == Some(hand)
    }

    fn attack_damage(player: &Player, stack: &ItemStack) -> f64 {
        let mut damage = Self::attribute_with_item_modifier(
            player,
            stack,
            &Attributes::ATTACK_DAMAGE,
            "minecraft:base_attack_damage",
        );
        if let Some(strength) = player.living_entity.get_effect(&StatusEffect::STRENGTH) {
            damage += 3.0 * (f64::from(strength.amplifier) + 1.0);
        }
        if let Some(weakness) = player.living_entity.get_effect(&StatusEffect::WEAKNESS) {
            damage -= 4.0 * (f64::from(weakness.amplifier) + 1.0);
        }
        damage.max(0.0)
    }

    fn attribute_with_item_modifier(
        player: &Player,
        stack: &ItemStack,
        attribute: &Attributes,
        modifier_id: &str,
    ) -> f64 {
        let living = &player.living_entity;
        let value = living.get_attribute_value(attribute);
        let already_applied = living
            .attributes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&attribute.id)
            .is_some_and(|instance| {
                instance
                    .modifiers
                    .iter()
                    .any(|modifier| modifier.id == modifier_id)
            });
        if already_applied {
            return value;
        }
        let Some(modifiers) = stack.get_data_component::<AttributeModifiersImpl>() else {
            return value;
        };
        modifiers
            .attribute_modifiers
            .iter()
            .filter(|modifier| {
                modifier.id == modifier_id && modifier.operation == Operation::AddValue
            })
            .fold(value, |value, modifier| value + modifier.amount)
    }

    fn targets_in_range(
        player: &Player,
        server: &Server,
        stack: &ItemStack,
    ) -> Vec<Arc<dyn EntityBase>> {
        let Some(range) = stack.get_data_component::<AttackRangeImpl>() else {
            return Vec::new();
        };
        let world = player.world();
        let look = Self::look_vector(player);
        let eye = player.eye_position();
        let (min_reach, max_reach) = if player.gamemode.load() == GameMode::Creative {
            (
                f64::from(range.min_creative_reach),
                f64::from(range.max_creative_reach),
            )
        } else {
            (f64::from(range.min_reach), f64::from(range.max_reach))
        };
        let from = eye.add(&(look * min_reach));
        let forward = player.get_entity().movement.load().dot(&look).max(0.0);
        let to = eye.add(&(look * (max_reach + forward)));
        Self::entities_along(&world, eye, from, to, range.hitbox_margin, |target| {
            Self::can_hit(player, server, target)
        })
    }

    /// Vanilla `ProjectileUtil.getHitEntitiesAlong` from the attacker's eye, `from` and `to`
    /// already offset by the reach.
    fn entities_along(
        world: &World,
        eye: Vector3<f64>,
        from: Vector3<f64>,
        mut to: Vector3<f64>,
        hitbox_margin: f32,
        can_hit: impl Fn(&dyn EntityBase) -> bool,
    ) -> Vec<Arc<dyn EntityBase>> {
        if let Some(block_hit) = Self::clip_blocks(world, eye, to) {
            if eye.squared_distance_to_vec(&block_hit) < eye.squared_distance_to_vec(&from) {
                return Vec::new();
            }
            to = block_hit;
        }

        let margin = f64::from(hitbox_margin);
        let half_margin = Vector3::new(margin / 2.0, margin / 2.0, margin / 2.0);
        let delta = to.sub(&from);
        let search_box = BoundingBox::new(from.sub(&half_margin), from.add(&half_margin))
            .expand_towards(delta.x, delta.y, delta.z)
            .expand_all(1.0);

        world
            .get_all_at_box(&search_box)
            .into_iter()
            .filter(|target| {
                can_hit(target.as_ref())
                    && Self::hits_entity(
                        world,
                        &target.get_entity().bounding_box.load(),
                        from,
                        to,
                        margin,
                    )
            })
            .collect()
    }

    fn hits_entity(
        world: &World,
        bounding_box: &BoundingBox,
        from: Vector3<f64>,
        to: Vector3<f64>,
        margin: f64,
    ) -> bool {
        if bounding_box.contains(from) || bounding_box.clip(from, to).is_some() {
            return true;
        }
        let Some(outside_hit) = bounding_box.expand_all(margin).clip(from, to) else {
            return false;
        };
        let mut towards_target = bounding_box.min.add(&bounding_box.max) * 0.5;
        if let Some(block_hit) = Self::clip_blocks(world, outside_hit, towards_target) {
            towards_target = block_hit;
        }
        bounding_box.clip(outside_hit, towards_target).is_some()
    }

    fn clip_blocks(world: &World, from: Vector3<f64>, to: Vector3<f64>) -> Option<Vector3<f64>> {
        if from == to {
            return None;
        }
        let start = from.lerp(&to, -1.0e-7);
        let end = to.lerp(&from, -1.0e-7);
        let mut block = BlockPos::floored(start.x, start.y, start.z);
        if let Some(hit) = Self::clip_block(world, &block, from, to) {
            return Some(hit);
        }

        let difference = end.sub(&start);
        let step = difference.sign();
        let delta = Vector3::new(
            if step.x == 0 {
                f64::MAX
            } else {
                f64::from(step.x) / difference.x
            },
            if step.y == 0 {
                f64::MAX
            } else {
                f64::from(step.y) / difference.y
            },
            if step.z == 0 {
                f64::MAX
            } else {
                f64::from(step.z) / difference.z
            },
        );
        let mut next = Vector3::new(
            delta.x
                * (if step.x > 0 {
                    1.0 - (start.x - start.x.floor())
                } else {
                    start.x - start.x.floor()
                }),
            delta.y
                * (if step.y > 0 {
                    1.0 - (start.y - start.y.floor())
                } else {
                    start.y - start.y.floor()
                }),
            delta.z
                * (if step.z > 0 {
                    1.0 - (start.z - start.z.floor())
                } else {
                    start.z - start.z.floor()
                }),
        );

        while next.x <= 1.0 || next.y <= 1.0 || next.z <= 1.0 {
            if next.x < next.y && next.x < next.z {
                block.0.x += step.x;
                next.x += delta.x;
            } else if next.y < next.z {
                block.0.y += step.y;
                next.y += delta.y;
            } else {
                block.0.z += step.z;
                next.z += delta.z;
            }
            if let Some(hit) = Self::clip_block(world, &block, from, to) {
                return Some(hit);
            }
        }
        None
    }

    fn clip_block(
        world: &World,
        pos: &BlockPos,
        from: Vector3<f64>,
        to: Vector3<f64>,
    ) -> Option<Vector3<f64>> {
        world
            .get_block_state(pos)
            .get_block_collision_shapes_at(pos)
            .filter_map(|shape| shape.shift(pos.0.to_f64()).clip(from, to))
            .min_by(|a, b| {
                from.squared_distance_to_vec(a)
                    .total_cmp(&from.squared_distance_to_vec(b))
            })
    }

    fn can_hit(player: &Player, server: &Server, target: &dyn EntityBase) -> bool {
        if let Some(other) = target.get_player() {
            let config = &server.advanced_config.pvp;
            if !config.enabled
                || (config.protect_creative && other.gamemode.load() == GameMode::Creative)
            {
                return false;
            }
        }
        Self::can_be_stabbed(player.get_entity(), target)
    }

    /// Vanilla `PiercingWeapon.canHitEntity` without the player-against-player rules.
    fn can_be_stabbed(attacker: &Entity, target: &dyn EntityBase) -> bool {
        let entity = target.get_entity();
        if entity.entity_id == attacker.entity_id
            || entity.is_removed()
            || entity.is_invulnerable()
            || target.is_spectator()
        {
            return false;
        }
        let Some(living) = target.get_living_entity() else {
            return false;
        };
        if living.health.load() <= 0.0 {
            return false;
        }
        Self::root_vehicle_id(attacker) != Self::root_vehicle_id(entity)
    }

    fn root_vehicle_id(entity: &Entity) -> i32 {
        let mut id = entity.entity_id;
        let mut vehicle = entity.get_vehicle();
        while let Some(current) = vehicle {
            id = current.get_entity().entity_id;
            vehicle = current.get_entity().get_vehicle();
        }
        id
    }
}
