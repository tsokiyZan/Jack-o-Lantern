use std::sync::{Arc, Weak};

use crate::entity::ai::control::flying_move_control::FlyingMoveControl;
use crate::entity::ai::goal::escape_danger::EscapeDangerGoal;
use crate::entity::ai::goal::follow_mob::FollowMobGoal;
use crate::entity::ai::goal::follow_owner::FollowOwnerGoal;
use crate::entity::ai::goal::sit_when_ordered_to::SitWhenOrderedToGoal;
use crate::entity::ai::goal::water_avoiding_random_flying::WaterAvoidingRandomFlyingGoal;
use crate::entity::ai::pathfinder::Navigator;
use crate::entity::passive::animal::Animal;
use crate::entity::passive::tamable::{TamableAnimal, TamableData};
use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::random::RandomImpl;

use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{look_at_entity::LookAtEntityGoal, swim::SwimGoal},
    mob::{Mob, MobEntity},
    passive::animal::set_animal_pathfinding_malus,
    player::Player,
};

/// Duration in ticks of the poison a parrot gets from eating a cookie, matching
/// vanilla `Parrot.mobInteract`.
const COOKIE_POISON_DURATION: i32 = 900;

/// Represents a Parrot, a passive flying mob that can mimic nearby mob sounds.
///
/// Wiki: <https://minecraft.wiki/w/Parrot>
/// Vanilla `Parrot.MOB_SOUND_MAP`: the sound a parrot makes to imitate each mob.
const PARROT_IMITATIONS: &[(&EntityType, pumpkin_data::sound::Sound)] = &[
    (
        &EntityType::BLAZE,
        pumpkin_data::sound::Sound::EntityParrotImitateBlaze,
    ),
    (
        &EntityType::BOGGED,
        pumpkin_data::sound::Sound::EntityParrotImitateBogged,
    ),
    (
        &EntityType::BREEZE,
        pumpkin_data::sound::Sound::EntityParrotImitateBreeze,
    ),
    (
        &EntityType::CAMEL_HUSK,
        pumpkin_data::sound::Sound::EntityParrotImitateCamelHusk,
    ),
    (
        &EntityType::CAVE_SPIDER,
        pumpkin_data::sound::Sound::EntityParrotImitateSpider,
    ),
    (
        &EntityType::CREAKING,
        pumpkin_data::sound::Sound::EntityParrotImitateCreaking,
    ),
    (
        &EntityType::CREEPER,
        pumpkin_data::sound::Sound::EntityParrotImitateCreeper,
    ),
    (
        &EntityType::DROWNED,
        pumpkin_data::sound::Sound::EntityParrotImitateDrowned,
    ),
    (
        &EntityType::ELDER_GUARDIAN,
        pumpkin_data::sound::Sound::EntityParrotImitateElderGuardian,
    ),
    (
        &EntityType::ENDER_DRAGON,
        pumpkin_data::sound::Sound::EntityParrotImitateEnderDragon,
    ),
    (
        &EntityType::ENDERMITE,
        pumpkin_data::sound::Sound::EntityParrotImitateEndermite,
    ),
    (
        &EntityType::EVOKER,
        pumpkin_data::sound::Sound::EntityParrotImitateEvoker,
    ),
    (
        &EntityType::GHAST,
        pumpkin_data::sound::Sound::EntityParrotImitateGhast,
    ),
    (
        &EntityType::HAPPY_GHAST,
        pumpkin_data::sound::Sound::IntentionallyEmpty,
    ),
    (
        &EntityType::GUARDIAN,
        pumpkin_data::sound::Sound::EntityParrotImitateGuardian,
    ),
    (
        &EntityType::HOGLIN,
        pumpkin_data::sound::Sound::EntityParrotImitateHoglin,
    ),
    (
        &EntityType::HUSK,
        pumpkin_data::sound::Sound::EntityParrotImitateHusk,
    ),
    (
        &EntityType::ILLUSIONER,
        pumpkin_data::sound::Sound::EntityParrotImitateIllusioner,
    ),
    (
        &EntityType::MAGMA_CUBE,
        pumpkin_data::sound::Sound::EntityParrotImitateMagmaCube,
    ),
    (
        &EntityType::PARCHED,
        pumpkin_data::sound::Sound::EntityParrotImitateParched,
    ),
    (
        &EntityType::PHANTOM,
        pumpkin_data::sound::Sound::EntityParrotImitatePhantom,
    ),
    (
        &EntityType::PIGLIN,
        pumpkin_data::sound::Sound::EntityParrotImitatePiglin,
    ),
    (
        &EntityType::PIGLIN_BRUTE,
        pumpkin_data::sound::Sound::EntityParrotImitatePiglinBrute,
    ),
    (
        &EntityType::PILLAGER,
        pumpkin_data::sound::Sound::EntityParrotImitatePillager,
    ),
    (
        &EntityType::RAVAGER,
        pumpkin_data::sound::Sound::EntityParrotImitateRavager,
    ),
    (
        &EntityType::SHULKER,
        pumpkin_data::sound::Sound::EntityParrotImitateShulker,
    ),
    (
        &EntityType::SILVERFISH,
        pumpkin_data::sound::Sound::EntityParrotImitateSilverfish,
    ),
    (
        &EntityType::SKELETON,
        pumpkin_data::sound::Sound::EntityParrotImitateSkeleton,
    ),
    (
        &EntityType::SLIME,
        pumpkin_data::sound::Sound::EntityParrotImitateSlime,
    ),
    (
        &EntityType::SPIDER,
        pumpkin_data::sound::Sound::EntityParrotImitateSpider,
    ),
    (
        &EntityType::STRAY,
        pumpkin_data::sound::Sound::EntityParrotImitateStray,
    ),
    (
        &EntityType::VEX,
        pumpkin_data::sound::Sound::EntityParrotImitateVex,
    ),
    (
        &EntityType::VINDICATOR,
        pumpkin_data::sound::Sound::EntityParrotImitateVindicator,
    ),
    (
        &EntityType::WARDEN,
        pumpkin_data::sound::Sound::EntityParrotImitateWarden,
    ),
    (
        &EntityType::WITCH,
        pumpkin_data::sound::Sound::EntityParrotImitateWitch,
    ),
    (
        &EntityType::WITHER,
        pumpkin_data::sound::Sound::EntityParrotImitateWither,
    ),
    (
        &EntityType::WITHER_SKELETON,
        pumpkin_data::sound::Sound::EntityParrotImitateWitherSkeleton,
    ),
    (
        &EntityType::ZOGLIN,
        pumpkin_data::sound::Sound::EntityParrotImitateZoglin,
    ),
    (
        &EntityType::ZOMBIE,
        pumpkin_data::sound::Sound::EntityParrotImitateZombie,
    ),
    (
        &EntityType::ZOMBIE_HORSE,
        pumpkin_data::sound::Sound::EntityParrotImitateZombieHorse,
    ),
    (
        &EntityType::ZOMBIE_NAUTILUS,
        pumpkin_data::sound::Sound::EntityParrotImitateZombieNautilus,
    ),
    (
        &EntityType::ZOMBIE_VILLAGER,
        pumpkin_data::sound::Sound::EntityParrotImitateZombieVillager,
    ),
];

pub struct ParrotEntity {
    pub mob_entity: MobEntity,
    pub tamable_data: TamableData,
}

impl ParrotEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Vanilla `Parrot`: FlyingMoveControl(10, false) and flying navigation that doesn't
        // open doors but floats.
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(FlyingMoveControl::new(10, false));
        let mut navigator = Navigator::flying();
        navigator.set_can_open_doors(false);
        navigator.set_can_float(true);
        *mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = navigator;
        // The path costs go on the flying navigation, made first as in `Mob`'s constructor.
        set_animal_pathfinding_malus(&mob_entity);
        // Vanilla `Parrot`'s constructor.
        mob_entity.set_pathfinding_malus(PathType::DangerFire, -1.0);
        mob_entity.set_pathfinding_malus(PathType::DamageFire, -1.0);
        mob_entity.set_pathfinding_malus(PathType::Cocoa, -1.0);
        let parrot = Self {
            mob_entity,
            tamable_data: TamableData::default(),
        };
        let mob_arc = Arc::new(parrot);
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

            // TamableAnimalPanicGoal
            goal_selector.add_goal(0, EscapeDangerGoal::new(1.25));
            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(
                1,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(2, Box::new(SitWhenOrderedToGoal::new()));
            goal_selector.add_goal(2, FollowOwnerGoal::new(1.0, 5.0, 1.0));
            goal_selector.add_goal(
                2,
                Box::new(WaterAvoidingRandomFlyingGoal::new(1.0).preferring_trees()),
            );
            // TODO: 3 LandOnOwnersShoulderGoal needs players to carry shoulder entities.
            goal_selector.add_goal(3, Box::new(FollowMobGoal::new(1.0, 3.0, 7.0)));
        };

        mob_arc
    }

    /// Feeds the parrot a cookie: it is poisoned and then killed, as in vanilla
    /// `Parrot.mobInteract`.
    fn eat_cookie(&self, player: &Arc<Player>, item_stack: &mut ItemStack) {
        item_stack.decrement_unless_creative(player.gamemode.load(), 1);

        self.mob_entity
            .living_entity
            .add_effect(pumpkin_data::potion::Effect {
                effect_type: &StatusEffect::POISON,
                duration: COOKIE_POISON_DURATION,
                amplifier: 0,
                ambient: false,
                show_particles: true,
                show_icon: true,
                blend: true,
            });

        // Vanilla guards this call with `player.isCreative() || !this.isInvulnerable()`,
        // but `hurt` re-checks invulnerability itself and `player_attack` doesn't bypass
        // it, so the guard only skips a call that would do nothing anyway.
        self.damage_with_context(
            self,
            f32::MAX,
            DamageType::PLAYER_ATTACK,
            None,
            Some(player.as_ref()),
            Some(player.as_ref()),
        );
    }
}

/// Parrots can't breed: vanilla `Parrot.isFood` and `canMate` are always false.
impl Animal for ParrotEntity {
    fn is_food(&self, _item_stack: &ItemStack) -> bool {
        false
    }

    fn can_mate(&self, _partner: &dyn Mob) -> bool {
        false
    }
}

impl TamableAnimal for ParrotEntity {
    fn get_tamable_data(&self) -> &TamableData {
        &self.tamable_data
    }
}

impl Mob for ParrotEntity {
    /// Vanilla `Parrot.checkFallDamage` does nothing.
    fn mob_check_fall_damage(
        &self,
        _caller: &dyn crate::entity::EntityBase,
        _ya: f64,
        _on_ground: bool,
        _state: &'static pumpkin_data::BlockState,
        _pos: pumpkin_util::math::position::BlockPos,
    ) {
    }

    fn omnidirectional_air_mover(&self) -> bool {
        true
    }

    /// Vanilla `Parrot.getAmbient`: one time in a thousand, outside peaceful, a parrot
    /// imitates a mob.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        // `getAmbient(this.level(), this.level().getRandom())`.
        let world = self.get_entity().world.load();
        let mut random = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if world.level_info.load().difficulty != pumpkin_util::Difficulty::Peaceful
            && random.next_bounded_i32(1000) == 0
        {
            let index = random.next_bounded_i32(PARROT_IMITATIONS.len() as i32) as usize;
            return Some(PARROT_IMITATIONS[index].1);
        }
        Some(pumpkin_data::sound::Sound::EntityParrotAmbient)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn as_tamable(&self) -> Option<&dyn TamableAnimal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.write_tamable_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.read_tamable_nbt(nbt);
    }

    /// Vanilla `Parrot.mobInteract`.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let item = item_stack.get_item();
        if !self.is_tame() && item.has_tag(&tag::Item::MINECRAFT_PARROT_FOOD) {
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            let entity = self.get_entity();
            if !entity.is_silent() {
                let pitch = {
                    let mut random = entity.random();
                    1.0 + (random.next_f32() - random.next_f32()) * 0.2
                };
                entity.world.load().play_sound_fine(
                    pumpkin_data::sound::Sound::EntityParrotEat,
                    self.sound_source(),
                    &entity.pos.load(),
                    1.0,
                    pitch,
                );
            }
            let (status, bedrock) = if entity.random().next_bounded_i32(10) == 0 {
                TamableAnimal::tame(self, player.gameprofile.id);
                Mob::tame(self, player);
                (EntityStatus::TamingSucceeded, ActorEventID::TamingSucceeded)
            } else {
                (EntityStatus::TamingFailed, ActorEventID::TamingFailed)
            };
            entity
                .world
                .load()
                .send_entity_status(entity, status, Some(bedrock));
            return true;
        }
        if !item.has_tag(&tag::Item::MINECRAFT_PARROT_POISONOUS_FOOD) {
            // `isFlying`: off the ground.
            let flying = !self
                .get_entity()
                .on_ground
                .load(std::sync::atomic::Ordering::Relaxed);
            if !flying && self.is_tame() && self.is_owned_by(&player.gameprofile.id) {
                self.set_ordered_to_sit(!self.is_ordered_to_sit());
                return true;
            }
            return self.mob_entity.mob_interact(player, item_stack);
        }

        self.eat_cookie(player, item_stack);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::COOKIE_POISON_DURATION;
    use pumpkin_data::item::Item;
    use pumpkin_data::tag::{self, Taggable};

    /// The interaction is gated on the vanilla `parrot_poisonous_food` tag rather than
    /// on a hardcoded cookie id, so check the tag actually resolves the way the
    /// interaction assumes.
    #[test]
    fn cookie_is_poisonous_parrot_food() {
        assert!(Item::COOKIE.has_tag(&tag::Item::MINECRAFT_PARROT_POISONOUS_FOOD));
    }

    /// Seeds tame a parrot in vanilla and must not reach the poison branch.
    #[test]
    fn parrot_food_is_not_poisonous() {
        assert!(!Item::WHEAT_SEEDS.has_tag(&tag::Item::MINECRAFT_PARROT_POISONOUS_FOOD));
        assert!(!Item::COOKED_CHICKEN.has_tag(&tag::Item::MINECRAFT_PARROT_POISONOUS_FOOD));
    }

    #[test]
    fn poison_lasts_45_seconds() {
        assert_eq!(COOKIE_POISON_DURATION, 900);
    }
}
