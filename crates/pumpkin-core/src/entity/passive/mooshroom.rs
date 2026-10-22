use crate::item::interaction_result::InteractionResult;
use crate::world::ParticleRandomization;
use pumpkin_data::data_component_impl::{
    DataComponentImpl, SuspiciousStewEffect, SuspiciousStewEffectsImpl,
};
use pumpkin_util::Hand;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicI32, Ordering},
};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::vector3::Vector3;
use uuid::Uuid;

use crate::entity::{
    Entity, EntityBase,
    ageable::{AgeableData, AgeableMob},
    ai::goal::{
        breed::BreedGoal, escape_danger::EscapeDangerGoal, follow_parent::FollowParentGoal,
        look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal, swim::SwimGoal,
        tempt::TemptGoal, wander_around::WanderAroundGoal,
    },
    item::ItemEntity,
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

const TEMPT_ITEMS: &[&Item] = &[&Item::WHEAT];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum MooshroomVariant {
    #[default]
    Red = 0,
    Brown = 1,
}

impl MooshroomVariant {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Brown,
            _ => Self::Red,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Brown => "brown",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "brown" => Self::Brown,
            _ => Self::Red,
        }
    }
}

pub struct MooshroomEntity {
    pub mob_entity: MobEntity,
    pub ageable_data: AgeableData,
    pub variant: AtomicI32,
    /// Vanilla `stewEffects`: what the flower it ate puts in its next stew.
    pub stew_effects: std::sync::Mutex<Option<SuspiciousStewEffectsImpl>>,
    pub last_lightning_bolt_uuid: AtomicCell<Option<Uuid>>,
}

impl MooshroomEntity {
    /// The flower branch of `MushroomCow.mobInteract` for a brown adult: a smoke puff when it
    /// already has stew effects, else it eats the flower and keeps its effect.
    fn eat_flower(
        &self,
        player: &Player,
        item_stack: &mut ItemStack,
        effect: (&'static str, i32),
    ) -> InteractionResult {
        let entity = self.get_entity();
        let world = entity.world.load();
        let pos = entity.pos.load();
        let particle_pos = pos + Vector3::new(0.0, f64::from(entity.height()) * 0.5, 0.0);
        let has_effects = self
            .stew_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some();
        if has_effects {
            world.send_particles(
                Particle::Smoke,
                &[],
                false,
                false,
                particle_pos,
                2,
                Vector3::new(0.5, 0.0, 0.5),
                Vector3::new(0.0, 0.2, 0.0),
                ParticleRandomization::AlternativeWithSpeed,
            );
        } else {
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            // `SpellParticleOption.create(EFFECT, -1, 1.0F)`: color, then power.
            let mut options = (-1i32).to_be_bytes().to_vec();
            options.extend_from_slice(&1.0f32.to_be_bytes());
            world.send_particles(
                Particle::Effect,
                &options,
                false,
                false,
                particle_pos,
                4,
                Vector3::new(0.5, 0.0, 0.5),
                Vector3::new(0.0, 0.2, 0.0),
                ParticleRandomization::AlternativeWithSpeed,
            );
            let (effect, duration) = effect;
            *self
                .stew_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(SuspiciousStewEffectsImpl {
                    effects: std::borrow::Cow::Owned(vec![SuspiciousStewEffect {
                        effect: std::borrow::Cow::Borrowed(effect),
                        duration,
                    }]),
                });
            self.play_sound_at_self(Sound::EntityMooshroomEat, 2.0, 1.0);
        }
        InteractionResult::SUCCESS
    }

    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        let mooshroom = Self {
            mob_entity,
            ageable_data: AgeableData::default(),
            variant: AtomicI32::new(MooshroomVariant::Red.id()),
            stew_effects: std::sync::Mutex::new(None),
            last_lightning_bolt_uuid: AtomicCell::new(None),
        };
        let mob_arc = Arc::new(mooshroom);
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

            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(1, EscapeDangerGoal::new(2.0));
            goal_selector.add_goal(2, BreedGoal::new(1.0));
            goal_selector.add_goal(3, Box::new(TemptGoal::new(1.25, TEMPT_ITEMS, false)));
            goal_selector.add_goal(4, Box::new(FollowParentGoal::new(1.25)));
            goal_selector.add_goal(5, Box::new(WanderAroundGoal::avoiding_water(1.0)));
            goal_selector.add_goal(
                6,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(7, Box::new(RandomLookAroundGoal::default()));
        };

        mob_arc
    }

    #[must_use]
    pub fn get_variant(&self) -> MooshroomVariant {
        MooshroomVariant::from_id(self.variant.load(Ordering::Relaxed))
    }

    pub fn set_variant(&self, variant: MooshroomVariant) {
        self.variant.store(variant.id(), Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::mooshroom::DATA_TYPE,
            VarInt(variant.id()),
        );
    }
}

impl AgeableMob for MooshroomEntity {
    fn get_ageable_data(&self) -> &AgeableData {
        &self.ageable_data
    }
}

impl Animal for MooshroomEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        item_stack.item.has_tag(&tag::Item::MINECRAFT_COW_FOOD)
            || TEMPT_ITEMS.iter().any(|i| i.id == item_stack.item.id)
    }
}

/// Vanilla `MushroomCow.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions =
    pumpkin_data::entity_dimensions::MUSHROOM_COW_BABY_DIMENSIONS;

impl Mob for MooshroomEntity {
    /// Vanilla `MushroomCow.getWalkTargetValue`.
    fn get_walk_target_value(&self, pos: &pumpkin_util::math::position::BlockPos) -> f32 {
        let world = self.get_entity().world.load();
        if world.get_block(&pos.down()) == &pumpkin_data::Block::MYCELIUM {
            10.0
        } else {
            world.get_light_level_dependent_magic_value(pos) - 0.5
        }
    }

    /// Vanilla `AbstractCow.playStepSound`.
    fn play_step_sound(
        &self,
        caller: &dyn crate::entity::EntityBase,
        _pos: &pumpkin_util::math::position::BlockPos,
        _block: &'static pumpkin_data::Block,
        _state: &'static pumpkin_data::BlockState,
    ) {
        let sound = pumpkin_data::cow_sound_variant::CowSoundVariant::default()
            .step_sound(self.get_entity().is_baby());
        caller.play_sound_at_self(sound, 0.15, 1.0);
    }

    /// Vanilla `AbstractCow.getAmbientSound`, with the mooshroom's classic sound set.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        Some(
            pumpkin_data::cow_sound_variant::CowSoundVariant::default()
                .ambient_sound(self.get_entity().is_baby()),
        )
    }

    /// Vanilla `MushroomCow.getDefaultDimensions`.
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
        nbt.put_string("Type", self.get_variant().as_str().to_string());
        if let Some(effects) = &*self
            .stew_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            nbt.put("stew_effects", effects.write_data());
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if let Some(variant_str) = nbt.get_string("Type") {
            self.set_variant(MooshroomVariant::from_name(variant_str));
        }
        *self
            .stew_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = nbt
            .get("stew_effects")
            .and_then(SuspiciousStewEffectsImpl::read_data);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::mooshroom::DATA_BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::mooshroom::DATA_TYPE,
            VarInt(self.get_variant().id()),
        );
    }

    fn mob_interact(
        &self,
        player: &Arc<Player>,
        hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        let item = item_stack.get_item();

        if item == &Item::BOWL && !self.is_baby() {
            let effects = self
                .stew_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let is_suspicious = effects.is_some();
            let stew = effects.map_or_else(
                || ItemStack::new(1, &Item::MUSHROOM_STEW),
                |effects| {
                    let mut stew = ItemStack::new(1, &Item::SUSPICIOUS_STEW);
                    stew.set_data_component(effects);
                    stew
                },
            );
            crate::item::create_filled_result(item_stack, player, stew, false);
            let sound = if is_suspicious {
                Sound::EntityMooshroomSuspiciousMilk
            } else {
                Sound::EntityMooshroomMilk
            };
            self.play_sound_at_self(sound, 1.0, 1.0);
            return InteractionResult::SUCCESS;
        }

        // `AbstractCow.mobInteract`.
        if item == &Item::BUCKET && !self.is_baby() {
            // Vanilla `Player.playSound`: everyone but the milking player, who plays it itself.
            let world = self.get_entity().world.load();
            world.play_sound_raw_expect(
                player,
                Sound::EntityCowMilk as u16,
                SoundCategory::Players,
                &player.get_entity().pos.load(),
                1.0,
                1.0,
            );
            crate::item::create_filled_result(
                item_stack,
                player,
                ItemStack::new(1, &Item::MILK_BUCKET),
                true,
            );
            return InteractionResult::SUCCESS;
        }

        // `readyForShearing`: not a baby.
        if item == &Item::SHEARS && !self.is_baby() {
            if crate::entity::mob::shear_event_cancelled(player, self.get_entity().entity_id, hand)
            {
                return InteractionResult::PASS;
            }
            self.shear(SoundCategory::Players, item_stack);
            let entity = self.get_entity();
            entity.world.load().game_event_from(
                Some(player.as_ref()),
                pumpkin_data::game_event::GameEvent::Shear,
                entity.pos.load(),
            );
            player.hurt_and_break_in_hand(item_stack, hand, 1);
            return InteractionResult::SUCCESS;
        }

        if self.get_variant() == MooshroomVariant::Brown && !self.is_baby() {
            let Some(effect) = flower_stew_effect(item) else {
                return self.animal_interact(player, item_stack, Sound::EntityCowAmbient);
            };
            return self.eat_flower(player, item_stack, effect);
        }

        self.animal_interact(player, item_stack, Sound::EntityCowAmbient)
    }
}

impl MooshroomEntity {
    /// Vanilla `MushroomCow.readyForShearing`: not a baby.
    #[must_use]
    pub fn ready_for_shearing(&self) -> bool {
        !self.is_baby()
    }

    /// Vanilla `MushroomCow.shear`: the snip, then the mooshroom turns into a cow
    /// (`convertTo(COW)`), leaving an explosion puff and its `shearing/mooshroom` loot, one
    /// item entity per item, at its top.
    pub fn shear(&self, source: SoundCategory, tool: &ItemStack) {
        let entity = self.get_entity();
        let world = entity.world.load_full();
        world.play_sound_from_entity(entity, Sound::EntityMooshroomShear, source, 1.0, 1.0);
        let pos = entity.pos.load();
        let height = f64::from(entity.height());
        crate::entity::mob::conversion::convert_to(
            self,
            &EntityType::COW,
            &crate::entity::mob::conversion::ConversionParams::single(self, false, false),
            |_cow| {
                world.spawn_particle(
                    pos + Vector3::new(0.0, height * 0.5, 0.0),
                    Vector3::new(0.0, 0.0, 0.0),
                    0.0,
                    1,
                    Particle::Explosion,
                );
                let drop_pos = pos + Vector3::new(0.0, height, 0.0);
                let table = if self.get_variant() == MooshroomVariant::Brown {
                    "minecraft:shearing/mooshroom/brown"
                } else {
                    "minecraft:shearing/mooshroom/red"
                };
                self.mob_entity
                    .living_entity
                    .drop_from_nested_shearing_loot_table(
                        "minecraft:shearing/mooshroom",
                        table,
                        tool,
                        |drop| {
                            for _ in 0..drop.item_count {
                                world.spawn_entity(Arc::new(ItemEntity::new(
                                    Entity::new(world.clone(), drop_pos, &EntityType::ITEM),
                                    drop.copy_with_count(1),
                                )));
                            }
                        },
                    );
            },
        );
    }
}

/// Vanilla `SuspiciousEffectHolder.tryGet` for an item: the stew effect of a flower
/// (`FlowerBlock`'s effect for `floor(seconds * 20)` ticks), or `None`.
fn flower_stew_effect(item: &Item) -> Option<(&'static str, i32)> {
    let effect = match item.registry_key {
        "dandelion" | "golden_dandelion" | "blue_orchid" => ("minecraft:saturation", 7),
        "torchflower" | "poppy" => ("minecraft:night_vision", 100),
        "allium" => ("minecraft:fire_resistance", 60),
        "azure_bluet" | "open_eyeblossom" => ("minecraft:blindness", 220),
        "red_tulip" | "orange_tulip" | "white_tulip" | "pink_tulip" => ("minecraft:weakness", 140),
        "oxeye_daisy" => ("minecraft:regeneration", 140),
        "cornflower" => ("minecraft:jump_boost", 100),
        "wither_rose" => ("minecraft:wither", 140),
        "lily_of_the_valley" => ("minecraft:poison", 220),
        "closed_eyeblossom" => ("minecraft:nausea", 140),
        _ => return None,
    };
    Some(effect)
}
