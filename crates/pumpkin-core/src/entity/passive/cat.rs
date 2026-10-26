use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::any::Any;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

use pumpkin_data::block_properties::{BlockProperties, FurnaceLikeProperties};
use pumpkin_data::cat_sound_variant::CatSoundVariant;
use pumpkin_data::cat_variant::CatVariant;
use pumpkin_data::data_component_impl::DyeImpl;
use pumpkin_data::dye_color::DyeColor;
use pumpkin_data::entity::{EntityPose, EntityStatus, EntityType};
use pumpkin_data::environment_attribute::EnvironmentAttribute;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{Block, block_properties::BedPart};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use uuid::Uuid;

use crate::block::entities::chest::ChestBlockEntity;
use crate::entity::ageable::{AgeableData, AgeableMob};
use crate::entity::ai::control::move_control::MoveControl;
use crate::entity::ai::goal::move_to_target_pos::{MoveToTargetPos, MoveToTargetPosGoal};
use crate::entity::ai::goal::{Controls, Goal, ParentHandle};
use crate::entity::ai::pathfinder::NavigatorGoal;
use crate::entity::ai::util::goal_utils;
use crate::entity::custom_sound::CustomSound;
use crate::entity::item::ItemEntity;
use crate::entity::living::LivingEntity;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::passive::animal::{animal_can_mate, get_mixed_color};
use crate::entity::r#type::from_type;
use crate::entity::{
    as_entity_base, Entity, EntityBase,
    ai::goal::{
        active_target::{ActiveTargetGoal, NonTameRandomTargetGoal},
        avoid_entity::AvoidEntityGoal,
        breed::BreedGoal,
        escape_danger::EscapeDangerGoal,
        follow_owner::FollowOwnerGoal,
        leap_at_target::LeapAtTargetGoal,
        look_at_entity::LookAtEntityGoal,
        ocelot_attack::OcelotAttackGoal,
        sit_when_ordered_to::SitWhenOrderedToGoal,
        swim::SwimGoal,
        tempt::TemptGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    passive::{
        animal::{Animal, set_animal_pathfinding_malus},
        tamable::{TamableAnimal, TamableData, tamable_animal_panic_tick},
    },
    player::Player,
};
use crate::world::World;

/// Vanilla `Cat.TEMPT_SPEED_MOD` and `Ocelot.CROUCH_SPEED_MOD`.
pub const CROUCH_SPEED_MOD: f64 = 0.6;
/// Vanilla `Cat.WALK_SPEED_MOD` and `Ocelot.WALK_SPEED_MOD`.
pub const WALK_SPEED_MOD: f64 = 0.8;
/// Vanilla `Cat.SPRINT_SPEED_MOD` and `Ocelot.SPRINT_SPEED_MOD`.
pub const SPRINT_SPEED_MOD: f64 = 1.33;
const DEFAULT_VARIANT: CatVariant = CatVariant::Black;
const DEFAULT_COLLAR_COLOR: DyeColor = DyeColor::Red;
const MORNING_GIFT_LOOT_TABLE: &str = "minecraft:gameplay/cat_morning_gift";

/// Vanilla `Cat.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::CAT_BABY_DIMENSIONS;

pub struct CatEntity {
    pub mob_entity: MobEntity,
    pub variant: AtomicU8,
    pub sound_variant: AtomicU8,
    pub collar_color: AtomicU8,
    pub tamable_data: TamableData,
    pub ageable_data: AgeableData,
    pub is_lying: AtomicBool,
    pub relax_state_one: AtomicBool,
}

impl CatEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        let cat = Self {
            mob_entity,
            variant: AtomicU8::new(DEFAULT_VARIANT.id()),
            sound_variant: AtomicU8::new(CatSoundVariant::Classic.id()),
            collar_color: AtomicU8::new(DEFAULT_COLLAR_COLOR.id()),
            tamable_data: TamableData::default(),
            ageable_data: AgeableData::default(),
            is_lying: AtomicBool::new(false),
            relax_state_one: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(cat);
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
            // TamableAnimalPanicGoal
            goal_selector.add_goal(
                1,
                EscapeDangerGoal::new(1.5).ticking_with(tamable_animal_panic_tick),
            );
            goal_selector.add_goal(2, Box::new(SitWhenOrderedToGoal::new()));
            goal_selector.add_goal(3, Box::new(CatRelaxOnOwnerGoal::default()));
            goal_selector.add_goal(4, Box::new(CatTemptGoal::new()));
            goal_selector.add_goal(5, CatLieOnBlockGoal::new(1.1, 8));
            goal_selector.add_goal(6, FollowOwnerGoal::new(1.0, 10.0, 5.0));
            goal_selector.add_goal(7, CatSitOnBlockGoal::new(0.8));
            goal_selector.add_goal(8, Box::new(LeapAtTargetGoal::new(0.3)));
            goal_selector.add_goal(9, Box::new(OcelotAttackGoal::new()));
            goal_selector.add_goal(10, BreedGoal::new(0.8));
            goal_selector.add_goal(
                11,
                Box::new(WanderAroundGoal::avoiding_water_with_probability(
                    0.8,
                    1.000_000_1E-5,
                )),
            );
            goal_selector.add_goal(
                12,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 10.0),
            );
            // `reassessTameGoals` in the constructor: `CatAvoidEntityGoal`, added last and
            // used only while untamed.
            goal_selector.add_goal(
                4,
                Box::new(
                    AvoidEntityGoal::new(
                        &EntityType::PLAYER,
                        16.0,
                        WALK_SPEED_MOD,
                        SPRINT_SPEED_MOD,
                    )
                    .gated_by(|mob| !mob.is_tamed()),
                ),
            );
        };

        {
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            // NonTameRandomTargetGoal: rabbits, and baby turtles on land.
            target_selector.add_goal(
                1,
                NonTameRandomTargetGoal::new(ActiveTargetGoal::with_default(
                    &mob_arc.mob_entity,
                    &EntityType::RABBIT,
                    false,
                )),
            );
            target_selector.add_goal(
                1,
                NonTameRandomTargetGoal::new(Box::new(ActiveTargetGoal::new(
                    &mob_arc.mob_entity,
                    &EntityType::TURTLE,
                    10,
                    false,
                    false,
                    Some(baby_on_land),
                ))),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn get_variant(&self) -> CatVariant {
        CatVariant::all()
            .get(usize::from(self.variant.load(Ordering::Relaxed)))
            .copied()
            .unwrap_or(DEFAULT_VARIANT)
    }

    pub fn set_variant(&self, variant: CatVariant) {
        self.variant.store(variant.id(), Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::cat::CAT_VARIANT,
            VarInt(i32::from(variant.id())),
        );
    }

    fn get_sound_variant(&self) -> CatSoundVariant {
        CatSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed)).unwrap_or_default()
    }

    pub fn set_sound_variant(&self, sound_variant: CatSoundVariant) {
        self.sound_variant
            .store(sound_variant.id(), Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::cat::SOUND_VARIANT,
            VarInt(i32::from(sound_variant.id())),
        );
    }

    pub fn is_lying(&self) -> bool {
        self.is_lying.load(Ordering::Relaxed)
    }

    pub fn set_lying(&self, lying: bool) {
        self.is_lying.store(lying, Ordering::Relaxed);
        self.get_entity()
            .set_synced_data(pumpkin_data::tracked_data::cat::IS_LYING, lying);
    }

    pub fn is_relax_state_one(&self) -> bool {
        self.relax_state_one.load(Ordering::Relaxed)
    }

    fn set_relax_state_one(&self, relax: bool) {
        self.relax_state_one.store(relax, Ordering::Relaxed);
        self.get_entity()
            .set_synced_data(pumpkin_data::tracked_data::cat::RELAX_STATE_ONE, relax);
    }

    pub fn get_collar_color(&self) -> u8 {
        self.collar_color.load(Ordering::Relaxed)
    }

    pub fn set_collar_color(&self, color: u8) {
        self.collar_color.store(color, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::cat::CAT_COLLAR_COLOR,
            VarInt(i32::from(color)),
        );
    }

    fn collar_dye_color(&self) -> DyeColor {
        DyeColor::by_id(self.get_collar_color()).unwrap_or(DEFAULT_COLLAR_COLOR)
    }

    /// Vanilla `Cat.hiss`, at a phantom that comes too close.
    pub fn hiss(&self) {
        self.make_sound(
            self.get_sound_variant()
                .hiss_sound(self.get_entity().is_baby()),
        );
    }

    /// Vanilla `Cat.playEatingSound`.
    fn play_eating_sound(&self) {
        let sound = self
            .get_sound_variant()
            .eat_sound(self.get_entity().is_baby());
        self.play_sound_at_self(sound, 1.0, 1.0);
    }

    /// Vanilla `Cat.handleLieDown`. The lie-down amounts and `isLyingOnTopOfSleepingPlayer`
    /// only feed the client's renderer.
    fn handle_lie_down(&self) {
        let tick_count = self.get_entity().tick_count.load(Ordering::Relaxed);
        if (self.is_lying() || self.is_relax_state_one()) && tick_count % 5 == 0 {
            let volume = {
                let mut random = self.get_entity().random();
                0.6 + 0.4 * (random.next_f32() - random.next_f32())
            };
            let sound = self
                .get_sound_variant()
                .purr_sound(self.get_entity().is_baby());
            self.play_sound_at_self(sound, volume, 1.0);
        }
    }

    /// Vanilla `Cat.tryToTame`.
    fn try_to_tame(&self, player: &Player) {
        let world = self.get_entity().world.load();
        if self.get_entity().random().next_bounded_i32(3) == 0 {
            TamableAnimal::tame(self, player.gameprofile.id);
            // The `complete_catalogue` criteria of vanilla's `TAME_ANIMAL` trigger.
            player.trigger_advancement_criterion(
                pumpkin_data::advancement::Advancement::HUSBANDRY_COMPLETE_CATALOGUE,
                self.get_variant().asset_id(),
            );
            self.set_ordered_to_sit(true);
            world.send_entity_status(
                self.get_entity(),
                EntityStatus::TamingSucceeded,
                Some(ActorEventID::TamingSucceeded),
            );
        } else {
            world.send_entity_status(
                self.get_entity(),
                EntityStatus::TamingFailed,
                Some(ActorEventID::TamingFailed),
            );
        }
    }

    fn set_persistence_required(&self) {
        self.mob_entity
            .persistence_required
            .store(true, Ordering::Relaxed);
    }
}

fn as_cat(mob: &dyn Mob) -> Option<&CatEntity> {
    mob.cast_any().downcast_ref::<CatEntity>()
}

/// Vanilla `VariantUtils.selectVariantToSpawn` over the cat variants: `all_black` alone
/// inside `#cats_spawn_as_black` structures, otherwise any variant, `all_black` only under a
/// full moon.
fn select_variant_to_spawn(world: &World, pos: &BlockPos) -> Option<CatVariant> {
    // TODO: `StructureCheck(#cats_spawn_as_black)` needs vanilla's
    // `StructureManager.getStructureWithPieceAt`; swamp huts place their cat as `all_black`.
    let full_moon = world.get_moon_brightness(pos) >= 0.9;
    let candidates: Vec<CatVariant> = CatVariant::all()
        .iter()
        .copied()
        .filter(|variant| *variant != CatVariant::AllBlack || full_moon)
        .collect();
    // `Util.getRandomSafe(selected, level.getRandom())`.
    if candidates.is_empty() {
        return None;
    }
    let index = world
        .random
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .next_bounded_i32(candidates.len() as i32);
    candidates.get(index as usize).copied()
}

/// Vanilla `Cat.customServerAiStep`, which `Ocelot` repeats: crouches while tempted, sprints
/// while fleeing or pouncing.
pub fn feline_custom_server_ai_step(mob: &dyn Mob) {
    // `this.getMoveControl()` (`Cat.java:236-237`, `Ocelot.java:119-120`).
    let speed = mob.get_mob_entity().with_move_control(|control| {
        let control: &dyn Any = &*control;
        control
            .downcast_ref::<MoveControl>()
            .filter(|control| control.has_wanted())
            .map(MoveControl::get_speed_modifier)
    });
    let (pose, sprinting) = match speed {
        Some(speed) if speed == CROUCH_SPEED_MOD => (EntityPose::Crouching, false),
        Some(speed) if speed == SPRINT_SPEED_MOD => (EntityPose::Standing, true),
        _ => (EntityPose::Standing, false),
    };
    mob.set_pose(pose);
    mob.get_mob_entity().living_entity.set_sprinting(sprinting);
}

impl CustomSound for CatEntity {
    fn death_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        Some(
            self.get_sound_variant()
                .death_sound(self.get_entity().is_baby()),
        )
    }

    fn hurt_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        Some(
            self.get_sound_variant()
                .hurt_sound(self.get_entity().is_baby()),
        )
    }
}

impl AgeableMob for CatEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for CatEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack
            .get_item()
            .has_tag(&tag::Item::MINECRAFT_CAT_FOOD)
    }

    /// Vanilla `Cat.canMate`.
    fn can_mate(&self, partner: &dyn Mob) -> bool {
        self.is_tame()
            && partner.get_entity().entity_type == &EntityType::CAT
            && partner.as_tamable().is_some_and(TamableAnimal::is_tame)
            && animal_can_mate(&self.mob_entity, partner)
    }

    /// Vanilla `Cat.getBreedOffspring`.
    fn get_breed_offspring(
        &self,
        world: &Arc<World>,
        partner: &dyn EntityBase,
    ) -> Option<Arc<dyn EntityBase>> {
        let baby = from_type(
            &EntityType::CAT,
            self.get_entity().pos.load(),
            world,
            Uuid::new_v4(),
        );
        if let Some(baby_cat) = baby.cast_any().downcast_ref::<Self>()
            && let Some(partner_cat) = partner.cast_any().downcast_ref::<Self>()
        {
            let use_own = self.get_entity().random().next_bool();
            baby_cat.set_variant(if use_own {
                self.get_variant()
            } else {
                partner_cat.get_variant()
            });
            if self.is_tame() {
                baby_cat.set_owner(self.get_owner());
                baby_cat.set_tame(true);
                baby_cat.set_collar_color(
                    get_mixed_color(
                        world,
                        self.collar_dye_color(),
                        partner_cat.collar_dye_color(),
                    )
                    .id(),
                );
            }
        }
        Some(baby)
    }
}

impl TamableAnimal for CatEntity {
    fn get_tamable_data(&self) -> &TamableData {
        &self.tamable_data
    }
}

impl Mob for CatEntity {
    /// Vanilla `Cat.getAmbientSound`.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        let sounds = self.get_sound_variant();
        let baby = self.get_entity().is_baby();
        Some(if !self.is_tame() {
            sounds.stray_ambient_sound(baby)
        } else if self.get_mob_entity().is_in_love() {
            sounds.purr_sound(baby)
        } else if self.get_entity().random().next_bounded_i32(4) == 0 {
            sounds.purreow_sound(baby)
        } else {
            sounds.ambient_sound(baby)
        })
    }

    /// Vanilla `Cat.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    /// Vanilla `Cat.removeWhenFarAway`.
    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        !self.is_tame() && self.get_entity().tick_count.load(Ordering::Relaxed) > 2400
    }

    /// Vanilla `Cat.tick` after `super.tick()`.
    fn post_tick(&self) {
        let tick_count = self.get_entity().tick_count.load(Ordering::Relaxed);
        let begging = self
            .mob_entity
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_running::<CatTemptGoal>();
        if begging && !self.is_tame() && tick_count % 100 == 0 {
            let sound = self
                .get_sound_variant()
                .beg_for_food_sound(self.get_entity().is_baby());
            self.play_sound_at_self(sound, 1.0, 1.0);
        }
        self.handle_lie_down();
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

    /// Vanilla `Cat.isSteppingCarefully`.
    fn mob_is_stepping_carefully(&self) -> bool {
        let entity = self.get_entity();
        entity.pose.load() == EntityPose::Crouching || entity.is_sneaking()
    }

    /// Vanilla `Cat.finalizeSpawn`.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        let group_data = self.finalize_ageable_spawn(group_data);
        self.mob_entity.finalize_spawn_base();
        if let Some(variant) = select_variant_to_spawn(world, &self.get_entity().block_pos.load()) {
            self.set_variant(variant);
        }
        // Vanilla `CatSoundVariants.pickRandomSoundVariant(.., level.getRandom())`.
        let sound_variants = CatSoundVariant::all();
        let index = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_bounded_i32(sound_variants.len() as i32);
        self.set_sound_variant(sound_variants[index as usize]);
        Some(group_data)
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn as_tamable(&self) -> Option<&dyn TamableAnimal> {
        Some(self)
    }

    /// Vanilla `Cat.addAdditionalSaveData`.
    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_string("variant", self.get_variant().asset_id().to_string());
        nbt.put_string(
            "sound_variant",
            format!("minecraft:{}", self.get_sound_variant().to_name()),
        );
        nbt.put_byte("CollarColor", self.get_collar_color() as i8);
    }

    /// Vanilla `Cat.readAdditionalSaveData`.
    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if let Some(variant) = nbt.get_string("variant").and_then(CatVariant::from_name) {
            self.set_variant(variant);
        }
        if let Some(sound_variant) = nbt
            .get_string("sound_variant")
            .and_then(CatSoundVariant::from_name)
        {
            self.set_sound_variant(sound_variant);
        }
        // `DyeColor.LEGACY_ID_CODEC`: an unknown id reads as white, a missing one as red.
        let collar = nbt
            .get_byte("CollarColor")
            .or_else(|| nbt.get_int("CollarColor").map(|id| id as i8))
            .map_or(DEFAULT_COLLAR_COLOR, |id| {
                DyeColor::by_id(id as u8).unwrap_or(DyeColor::White)
            });
        self.set_collar_color(collar.id());
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_set_variant_name(&self, name: &str) {
        if let Some(variant) = CatVariant::from_name(name) {
            self.set_variant(variant);
        }
    }

    fn mob_set_sound_variant_name(&self, name: &str) {
        if let Some(v) = CatSoundVariant::from_name(name) {
            self.set_sound_variant(v);
        }
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        if entity.is_baby() {
            entity.set_synced_data(pumpkin_data::tracked_data::cat::BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::cat::CAT_VARIANT,
            VarInt(i32::from(self.variant.load(Ordering::Relaxed))),
        );
        entity.set_synced_data(pumpkin_data::tracked_data::cat::IS_LYING, self.is_lying());
        entity.set_synced_data(
            pumpkin_data::tracked_data::cat::RELAX_STATE_ONE,
            self.is_relax_state_one(),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::cat::CAT_COLLAR_COLOR,
            VarInt(i32::from(self.get_collar_color())),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::cat::SOUND_VARIANT,
            VarInt(i32::from(self.sound_variant.load(Ordering::Relaxed))),
        );
    }

    /// Vanilla `Cat.mobInteract`.
    fn mob_interact(
        &self,
        player: &Arc<Player>,
        _hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        let eat_sound = self
            .get_sound_variant()
            .eat_sound(self.get_entity().is_baby());
        if self.is_tame() {
            if self.is_owned_by(&player.gameprofile.id) {
                if item_stack
                    .get_item()
                    .has_tag(&tag::Item::MINECRAFT_CAT_COLLAR_DYES)
                {
                    let color = item_stack
                        .get_data_component::<DyeImpl>()
                        .map(|dye| dye.color);
                    if let Some(color) = color
                        && color != self.collar_dye_color()
                    {
                        self.set_collar_color(color.id());
                        item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                        self.set_persistence_required();
                        return InteractionResult::SUCCESS;
                    }
                } else if self.is_food(item_stack)
                    && self.mob_entity.living_entity.health.load()
                        < self.mob_entity.living_entity.get_max_health()
                {
                    self.feed(player, item_stack, 1.0, 1.0);
                    self.play_eating_sound();
                    return InteractionResult::SUCCESS;
                }

                let parent_interaction = self.animal_interact(player, item_stack, eat_sound);
                if !parent_interaction.consumes_action() {
                    self.set_ordered_to_sit(!self.is_ordered_to_sit());
                    return InteractionResult::SUCCESS;
                }
                return parent_interaction;
            }
        } else if self.is_food(item_stack) {
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            self.try_to_tame(player);
            self.set_persistence_required();
            self.play_eating_sound();
            return InteractionResult::SUCCESS;
        }

        let interact = self.animal_interact(player, item_stack, eat_sound);
        if interact.consumes_action() {
            self.set_persistence_required();
        }
        interact
    }
}

/// Vanilla `Turtle.BABY_ON_LAND_SELECTOR`.
pub fn baby_on_land(target: &LivingEntity, _world: &World) -> bool {
    target.entity.is_baby() && !target.entity.is_in_water()
}

/// Vanilla `Cat.CatTemptGoal`: an untamed cat follows fish, and stops being scared of a
/// player it has taken to.
struct CatTemptGoal {
    goal: TemptGoal,
    selected_player: Option<Arc<dyn EntityBase>>,
}

impl CatTemptGoal {
    fn new() -> Self {
        Self {
            goal: TemptGoal::with_tag(CROUCH_SPEED_MOD, &tag::Item::MINECRAFT_CAT_FOOD, true),
            selected_player: None,
        }
    }

    /// Vanilla `CatTemptGoal.canScare`.
    fn can_scare(&self) -> bool {
        let is_selected = self
            .selected_player
            .as_ref()
            .zip(self.goal.player())
            .is_some_and(|(selected, player)| {
                selected.get_entity().entity_id == player.get_entity().entity_id
            });
        !is_selected && self.goal.can_scare()
    }
}

impl Goal for CatTemptGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.goal.can_start(mob) && !mob.is_tamed()
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let can_scare = self.can_scare();
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
        let entity = mob.get_entity();
        if self.selected_player.is_none()
            && entity.random().next_bounded_i32(self.get_tick_count(600)) == 0
        {
            self.selected_player = self.goal.player().cloned();
        } else if entity.random().next_bounded_i32(self.get_tick_count(500)) == 0 {
            self.selected_player = None;
        }
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}

/// Vanilla `Cat.CatRelaxOnOwnerGoal`: a tame cat curls up at the foot of its sleeping
/// owner's bed and may leave a gift when they wake.
#[derive(Default)]
struct CatRelaxOnOwnerGoal {
    owner_player: Option<Arc<Player>>,
    goal_pos: Option<BlockPos>,
    on_bed_ticks: i32,
}

impl CatRelaxOnOwnerGoal {
    /// Vanilla `spaceIsOccupied`.
    fn space_is_occupied(&self, cat: &CatEntity) -> bool {
        let Some(goal_pos) = self.goal_pos else {
            return false;
        };
        let world = cat.get_entity().world.load();
        let area = BoundingBox::from_block(&goal_pos).expand(2.0, 2.0, 2.0);
        goal_utils::get_entities_of_class(&world, Some(&EntityType::CAT), &area)
            .iter()
            .filter(|other| other.get_entity().entity_id != cat.get_entity().entity_id)
            .filter_map(|other| other.cast_any().downcast_ref::<CatEntity>())
            .any(|other| other.is_lying() || other.is_relax_state_one())
    }

    /// `this.cat.getNavigation().moveTo(goalPos, 1.1F)` (`Cat.java:587`, `:638`).
    fn move_to_goal_pos(cat: &CatEntity, goal_pos: BlockPos) {
        cat.mob_entity.with_navigation(|navigation, owner| {
            navigation.set_progress(NavigatorGoal::new(
                owner.living_entity.entity.pos.load(),
                goal_pos.0.to_f64(),
                f64::from(1.1f32),
            ));
        });
    }

    /// Vanilla `giveMorningGift`.
    fn give_morning_gift(cat: &CatEntity) {
        let entity = cat.get_entity();
        let leash_holder = entity
            .leashed_to
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let center = leash_holder.map_or_else(
            || entity.block_pos.load(),
            |holder| holder.get_entity().block_pos.load(),
        );
        let (x, y, z) = {
            let mut random = entity.random();
            let x = center.0.x + random.next_bounded_i32(11) - 5;
            let y = center.0.y + random.next_bounded_i32(5) - 2;
            (x, y, center.0.z + random.next_bounded_i32(11) - 5)
        };
        cat.mob_entity.living_entity.random_teleport(
            cat,
            f64::from(x),
            f64::from(y),
            f64::from(z),
            false,
            &tag::Block::MINECRAFT_CAT_DOES_NOT_TELEPORT_TO,
        );

        let world = entity.world.load_full();
        let Some(loot_table) = world.get_loot_table(MORNING_GIFT_LOOT_TABLE) else {
            return;
        };
        let cat_pos = entity.block_pos.load();
        // `dropFromGiftLootTable`: the gift context has the origin and this entity.
        let params = crate::world::loot::LootContextParameters {
            this_entity: Some(as_entity_base(&entity.entity_type)),
            position: Some(entity.pos.load()),
            ..Default::default()
        };
        let body_yaw = entity.body_yaw.load() * (std::f32::consts::PI / 180.0);
        let item_pos = Vector3::new(
            f64::from(cat_pos.0.x) - f64::from(pumpkin_util::math::sin(body_yaw)),
            f64::from(cat_pos.0.y),
            f64::from(cat_pos.0.z) + f64::from(pumpkin_util::math::cos(body_yaw)),
        );
        for stack in crate::world::loot::generate_loot_from_handle(&loot_table, &world, 0, &params)
        {
            // `new ItemEntity(level, x, y, z, stack)` keeps the default pickup delay of 0.
            let item = ItemEntity::new(
                Entity::new(world.clone(), item_pos, &EntityType::ITEM),
                stack,
            );
            item.set_pickup_delay(0);
            world.spawn_entity(Arc::new(item));
        }
    }
}

impl Goal for CatRelaxOnOwnerGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(cat) = as_cat(mob) else {
            return false;
        };
        if !cat.is_tame() || cat.is_ordered_to_sit() {
            return false;
        }
        let world = cat.get_entity().world.load();
        let Some(owner) = cat
            .get_owner()
            .and_then(|uuid| world.get_player_by_uuid(uuid))
        else {
            return false;
        };
        self.owner_player = Some(owner.clone());
        if !owner.is_sleeping()
            || cat
                .get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&owner.get_entity().pos.load())
                > 100.0
        {
            return false;
        }
        let owner_pos = owner.get_entity().block_pos.load();
        let (block, state) = world.get_block_and_state_id(&owner_pos);
        if !block.has_tag(&tag::Block::MINECRAFT_CATS_CAN_LIE_ON) {
            return false;
        }
        // Vanilla `getOptionalValue(BedBlock.FACING)`: the foot of a bed.
        self.goal_pos = Some(if BedProperties::handles_block_id(block.id) {
            owner_pos.offset(
                BedProperties::from_state_id(state)
                    .facing
                    .opposite()
                    .to_offset(),
            )
        } else {
            owner_pos
        });
        !self.space_is_occupied(cat)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        let Some(cat) = as_cat(mob) else {
            return false;
        };
        cat.is_tame()
            && !cat.is_ordered_to_sit()
            && self
                .owner_player
                .as_ref()
                .is_some_and(|owner| owner.is_sleeping())
            && self.goal_pos.is_some()
            && !self.space_is_occupied(cat)
    }

    fn start(&mut self, mob: &dyn Mob) {
        let Some(cat) = as_cat(mob) else {
            return;
        };
        if let Some(goal_pos) = self.goal_pos {
            cat.set_in_sitting_pose(false);
            Self::move_to_goal_pos(cat, goal_pos);
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        let Some(cat) = as_cat(mob) else {
            return;
        };
        cat.set_lying(false);
        if let Some(owner) = &self.owner_player
            && owner.sleep_counter.load(Ordering::Relaxed) >= 100
        {
            let world = cat.get_entity().world.load();
            let chance = world.environment_attributes().get_value_f32(
                EnvironmentAttribute::GameplayCatWakingUpGiftChance,
                &cat.get_entity().block_pos.load(),
            );
            // `this.cat.level().getRandom().nextFloat()`.
            let roll = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .next_f32();
            if roll < chance {
                Self::give_morning_gift(cat);
            }
        }
        self.on_bed_ticks = 0;
        cat.set_relax_state_one(false);
        // `this.cat.getNavigation().stop()` (`Cat.java:602`).
        cat.mob_entity
            .with_navigation(|navigation, _| navigation.stop());
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(cat) = as_cat(mob) else {
            return;
        };
        let (Some(owner), Some(goal_pos)) = (&self.owner_player, self.goal_pos) else {
            return;
        };
        cat.set_in_sitting_pose(false);
        Self::move_to_goal_pos(cat, goal_pos);
        if cat
            .get_entity()
            .pos
            .load()
            .squared_distance_to_vec(&owner.get_entity().pos.load())
            < 2.5
        {
            self.on_bed_ticks += 1;
            if self.on_bed_ticks > self.get_tick_count(16) {
                cat.set_lying(true);
                cat.set_relax_state_one(false);
            } else {
                cat.mob_entity.look_at(owner.as_ref(), 45.0, 45.0);
                cat.set_relax_state_one(true);
            }
        } else {
            cat.set_lying(false);
        }
    }
}

type BedProperties = pumpkin_data::block_properties::WhiteBedLikeProperties;

/// Vanilla `CatLieOnBlockGoal`: a tame cat lies down on a nearby bed.
struct CatLieOnBlockGoal {
    goal: MoveToTargetPosGoal<Self>,
}

impl CatLieOnBlockGoal {
    #[expect(
        clippy::unnecessary_box_returns,
        reason = "the goal's parent handle points into the box"
    )]
    fn new(speed_modifier: f64, search_range: i32) -> Box<Self> {
        let mut this = Box::new(Self {
            goal: MoveToTargetPosGoal::new(ParentHandle::none(), speed_modifier, search_range, 6),
        });
        this.goal.lowest_y = -2;
        // SAFETY: `this` heap allocation address is pinned in Box and outlives `ParentHandle` references.
        this.goal.move_to_target_pos = unsafe { ParentHandle::new(&this) };
        this
    }
}

impl MoveToTargetPos for CatLieOnBlockGoal {
    /// Vanilla `CatLieOnBlockGoal.isValidTarget`.
    fn is_target_pos(&self, world: Arc<World>, block_pos: BlockPos) -> bool {
        world.get_block_state(&block_pos.up()).is_air()
            && world
                .get_block(&block_pos)
                .has_tag(&tag::Block::MINECRAFT_CATS_CAN_LIE_ON)
    }

    fn next_start_tick(&self, _mob: &dyn Mob) -> i32 {
        40
    }
}

impl Goal for CatLieOnBlockGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_cat(mob).is_some_and(|cat| cat.is_tame() && !cat.is_ordered_to_sit() && !cat.is_lying())
            && self.goal.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.goal.start(mob);
        if let Some(cat) = as_cat(mob) {
            cat.set_in_sitting_pose(false);
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.goal.stop(mob);
        if let Some(cat) = as_cat(mob) {
            cat.set_lying(false);
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.goal.tick(mob);
        let Some(cat) = as_cat(mob) else {
            return;
        };
        cat.set_in_sitting_pose(false);
        if !self.goal.is_reached_target() {
            cat.set_lying(false);
        } else if !cat.is_lying() {
            cat.set_lying(true);
        }
    }

    fn should_run_every_tick(&self) -> bool {
        self.goal.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}

/// Vanilla `CatSitOnBlockGoal`: a tame cat sits on a closed chest, a lit furnace or the
/// foot of a bed.
struct CatSitOnBlockGoal {
    goal: MoveToTargetPosGoal<Self>,
}

impl CatSitOnBlockGoal {
    #[expect(
        clippy::unnecessary_box_returns,
        reason = "the goal's parent handle points into the box"
    )]
    fn new(speed_modifier: f64) -> Box<Self> {
        let mut this = Box::new(Self {
            goal: MoveToTargetPosGoal::with_default(ParentHandle::none(), speed_modifier, 8),
        });
        // SAFETY: `this` heap allocation address is pinned in Box and outlives `ParentHandle` references.
        this.goal.move_to_target_pos = unsafe { ParentHandle::new(&this) };
        this
    }
}

impl MoveToTargetPos for CatSitOnBlockGoal {
    /// Vanilla `CatSitOnBlockGoal.isValidTarget` with its `BLOCK_TYPE_VALIDATION` list.
    fn is_target_pos(&self, world: Arc<World>, block_pos: BlockPos) -> bool {
        if !world.get_block_state(&block_pos.up()).is_air() {
            return false;
        }
        let (block, state) = world.get_block_and_state_id(&block_pos);
        if !block.has_tag(&tag::Block::MINECRAFT_CATS_CAN_SIT_ON) {
            return false;
        }
        if block == &Block::CHEST {
            // `ChestBlockEntity.getOpenCount`.
            let open_count = world
                .get_block_entity(&block_pos)
                .map_or(0, |block_entity| {
                    block_entity
                        .as_any()
                        .downcast_ref::<ChestBlockEntity>()
                        .map_or(0, ChestBlockEntity::get_viewer_count)
                });
            return open_count < 1;
        }
        if block == &Block::FURNACE {
            return FurnaceLikeProperties::from_state_id(state).lit;
        }
        if block.has_tag(&tag::Block::MINECRAFT_BEDS) {
            return !BedProperties::handles_block_id(block.id)
                || BedProperties::from_state_id(state).part != BedPart::Head;
        }
        true
    }
}

impl Goal for CatSitOnBlockGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        as_cat(mob).is_some_and(|cat| cat.is_tame() && !cat.is_ordered_to_sit())
            && self.goal.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.goal.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.goal.start(mob);
        if let Some(cat) = as_cat(mob) {
            cat.set_in_sitting_pose(false);
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.goal.stop(mob);
        if let Some(cat) = as_cat(mob) {
            cat.set_in_sitting_pose(false);
        }
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.goal.tick(mob);
        if let Some(cat) = as_cat(mob) {
            cat.set_in_sitting_pose(self.goal.is_reached_target());
        }
    }

    fn should_run_every_tick(&self) -> bool {
        self.goal.should_run_every_tick()
    }

    fn controls(&self) -> Controls {
        self.goal.controls()
    }
}
