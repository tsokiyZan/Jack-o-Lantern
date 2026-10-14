use std::sync::{
    Arc, PoisonError, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data::dolphin::{GOT_FISH, MOISTNESS_LEVEL};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::VarInt;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

use crate::entity::{
    Entity, EntityBase,
    ai::{
        control::{Control, MoveControlTrait, look_control::LookControl},
        goal::{
            Controls, Goal, avoid_entity::AvoidEntityGoal, breath_air::BreathAirGoal,
            dolphin_jump::DolphinJumpGoal, look_around::RandomLookAroundGoal,
            look_at_entity::LookAtEntityGoal, melee_attack::MeleeAttackGoal, revenge::RevengeGoal,
            try_find_water::TryFindWaterGoal, wander_around::WanderAroundGoal,
        },
        pathfinder::{Navigator, NavigatorGoal},
    },
    mob::{Mob, MobEntity},
    player::Player,
};

/// Vanilla `Dolphin.TOTAL_AIR_SUPPLY`.
pub const TOTAL_AIR_SUPPLY: i32 = 4800;
/// Vanilla `Dolphin.TOTAL_MOISTNESS_LEVEL`.
const TOTAL_MOISTNESS_LEVEL: i32 = 2400;
/// The `inWaterSpeedModifier` vanilla gives the dolphin's `SmoothSwimmingMoveControl`.
const IN_WATER_SPEED_MODIFIER: f64 = 0.02;
/// Vanilla `SWIM_WITH_PLAYER_TARGETING` range.
const SWIM_WITH_PLAYER_RANGE: f64 = 10.0;
/// Guardians, which vanilla reaches through `Guardian.class` and so also matches elder guardians.
const GUARDIANS: [&EntityType; 2] = [&EntityType::GUARDIAN, &EntityType::ELDER_GUARDIAN];

pub struct DolphinEntity {
    pub mob_entity: MobEntity,
    pub got_fish: AtomicBool,
    pub moistness_level: AtomicI32,
}

impl DolphinEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        *mob_entity
            .navigator
            .lock()
            // Vanilla `WaterBoundPathNavigation` allows breaching for dolphins only.
            .unwrap_or_else(PoisonError::into_inner) = Navigator::water_bound(true);
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(DolphinMoveControl);
        *mob_entity
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = LookControl::keeping_pitch();
        let dolphin = Self {
            mob_entity,
            got_fish: AtomicBool::new(false),
            moistness_level: AtomicI32::new(TOTAL_MOISTNESS_LEVEL),
        };
        let mob_arc = Arc::new(dolphin);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        // Vanilla `Dolphin.registerGoals`. Not ported yet: `DolphinSwimToTreasureGoal(1)`,
        // `MoveToItemGoal(7)`, `PlayWithItemsGoal(8)` and the two
        // `FollowPlayerRiddenEntityGoal(8)`s for boats and nautiluses.
        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            goal_selector.add_goal(0, Box::new(BreathAirGoal::new()));
            goal_selector.add_goal(0, Box::new(TryFindWaterGoal::new()));
            goal_selector.add_goal(2, Box::new(DolphinSwimWithPlayerGoal::new(4.0)));
            goal_selector.add_goal(4, Box::new(WanderAroundGoal::swimming(1.0, 10)));
            goal_selector.add_goal(4, Box::new(RandomLookAroundGoal::default()));
            goal_selector.add_goal(
                5,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(5, Box::new(DolphinJumpGoal::new(10)));
            goal_selector.add_goal(6, Box::new(MeleeAttackGoal::new(1.2, true)));
            goal_selector.add_goal(
                9,
                Box::new(AvoidEntityGoal::new_for_types(&GUARDIANS, 8.0, 1.0, 1.0)),
            );
        };

        {
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            target_selector.add_goal(
                1,
                Box::new(
                    RevengeGoal::new(true)
                        .ignoring(|entity_type| GUARDIANS.contains(&entity_type))
                        .alerting_others(),
                ),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn got_fish(&self) -> bool {
        self.got_fish.load(Ordering::Relaxed)
    }

    pub fn set_got_fish(&self, val: bool) {
        self.got_fish.store(val, Ordering::Relaxed);
        self.get_entity().set_synced_data(GOT_FISH, val);
    }

    fn set_moistness_level(&self, level: i32) {
        if self.moistness_level.swap(level, Ordering::Relaxed) != level {
            self.get_entity()
                .set_synced_data(MOISTNESS_LEVEL, VarInt(level));
        }
    }
}

impl Mob for DolphinEntity {
    /// Vanilla `Dolphin.getMaxAirSupply`.
    fn max_air_supply(&self) -> i32 {
        TOTAL_AIR_SUPPLY
    }

    /// Vanilla `Dolphin.increaseAirSupply`: one breath refills it.
    fn increase_air_supply(&self, _current: i32) -> i32 {
        TOTAL_AIR_SUPPLY
    }

    /// Vanilla `Dolphin.tick`: drying out on land, and flopping about there.
    fn mob_tick(&self, caller: &dyn EntityBase) {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        if self.mob_entity.is_no_ai() {
            living.breath.set_air_supply(caller, TOTAL_AIR_SUPPLY);
            return;
        }
        if entity.is_in_water_or_rain() {
            self.set_moistness_level(TOTAL_MOISTNESS_LEVEL);
            return;
        }
        self.set_moistness_level(self.moistness_level.load(Ordering::Relaxed) - 1);
        if self.moistness_level.load(Ordering::Relaxed) <= 0 {
            living.damage(caller, 1.0, DamageType::DRY_OUT);
        }
        if entity.on_ground.load(Ordering::Relaxed) {
            let mut rng = self.get_random();
            let velocity = entity.velocity.load();
            // Sent at once, as vanilla flags the hop with needsSync.
            entity.set_velocity(Vector3::new(
                velocity.x + f64::from((rng.random::<f32>() * 2.0 - 1.0) * 0.2),
                velocity.y + 0.5,
                velocity.z + f64::from((rng.random::<f32>() * 2.0 - 1.0) * 0.2),
            ));
            entity.yaw.store(rng.random::<f32>() * 360.0);
            entity.on_ground.store(false, Ordering::Relaxed);
        }
    }

    /// Vanilla `Dolphin.travelInWater`; on land it moves like any other mob.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let living = &self.mob_entity.living_entity;
        let entity = &living.entity;
        if !entity.is_in_water() {
            return false;
        }
        // `getSpeed()`, which the dolphin's move control sets to the swim speed times
        // its in-water modifier.
        let input = living.movement_input.load();
        entity.update_velocity_from_input(input, input.z.abs() * IN_WATER_SPEED_MODIFIER);
        living.make_move(caller);

        let mut velocity = entity.velocity.load() * 0.9;
        let has_target = self
            .mob_entity
            .target
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if !has_target {
            velocity.y -= 0.005;
        }
        entity.velocity.store(velocity);
        true
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("GotFish", self.got_fish());
        nbt.put_int("Moistness", self.moistness_level.load(Ordering::Relaxed));
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.set_got_fish(nbt.get_bool("GotFish").unwrap_or(false));
        self.set_moistness_level(nbt.get_int("Moistness").unwrap_or(TOTAL_MOISTNESS_LEVEL));
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `Dolphin.mobInteract` for an adult: any fish makes it remember it was fed.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if item_stack.is_empty() || !item_stack.get_item().has_tag(&tag::Item::MINECRAFT_FISHES) {
            return false;
        }
        self.get_entity().play_sound(Sound::EntityDolphinEat);
        self.set_got_fish(true);
        item_stack.decrement_unless_creative(player.gamemode.load(), 1);
        true
    }
}

/// The part of vanilla's `SmoothSwimmingMoveControl(.., applyGravity = true)` that the
/// water navigator does not already do: a small lift each tick in water, which
/// balances the sink in `travelInWater`.
struct DolphinMoveControl;

impl Control for DolphinMoveControl {}

impl MoveControlTrait for DolphinMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        let entity = mob.get_entity();
        if entity.is_in_water() {
            // A plain store: the tracker syncs motion, as vanilla's setDeltaMovement leaves to it.
            let velocity = entity.velocity.load();
            entity
                .velocity
                .store(Vector3::new(velocity.x, velocity.y + 0.005, velocity.z));
        }
    }
}

/// Vanilla `Dolphin.DolphinSwimWithPlayerGoal`: swims alongside a nearby swimming
/// player and gives them Dolphin's Grace.
struct DolphinSwimWithPlayerGoal {
    speed_modifier: f64,
    player: Option<Arc<Player>>,
}

impl DolphinSwimWithPlayerGoal {
    const fn new(speed_modifier: f64) -> Self {
        Self {
            speed_modifier,
            player: None,
        }
    }

    fn give_dolphins_grace(player: &Player) {
        player
            .living_entity
            .add_effect(pumpkin_data::potion::Effect {
                effect_type: &StatusEffect::DOLPHINS_GRACE,
                duration: 100,
                amplifier: 0,
                ambient: false,
                show_particles: true,
                show_icon: true,
                blend: true,
            });
    }
}

impl Goal for DolphinSwimWithPlayerGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let entity = mob.get_entity();
        self.player = entity.world.load().get_nearest_player(
            entity.pos.load(),
            SWIM_WITH_PLAYER_RANGE,
            |player| !player.is_spectator() && player.get_entity().is_alive(),
        );
        let Some(player) = &self.player else {
            return false;
        };
        let is_target = mob
            .get_mob_entity()
            .target
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|target| target.get_entity().entity_id == player.entity_id());
        player.is_swimming() && !is_target
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.player.as_ref().is_some_and(|player| {
            player.is_swimming()
                && mob
                    .get_entity()
                    .pos
                    .load()
                    .squared_distance_to_vec(&player.get_entity().pos.load())
                    < 256.0
        })
    }

    fn start(&mut self, _mob: &dyn Mob) {
        if let Some(player) = &self.player {
            Self::give_dolphins_grace(player);
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.player = None;
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(player) = self.player.clone() else {
            return;
        };
        let mob_entity = mob.get_mob_entity();
        let player_entity: Arc<dyn EntityBase> = player.clone();
        // Vanilla looks with `getMaxHeadYRot() + 20` and `getMaxHeadXRot()`, both 1 for dolphins.
        mob_entity
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .look_at_entity_with_range(&player_entity, 21.0, 1.0);

        let pos = mob.get_entity().pos.load();
        let player_pos = player.get_entity().pos.load();
        let mut navigator = mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if pos.squared_distance_to_vec(&player_pos) < 6.25 {
            navigator.stop();
        } else {
            navigator.set_progress(NavigatorGoal::new(pos, player_pos, self.speed_modifier));
        }
        drop(navigator);

        if player.is_swimming() && rand::rng().random_range(0..6) == 0 {
            Self::give_dolphins_grace(&player);
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}
