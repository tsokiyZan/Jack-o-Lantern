use crate::entity::ai::goal::run_around_like_crazy::RunAroundLikeCrazyGoal;
use crate::entity::ai::util::goal_utils;
use crate::entity::mob::mount_inventory::MountInventory;
use crate::entity::passive::abstract_horse::AbstractHorse;
use crate::entity::passive::animal::horse_can_parent;
use crate::entity::passive::llama::{CaravanData, LlamaFollowCaravanGoal};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering},
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data::{self, TrackedData};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::random::RandomImpl;
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
pub enum TraderLlamaVariant {
    #[default]
    Creamy = 0,
    White = 1,
    Brown = 2,
    Gray = 3,
}

impl TraderLlamaVariant {
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

pub struct TraderLlamaEntity {
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
    /// Vanilla `AbstractHorse.standAnim` and `standAnimO`.
    pub stand_anim: crossbeam::atomic::AtomicCell<[f32; 2]>,
    pub inventory: MountInventory,
    pub caravan: CaravanData,
    /// Vanilla `Llama.didSpit`: ends the retaliation after a spit.
    pub did_spit: AtomicBool,
}

impl TraderLlamaEntity {
    #[expect(clippy::too_many_lines)]
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        // Vanilla `Llama`'s constructor.
        mob_entity.set_required_path_length(40.0);
        let mut rng = rand::rng();
        let variant = TraderLlamaVariant::random_variant();
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
            stand_anim: crossbeam::atomic::AtomicCell::new([0.0; 2]),
            inventory: MountInventory::default(),
            did_spit: AtomicBool::new(false),
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
            goal_selector.add_goal(1, EscapeDangerGoal::new(2.0));

            // `LlamaHurtByTargetGoal`: a spit ends the retaliation.
            target_selector.add_goal(
                1,
                Box::new(
                    RevengeGoal::new(true)
                        .continuing_while(|mob| !super::llama::take_did_spit(mob)),
                ),
            );
            // `LlamaAttackWolfGoal`: untamed wolves, reaching them, at a quarter of the range.
            target_selector.add_goal(
                2,
                Box::new(ActiveTargetGoal::new(
                    &mob_arc.mob_entity,
                    &EntityType::WOLF,
                    16,
                    false,
                    true,
                    Some(
                        |target: &crate::entity::living::LivingEntity, _: &crate::world::World| {
                            !crate::entity::mob::is_tame(target)
                        },
                    ),
                ))
                .with_follow_distance_scale(0.25),
            );
            // `TraderLlama.registerGoals`, after the llama's.
            target_selector.add_goal(1, Box::new(DefendWanderingTraderGoal::new()));
            target_selector.add_goal(
                2,
                ActiveTargetGoal::predicated(&mob_arc.mob_entity, 10, true, |target, _world| {
                    goal_utils::is_instance_of(&EntityType::ZOMBIE, target.entity.entity_type)
                        && target.entity.entity_type != &EntityType::ZOMBIFIED_PIGLIN
                }),
            );
            target_selector.add_goal(
                2,
                ActiveTargetGoal::predicated(&mob_arc.mob_entity, 10, true, |target, _world| {
                    [
                        &EntityType::PILLAGER,
                        &EntityType::VINDICATOR,
                        &EntityType::EVOKER,
                        &EntityType::ILLUSIONER,
                    ]
                    .contains(&target.entity.entity_type)
                }),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn get_variant(&self) -> TraderLlamaVariant {
        TraderLlamaVariant::from_id(self.variant.load(Ordering::Relaxed))
    }

    pub fn set_variant(&self, variant: TraderLlamaVariant) {
        self.variant.store(variant.id(), Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::trader_llama::DATA_VARIANT_ID,
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
            pumpkin_data::tracked_data::trader_llama::DATA_STRENGTH_ID,
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
            pumpkin_data::tracked_data::trader_llama::DATA_ID_FLAGS,
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
        let yo = horizontal_distance * f64::from(0.2f32);

        spit.thrown.set_velocity(dx, dy + yo, dz, 1.5, 10.0);

        if !entity.silent.load(Ordering::Relaxed) {
            // `1.0F + (this.random.nextFloat() - this.random.nextFloat()) * 0.2F`.
            let pitch = {
                let mut random = entity.random();
                1.0 + (random.next_f32() - random.next_f32()) * 0.2
            };
            world.play_sound_fine(
                Sound::EntityLlamaSpit,
                SoundCategory::Neutral,
                &mob_pos,
                1.0,
                pitch,
            );
        }

        let spit_arc: Arc<dyn EntityBase> = Arc::new(spit);
        world.spawn_entity(spit_arc);
        self.did_spit.store(true, Ordering::Relaxed);
    }
}

impl AgeableMob for TraderLlamaEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for TraderLlamaEntity {
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
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::LLAMA_BABY_DIMENSIONS;

impl Mob for TraderLlamaEntity {
    fn mob_after_position_rider(&self, passenger: &dyn EntityBase) {
        self.turn_rider_body(passenger);
    }

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
        self.write_chest_nbt(nbt);
        nbt.put_bool("Tame", self.is_tame());
        nbt.put_int("Temper", self.temper.load(Ordering::Relaxed));
        if let Some(owner) = self.owner.load() {
            nbt.put_uuid("Owner", owner);
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        if let Some(variant) = nbt.get_int("Variant") {
            self.set_variant(TraderLlamaVariant::from_id(variant));
        }
        if let Some(strength) = nbt.get_int("Strength") {
            self.set_strength(strength);
        }
        self.read_chest_nbt(nbt);
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

    fn open_custom_inventory_screen(&self, player: &Arc<Player>) {
        self.open_horse_inventory(player);
    }

    fn drop_custom_death_loot(&self) {
        self.drop_horse_inventory();
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
            entity.set_synced_data(pumpkin_data::tracked_data::trader_llama::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::trader_llama::DATA_VARIANT_ID,
            VarInt(self.get_variant().id()),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::trader_llama::DATA_STRENGTH_ID,
            VarInt(self.get_strength()),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::trader_llama::DATA_ID_CHEST,
            self.has_chest(),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::trader_llama::DATA_ID_FLAGS,
            self.flags.load(Ordering::Relaxed) as i8,
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let item = item_stack.get_item();

        // Vanilla `AbstractChestedHorse.mobInteract` and `AbstractHorse.mobInteract`.
        if !self.is_vehicle()
            && !self.is_baby()
            && self.is_tame()
            && player.get_entity().is_sneaking()
        {
            self.open_custom_inventory_screen(player);
            return true;
        }
        if self.is_tame()
            && item == &Item::CHEST
            && !self.has_chest()
            && !self.is_baby()
            && self.equip_chest(player, item_stack)
        {
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

impl RangedAttackMob for TraderLlamaEntity {
    fn perform_ranged_attack(&self, target: &Arc<dyn EntityBase>, _power: f32) {
        self.spit(target);
    }
}

impl AbstractHorse for TraderLlamaEntity {
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

    fn stand_anim(&self) -> &crossbeam::atomic::AtomicCell<[f32; 2]> {
        &self.stand_anim
    }

    fn horse_inventory(&self) -> &MountInventory {
        &self.inventory
    }

    fn chest(&self) -> Option<(&AtomicBool, TrackedData)> {
        Some((&self.has_chest, tracked_data::trader_llama::DATA_ID_CHEST))
    }

    /// Vanilla `Llama.getInventoryColumns`.
    fn inventory_columns(&self) -> usize {
        if self.has_chest() {
            self.get_strength() as usize
        } else {
            0
        }
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

/// Vanilla `TraderLlama.TraderLlamaDefendWanderingTraderGoal`: a trader llama on its trader's
/// leash goes for whatever last hurt the trader.
struct DefendWanderingTraderGoal {
    track_target_goal: crate::entity::ai::goal::track_target::TrackTargetGoal,
    target_predicate: crate::entity::ai::target_predicate::TargetPredicate,
    owner_last_hurt_by: Option<Arc<dyn EntityBase>>,
    timestamp: i32,
}

impl DefendWanderingTraderGoal {
    fn new() -> Self {
        Self {
            track_target_goal: crate::entity::ai::goal::track_target::TrackTargetGoal::with_default(
                false,
            ),
            target_predicate:
                crate::entity::ai::target_predicate::TargetPredicate::create_attackable(),
            owner_last_hurt_by: None,
            timestamp: 0,
        }
    }

    /// The wandering trader holding the llama's leash.
    fn leash_trader(mob: &dyn Mob) -> Option<Arc<dyn EntityBase>> {
        mob.get_entity()
            .leashed_to
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .filter(|holder| holder.get_entity().entity_type == &EntityType::WANDERING_TRADER)
    }
}

impl crate::entity::ai::goal::Goal for DefendWanderingTraderGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(owner) = Self::leash_trader(mob) else {
            return false;
        };
        let Some(owner_living) = owner.get_living_entity() else {
            return false;
        };
        let world = mob.get_entity().world.load();
        self.owner_last_hurt_by = match owner_living.last_attacker_id.load(Ordering::Relaxed) {
            0 => None,
            id => world.get_entity_by_id(id),
        };
        let timestamp = owner_living.last_attacked_time.load(Ordering::Relaxed);
        timestamp != self.timestamp
            && self.track_target_goal.can_track(
                mob,
                self.owner_last_hurt_by.as_deref(),
                &self.target_predicate,
            )
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.track_target_goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.set_mob_target(self.owner_last_hurt_by.clone());
        if let Some(owner) = Self::leash_trader(mob)
            && let Some(owner_living) = owner.get_living_entity()
        {
            self.timestamp = owner_living.last_attacked_time.load(Ordering::Relaxed);
        }
        self.track_target_goal.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.track_target_goal.stop(mob);
    }

    fn controls(&self) -> crate::entity::ai::goal::Controls {
        crate::entity::ai::goal::Controls::TARGET
    }
}
