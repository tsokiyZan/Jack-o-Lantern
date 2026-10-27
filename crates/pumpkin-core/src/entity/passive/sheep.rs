use crate::item::interaction_result::InteractionResult;
use pumpkin_util::Hand;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicU8, Ordering},
};

use pumpkin_data::entity::EntityPose;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::{entity::EntityType, item::Item};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;

use crate::entity::{
    Entity, EntityBase,
    ageable::AgeableMob,
    ai::goal::{
        breed::BreedGoal, eat_grass::EatGrassGoal, escape_danger::EscapeDangerGoal,
        follow_parent::FollowParentGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, swim::SwimGoal, tempt::TemptGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
    passive::animal::{Animal, set_animal_pathfinding_malus},
    player::Player,
};

use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;

const TEMPT_ITEMS: &[&Item] = &[&Item::WHEAT];

pub struct SheepEntity {
    pub mob_entity: MobEntity,
    color_and_sheared: AtomicU8,
    pub ageable_data: crate::entity::ageable::AgeableData,
}

impl SheepEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        let sheep = Self {
            mob_entity,
            color_and_sheared: AtomicU8::new(0),
            ageable_data: crate::entity::ageable::AgeableData::default(),
        };
        let mob_arc = Arc::new(sheep);
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
            goal_selector.add_goal(1, EscapeDangerGoal::new(1.25));
            goal_selector.add_goal(2, BreedGoal::new(1.0));
            goal_selector.add_goal(3, Box::new(TemptGoal::new(1.1, TEMPT_ITEMS, false)));
            goal_selector.add_goal(4, Box::new(FollowParentGoal::new(1.1)));
            goal_selector.add_goal(5, Box::new(EatGrassGoal::default()));
            goal_selector.add_goal(6, Box::new(WanderAroundGoal::avoiding_water(1.0)));
            goal_selector.add_goal(
                7,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 6.0),
            );
            goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));
        };

        mob_arc
    }

    fn get_packed_byte(&self) -> u8 {
        self.color_and_sheared.load(Ordering::Relaxed)
    }

    pub fn get_color(&self) -> u8 {
        self.get_packed_byte() & 0x0F
    }

    pub fn is_sheared(&self) -> bool {
        (self.get_packed_byte() & 0x10) != 0
    }

    fn set_packed_and_sync(&self, byte: u8) {
        self.color_and_sheared.store(byte, Ordering::Relaxed);
        self.mob_entity
            .living_entity
            .entity
            .set_synced_data(pumpkin_data::tracked_data::sheep::WOOL_ID, byte as i8);
    }

    pub fn set_color(&self, color: u8) {
        let byte = (self.get_packed_byte() & 0xF0) | (color & 0x0F);
        self.set_packed_and_sync(byte);
    }

    /// Vanilla `Sheep.readyForShearing`.
    #[must_use]
    pub fn ready_for_shearing(&self) -> bool {
        !self.is_sheared() && !self.is_baby()
    }

    /// Vanilla `Sheep.shear`: the snip from the sheep, its color's wool from the shearing loot
    /// one item entity at a time a block up with a random nudge, then sheared.
    pub fn shear(&self, source: SoundCategory, tool: &ItemStack) {
        const COLORS: [&str; 16] = [
            "white",
            "orange",
            "magenta",
            "light_blue",
            "yellow",
            "lime",
            "pink",
            "gray",
            "light_gray",
            "cyan",
            "purple",
            "blue",
            "brown",
            "green",
            "red",
            "black",
        ];
        let entity = self.get_entity();
        let world = entity.world.load_full();
        world.play_sound_from_entity(entity, Sound::EntitySheepShear, source, 1.0, 1.0);
        let color = COLORS[usize::from(self.get_color()).min(15)];
        self.mob_entity
            .living_entity
            .drop_from_nested_shearing_loot_table(
                self,
                "minecraft:shearing/sheep",
                &format!("minecraft:shearing/sheep/{color}"),
                tool,
                |drop| {
                    for _ in 0..drop.item_count {
                        // `spawnAtLocation(level, drop.copyWithCount(1), 1.0F)`.
                        let item = crate::entity::item::ItemEntity::new(
                            Entity::new(
                                world.clone(),
                                entity.pos.load() + Vector3::new(0.0, 1.0, 0.0),
                                &EntityType::ITEM,
                            ),
                            drop.copy_with_count(1),
                        );
                        let nudge = {
                            let mut random = entity.random();
                            let x = (random.next_f32() - random.next_f32()) * 0.1;
                            let y = random.next_f32() * 0.05;
                            let z = (random.next_f32() - random.next_f32()) * 0.1;
                            Vector3::new(f64::from(x), f64::from(y), f64::from(z))
                        };
                        let velocity = item.get_entity().velocity.load();
                        item.get_entity().velocity.store(velocity + nudge);
                        world.spawn_entity(Arc::new(item));
                    }
                },
            );
        self.set_sheared(true);
    }

    pub fn set_sheared(&self, sheared: bool) {
        let byte = if sheared {
            self.get_packed_byte() | 0x10
        } else {
            self.get_packed_byte() & !0x10
        };
        self.set_packed_and_sync(byte);
    }
}

impl AgeableMob for SheepEntity {
    fn get_ageable_data(&self) -> &crate::entity::ageable::AgeableData {
        &self.ageable_data
    }
}

impl Animal for SheepEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        use pumpkin_data::tag::Taggable;
        item_stack
            .item
            .has_tag(&pumpkin_data::tag::Item::MINECRAFT_SHEEP_FOOD)
            || TEMPT_ITEMS.iter().any(|i| i.id == item_stack.item.id)
    }
}

/// Vanilla `Sheep.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::SHEEP_BABY_DIMENSIONS;

impl Mob for SheepEntity {
    fn mob_get_component(
        &self,
        component: pumpkin_data::data_component::DataComponent,
    ) -> Option<Box<dyn pumpkin_data::data_component_impl::DataComponentImpl>> {
        use pumpkin_data::{
            data_component::DataComponent, data_component_impl::SheepColorImpl, dye_color::DyeColor,
        };
        match component {
            DataComponent::SheepColor => Some(Box::new(SheepColorImpl {
                value: DyeColor::from(self.get_color()).name().into(),
            })),
            _ => None,
        }
    }

    /// Vanilla `Sheep.getDefaultDimensions`.
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

    /// Vanilla `AgeableMob.aiStep`, which runs inside this mob's `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("Sheared", self.is_sheared());
        nbt.put_byte("Color", self.get_color() as i8);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        let sheared = nbt
            .get_bool("Sheared")
            .or_else(|| nbt.get_byte("Sheared").map(|b| b == 1))
            .unwrap_or(false);
        let color = nbt.get_byte("Color").unwrap_or(0) as u8;
        let byte = (color & 0x0F) | if sheared { 0x10 } else { 0 };
        self.color_and_sheared.store(byte, Ordering::Relaxed);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn on_eating_grass(&self) {
        self.set_sheared(false);
    }

    fn mob_interact(
        &self,
        player: &Arc<Player>,
        hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        use super::animal::Animal;
        let item = item_stack.get_item();

        if item == &Item::SHEARS {
            // `readyForShearing`: alive, unsheared and grown up.
            if !self.get_entity().is_alive() || self.is_sheared() || self.is_baby() {
                return InteractionResult::CONSUME;
            }
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
            return InteractionResult::SUCCESS_SERVER;
        }

        // A dye is no sheep food: `DyeItem.interactLivingEntity` colours the sheep after
        // `mobInteract` passes.

        self.animal_interact(player, item_stack, Sound::EntitySheepAmbient)
    }
}
