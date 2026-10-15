use crate::entity::ai::goal::run_around_like_crazy::RunAroundLikeCrazyGoal;
use crate::entity::ai::util::goal_utils;
use crate::entity::passive::abstract_horse::AbstractHorse;
use crate::entity::passive::animal::horse_can_parent;
use std::sync::{
    Arc, Mutex, PoisonError, Weak,
    atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering},
};

use crate::entity::ai::goal::{Controls, Goal, to_goal_ticks};
use crate::entity::ai::pathfinder::NavigatorGoal;
use pumpkin_util::math::vector3::Vector3;

use crossbeam::atomic::AtomicCell;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::EntityDimensions;
use rand::RngExt;
use uuid::Uuid;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        active_target::ActiveTargetGoal, breed::BreedGoal, escape_danger::EscapeDangerGoal,
        follow_parent::FollowParentGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, ranged_attack::RangedAttackGoal, revenge::RevengeGoal,
        swim::SwimGoal, tempt::TemptGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity, RangedAttackMob},
    passive::animal::{Animal, get_carpet_color_from_item, set_animal_pathfinding_malus},
    player::Player,
    projectile::llama_spit::LlamaSpitEntity,
};

const TEMPT_ITEMS: &[&Item] = &[&Item::HAY_BLOCK];

pub const FLAG_TAME: u8 = 2;
pub const FLAG_BRED: u8 = 8;
pub const FLAG_EATING: u8 = 16;
pub const FLAG_STANDING: u8 = 32;
pub const FLAG_OPEN_MOUTH: u8 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum LlamaVariant {
    #[default]
    Creamy = 0,
    White = 1,
    Brown = 2,
    Gray = 3,
}

impl LlamaVariant {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::White,
            2 => Self::Brown,
            3 => Self::Gray,
            _ => Self::Creamy,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub fn random_variant() -> Self {
        let mut rng = rand::rng();
        match rng.random_range(0..4) {
            1 => Self::White,
            2 => Self::Brown,
            3 => Self::Gray,
            _ => Self::Creamy,
        }
    }
}

pub struct LlamaEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub variant: AtomicI32,
    pub strength: AtomicI32,
    pub carpet_color: AtomicCell<Option<u8>>,
    pub flags: AtomicU8,
    pub has_chest: AtomicBool,
    pub temper: AtomicI32,
    pub owner: AtomicCell<Option<Uuid>>,
    /// Vanilla `AbstractHorse.standCounter`.
    pub stand_counter: AtomicI32,
    pub caravan: CaravanData,
}

impl LlamaEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        // Vanilla `Llama`'s constructor.
        mob_entity.set_required_path_length(40.0);
        let mut rng = rand::rng();
        let variant = LlamaVariant::random_variant();
        let strength = rng.random_range(1..=5);

        let llama = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            variant: AtomicI32::new(variant.id()),
            strength: AtomicI32::new(strength),
            carpet_color: AtomicCell::new(None),
            flags: AtomicU8::new(0),
            has_chest: AtomicBool::new(false),
            temper: AtomicI32::new(0),
            owner: AtomicCell::new(None),
            stand_counter: AtomicI32::new(0),
            caravan: CaravanData::default(),
        };
        let mob_arc = Arc::new(llama);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };
        let ranged_weak: Weak<dyn RangedAttackMob> = {
            let ranged_arc: Arc<dyn RangedAttackMob> = mob_arc.clone();
            Arc::downgrade(&ranged_arc)
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

            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(1, Box::new(RunAroundLikeCrazyGoal::new(1.2)));
            goal_selector.add_goal(2, Box::new(LlamaFollowCaravanGoal::new(2.1)));
            goal_selector.add_goal(
                3,
                Box::new(RangedAttackGoal::new(ranged_weak, 1.25, 40, 20.0)),
            );
            goal_selector.add_goal(3, EscapeDangerGoal::new(1.2));
            goal_selector.add_goal(4, BreedGoal::new(1.0));
            goal_selector.add_goal(5, Box::new(TemptGoal::new(1.25, TEMPT_ITEMS, false)));
            goal_selector.add_goal(6, Box::new(FollowParentGoal::new(1.0)));
            goal_selector.add_goal(7, Box::new(WanderAroundGoal::avoiding_water(0.7)));
            goal_selector.add_goal(
                8,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(9, Box::new(RandomLookAroundGoal::default()));

            // TODO: LlamaHurtByTargetGoal (stops after spitting) and LlamaAttackWolfGoal
            // (untamed wolves, reciprocal 16, must reach) are approximated.
            target_selector.add_goal(1, Box::new(RevengeGoal::new(true)));
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::WOLF, true),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn get_variant(&self) -> LlamaVariant {
        LlamaVariant::from_id(self.variant.load(Ordering::Relaxed))
    }

    pub fn set_variant(&self, variant: LlamaVariant) {
        self.variant.store(variant.id(), Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::llama::DATA_VARIANT_ID,
            VarInt(variant.id()),
        );
    }

    #[must_use]
    pub fn get_strength(&self) -> i32 {
        self.strength.load(Ordering::Relaxed)
    }

    pub fn set_strength(&self, strength: i32) {
        self.strength.store(strength, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::llama::DATA_STRENGTH_ID,
            VarInt(strength),
        );
    }

    #[must_use]
    pub fn has_flag(&self, flag: u8) -> bool {
        (self.flags.load(Ordering::Relaxed) & flag) != 0
    }

    pub fn set_flag(&self, flag: u8, val: bool) {
        let current = self.flags.load(Ordering::Relaxed);
        let new_flags = if val { current | flag } else { current & !flag };
        self.flags.store(new_flags, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::llama::DATA_ID_FLAGS,
            new_flags as i8,
        );
    }

    #[must_use]
    pub fn is_tame(&self) -> bool {
        self.has_flag(FLAG_TAME)
    }

    pub fn set_tame(&self, val: bool) {
        self.set_flag(FLAG_TAME, val);
    }

    #[must_use]
    pub fn has_chest(&self) -> bool {
        self.has_chest.load(Ordering::Relaxed)
    }

    pub fn set_has_chest(&self, val: bool) {
        self.has_chest.store(val, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(pumpkin_data::tracked_data::llama::DATA_ID_CHEST, val);
    }

    pub fn spit(&self, target: &Arc<dyn EntityBase>) {
        let entity = self.get_entity();
        let world = entity.world.load_full();

        let spit_entity = Entity::new(world.clone(), entity.pos.load(), &EntityType::LLAMA_SPIT);
        let spit = LlamaSpitEntity::new_shot(spit_entity, entity);

        let mob_pos = entity.pos.load();
        let target_entity = target.get_entity();
        let target_pos = target_entity.pos.load();
        let target_height = f64::from(target_entity.entity_dimension.load().height);

        let dx = target_pos.x - mob_pos.x;
        let dy = (target_pos.y + target_height / 3.0) - spit.get_entity().pos.load().y;
        let dz = target_pos.z - mob_pos.z;
        let horizontal_distance = dx.hypot(dz);
        let yo = horizontal_distance * 0.2;

        spit.thrown.set_velocity(dx, dy + yo, dz, 1.5, 10.0);

        if !entity.silent.load(Ordering::Relaxed) {
            world.play_sound(Sound::EntityLlamaSpit, SoundCategory::Neutral, &mob_pos);
        }

        let spit_arc: Arc<dyn EntityBase> = Arc::new(spit);
        world.spawn_entity(spit_arc);
    }
}

impl AgeableMob for LlamaEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for LlamaEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_LLAMA_FOOD)
            || item_stack
                .item
                .has_tag(&tag::Item::MINECRAFT_LLAMA_TEMPT_ITEMS)
            || item_stack.item == &Item::WHEAT
            || item_stack.item == &Item::HAY_BLOCK
    }

    /// Vanilla `Llama.canMate`.
    fn can_mate(&self, partner: &dyn Mob) -> bool {
        partner.get_entity().entity_id != self.get_entity().entity_id
            && goal_utils::is_instance_of(&EntityType::LLAMA, partner.get_entity().entity_type)
            && self.can_parent()
            && partner.as_animal().is_some_and(Animal::can_parent)
    }

    fn can_parent(&self) -> bool {
        horse_can_parent(self, self.is_tame())
    }
}

/// Vanilla `Llama.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = Entity::type_dimensions(&EntityType::LLAMA).scale(0.5);

impl Mob for LlamaEntity {
    fn as_abstract_horse(&self) -> Option<&dyn AbstractHorse> {
        Some(self)
    }

    /// Vanilla `Llama.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_ageable_nbt(nbt);
        nbt.put_int("Variant", self.get_variant().id());
        nbt.put_int("Strength", self.get_strength());
        nbt.put_bool("ChestedHorse", self.has_chest());
        nbt.put_bool("Tame", self.is_tame());
        nbt.put_int("Temper", self.temper.load(Ordering::Relaxed));
        if let Some(owner) = self.owner.load() {
            nbt.put_uuid("Owner", owner);
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        if let Some(variant) = nbt.get_int("Variant") {
            self.set_variant(LlamaVariant::from_id(variant));
        }
        if let Some(strength) = nbt.get_int("Strength") {
            self.set_strength(strength);
        }
        if let Some(chested) = nbt.get_bool("ChestedHorse") {
            self.set_has_chest(chested);
        }
        if let Some(tame) = nbt.get_bool("Tame") {
            self.set_tame(tame);
        }
        if let Some(temper) = nbt.get_int("Temper") {
            self.temper.store(temper, Ordering::Relaxed);
        }
        if let Some(owner) = nbt.get_uuid("Owner") {
            self.owner.store(Some(owner));
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        AbstractHorse::tick_standing(self);
        self.ageable_ai_step();
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::llama::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::llama::DATA_VARIANT_ID,
            VarInt(self.get_variant().id()),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::llama::DATA_STRENGTH_ID,
            VarInt(self.get_strength()),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::llama::DATA_ID_CHEST,
            self.has_chest(),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::llama::DATA_ID_FLAGS,
            self.flags.load(Ordering::Relaxed) as i8,
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let item = item_stack.get_item();

        if self.is_tame() && item == &Item::CHEST && !self.has_chest() && !self.is_baby() {
            self.set_has_chest(true);
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            let entity = self.get_entity();
            let world = entity.world.load();
            world.play_sound(
                Sound::EntityDonkeyChest,
                SoundCategory::Neutral,
                &entity.pos.load(),
            );
            return true;
        }

        if self.is_tame()
            && !self.is_baby()
            && let Some(color) = get_carpet_color_from_item(item)
        {
            self.carpet_color.store(Some(color));
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            let entity = self.get_entity();
            let world = entity.world.load();
            world.play_sound(
                Sound::EntityLlamaSwag,
                SoundCategory::Neutral,
                &entity.pos.load(),
            );
            return true;
        }

        if !self.is_baby() && !self.is_food(item_stack) {
            let world = player.world();
            let ent = &self.mob_entity.living_entity.entity;
            if let Some(vehicle) = world.get_entity_by_id(ent.entity_id)
                && let Some(passenger) = world.get_player_by_id(player.entity_id())
            {
                ent.add_passenger(vehicle, passenger as Arc<dyn EntityBase>);
                return true;
            }
        }

        self.animal_interact(player, item_stack, Sound::EntityLlamaAmbient)
    }
}

impl RangedAttackMob for LlamaEntity {
    fn perform_ranged_attack(&self, target: &Arc<dyn EntityBase>, _power: f32) {
        self.spit(target);
    }
}

impl AbstractHorse for LlamaEntity {
    fn horse_flag(&self, flag: u8) -> bool {
        (self.flags.load(Ordering::Relaxed) & flag) != 0
    }

    fn set_horse_flag(&self, flag: u8, value: bool) {
        self.set_flag(flag, value);
    }

    fn set_horse_tamed(&self, tamed: bool) {
        self.set_tame(tamed);
    }

    fn set_horse_owner(&self, owner: Uuid) {
        self.owner.store(Some(owner));
    }

    fn horse_temper(&self) -> &AtomicI32 {
        &self.temper
    }

    fn stand_counter(&self) -> &AtomicI32 {
        &self.stand_counter
    }

    fn angry_sound(&self) -> Option<Sound> {
        Some(Sound::EntityLlamaAngry)
    }

    fn max_temper(&self) -> i32 {
        30
    }

    fn can_perform_rearing(&self) -> bool {
        false
    }
}

/// Vanilla `Llama.caravanHead` and `caravanTail`, for llamas and trader llamas.
#[derive(Default)]
pub struct CaravanData {
    head: Mutex<Option<Weak<dyn EntityBase>>>,
    tail: Mutex<Option<Weak<dyn EntityBase>>>,
}

impl CaravanData {
    fn head(&self) -> Option<Arc<dyn EntityBase>> {
        self.head
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade)
    }

    /// Vanilla `inCaravan`.
    fn in_caravan(&self) -> bool {
        self.head().is_some()
    }

    /// Vanilla `hasCaravanTail`.
    fn has_caravan_tail(&self) -> bool {
        self.tail
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|tail| tail.strong_count() > 0)
    }

    /// Vanilla `leaveCaravan`.
    fn leave_caravan(&self) {
        if let Some(head) = self.head()
            && let Some(head_caravan) = caravan_of(head.as_ref())
        {
            *head_caravan
                .tail
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = None;
        }
        *self.head.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Vanilla `joinCaravan`.
    fn join_caravan(&self, this: &Arc<dyn EntityBase>, head: &Arc<dyn EntityBase>) {
        *self.head.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::downgrade(head));
        if let Some(head_caravan) = caravan_of(head.as_ref()) {
            *head_caravan
                .tail
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(Arc::downgrade(this));
        }
    }
}

/// The caravan links of a llama or trader llama.
fn caravan_of(entity: &dyn EntityBase) -> Option<&CaravanData> {
    let any = entity.get_mob()?.cast_any();
    if let Some(llama) = any.downcast_ref::<LlamaEntity>() {
        return Some(&llama.caravan);
    }
    any.downcast_ref::<crate::entity::passive::trader_llama::TraderLlamaEntity>()
        .map(|llama| &llama.caravan)
}

/// Vanilla `LlamaFollowCaravanGoal.CARAVAN_LIMIT`.
const CARAVAN_LIMIT: i32 = 8;

/// Vanilla `LlamaFollowCaravanGoal`: a llama near a leashed llama, or the end of a caravan
/// led by one, joins on behind it and keeps 2 blocks back.
pub struct LlamaFollowCaravanGoal {
    speed_modifier: f64,
    dist_check_counter: i32,
}

impl LlamaFollowCaravanGoal {
    #[must_use]
    pub const fn new(speed_modifier: f64) -> Self {
        Self {
            speed_modifier,
            dist_check_counter: 0,
        }
    }

    /// Vanilla `firstIsLeashed`: whether the caravan ahead of `llama` is led by a leash.
    fn first_is_leashed(llama: &dyn EntityBase, mut counter: i32) -> bool {
        if counter > CARAVAN_LIMIT {
            return false;
        }
        let Some(head) = caravan_of(llama).and_then(CaravanData::head) else {
            return false;
        };
        if head.get_entity().is_leashed() {
            return true;
        }
        counter += 1;
        Self::first_is_leashed(head.as_ref(), counter)
    }
}

impl Goal for LlamaFollowCaravanGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let entity = mob.get_entity();
        let Some(caravan) = caravan_of(mob) else {
            return false;
        };
        if entity.is_leashed() || caravan.in_caravan() {
            return false;
        }
        let world = entity.world.load();
        let pos = entity.pos.load();
        let llamas: Vec<_> = world
            .get_entities_at_box(&entity.bounding_box.load().expand(9.0, 4.0, 9.0))
            .into_iter()
            .filter(|other| {
                let other_entity = other.get_entity();
                other_entity.entity_id != entity.entity_id
                    && (other_entity.entity_type == &EntityType::LLAMA
                        || other_entity.entity_type == &EntityType::TRADER_LLAMA)
            })
            .collect();
        let closest_where = |test: &dyn Fn(&dyn EntityBase, &CaravanData) -> bool| {
            let mut closest = None;
            let mut closest_dist = f64::MAX;
            for candidate in &llamas {
                let Some(candidate_caravan) = caravan_of(candidate.as_ref()) else {
                    continue;
                };
                if test(candidate.as_ref(), candidate_caravan) {
                    let dist = pos.squared_distance_to_vec(&candidate.get_entity().pos.load());
                    if dist <= closest_dist {
                        closest_dist = dist;
                        closest = Some(candidate.clone());
                    }
                }
            }
            closest.map(|closest| (closest, closest_dist))
        };
        let Some((closest, closest_dist)) =
            closest_where(&|_, candidate| candidate.in_caravan() && !candidate.has_caravan_tail())
                .or_else(|| {
                    closest_where(&|candidate, candidate_caravan| {
                        candidate.get_entity().is_leashed() && !candidate_caravan.has_caravan_tail()
                    })
                })
        else {
            return false;
        };
        if closest_dist < 4.0
            || (!closest.get_entity().is_leashed() && !Self::first_is_leashed(closest.as_ref(), 1))
        {
            return false;
        }
        let Some(this) = world.get_entity_by_id(entity.entity_id) else {
            return false;
        };
        caravan.join_caravan(&this, &closest);
        true
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let Some(head) = caravan_of(mob).and_then(CaravanData::head) else {
            return false;
        };
        if !head.get_entity().is_alive() || !Self::first_is_leashed(mob, 0) {
            return false;
        }
        let dist_sq = mob
            .get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&head.get_entity().pos.load());
        if dist_sq > 676.0 {
            if self.speed_modifier <= 3.0 {
                self.speed_modifier *= 1.2;
                self.dist_check_counter = to_goal_ticks(40);
                return true;
            }
            if self.dist_check_counter == 0 {
                return false;
            }
        }
        if self.dist_check_counter > 0 {
            self.dist_check_counter -= 1;
        }
        true
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(caravan) = caravan_of(mob) {
            caravan.leave_caravan();
        }
        self.speed_modifier = 2.1;
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let entity = mob.get_entity();
        let Some(head) = caravan_of(mob).and_then(CaravanData::head) else {
            return;
        };
        let on_fence_knot = entity
            .leashed_to
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|holder| holder.get_entity().entity_type == &EntityType::LEASH_KNOT);
        if on_fence_knot {
            return;
        }
        let pos = entity.pos.load();
        let head_pos = head.get_entity().pos.load();
        let distance = pos.squared_distance_to_vec(&head_pos).sqrt();
        let delta = head_pos - pos;
        let length = delta.length();
        // `Vec3.normalize`, then `scale(max(distanceTo - 2.0, 0.0))`.
        let delta = if length < 1.0E-5 {
            Vector3::new(0.0, 0.0, 0.0)
        } else {
            delta * ((distance - 2.0).max(0.0) / length)
        };
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_progress(NavigatorGoal::new(pos, pos + delta, self.speed_modifier));
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}
