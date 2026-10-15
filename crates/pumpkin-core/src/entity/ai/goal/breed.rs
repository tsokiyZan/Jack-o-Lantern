use std::sync::Arc;

use uuid::Uuid;

use pumpkin_data::entity::EntityStatus;
use rand::RngExt;

use crate::entity::ai::goal::revenge::EntityTypeFilter;
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::ai::util::goal_utils;
use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::living::LivingEntity;
use crate::entity::mob::MobEntity;
use crate::entity::{EntityBase, mob::Mob, r#type::from_type};

use super::{Controls, Goal};

const PARTNER_RANGE: f64 = 8.0;

pub struct BreedGoal {
    speed: f64,
    /// Vanilla `partnerClass`; `None` is the breeder's own class.
    partner_class: Option<EntityTypeFilter>,
    partner_targeting: TargetPredicate,
    mate: Option<Arc<dyn EntityBase>>,
    timer: i32,
    /// Vanilla `BreedGoal.breed`, which turtles override to carry an egg.
    breed: fn(&dyn Mob, &dyn EntityBase),
}

impl BreedGoal {
    #[must_use]
    pub fn new(speed: f64) -> Box<Self> {
        Box::new(Self {
            speed,
            partner_class: None,
            partner_targeting: TargetPredicate::create_non_attackable()
                .set_base_max_distance(PARTNER_RANGE)
                .ignore_visibility(),
            mate: None,
            timer: 0,
            breed: Self::breed,
        })
    }

    /// A breed goal whose `breed` is the mob's own, like `Turtle.TurtleBreedGoal`.
    #[must_use]
    pub fn with_breed(speed: f64, breed: fn(&dyn Mob, &dyn EntityBase)) -> Box<Self> {
        let mut goal = Self::new(speed);
        goal.breed = breed;
        goal
    }

    /// Looks for partners of a wider class than the breeder's own, like `AbstractHorse`.
    #[must_use]
    pub fn with_partner_class(speed: f64, partner_class: EntityTypeFilter) -> Box<Self> {
        let mut goal = Self::new(speed);
        goal.partner_class = Some(partner_class);
        goal
    }

    /// Vanilla `BreedGoal.getFreePartner`.
    fn get_free_partner(&self, mob: &dyn Mob) -> Option<Arc<dyn EntityBase>> {
        let animal = mob.as_animal()?;
        let entity = mob.get_entity();
        let world = entity.world.load();
        let own_type = entity.entity_type;
        let area = entity
            .bounding_box
            .load()
            .expand(PARTNER_RANGE, PARTNER_RANGE, PARTNER_RANGE);
        let pos = entity.pos.load();

        let mut partner = None;
        let mut dist = f64::MAX;
        for candidate in world.get_entities_at_box(&area) {
            let candidate_type = candidate.get_entity().entity_type;
            let in_class = self.partner_class.map_or_else(
                || goal_utils::is_instance_of(own_type, candidate_type),
                |partner_class| partner_class(candidate_type),
            );
            let Some(candidate_mob) = candidate.get_mob() else {
                continue;
            };
            if !in_class
                || candidate_mob.as_animal().is_none()
                || !self
                    .partner_targeting
                    .test(&world, Some(mob), candidate.as_ref())
            {
                continue;
            }
            let candidate_dist = pos.squared_distance_to_vec(&candidate.get_entity().pos.load());
            if animal.can_mate(candidate_mob) && !candidate.is_panicking() && candidate_dist < dist
            {
                dist = candidate_dist;
                partner = Some(candidate.clone());
            }
        }
        partner
    }

    /// Vanilla `Animal.spawnChildFromBreeding` and `finalizeSpawnChildFromBreeding`.
    pub(crate) fn breed(mob: &dyn Mob, mate: &dyn EntityBase) {
        let mob_entity = mob.get_mob_entity();
        let entity = mob.get_entity();
        let world = entity.world.load_full();
        let parent_pos = entity.pos.load();

        // TODO: vanilla `getBreedOffspring` is per species and may decline (horse and donkey
        // make a mule); this always clones the breeder's type.
        let baby = from_type(entity.entity_type, parent_pos, &world, Uuid::new_v4());
        baby.get_entity().set_age(-24000);
        baby.refresh_dimensions();

        award_breeding(mob, mate);

        mob_entity
            .breeding_cooldown
            .store(6000, std::sync::atomic::Ordering::Relaxed);
        mate.set_breeding_cooldown(6000);
        mob_entity.reset_love_ticks();
        mate.reset_love();

        world.send_entity_status(entity, EntityStatus::InLoveHearts, None);
        if world.level_info.load().game_rules.mob_drops {
            ExperienceOrbEntity::spawn(&world, parent_pos, mob.get_random().random_range(1..8));
        }

        world.spawn_entity(baby);
    }
}

/// The love cause's stat and advancements from vanilla `spawnChildFromBreeding`.
pub(crate) fn award_breeding(mob: &dyn Mob, mate: &dyn EntityBase) {
    let entity = mob.get_entity();
    let world = entity.world.load_full();
    // Vanilla `getLoveCause`: the breeding player, only while they are online.
    let love_cause = |animal: &MobEntity| {
        animal
            .breeder
            .load()
            .and_then(|uuid| world.get_player_by_uuid(uuid))
    };
    if let Some(player) = love_cause(mob.get_mob_entity()).or_else(|| {
        mate.get_mob()
            .and_then(|mate| love_cause(mate.get_mob_entity()))
    }) {
        player.increment_stat(
            pumpkin_data::statistic::StatisticCategory::Custom,
            pumpkin_data::statistic::CustomStatistic::AnimalsBred as i32,
            1,
        );
        player.trigger_advancement_criterion(
            pumpkin_data::advancement::Advancement::HUSBANDRY_BREED_AN_ANIMAL,
            "bred",
        );
        player.trigger_advancement_criterion(
            pumpkin_data::advancement::Advancement::HUSBANDRY_BRED_ALL_ANIMALS,
            &format!("minecraft:{}", entity.entity_type.resource_name),
        );
    }
}

impl Goal for BreedGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if !mob.get_mob_entity().is_in_love() {
            return false;
        }

        self.mate = self.get_free_partner(mob);
        self.mate.is_some()
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        let Some(mate) = &self.mate else {
            return false;
        };
        mate.get_living_entity().is_some_and(LivingEntity::is_alive)
            && mate.is_in_love()
            && self.timer < 60
            && !mate.is_panicking()
    }

    fn stop(&mut self, _mob: &dyn Mob) {
        self.mate = None;
        self.timer = 0;
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(mate) = &self.mate else {
            return;
        };

        let mob_entity = mob.get_mob_entity();
        mob_entity
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .look_at_entity_with_range(mate, 10.0, mob.get_max_look_pitch_change());

        let living = &mob_entity.living_entity;
        {
            let mut navigator = mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Vanilla `moveTo(partner, speed)` paths with a reach of 1.
            if let Some(path) = navigator.create_path(living, mate.get_entity().pos.load(), 1) {
                navigator.move_to_path(Some(path), self.speed, living);
            }
        }

        self.timer += 1;
        let dist_sq = living
            .entity
            .pos
            .load()
            .squared_distance_to_vec(&mate.get_entity().pos.load());
        if self.timer >= self.get_tick_count(60) && dist_sq < 9.0 {
            (self.breed)(mob, mate.as_ref());
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}
