use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::random::RandomImpl;

use crate::entity::ageable::{AgeableData, AgeableMob};
use crate::entity::ai::goal::leap_at_target::LeapAtTargetGoal;
use crate::entity::ai::goal::ocelot_attack::OcelotAttackGoal;
use crate::entity::ai::goal::{Controls, Goal};
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::passive::cat::{
    CROUCH_SPEED_MOD, SPRINT_SPEED_MOD, WALK_SPEED_MOD, baby_on_land, feline_custom_server_ai_step,
};
use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::ActiveTargetGoal, avoid_entity::AvoidEntityGoal, breed::BreedGoal,
        look_at_entity::LookAtEntityGoal, swim::SwimGoal, tempt::TemptGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};
use crate::world::World;

/// Represents an Ocelot, a shy passive mob found in jungles.
///
/// Wiki: <https://minecraft.wiki/w/Ocelot>
pub struct OcelotEntity {
    pub mob_entity: MobEntity,
    pub is_trusting: AtomicBool,
    pub ageable_data: AgeableData,
}

impl OcelotEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        let ocelot = Self {
            mob_entity,
            is_trusting: AtomicBool::new(false),
            ageable_data: AgeableData::default(),
        };
        let mob_arc = Arc::new(ocelot);
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

            goal_selector.add_goal(1, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(3, Box::new(OcelotTemptGoal::new()));
            goal_selector.add_goal(7, Box::new(LeapAtTargetGoal::new(0.3)));
            goal_selector.add_goal(8, Box::new(OcelotAttackGoal::new()));
            goal_selector.add_goal(9, BreedGoal::new(0.8));
            goal_selector.add_goal(
                10,
                Box::new(WanderAroundGoal::avoiding_water_with_probability(
                    0.8,
                    1.000_000_1E-5,
                )),
            );
            goal_selector.add_goal(
                11,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 10.0),
            );
            // `reassessTrustingGoals` in the constructor: `OcelotAvoidEntityGoal`, added last
            // and used only while not trusting.
            goal_selector.add_goal(
                4,
                Box::new(
                    AvoidEntityGoal::new(
                        &EntityType::PLAYER,
                        16.0,
                        WALK_SPEED_MOD,
                        SPRINT_SPEED_MOD,
                    )
                    .gated_by(|mob| as_ocelot(mob).is_none_or(|ocelot| !ocelot.is_trusting())),
                ),
            );
        };

        {
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::CHICKEN, false),
            );
            target_selector.add_goal(
                1,
                Box::new(ActiveTargetGoal::new(
                    &mob_arc.mob_entity,
                    &EntityType::TURTLE,
                    10,
                    false,
                    false,
                    Some(baby_on_land),
                )),
            );
        };

        mob_arc
    }

    pub fn is_trusting(&self) -> bool {
        self.is_trusting.load(Ordering::Relaxed)
    }

    pub fn set_trusting(&self, trusting: bool) {
        self.is_trusting.store(trusting, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(pumpkin_data::tracked_data::ocelot::TRUSTING, trusting);
    }
}

fn as_ocelot(mob: &dyn Mob) -> Option<&OcelotEntity> {
    mob.cast_any().downcast_ref::<OcelotEntity>()
}

impl AgeableMob for OcelotEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for OcelotEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack
            .get_item()
            .has_tag(&tag::Item::MINECRAFT_OCELOT_FOOD)
    }
}

/// Vanilla `Ocelot.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::OCELOT_BABY_DIMENSIONS;

impl Mob for OcelotEntity {
    /// Vanilla `Ocelot.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    /// Vanilla `Ocelot.removeWhenFarAway`.
    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        !self.is_trusting() && self.get_entity().tick_count.load(Ordering::Relaxed) > 2400
    }

    /// The ageing at the end of vanilla `AgeableMob.aiStep`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        if self.get_entity().is_alive() {
            self.ageable_ai_step();
        }
    }

    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        feline_custom_server_ai_step(self);
    }

    /// Vanilla `Ocelot.isSteppingCarefully`.
    fn mob_is_stepping_carefully(&self) -> bool {
        let entity = self.get_entity();
        entity.pose.load() == EntityPose::Crouching || entity.is_sneaking()
    }

    /// Vanilla `Ocelot.finalizeSpawn`: an `AgeableMobGroupData(1.0F)` group, so every
    /// ocelot after the first spawns as a baby.
    fn finalize_spawn(
        &self,
        _world: &Arc<World>,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        let group_data =
            self.finalize_ageable_spawn(group_data.or(Some(SpawnGroupData::ageable(1.0))));
        self.mob_entity.finalize_spawn_base();
        Some(group_data)
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("Trusting", self.is_trusting());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.set_trusting(nbt.get_bool("Trusting").unwrap_or(false));
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        if entity.is_baby() {
            entity.set_synced_data(pumpkin_data::tracked_data::ocelot::BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::ocelot::TRUSTING,
            self.is_trusting(),
        );
    }

    /// Vanilla `Ocelot.mobInteract`.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let tempted = self
            .mob_entity
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_running::<OcelotTemptGoal>();
        let dist_sqr = player
            .get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&self.get_entity().pos.load());

        if tempted && !self.is_trusting() && self.is_food(item_stack) && dist_sqr < 9.0 {
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            // `spawnTrustingParticles` adds nothing on the server; the entity event makes
            // the client show them.
            let world = self.get_entity().world.load();
            if self.get_entity().random().next_bounded_i32(3) == 0 {
                self.set_trusting(true);
                world.send_entity_status(
                    self.get_entity(),
                    EntityStatus::TrustingSucceeded,
                    Some(ActorEventID::TamingSucceeded),
                );
            } else {
                world.send_entity_status(
                    self.get_entity(),
                    EntityStatus::TrustingFailed,
                    Some(ActorEventID::TamingFailed),
                );
            }

            return true;
        }

        // TODO: vanilla `Animal.mobInteract` plays `Ocelot.playEatingSound`, which is silent.
        self.animal_interact(
            player,
            item_stack,
            pumpkin_data::sound::Sound::EntityOcelotAmbient,
        )
    }
}

/// Vanilla `Ocelot.OcelotTemptGoal`: a trusting ocelot is no longer scared off by sudden
/// moves.
struct OcelotTemptGoal {
    goal: TemptGoal,
}

impl OcelotTemptGoal {
    fn new() -> Self {
        Self {
            goal: TemptGoal::with_tag(CROUCH_SPEED_MOD, &tag::Item::MINECRAFT_OCELOT_FOOD, true),
        }
    }
}

impl Goal for OcelotTemptGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.goal.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        // Vanilla `OcelotTemptGoal.canScare`.
        let can_scare =
            self.goal.can_scare() && as_ocelot(mob).is_some_and(|ocelot| !ocelot.is_trusting());
        self.goal.passes_scare_check(mob, can_scare) && self.can_start(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.goal.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.goal.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.goal.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}
