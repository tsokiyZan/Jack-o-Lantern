use std::sync::atomic::Ordering;

use crate::enchantment::EnchantmentHelper;
use crate::entity::EntityBase;
use crate::entity::damage_source::DamageSource;
use crate::entity::decoration::armor_stand::ArmorStandEntity;
use crate::entity::living::LivingEntity;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::world::World;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::world::WorldEvent;
use pumpkin_util::GameMode;
use pumpkin_util::math::vector3::Vector3;

const SMASH_ATTACK_FALL_THRESHOLD: f64 = 1.5;
const SMASH_ATTACK_HEAVY_THRESHOLD: f64 = 5.0;
const SMASH_ATTACK_KNOCKBACK_RADIUS: f32 = 3.5;
const SMASH_ATTACK_KNOCKBACK_POWER: f32 = 0.7;

pub struct MaceItem;

impl ItemMetadata for MaceItem {
    fn ids() -> Box<[u16]> {
        [Item::MACE.id].into()
    }
}
impl ItemBehaviour for MaceItem {
    fn can_mine(&self, player: &Player) -> bool {
        player.gamemode.load() != GameMode::Creative
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl MaceItem {
    /// Vanilla `MaceItem.canSmashAttack`.
    #[must_use]
    pub fn can_smash_attack(attacker: &LivingEntity) -> bool {
        attacker.entity.fall_distance.load() > SMASH_ATTACK_FALL_THRESHOLD
            && !attacker.entity.is_fall_flying()
    }

    /// Vanilla `MaceItem.hurtEnemy` for a player's smash: stop the fall, keep the bounce that
    /// follows from counting as a fall, and push everything around the target away.
    pub fn hurt_enemy(victim: &dyn EntityBase, attacker: &Player) {
        let living = &attacker.living_entity;
        if !Self::can_smash_attack(living) {
            return;
        }
        let velocity = living.entity.velocity.load();
        living
            .entity
            .velocity
            .store(Vector3::new(velocity.x, f64::from(0.01f32), velocity.z));
        attacker.send_own_motion();
        living.set_ignore_fall_damage_from_current_impulse(
            true,
            Self::calculate_impact_position(living),
        );

        let world = living.entity.world.load_full();
        let sound = if victim.get_entity().on_ground.load(Ordering::Relaxed) {
            attacker
                .spawn_extra_particles_on_fall
                .store(true, Ordering::Relaxed);
            if living.entity.fall_distance.load() > SMASH_ATTACK_HEAVY_THRESHOLD {
                Sound::ItemMaceSmashGroundHeavy
            } else {
                Sound::ItemMaceSmashGround
            }
        } else {
            Sound::ItemMaceSmashAir
        };
        world.play_sound_fine(
            sound,
            attacker.entity_sound_source(),
            &living.entity.pos.load(),
            1.0,
            1.0,
        );

        Self::knockback(&world, attacker, victim);
    }

    /// Vanilla `MaceItem.getAttackDamageBonus`: falling turns into damage, steeply for the first
    /// blocks and less after that.
    #[must_use]
    pub fn get_attack_damage_bonus(
        attacker: &LivingEntity,
        weapon: &ItemStack,
        victim: &dyn EntityBase,
        damage_source: &DamageSource<'_>,
    ) -> f32 {
        if !Self::can_smash_attack(attacker) {
            return 0.0;
        }
        let fall_distance = attacker.entity.fall_distance.load();
        let damage = if fall_distance <= 3.0 {
            4.0 * fall_distance
        } else if fall_distance <= 8.0 {
            12.0 + 2.0 * (fall_distance - 3.0)
        } else {
            22.0 + fall_distance - 8.0
        };
        let world = attacker.entity.world.load();
        let per_block =
            EnchantmentHelper::modify_fall_based_damage(&world, weapon, victim, damage_source, 0.0);
        (damage + f64::from(per_block) * fall_distance) as f32
    }

    /// Vanilla `MaceItem.calculateImpactPosition`.
    fn calculate_impact_position(attacker: &LivingEntity) -> Vector3<f64> {
        let pos = attacker.entity.pos.load();
        attacker
            .current_impulse_impact_pos
            .load()
            .filter(|impact| impact.y <= pos.y)
            .unwrap_or(pos)
    }

    /// Vanilla `MaceItem.postHurtEnemy`.
    pub fn post_hurt_enemy(attacker: &LivingEntity) {
        if Self::can_smash_attack(attacker) {
            attacker.entity.reset_fall_distance();
        }
    }

    /// Vanilla `MaceItem.knockback`.
    fn knockback(world: &World, attacker: &Player, victim: &dyn EntityBase) {
        let victim_entity = victim.get_entity();
        world.sync_world_event(
            WorldEvent::ParticlesSmashAttack,
            victim_entity.get_on_pos(),
            750,
        );
        let victim_pos = victim_entity.pos.load();
        let radius = f64::from(SMASH_ATTACK_KNOCKBACK_RADIUS);
        let area = victim_entity
            .bounding_box
            .load()
            .expand(radius, radius, radius);
        for nearby in world.get_all_at_box(&area) {
            let Some(nearby_living) = nearby.get_living_entity() else {
                continue;
            };
            if !Self::knockback_predicate(attacker, victim, nearby.as_ref()) {
                continue;
            }
            let direction = nearby.get_entity().pos.load() - victim_pos;
            let power = Self::get_knockback_power(attacker, nearby_living, direction);
            if power > 0.0 {
                let knockback = direction.normalize() * power;
                nearby.get_entity().push_impulse(Vector3::new(
                    knockback.x,
                    f64::from(SMASH_ATTACK_KNOCKBACK_POWER),
                    knockback.z,
                ));
                if let Some(player) = nearby.get_player() {
                    player.send_own_motion();
                }
            }
        }
    }

    /// Vanilla `MaceItem.knockbackPredicate`.
    fn knockback_predicate(
        attacker: &Player,
        victim: &dyn EntityBase,
        nearby: &dyn EntityBase,
    ) -> bool {
        let nearby_entity = nearby.get_entity();
        let nearby_id = nearby_entity.entity_id;
        let not_spectator = !nearby.is_spectator();
        let not_player = nearby_id != attacker.get_entity().entity_id
            && nearby_id != victim.get_entity().entity_id;
        let not_allied_to_player = !(attacker as &dyn EntityBase).is_allied_to(nearby);
        // Vanilla checks the owner against the target, not the attacker.
        let victim_uuid = victim.get_entity().entity_uuid;
        let not_tamed_by_player = !(victim.get_living_entity().is_some()
            && nearby
                .get_mob()
                .and_then(|mob| mob.as_tamable())
                .is_some_and(|tamable| tamable.is_tame() && tamable.is_owned_by(&victim_uuid)));
        let not_armor_stand = nearby
            .cast_any()
            .downcast_ref::<ArmorStandEntity>()
            .is_none_or(|armor_stand| !armor_stand.is_marker());
        let within_range = victim
            .get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&nearby_entity.pos.load())
            <= f64::from(SMASH_ATTACK_KNOCKBACK_RADIUS).powi(2);
        let not_flying_in_creative = nearby
            .get_player()
            .is_none_or(|player| !(player.is_creative() && player.is_flying()));
        not_spectator
            && not_player
            && not_allied_to_player
            && not_tamed_by_player
            && not_armor_stand
            && within_range
            && not_flying_in_creative
    }

    /// Vanilla `MaceItem.getKnockbackPower`.
    fn get_knockback_power(
        attacker: &Player,
        nearby: &LivingEntity,
        direction: Vector3<f64>,
    ) -> f64 {
        let fall_multiplier =
            if attacker.living_entity.entity.fall_distance.load() > SMASH_ATTACK_HEAVY_THRESHOLD {
                2.0
            } else {
                1.0
            };
        (f64::from(SMASH_ATTACK_KNOCKBACK_RADIUS) - direction.length())
            * f64::from(SMASH_ATTACK_KNOCKBACK_POWER)
            * fall_multiplier
            * (1.0 - nearby.get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE))
    }
}
