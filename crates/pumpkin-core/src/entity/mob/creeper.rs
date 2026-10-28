use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{entity::EntityType, game_event::GameEvent, item::Item, sound::Sound, tag};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::random::RandomImpl;

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::ActiveTargetGoal, avoid_entity::AvoidEntityGoal,
        creeper_ignite::CreeperIgniteGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, melee_attack::MeleeAttackGoal, revenge::RevengeGoal,
        swim::SwimGoal, wander_around::WanderAroundGoal,
    },
    living::LivingEntity,
    mob::{Mob, MobEntity, set_mob_target_base},
    player::Player,
};
use crate::world::loot::LootContextParameters;

const DEFAULT_FUSE_TIME: i32 = 30;
const DEFAULT_EXPLOSION_RADIUS: i32 = 3;
/// Vanilla `BuiltInLootTables.CHARGED_CREEPER`.
const CHARGED_CREEPER_LOOT_TABLE: &str = "minecraft:charged_creeper/root";

pub struct CreeperEntity {
    pub mob_entity: MobEntity,
    pub fuse_speed: AtomicI32,
    pub current_fuse_time: AtomicI32,
    pub last_fuse_time: AtomicI32,
    pub fuse_time: AtomicI32,
    pub explosion_radius: AtomicI32,
    pub ignited: AtomicBool,
    pub charged: AtomicBool,
    dropped_skulls: AtomicBool,
}

impl CreeperEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let entity = Self {
            mob_entity,
            fuse_speed: AtomicI32::new(-1),
            current_fuse_time: AtomicI32::new(0),
            last_fuse_time: AtomicI32::new(0),
            fuse_time: AtomicI32::new(DEFAULT_FUSE_TIME),
            explosion_radius: AtomicI32::new(DEFAULT_EXPLOSION_RADIUS),
            ignited: AtomicBool::new(false),
            charged: AtomicBool::new(false),
            dropped_skulls: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(entity);
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
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(1, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(2, Box::new(CreeperIgniteGoal::new(mob_arc.clone())));
            goal_selector.add_goal(
                3,
                Box::new(AvoidEntityGoal::new(&EntityType::OCELOT, 6.0, 1.0, 1.2)),
            );
            goal_selector.add_goal(
                3,
                Box::new(AvoidEntityGoal::new(&EntityType::CAT, 6.0, 1.0, 1.2)),
            );
            goal_selector.add_goal(4, Box::new(MeleeAttackGoal::new(1.0, false)));
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::avoiding_water(0.8)));

            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(6, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true),
            );
            target_selector.add_goal(2, Box::new(RevengeGoal::new(true)));
        };

        mob_arc
    }

    pub fn set_fuse_speed(&self, speed: i32) {
        self.fuse_speed.store(speed, Ordering::Relaxed);
        self.mob_entity
            .living_entity
            .entity
            .set_synced_data(pumpkin_data::tracked_data::creeper::FUSE_ID, VarInt(speed));
    }

    pub fn explode(&self) {
        let entity = &self.mob_entity.living_entity.entity;
        let radius = self.explosion_radius.load(Ordering::Relaxed) as f32;
        let multiplier = if self.charged.load(Ordering::Relaxed) {
            2.0
        } else {
            1.0
        };
        self.mob_entity
            .living_entity
            .dead
            .store(true, Ordering::Relaxed);
        let world = entity.world.load();
        let pos = entity.pos.load();
        world.explode(
            world.get_entity_by_id(entity.entity_id),
            pos,
            radius * multiplier,
            crate::world::ExplosionInteraction::Mob,
        );
        self.spawn_lingering_cloud();
        // `triggerOnDeathMobEffects(level, KILLED)`, which then clears the effects.
        let living = &self.mob_entity.living_entity;
        living.trigger_on_death_mob_effects();
        living
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        entity.remove();
    }

    /// Vanilla `Creeper.spawnLingeringCloud`: its effects linger in a small cloud.
    fn spawn_lingering_cloud(&self) {
        let living = &self.mob_entity.living_entity;
        let effects: Vec<_> = living
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .map(|e| {
                (
                    e.effect_type,
                    e.duration,
                    e.amplifier,
                    e.ambient,
                    e.show_particles,
                    e.show_icon,
                )
            })
            .collect();
        if effects.is_empty() {
            return;
        }
        let entity = &living.entity;
        let world = entity.world.load_full();
        let cloud_entity = crate::entity::Entity::from_uuid(
            uuid::Uuid::new_v4(),
            world.clone(),
            entity.pos.load(),
            &EntityType::AREA_EFFECT_CLOUD,
        );
        // Radius 2.5 shrinking 0.5 per use, 300 ticks after a 10-tick wait, durations
        // scaled by 0.25 (the cloud's own scaling).
        let cloud = crate::entity::area_effect_cloud::AreaEffectCloudEntity::create(
            cloud_entity,
            pumpkin_data::item_stack::ItemStack::EMPTY.clone(),
            effects,
            300,
            2.5,
            20,
            10,
            -0.5,
            0,
        );
        world.spawn_entity(cloud);
    }
}

impl Mob for CreeperEntity {
    /// Vanilla `Creeper.causeFallDamage`: a fall brings the creeper closer to exploding.
    fn mob_cause_fall_damage(
        &self,
        caller: &dyn EntityBase,
        fall_distance: f64,
        damage_modifier: f32,
        damage_type: pumpkin_data::damage::DamageType,
    ) -> bool {
        let damaged = self.get_mob_entity().living_entity.cause_fall_damage(
            caller,
            fall_distance,
            damage_modifier,
            damage_type,
        );
        let max_swell = self.fuse_time.load(Ordering::Relaxed);
        let swell = self.current_fuse_time.load(Ordering::Relaxed) + (fall_distance * 1.5) as i32;
        self.current_fuse_time
            .store(swell.min(max_swell - 5), Ordering::Relaxed);
        damaged
    }

    /// Vanilla `Creeper.getMaxFallDistance`: with a target it will drop as far as
    /// it can survive.
    fn get_max_fall_distance(&self) -> i32 {
        let living = &self.mob_entity.living_entity;
        let allowed_damage = if self.get_target().is_none() {
            0.0
        } else {
            living.health.load() - 1.0
        };
        LivingEntity::get_comfortable_fall_distance(allowed_damage)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("powered", self.charged.load(Ordering::Relaxed));
        nbt.put_short("Fuse", self.fuse_time.load(Ordering::Relaxed) as i16);
        nbt.put_byte(
            "ExplosionRadius",
            self.explosion_radius.load(Ordering::Relaxed) as i8,
        );
        nbt.put_bool("ignited", self.ignited.load(Ordering::Relaxed));
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.set_charged(nbt.get_bool("powered").unwrap_or(false));
        self.fuse_time.store(
            nbt.get_short("Fuse").map_or(DEFAULT_FUSE_TIME, i32::from),
            Ordering::Relaxed,
        );
        self.explosion_radius.store(
            nbt.get_byte("ExplosionRadius")
                .map_or(DEFAULT_EXPLOSION_RADIUS, i32::from),
            Ordering::Relaxed,
        );
        if nbt.get_bool("ignited").unwrap_or(false) {
            self.set_ignited(true);
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_on_lightning_strike(
        &self,
        caller: &dyn EntityBase,
        lightning: &crate::entity::lightning::LightningBoltEntity,
    ) {
        self.mob_entity
            .living_entity
            .on_lightning_strike(caller, lightning);
        self.set_charged(true);
    }

    fn mob_tick(&self, caller: &dyn EntityBase) {
        let entity = &self.mob_entity.living_entity.entity;
        if !entity.is_alive() {
            return;
        }

        self.last_fuse_time.store(
            self.current_fuse_time.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );

        if self.ignited.load(Ordering::Relaxed) {
            self.set_fuse_speed(1);
        }

        let fuse_speed = self.fuse_speed.load(Ordering::Relaxed);
        let current = self.current_fuse_time.load(Ordering::Relaxed);

        if fuse_speed > 0 && current == 0 {
            caller.play_sound_at_self(Sound::EntityCreeperPrimed, 1.0, 0.5);
            entity.world.load().game_event_from(
                Some(caller),
                GameEvent::PrimeFuse,
                entity.pos.load(),
            );
        }

        let fuse_time = self.fuse_time.load(Ordering::Relaxed);
        let new_fuse = (current + fuse_speed).max(0);
        self.current_fuse_time.store(new_fuse, Ordering::Relaxed);

        if new_fuse >= fuse_time {
            self.current_fuse_time.store(fuse_time, Ordering::Relaxed);
            self.explode();
        }
    }

    /// Vanilla `Creeper.mobInteract`.
    fn mob_interact(
        &self,
        player: &Arc<Player>,
        hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        if !item_stack
            .item
            .has_tag(&tag::Item::MINECRAFT_CREEPER_IGNITERS)
        {
            return InteractionResult::PASS;
        }

        let sound = if item_stack.item.id == Item::FIRE_CHARGE.id {
            Sound::ItemFirechargeUse
        } else {
            Sound::ItemFlintandsteelUse
        };
        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load();
        let pitch = entity.random().next_f32() * 0.4 + 0.8;
        // `level.playSound(player, ...)`: the player plays its own copy.
        world.play_sound_raw_expect(
            player,
            sound as u16,
            self.sound_source(),
            &entity.pos.load(),
            1.0,
            pitch,
        );

        self.set_ignited(true);
        if item_stack.is_damageable() {
            player.hurt_and_break_in_hand(item_stack, hand, 1);
        } else {
            item_stack.decrement(1);
        }

        InteractionResult::SUCCESS
    }

    /// Vanilla `Creeper.setTarget`: creepers never target goats.
    fn set_mob_target(&self, target: Option<Arc<dyn EntityBase>>) {
        if target
            .as_ref()
            .is_some_and(|target| target.get_entity().entity_type == &EntityType::GOAT)
        {
            return;
        }
        set_mob_target_base(self, target);
    }

    /// Vanilla `Creeper.doHurtTarget`: a creeper's melee attack does nothing.
    fn goal_do_hurt_target(&self, _attacker: &dyn EntityBase, _target: &dyn EntityBase) -> bool {
        true
    }

    /// Vanilla `Creeper.killedEntity`: a charged creeper's first kill drops a skull.
    fn killed_entity(&self, victim: &dyn EntityBase) -> bool {
        if self.should_drop_loot()
            && self.is_charged()
            && !self.dropped_skulls.load(Ordering::Relaxed)
            && let Some(victim_living) = victim.get_living_entity()
        {
            victim_living.drop_from_loot_table(
                CHARGED_CREEPER_LOOT_TABLE,
                &LootContextParameters {
                    this_entity: Some(victim),
                    killer_entity: Some(self),
                    direct_killer_entity: Some(self),
                    position: Some(victim.get_entity().pos.load()),
                    ..Default::default()
                },
                |stack| {
                    victim.get_entity().spawn_at_location(stack);
                    self.dropped_skulls.store(true, Ordering::Relaxed);
                },
            );
        }
        true
    }
}

impl CreeperEntity {
    pub fn is_charged(&self) -> bool {
        self.charged.load(Ordering::Relaxed)
    }

    pub fn set_charged(&self, charged: bool) {
        self.charged.store(charged, Ordering::Relaxed);
        let entity = &self.mob_entity.living_entity.entity;
        entity.set_synced_data(pumpkin_data::tracked_data::creeper::CHARGED, charged);
    }

    pub fn is_ignited(&self) -> bool {
        self.ignited.load(Ordering::Relaxed)
    }

    pub fn set_ignited(&self, ignited: bool) {
        self.ignited.store(ignited, Ordering::Relaxed);
        let entity = &self.mob_entity.living_entity.entity;
        entity.set_synced_data(pumpkin_data::tracked_data::creeper::IS_IGNITED, ignited);
    }

    pub fn get_fuse(&self) -> i32 {
        self.fuse_time.load(Ordering::Relaxed)
    }

    pub fn set_fuse(&self, fuse: i32) {
        self.fuse_time.store(fuse, Ordering::Relaxed);
    }

    pub fn get_explosion_radius(&self) -> i32 {
        self.explosion_radius.load(Ordering::Relaxed)
    }

    pub fn set_explosion_radius(&self, radius: i32) {
        self.explosion_radius.store(radius, Ordering::Relaxed);
    }
}
