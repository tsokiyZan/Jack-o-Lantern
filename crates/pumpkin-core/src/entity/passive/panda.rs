use std::sync::{
    Arc, Mutex, PoisonError, Weak,
    atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering},
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::Block;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::{EntityPose, EntityStatus, EntityType};
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::GameMode;
use pumpkin_util::math;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::entity::ai::control::move_control::MoveControl;
use crate::entity::ai::control::{Control, MoveControlTrait};
use crate::entity::ai::goal::avoid_entity::AvoidEntityGoal;
use crate::entity::ai::goal::melee_attack::MeleeAttackGoal;
use crate::entity::ai::goal::revenge::RevengeGoal;
use crate::entity::ai::goal::{Controls, Goal, to_goal_ticks};
use crate::entity::ai::target_predicate::TargetPredicate;
use crate::entity::attributes::send_attribute_updates_for_living;
use crate::entity::item::ItemEntity;
use crate::entity::mob::is_monster;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::r#type::from_type;
use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        breed::BreedGoal, escape_danger::EscapeDangerGoal, follow_parent::FollowParentGoal,
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal,
        tempt::TemptGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};
use crate::world::World;
use crate::world::loot::LootContextParameters;

pub const FLAG_SNEEZE: u8 = 2;
pub const FLAG_ROLL: u8 = 4;
pub const FLAG_SIT: u8 = 8;
pub const FLAG_ON_BACK: u8 = 16;
const EAT_TICK_INTERVAL: i32 = 5;
pub const TOTAL_ROLL_STEPS: i32 = 32;
const TOTAL_UNHAPPY_TIME: i32 = 32;

/// Vanilla `BuiltInLootTables.PANDA_SNEEZE`.
const PANDA_SNEEZE_LOOT_TABLE: &str = "minecraft:gameplay/panda_sneeze";

/// Vanilla `Panda.Gene`: the main and hidden genes decide the variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum PandaGene {
    #[default]
    Normal = 0,
    Lazy = 1,
    Worried = 2,
    Playful = 3,
    Brown = 4,
    Weak = 5,
    Aggressive = 6,
}

impl PandaGene {
    /// Vanilla `Gene.byId`, out of range ids read as `NORMAL`.
    #[must_use]
    pub const fn by_id(id: u8) -> Self {
        match id {
            1 => Self::Lazy,
            2 => Self::Worried,
            3 => Self::Playful,
            4 => Self::Brown,
            5 => Self::Weak,
            6 => Self::Aggressive,
            _ => Self::Normal,
        }
    }

    #[must_use]
    pub const fn id(self) -> u8 {
        self as u8
    }

    /// Vanilla `Gene.getSerializedName`.
    #[must_use]
    pub const fn serialized_name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Lazy => "lazy",
            Self::Worried => "worried",
            Self::Playful => "playful",
            Self::Brown => "brown",
            Self::Weak => "weak",
            Self::Aggressive => "aggressive",
        }
    }

    /// Vanilla `Gene.CODEC`, which fails on an unknown name.
    #[must_use]
    pub fn from_serialized_name(name: &str) -> Option<Self> {
        Some(match name {
            "normal" => Self::Normal,
            "lazy" => Self::Lazy,
            "worried" => Self::Worried,
            "playful" => Self::Playful,
            "brown" => Self::Brown,
            "weak" => Self::Weak,
            "aggressive" => Self::Aggressive,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn is_recessive(self) -> bool {
        matches!(self, Self::Brown | Self::Weak)
    }

    /// Vanilla `Gene.getVariantFromGenes`: a recessive main gene only shows when the hidden
    /// gene matches it.
    #[must_use]
    pub fn get_variant_from_genes(main_gene: Self, hidden_gene: Self) -> Self {
        if main_gene.is_recessive() {
            if main_gene == hidden_gene {
                main_gene
            } else {
                Self::Normal
            }
        } else {
            main_gene
        }
    }

    /// Vanilla `Gene.getRandom`.
    pub fn get_random(random: &mut impl RandomImpl) -> Self {
        let next_int = random.next_bounded_i32(16);
        if next_int == 0 {
            Self::Lazy
        } else if next_int == 1 {
            Self::Worried
        } else if next_int == 2 {
            Self::Playful
        } else if next_int == 4 {
            Self::Aggressive
        } else if next_int < 9 {
            Self::Weak
        } else if next_int < 11 {
            Self::Brown
        } else {
            Self::Normal
        }
    }
}

pub struct PandaEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub main_gene: AtomicU8,
    pub hidden_gene: AtomicU8,
    pub flags: AtomicU8,
    pub eat_counter: AtomicI32,
    pub sneeze_counter: AtomicI32,
    pub unhappy_counter: AtomicI32,
    got_bamboo: AtomicBool,
    did_bite: AtomicBool,
    pub roll_counter: AtomicI32,
    roll_delta: AtomicCell<Vector3<f64>>,
    /// Vanilla `PandaLookAtPlayerGoal.lookAt`, which the breed goal also sets.
    look_at_player: Mutex<Option<Arc<dyn EntityBase>>>,
}

impl PandaEntity {
    #[expect(
        clippy::too_many_lines,
        reason = "vanilla registerGoals plus the constructor"
    )]
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Box::new(PandaMoveControl::default());
        // Vanilla's constructor; `/summon` reads `CanPickUpLoot` from the tag afterwards.
        if !mob_entity.living_entity.entity.is_baby() {
            mob_entity.set_can_pick_up_loot(true);
        }
        let panda = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            main_gene: AtomicU8::new(PandaGene::Normal.id()),
            hidden_gene: AtomicU8::new(PandaGene::Normal.id()),
            flags: AtomicU8::new(0),
            eat_counter: AtomicI32::new(0),
            sneeze_counter: AtomicI32::new(0),
            unhappy_counter: AtomicI32::new(0),
            got_bamboo: AtomicBool::new(false),
            did_bite: AtomicBool::new(false),
            roll_counter: AtomicI32::new(0),
            roll_delta: AtomicCell::new(Vector3::default()),
            look_at_player: Mutex::new(None),
        };
        let mob_arc = Arc::new(panda);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        // Vanilla `Panda.registerGoals`.
        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(
                2,
                EscapeDangerGoal::with_panic_causes(
                    2.0,
                    &tag::DamageType::MINECRAFT_PANIC_ENVIRONMENTAL_CAUSES,
                )
                .continuing_while(panda_panic_can_continue),
            );
            goal_selector.add_goal(2, Box::new(PandaBreedGoal::new(1.0)));
            goal_selector.add_goal(
                3,
                Box::new(PandaAttackGoal(MeleeAttackGoal::new(
                    f64::from(1.2f32),
                    true,
                ))),
            );
            goal_selector.add_goal(
                4,
                Box::new(TemptGoal::with_tag(
                    1.0,
                    &tag::Item::MINECRAFT_PANDA_FOOD,
                    false,
                )),
            );
            goal_selector.add_goal(
                6,
                Box::new(PandaAvoidGoal(AvoidEntityGoal::new(
                    &EntityType::PLAYER,
                    8.0,
                    2.0,
                    2.0,
                ))),
            );
            goal_selector.add_goal(
                6,
                Box::new(PandaAvoidGoal(
                    AvoidEntityGoal::any_mob(4.0, 2.0, 2.0)
                        .avoiding_only(|target| is_monster(target.entity.entity_type)),
                )),
            );
            goal_selector.add_goal(7, Box::new(PandaSitGoal { cooldown: 0 }));
            goal_selector.add_goal(8, Box::new(PandaLieOnBackGoal { cooldown: 0 }));
            goal_selector.add_goal(8, Box::new(PandaSneezeGoal));
            goal_selector.add_goal(9, Box::new(PandaLookAtPlayerGoal::new(mob_weak, 6.0)));
            goal_selector.add_goal(10, Box::new(RandomLookAroundGoal::default()));
            goal_selector.add_goal(12, Box::new(PandaRollGoal));
            goal_selector.add_goal(13, Box::new(FollowParentGoal::new(1.25)));
            goal_selector.add_goal(14, Box::new(WanderAroundGoal::avoiding_water(1.0)));
        };

        {
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // Vanilla `PandaHurtByTargetGoal(this).setAlertOthers()`.
            target_selector.add_goal(
                1,
                Box::new(
                    RevengeGoal::new(true)
                        .alerting_others()
                        .alerting_only(is_aggressive_panda)
                        .continuing_while(panda_keeps_grudge),
                ),
            );
        };

        mob_arc
    }

    fn main_hand(&self) -> ItemStack {
        self.mob_entity
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&EquipmentSlot::MAIN_HAND)
    }

    #[must_use]
    pub fn get_unhappy_counter(&self) -> i32 {
        self.unhappy_counter.load(Ordering::Relaxed)
    }

    pub fn set_unhappy_counter(&self, value: i32) {
        self.unhappy_counter.store(value, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::panda::UNHAPPY_COUNTER,
            VarInt(value),
        );
    }

    #[must_use]
    pub fn is_sneezing(&self) -> bool {
        self.get_flag(FLAG_SNEEZE)
    }

    #[must_use]
    pub fn is_sitting(&self) -> bool {
        self.get_flag(FLAG_SIT)
    }

    /// Vanilla `Panda.sit`.
    pub fn sit(&self, value: bool) {
        self.set_flag(FLAG_SIT, value);
    }

    #[must_use]
    pub fn is_on_back(&self) -> bool {
        self.get_flag(FLAG_ON_BACK)
    }

    pub fn set_on_back(&self, value: bool) {
        self.set_flag(FLAG_ON_BACK, value);
    }

    #[must_use]
    pub fn is_eating(&self) -> bool {
        self.get_eat_counter() > 0
    }

    /// Vanilla `Panda.eat`.
    pub fn eat(&self, value: bool) {
        self.set_eat_counter(i32::from(value));
    }

    fn get_eat_counter(&self) -> i32 {
        self.eat_counter.load(Ordering::Relaxed)
    }

    fn set_eat_counter(&self, value: i32) {
        self.eat_counter.store(value, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::panda::EAT_COUNTER,
            VarInt(value),
        );
    }

    /// Vanilla `Panda.sneeze`.
    pub fn sneeze(&self, value: bool) {
        self.set_flag(FLAG_SNEEZE, value);
        if !value {
            self.set_sneeze_counter(0);
        }
    }

    #[must_use]
    pub fn get_sneeze_counter(&self) -> i32 {
        self.sneeze_counter.load(Ordering::Relaxed)
    }

    pub fn set_sneeze_counter(&self, value: i32) {
        self.sneeze_counter.store(value, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::panda::SNEEZE_COUNTER,
            VarInt(value),
        );
    }

    #[must_use]
    pub fn get_main_gene(&self) -> PandaGene {
        PandaGene::by_id(self.main_gene.load(Ordering::Relaxed))
    }

    pub fn set_main_gene(&self, gene: PandaGene) {
        self.main_gene.store(gene.id(), Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::panda::MAIN_GENE_ID,
            gene.id() as i8,
        );
    }

    #[must_use]
    pub fn get_hidden_gene(&self) -> PandaGene {
        PandaGene::by_id(self.hidden_gene.load(Ordering::Relaxed))
    }

    pub fn set_hidden_gene(&self, gene: PandaGene) {
        self.hidden_gene.store(gene.id(), Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::panda::HIDDEN_GENE_ID,
            gene.id() as i8,
        );
    }

    #[must_use]
    pub fn is_rolling(&self) -> bool {
        self.get_flag(FLAG_ROLL)
    }

    /// Vanilla `Panda.roll`.
    pub fn roll(&self, value: bool) {
        self.set_flag(FLAG_ROLL, value);
    }

    fn get_flag(&self, flag: u8) -> bool {
        (self.flags.load(Ordering::Relaxed) & flag) != 0
    }

    fn set_flag(&self, flag: u8, value: bool) {
        let current = self.flags.load(Ordering::Relaxed);
        let flags = if value {
            current | flag
        } else {
            current & !flag
        };
        self.flags.store(flags, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::panda::DATA_ID_FLAGS,
            flags as i8,
        );
    }

    #[must_use]
    pub fn get_variant(&self) -> PandaGene {
        PandaGene::get_variant_from_genes(self.get_main_gene(), self.get_hidden_gene())
    }

    #[must_use]
    pub fn is_lazy(&self) -> bool {
        self.get_variant() == PandaGene::Lazy
    }

    #[must_use]
    pub fn is_worried(&self) -> bool {
        self.get_variant() == PandaGene::Worried
    }

    #[must_use]
    pub fn is_playful(&self) -> bool {
        self.get_variant() == PandaGene::Playful
    }

    #[must_use]
    pub fn is_brown(&self) -> bool {
        self.get_variant() == PandaGene::Brown
    }

    #[must_use]
    pub fn is_weak(&self) -> bool {
        self.get_variant() == PandaGene::Weak
    }

    /// Vanilla `Panda.isAggressive`, which is the gene and not the mob's aggressive flag.
    #[must_use]
    pub fn is_aggressive(&self) -> bool {
        self.get_variant() == PandaGene::Aggressive
    }

    /// Vanilla `Panda.isScared`.
    #[must_use]
    pub fn is_scared(&self) -> bool {
        self.is_worried() && self.get_entity().world.load().is_thundering()
    }

    /// Vanilla `Panda.canPerformAction`.
    #[must_use]
    pub fn can_perform_action(&self) -> bool {
        !self.is_on_back()
            && !self.is_scared()
            && !self.is_eating()
            && !self.is_rolling()
            && !self.is_sitting()
    }

    /// The part of vanilla `Panda.tick` after `super.tick()`.
    fn panda_tick(&self) {
        let entity = self.get_entity();
        if self.is_worried() {
            if entity.world.load().is_thundering() && !entity.is_in_water() {
                self.sit(true);
                self.eat(false);
            } else if !self.is_eating() {
                self.sit(false);
            }
        }

        let target = self.mob_entity.get_target();
        if target.is_none() {
            self.got_bamboo.store(false, Ordering::Relaxed);
            self.did_bite.store(false, Ordering::Relaxed);
        }

        if self.get_unhappy_counter() > 0 {
            if let Some(target) = &target {
                self.mob_entity.look_at(target.as_ref(), 90.0, 90.0);
            }
            if self.get_unhappy_counter() == 29 || self.get_unhappy_counter() == 14 {
                self.play_sound_at_self(Sound::EntityPandaCantBreed, 1.0, 1.0);
            }
            self.set_unhappy_counter(self.get_unhappy_counter() - 1);
        }

        if self.is_sneezing() {
            self.set_sneeze_counter(self.get_sneeze_counter() + 1);
            if self.get_sneeze_counter() > 20 {
                self.sneeze(false);
                self.after_sneeze();
            } else if self.get_sneeze_counter() == 1 {
                self.play_sound_at_self(Sound::EntityPandaPreSneeze, 1.0, 1.0);
            }
        }

        if self.is_rolling() {
            self.handle_roll();
        } else {
            self.roll_counter.store(0, Ordering::Relaxed);
        }

        if self.is_sitting() {
            entity.set_pitch(0.0);
        }

        // `updateSitAmount`, `updateOnBackAnimation` and `updateRollAmount` only animate the
        // client's model.
        self.handle_eating();
    }

    /// Vanilla `Panda.handleEating`.
    fn handle_eating(&self) {
        if !self.is_eating()
            && self.is_sitting()
            && !self.is_scared()
            && !self.main_hand().is_empty()
            && self.get_entity().random().next_bounded_i32(80) == 1
        {
            self.eat(true);
        } else if self.main_hand().is_empty() || !self.is_sitting() {
            self.eat(false);
        }

        if !self.is_eating() {
            return;
        }
        self.add_eating_particles();
        if self.get_eat_counter() > 80 && self.get_entity().random().next_bounded_i32(20) == 1 {
            if self.get_eat_counter() > 100
                && self
                    .main_hand()
                    .item
                    .has_tag(&tag::Item::MINECRAFT_PANDA_EATS_FROM_GROUND)
            {
                self.mob_entity
                    .set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::EMPTY.clone());
                let entity = self.get_entity();
                entity
                    .world
                    .load()
                    .game_event_from(Some(self), GameEvent::Eat, entity.pos.load());
                self.sit(false);
            }
            self.eat(false);
            return;
        }
        self.set_eat_counter(self.get_eat_counter() + 1);
    }

    /// Vanilla `Panda.addEatingParticles`. The item particles are the client's own, so only the
    /// sound is left on the server, but the server still draws the particles' random numbers.
    fn add_eating_particles(&self) {
        if self.get_eat_counter() % EAT_TICK_INTERVAL == 0 {
            let (volume, pitch) = {
                let mut random = self.get_entity().random();
                let volume = 0.5 + 0.5 * random.next_bounded_i32(2) as f32;
                (volume, (random.next_f32() - random.next_f32()) * 0.2 + 1.0)
            };
            self.play_sound_at_self(Sound::EntityPandaEat, volume, pitch);
            if !self.main_hand().is_empty() {
                // Six particles, each with three velocity and three position `nextFloat`s;
                // `addParticle` does nothing on the server.
                let mut random = self.get_entity().random();
                for _ in 0..6 * 6 {
                    random.next_f32();
                }
            }
        }
    }

    /// Vanilla `Panda.handleRoll`.
    fn handle_roll(&self) {
        let roll_counter = self.roll_counter.fetch_add(1, Ordering::Relaxed) + 1;
        if roll_counter > TOTAL_ROLL_STEPS {
            self.roll(false);
            return;
        }
        let entity = self.get_entity();
        let movement = entity.velocity.load();
        if roll_counter == 1 {
            let angle = entity.yaw.load() * (std::f64::consts::PI / 180.0) as f32;
            let multiplier = if entity.is_baby() { 0.1f32 } else { 0.2f32 };
            let roll_delta = Vector3::new(
                movement.x + f64::from(-math::sin(angle) * multiplier),
                0.0,
                movement.z + f64::from(math::cos(angle) * multiplier),
            );
            self.roll_delta.store(roll_delta);
            entity.velocity.store(Vector3::new(
                roll_delta.x,
                roll_delta.y + 0.27,
                roll_delta.z,
            ));
        } else if roll_counter != 7 && roll_counter != 15 && roll_counter != 23 {
            let roll_delta = self.roll_delta.load();
            entity
                .velocity
                .store(Vector3::new(roll_delta.x, movement.y, roll_delta.z));
        } else {
            let y = if entity.on_ground.load(Ordering::Relaxed) {
                0.27
            } else {
                movement.y
            };
            entity.velocity.store(Vector3::new(0.0, y, 0.0));
        }
    }

    /// Vanilla `Panda.afterSneeze`. The sneeze particle is the client's own.
    fn after_sneeze(&self) {
        self.play_sound_at_self(Sound::EntityPandaSneeze, 1.0, 1.0);
        let entity = self.get_entity();
        let world = entity.world.load_full();
        let area = entity.bounding_box.load().expand(10.0, 10.0, 10.0);
        for other in world.get_entities_at_box(&area) {
            let Some(panda) = other.cast_any().downcast_ref::<Self>() else {
                continue;
            };
            let panda_entity = panda.get_entity();
            if !panda_entity.is_baby()
                && panda_entity.on_ground.load(Ordering::Relaxed)
                && !panda_entity.is_in_water()
                && panda.can_perform_action()
            {
                panda.mob_entity.living_entity.jump();
            }
        }

        let level_info = world.level_info.load();
        if level_info.game_rules.mob_drops
            && let Some(loot_table) = world.get_loot_table(PANDA_SNEEZE_LOOT_TABLE)
        {
            // `dropFromGiftLootTable`: the gift context has the origin and this entity.
            let params = LootContextParameters {
                this_entity: Some(entity.entity_type),
                position: Some(entity.pos.load()),
                ..Default::default()
            };
            for stack in
                crate::world::loot::generate_loot_from_handle(&loot_table, rand::random(), &params)
            {
                entity.spawn_at_location(stack);
            }
        }
    }

    /// Vanilla `Panda.pickUpItem`: an empty paw takes the whole stack, if it is food.
    fn panda_pick_up_item(&self, item: &ItemEntity) {
        if !self.main_hand().is_empty() || !can_pick_up_and_eat(item) {
            return;
        }
        let stack = item
            .get_item_stack()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        // `take` tells the clients before the item goes.
        if !self
            .mob_entity
            .living_entity
            .pickup(item.get_entity(), u32::from(stack.item_count))
        {
            return;
        }
        self.set_item_slot_and_notify(&EquipmentSlot::MAIN_HAND, stack);
        self.mob_entity
            .set_guaranteed_drop(&EquipmentSlot::MAIN_HAND);
        item.get_entity().remove();
    }

    /// Vanilla `Panda.setGeneFromParents`.
    pub fn set_gene_from_parents(&self, parent1: &Self, parent2: Option<&Self>) {
        // `this.random`, one lock per draw: the parents draw from their own randoms in
        // between.
        let random = || self.get_entity().random();
        if let Some(parent2) = parent2 {
            if random().next_bool() {
                self.set_main_gene(parent1.get_one_of_genes_randomly());
                self.set_hidden_gene(parent2.get_one_of_genes_randomly());
            } else {
                self.set_main_gene(parent2.get_one_of_genes_randomly());
                self.set_hidden_gene(parent1.get_one_of_genes_randomly());
            }
        } else if random().next_bool() {
            self.set_main_gene(parent1.get_one_of_genes_randomly());
            let gene = PandaGene::get_random(&mut *random());
            self.set_hidden_gene(gene);
        } else {
            let gene = PandaGene::get_random(&mut *random());
            self.set_main_gene(gene);
            self.set_hidden_gene(parent1.get_one_of_genes_randomly());
        }

        if random().next_bounded_i32(32) == 0 {
            let gene = PandaGene::get_random(&mut *random());
            self.set_main_gene(gene);
        }
        if random().next_bounded_i32(32) == 0 {
            let gene = PandaGene::get_random(&mut *random());
            self.set_hidden_gene(gene);
        }
    }

    fn get_one_of_genes_randomly(&self) -> PandaGene {
        let main = self.get_entity().random().next_bool();
        if main {
            self.get_main_gene()
        } else {
            self.get_hidden_gene()
        }
    }

    /// Vanilla `Panda.setAttributes`: weak pandas have less health, lazy ones walk slower.
    pub fn set_attributes(&self) {
        let living = &self.mob_entity.living_entity;
        if self.is_weak() {
            living.set_max_health(10.0);
        }
        if self.is_lazy() {
            living.set_attribute_base(&Attributes::MOVEMENT_SPEED, f64::from(0.07f32));
            send_attribute_updates_for_living(living, vec![Attributes::MOVEMENT_SPEED]);
        }
    }

    /// Vanilla `Panda.tryToSit`.
    fn try_to_sit(&self) {
        if self.get_entity().is_in_water() {
            return;
        }
        let living = &self.mob_entity.living_entity;
        let input = living.movement_input.load();
        living
            .movement_input
            .store(Vector3::new(input.x, input.y, 0.0));
        self.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
        self.sit(true);
    }

    /// Vanilla `Animal.setInLove(player)`.
    fn set_in_love(&self, player: &Player) {
        self.mob_entity
            .set_love_ticks(600, Some(player.gameprofile.id));
        let entity = self.get_entity();
        entity.world.load().send_entity_status(
            entity,
            EntityStatus::InLoveHearts,
            Some(ActorEventID::InLoveHearts),
        );
    }
}

/// Vanilla `Panda.canPickUpAndEat`.
fn can_pick_up_and_eat(item: &ItemEntity) -> bool {
    item.get_item_stack()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .item
        .has_tag(&tag::Item::MINECRAFT_PANDA_EATS_FROM_GROUND)
        && !item.get_entity().removed.load(Ordering::Relaxed)
        && item.get_pickup_delay() == 0
}

fn as_panda(mob: &dyn Mob) -> Option<&PandaEntity> {
    mob.cast_any().downcast_ref::<PandaEntity>()
}

/// Item entities passing `canPickUpAndEat` in `area`.
fn edible_items_in(mob: &dyn Mob, inflate: f64) -> Vec<Arc<dyn EntityBase>> {
    let entity = mob.get_entity();
    let area = entity.bounding_box.load().expand(inflate, inflate, inflate);
    entity
        .world
        .load()
        .get_entities_at_box(&area)
        .into_iter()
        .filter(|candidate| candidate.get_item_entity().is_some_and(can_pick_up_and_eat))
        .collect()
}

/// Vanilla `PandaHurtByTargetGoal.alertOther`: only aggressive pandas join in.
fn is_aggressive_panda(mob: &dyn Mob) -> bool {
    as_panda(mob).is_some_and(PandaEntity::is_aggressive)
}

/// Vanilla `PandaHurtByTargetGoal.canContinueToUse`: a panda that got bamboo or bit back
/// drops the grudge.
fn panda_keeps_grudge(mob: &dyn Mob) -> bool {
    let Some(panda) = as_panda(mob) else {
        return true;
    };
    if !panda.got_bamboo.load(Ordering::Relaxed) && !panda.did_bite.load(Ordering::Relaxed) {
        return true;
    }
    mob.set_mob_target(None);
    false
}

/// Vanilla `PandaPanicGoal.canContinueToUse`: a panda that sat down stops running.
fn panda_panic_can_continue(mob: &dyn Mob) -> bool {
    if !as_panda(mob).is_some_and(PandaEntity::is_sitting) {
        return true;
    }
    mob.get_mob_entity()
        .navigator
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .stop();
    false
}

impl AgeableMob for PandaEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for PandaEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_PANDA_FOOD)
    }

    /// Vanilla `Panda.getBreedOffspring`: the baby inherits the parents' genes.
    fn get_breed_offspring(
        &self,
        world: &Arc<World>,
        partner: &dyn EntityBase,
    ) -> Option<Arc<dyn EntityBase>> {
        let entity = self.get_entity();
        let baby = from_type(
            &EntityType::PANDA,
            entity.pos.load(),
            world,
            uuid::Uuid::new_v4(),
        );
        if let Some(baby_panda) = baby.cast_any().downcast_ref::<Self>() {
            if let Some(partner_panda) = partner.cast_any().downcast_ref::<Self>() {
                baby_panda.set_gene_from_parents(self, Some(partner_panda));
            }
            baby_panda.set_attributes();
        }
        Some(baby)
    }
}

/// Vanilla `Panda.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::PANDA_BABY_DIMENSIONS;

/// Vanilla `Panda.finalizeSpawn`'s baby chance for the rest of a group.
const BABY_SPAWN_CHANCE: f32 = 0.2;

impl Mob for PandaEntity {
    /// Vanilla `Panda.getAmbientSound`.
    fn ambient_sound(&self) -> Option<Sound> {
        Some(if self.is_aggressive() {
            Sound::EntityPandaAggressiveAmbient
        } else if self.is_worried() {
            Sound::EntityPandaWorriedAmbient
        } else {
            Sound::EntityPandaAmbient
        })
    }

    /// Vanilla `Panda.getDefaultDimensions`.
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
        nbt.put_string(
            "MainGene",
            self.get_main_gene().serialized_name().to_string(),
        );
        nbt.put_string(
            "HiddenGene",
            self.get_hidden_gene().serialized_name().to_string(),
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.read_ageable_nbt(nbt);
        let gene = |key| {
            nbt.get_string(key)
                .and_then(PandaGene::from_serialized_name)
                .unwrap_or(PandaGene::Normal)
        };
        self.set_main_gene(gene("MainGene"));
        self.set_hidden_gene(gene("HiddenGene"));
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();
    }

    fn post_tick(&self) {
        self.panda_tick();
    }

    /// Vanilla `Panda.pickUpItem`.
    fn mob_pick_up_item(&self, item: &ItemEntity) {
        self.panda_pick_up_item(item);
    }

    /// Vanilla `Panda.hurtServer`: being hurt makes a panda stand up.
    fn before_hurt(
        &self,
        _amount: f32,
        _damage_type: pumpkin_data::damage::DamageType,
        _source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) {
        self.sit(false);
    }

    /// Vanilla `Panda.playAttackSound`.
    fn play_attack_sound(&self) {
        self.play_sound_at_self(Sound::EntityPandaBite, 1.0, 1.0);
    }

    /// Vanilla `Panda.finalizeSpawn`: random genes, and a fifth of a group's later pandas are
    /// babies.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        // `RandomSource random = level.getRandom()`.
        let (main, hidden) = {
            let mut random = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let main = PandaGene::get_random(&mut *random);
            (main, PandaGene::get_random(&mut *random))
        };
        self.set_main_gene(main);
        self.set_hidden_gene(hidden);
        self.set_attributes();
        let group_data = group_data.or(Some(SpawnGroupData::ageable(BABY_SPAWN_CHANCE)));
        let group_data = self.finalize_ageable_spawn(group_data);
        self.mob_entity.finalize_spawn_base();
        Some(group_data)
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::panda::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::panda::MAIN_GENE_ID,
            self.main_gene.load(Ordering::Relaxed) as i8,
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::panda::HIDDEN_GENE_ID,
            self.hidden_gene.load(Ordering::Relaxed) as i8,
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::panda::DATA_ID_FLAGS,
            self.flags.load(Ordering::Relaxed) as i8,
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::panda::UNHAPPY_COUNTER,
            VarInt(self.unhappy_counter.load(Ordering::Relaxed)),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::panda::SNEEZE_COUNTER,
            VarInt(self.sneeze_counter.load(Ordering::Relaxed)),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::panda::EAT_COUNTER,
            VarInt(self.eat_counter.load(Ordering::Relaxed)),
        );
    }

    /// Vanilla `Panda.mobInteract`. Pandas cannot be leashed, so the leash handling of the
    /// base interaction never applies.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if self.is_scared() {
            return false;
        }
        if self.is_on_back() {
            self.set_on_back(false);
            return true;
        }
        if !self.is_food(item_stack) {
            // TODO: `super.mobInteract` for a baby and a golden dandelion, once age locking
            // with it is ported.
            return false;
        }

        if self.mob_entity.get_target().is_some() {
            self.got_bamboo.store(true, Ordering::Relaxed);
        }
        let gamemode = player.gamemode.load();
        if self.can_age_up() {
            item_stack.decrement_unless_creative(gamemode, 1);
            self.age_up(((-self.get_age() / 20) as f32 * 0.1) as i32, true);
        } else if self.is_baby() {
            return false;
        } else if self.get_age() == 0
            && self.mob_entity.is_breeding_ready()
            && !self.mob_entity.is_in_love()
        {
            item_stack.decrement_unless_creative(gamemode, 1);
            self.set_in_love(player);
        } else {
            if self.is_sitting() || self.get_entity().is_in_water() {
                return false;
            }
            self.try_to_sit();
            self.eat(true);
            let current = self.main_hand();
            if !current.is_empty() && gamemode != GameMode::Creative {
                self.get_entity().spawn_at_location(current);
            }
            self.mob_entity.set_item_slot(
                &EquipmentSlot::MAIN_HAND,
                ItemStack::new(1, item_stack.item),
            );
            item_stack.decrement_unless_creative(gamemode, 1);
        }
        true
    }
}

/// Vanilla `Panda.PandaMoveControl`: a panda busy with something else stays put.
#[derive(Default)]
struct PandaMoveControl(MoveControl);

impl Control for PandaMoveControl {}

impl MoveControlTrait for PandaMoveControl {
    fn tick(&mut self, mob: &dyn Mob) {
        if as_panda(mob).is_some_and(PandaEntity::can_perform_action) {
            self.0.tick(mob);
        }
    }

    fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed_modifier: f64) {
        self.0.set_wanted_position(x, y, z, speed_modifier);
    }

    fn strafe(&mut self, forward: f32, right: f32) {
        self.0.strafe(forward, right);
    }

    fn has_wanted(&self) -> bool {
        self.0.has_wanted()
    }
}

/// Vanilla `Panda.PandaBreedGoal`: without bamboo nearby a panda in love sulks instead.
struct PandaBreedGoal {
    breed: Box<BreedGoal>,
    unhappy_cooldown: i32,
}

impl PandaBreedGoal {
    fn new(speed_modifier: f64) -> Self {
        Self {
            breed: BreedGoal::new(speed_modifier),
            unhappy_cooldown: 0,
        }
    }

    /// Vanilla `PandaBreedGoal.canFindBamboo`: bamboo within eight blocks, on or up to two
    /// blocks above the panda's level, searched in rings outwards.
    fn can_find_bamboo(mob: &dyn Mob) -> bool {
        let entity = mob.get_entity();
        let panda_pos = entity.block_pos.load();
        let world = entity.world.load();
        for y_off in 0..3 {
            for r in 0..8 {
                let mut x = 0;
                while x <= r {
                    let mut z = if x < r && x > -r { r } else { 0 };
                    while z <= r {
                        let pos = panda_pos.add(x, y_off, z);
                        if world.get_block(&pos) == &Block::BAMBOO {
                            return true;
                        }
                        z = if z > 0 { -z } else { 1 - z };
                    }
                    x = if x > 0 { -x } else { 1 - x };
                }
            }
        }
        false
    }
}

impl Goal for PandaBreedGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(panda) = as_panda(mob) else {
            return false;
        };
        if !self.breed.can_start(mob) || panda.get_unhappy_counter() != 0 {
            return false;
        }
        if Self::can_find_bamboo(mob) {
            return true;
        }
        let tick_count = panda.get_entity().tick_count.load(Ordering::Relaxed);
        if self.unhappy_cooldown <= tick_count {
            panda.set_unhappy_counter(TOTAL_UNHAPPY_TIME);
            self.unhappy_cooldown = tick_count + 600;
            // Vanilla `BREED_TARGETING`, which unlike the breed partner search needs sight.
            let entity = panda.get_entity();
            let player = TargetPredicate::create_non_attackable()
                .set_base_max_distance(8.0)
                .get_nearest_player(&entity.world.load(), Some(mob), entity.pos.load());
            *panda
                .look_at_player
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = player;
        }
        false
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.breed.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.breed.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.breed.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.breed.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.breed.controls()
    }
}

/// Vanilla `Panda.PandaAttackGoal`, which also stands for `Panda.doHurtTarget`'s bite.
struct PandaAttackGoal(MeleeAttackGoal);

impl Goal for PandaAttackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_panda(mob).is_some_and(PandaEntity::can_perform_action) && self.0.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.0.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.0.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.0.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.0.tick(mob);
        // The cooldown is only back at its maximum when this tick swung, which is when vanilla
        // runs `Panda.doHurtTarget`: a panda that is not aggressive has had its bite.
        if self.0.cooldown == self.0.get_max_cooldown()
            && let Some(panda) = as_panda(mob)
            && !panda.is_aggressive()
        {
            panda.did_bite.store(true, Ordering::Relaxed);
        }
    }

    fn should_run_every_tick(&self) -> bool {
        self.0.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        self.0.controls()
    }
}

/// Vanilla `Panda.PandaAvoidGoal`: only worried pandas flee.
struct PandaAvoidGoal(AvoidEntityGoal);

impl Goal for PandaAvoidGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_panda(mob).is_some_and(|panda| panda.is_worried() && panda.can_perform_action())
            && self.0.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.0.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.0.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.0.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.0.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.0.controls()
    }
}

/// Vanilla `Panda.PandaSitGoal`: an adult with food sits down to eat it, or walks over to
/// food on the ground first.
struct PandaSitGoal {
    cooldown: i32,
}

impl Goal for PandaSitGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(panda) = as_panda(mob) else {
            return false;
        };
        let entity = panda.get_entity();
        if self.cooldown > entity.tick_count.load(Ordering::Relaxed)
            || entity.is_baby()
            || entity.is_in_water()
            || !panda.can_perform_action()
            || panda.get_unhappy_counter() > 0
        {
            return false;
        }
        !panda.main_hand().is_empty() || !edible_items_in(mob, 6.0).is_empty()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let Some(panda) = as_panda(mob) else {
            return false;
        };
        // The panda's own random, one lock per draw.
        let roll = |bound: i32| mob.get_entity().random().next_bounded_i32(bound);
        if !panda.get_entity().is_in_water() && (panda.is_lazy() || roll(to_goal_ticks(600)) != 1) {
            roll(to_goal_ticks(2000)) != 1
        } else {
            false
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        if let Some(panda) = as_panda(mob)
            && !panda.is_sitting()
            && !panda.main_hand().is_empty()
        {
            panda.try_to_sit();
        }
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(panda) = as_panda(mob) else {
            return;
        };
        if panda.main_hand().is_empty() {
            if let Some(item) = edible_items_in(mob, 8.0).first() {
                let mob_entity = &panda.mob_entity;
                let living = &mob_entity.living_entity;
                let mut navigator = mob_entity
                    .navigator
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                // Vanilla `moveTo(Entity, speed)` paths with a reach of 1.
                let path = navigator.create_path(living, item.get_entity().pos.load(), 1);
                if path.is_some() {
                    navigator.move_to_path(path, f64::from(1.2f32), living);
                }
            }
        } else {
            panda.try_to_sit();
        }
        self.cooldown = 0;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        let Some(panda) = as_panda(mob) else {
            return;
        };
        let held = panda.main_hand();
        if !held.is_empty() {
            panda.get_entity().spawn_at_location(held);
            panda
                .mob_entity
                .set_item_slot(&EquipmentSlot::MAIN_HAND, ItemStack::EMPTY.clone());
            // The panda's own random, one lock per draw.
            let roll = |bound: i32| mob.get_entity().random().next_bounded_i32(bound);
            let wait_seconds = if panda.is_lazy() {
                roll(50) + 10
            } else {
                roll(150) + 10
            };
            self.cooldown =
                panda.get_entity().tick_count.load(Ordering::Relaxed) + wait_seconds * 20;
        }
        panda.sit(false);
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

/// Vanilla `Panda.PandaLieOnBackGoal`: lazy pandas now and then roll over for a nap.
struct PandaLieOnBackGoal {
    cooldown: i32,
}

impl Goal for PandaLieOnBackGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_panda(mob).is_some_and(|panda| {
            self.cooldown < panda.get_entity().tick_count.load(Ordering::Relaxed)
                && panda.is_lazy()
                && panda.can_perform_action()
                && mob
                    .get_entity()
                    .random()
                    .next_bounded_i32(to_goal_ticks(400))
                    == 1
        })
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let Some(panda) = as_panda(mob) else {
            return false;
        };
        // The panda's own random, one lock per draw.
        let roll = |bound: i32| mob.get_entity().random().next_bounded_i32(bound);
        if !panda.get_entity().is_in_water() && (panda.is_lazy() || roll(to_goal_ticks(600)) != 1) {
            roll(to_goal_ticks(2000)) != 1
        } else {
            false
        }
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(panda) = as_panda(mob) {
            panda.set_on_back(true);
        }
        self.cooldown = 0;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(panda) = as_panda(mob) {
            panda.set_on_back(false);
            self.cooldown = panda.get_entity().tick_count.load(Ordering::Relaxed) + 200;
        }
    }
}

/// Vanilla `Panda.PandaSneezeGoal`: babies, weak ones far more often, start a sneeze.
struct PandaSneezeGoal;

impl Goal for PandaSneezeGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(panda) = as_panda(mob) else {
            return false;
        };
        if !panda.get_entity().is_baby() || !panda.can_perform_action() {
            return false;
        }
        // The panda's own random, one lock per draw.
        let roll = |bound: i32| mob.get_entity().random().next_bounded_i32(bound);
        if panda.is_weak() && roll(to_goal_ticks(500)) == 1 {
            true
        } else {
            roll(to_goal_ticks(6000)) == 1
        }
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(panda) = as_panda(mob) {
            panda.sneeze(true);
        }
    }
}

/// Vanilla `Panda.PandaRollGoal`: babies and playful pandas roll forwards, always over an
/// edge.
struct PandaRollGoal;

impl Goal for PandaRollGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(panda) = as_panda(mob) else {
            return false;
        };
        let entity = panda.get_entity();
        if !(entity.is_baby() || panda.is_playful()) || !entity.on_ground.load(Ordering::Relaxed) {
            return false;
        }
        if !panda.can_perform_action() {
            return false;
        }
        let angle = entity.yaw.load() * (std::f64::consts::PI / 180.0) as f32;
        let x_dir = -math::sin(angle);
        let z_dir = math::cos(angle);
        // `Mth.sign` of a float.
        let step = |dir: f32| {
            if f64::from(dir.abs()) > 0.5 {
                dir.signum() as i32
            } else {
                0
            }
        };
        let below = entity.block_pos.load().add(step(x_dir), -1, step(z_dir));
        if entity.world.load().get_block_state(&below).is_air() {
            return true;
        }
        // The panda's own random, one lock per draw.
        let roll = |bound: i32| mob.get_entity().random().next_bounded_i32(bound);
        if panda.is_playful() && roll(to_goal_ticks(60)) == 1 {
            true
        } else {
            roll(to_goal_ticks(500)) == 1
        }
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(panda) = as_panda(mob) {
            panda.roll(true);
        }
    }

    fn can_stop(&self) -> bool {
        false
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK | Controls::JUMP
    }
}

/// Vanilla `Panda.PandaLookAtPlayerGoal`: keeps a player the breed goal picked, and only
/// looks while the panda is free.
#[expect(
    clippy::struct_field_names,
    reason = "vanilla LookAtPlayerGoal field names"
)]
struct PandaLookAtPlayerGoal {
    look_distance: f32,
    look_time: i32,
    look_at_context: TargetPredicate,
}

impl PandaLookAtPlayerGoal {
    /// Vanilla `LookAtPlayerGoal.DEFAULT_PROBABILITY`.
    const PROBABILITY: f32 = 0.02;

    fn new(mob_weak: Weak<dyn Mob>, look_distance: f32) -> Self {
        Self {
            look_distance,
            look_time: 0,
            look_at_context: LookAtEntityGoal::create_target_predicate(
                mob_weak,
                &EntityType::PLAYER,
                look_distance,
            ),
        }
    }

    fn look_at(panda: &PandaEntity) -> Option<Arc<dyn EntityBase>> {
        panda
            .look_at_player
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Goal for PandaLookAtPlayerGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(panda) = as_panda(mob) else {
            return false;
        };
        if mob.get_entity().random().next_f32() >= Self::PROBABILITY {
            return false;
        }
        let mut look_at = panda
            .look_at_player
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if look_at.is_none() {
            let entity = panda.get_entity();
            let pos = entity.pos.load();
            *look_at = self.look_at_context.get_nearest_player(
                &entity.world.load(),
                Some(mob),
                Vector3::new(pos.x, entity.get_eye_y(), pos.z),
            );
        }
        panda.can_perform_action() && look_at.is_some()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let Some(look_at) = as_panda(mob).and_then(Self::look_at) else {
            return false;
        };
        let look_at = look_at.get_entity();
        if !look_at.is_alive() {
            return false;
        }
        let dist_sq = mob
            .get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&look_at.pos.load());
        if dist_sq > f64::from(self.look_distance * self.look_distance) {
            return false;
        }
        self.look_time > 0
    }

    fn start(&mut self, mob: &dyn Mob) {
        let extra = mob.get_entity().random().next_bounded_i32(40);
        self.look_time = self.get_tick_count(40 + extra);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        if let Some(panda) = as_panda(mob) {
            *panda
                .look_at_player
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = None;
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(look_at) = as_panda(mob).and_then(Self::look_at) else {
            return;
        };
        let look_at = look_at.get_entity();
        if look_at.is_alive() {
            let pos = look_at.pos.load();
            mob.get_mob_entity()
                .look_control
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .look_at(mob, pos.x, look_at.get_eye_y(), pos.z);
            self.look_time -= 1;
        }
    }

    fn controls(&self) -> Controls {
        Controls::LOOK
    }
}
