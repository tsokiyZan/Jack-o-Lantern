use std::sync::Arc;

use crate::item::interaction_result::InteractionResult;

use pumpkin_data::data_component_impl::DyeImpl;
use pumpkin_data::dye_color::DyeColor;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_inventory::crafting::crafting_inventory::CraftingInventory;
use pumpkin_inventory::crafting::crafting_screen_handler::match_crafting_recipe;
use pumpkin_inventory::crafting::recipe_provider::RecipeProvider;
use pumpkin_inventory::inventory::Inventory;

use crate::entity::ageable::get_speed_up_seconds_when_feeding;
use crate::entity::{
    EntityBase,
    ai::brain::BrainTick,
    ai::goal::breed::BreedGoal,
    ai::pathfinder::node::PathType,
    mob::{Mob, MobEntity},
    player::Player,
    r#type::from_type,
};
use crate::world::World;
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::RandomImpl;
use uuid::Uuid;

/// Vanilla `Animal`'s constructor: animals keep clear of fire.
pub fn set_animal_pathfinding_malus(mob_entity: &MobEntity) {
    mob_entity.set_pathfinding_malus(PathType::DangerFire, 16.0);
    mob_entity.set_pathfinding_malus(PathType::DamageFire, -1.0);
}

pub trait Animal: Mob {
    fn is_food(&self, item_stack: &ItemStack) -> bool;

    /// Vanilla `Animal.canMate`.
    fn can_mate(&self, partner: &dyn Mob) -> bool {
        animal_can_mate(self.get_mob_entity(), partner)
    }

    /// Vanilla `Animal.canFallInLove`.
    fn can_fall_in_love(&self) -> bool {
        !self.get_mob_entity().is_in_love()
    }

    /// Vanilla `AbstractHorse.canParent`; only the horse family breeds through it.
    fn can_parent(&self) -> bool {
        false
    }

    /// Vanilla `getBreedOffspring`: the baby this animal makes with `partner`, or `None` when
    /// they make none.
    // TODO: species whose offspring differs (horse and donkey make a mule) still override this.
    fn get_breed_offspring(
        &self,
        world: &Arc<World>,
        _partner: &dyn EntityBase,
    ) -> Option<Arc<dyn EntityBase>> {
        let entity = self.get_entity();
        Some(from_type(
            entity.entity_type,
            entity.pos.load(),
            world,
            Uuid::new_v4(),
        ))
    }

    /// Vanilla `Animal.spawnChildFromBreeding`, run from `AnimalMakeLove` inside the
    /// brain tick.
    fn spawn_child_from_breeding(&self, tick: &mut BrainTick<'_>, partner: &dyn EntityBase) {
        BreedGoal::breed(tick.mob, partner, None);
    }

    /// Animals prefer grass, then bright spots.
    fn animal_walk_target_value(&self, pos: &pumpkin_util::math::position::BlockPos) -> f32 {
        let world = self.get_mob_entity().living_entity.entity.world.load();
        if world.get_block(&pos.down()).id == pumpkin_data::Block::GRASS_BLOCK.id {
            return 10.0;
        }
        world.get_light_level_dependent_magic_value(pos) - 0.5
    }

    /// Vanilla `Animal.getBaseExperienceReward`.
    fn animal_base_experience_reward(&self) -> u32 {
        1 + self.get_entity().random().next_bounded_i32(3) as u32
    }

    /// Vanilla `Mob.usePlayerItem`, short of the `use_remainder` component.
    fn use_player_item(&self, player: &Player, item_stack: &mut ItemStack) {
        item_stack.decrement_unless_creative(player.gamemode.load(), 1);
    }

    fn play_eating_sound(&self, sound: Sound) {
        let mob_entity = self.get_mob_entity();
        let entity = &mob_entity.living_entity.entity;
        let world = entity.world.load();
        world.play_sound(sound, SoundCategory::Neutral, &entity.pos.load());
    }

    fn write_animal_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        let mob_entity = self.get_mob_entity();
        let in_love = mob_entity
            .love_ticks
            .load(std::sync::atomic::Ordering::Relaxed);
        nbt.put_int("InLove", in_love);
        if let Some(uuid) = mob_entity.breeder.load() {
            nbt.put_uuid("LoveCause", uuid);
        }
    }

    fn read_animal_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        let mob_entity = self.get_mob_entity();
        let in_love = nbt.get_int("InLove").unwrap_or(0);
        let love_cause = nbt.get_uuid("LoveCause");
        mob_entity.set_love_ticks(in_love, love_cause);
    }

    /// Vanilla `Animal.mobInteract`: food breeds a ready adult (`SUCCESS_SERVER`) or grows a
    /// baby up (`SUCCESS`).
    fn animal_interact(
        &self,
        player: &Arc<Player>,
        item_stack: &mut ItemStack,
        ambient_sound: Sound,
    ) -> InteractionResult {
        let mob_entity = self.get_mob_entity();
        if self.is_food(item_stack) {
            let age = mob_entity
                .living_entity
                .entity
                .age
                .load(std::sync::atomic::Ordering::Relaxed);

            if age >= 0 && mob_entity.is_breeding_ready() && self.can_fall_in_love() {
                self.use_player_item(player, item_stack);

                mob_entity.set_love_ticks(600, Some(player.gameprofile.id));
                let entity = &mob_entity.living_entity.entity;
                let world = entity.world.load();
                let pos = entity.pos.load();

                world.send_entity_status(
                    entity,
                    pumpkin_data::entity::EntityStatus::InLoveHearts,
                    Some(ActorEventID::InLoveHearts),
                );

                world.spawn_particle(
                    pos + Vector3::new(0.0, f64::from(entity.height()), 0.0),
                    Vector3::new(0.5, 0.5, 0.5),
                    1.0,
                    7,
                    Particle::Heart,
                );
                self.play_eating_sound(ambient_sound);
                return InteractionResult::SUCCESS_SERVER;
            }

            if let Some(ageable) = self.as_ageable()
                && ageable.can_age_up()
            {
                self.use_player_item(player, item_stack);
                ageable.age_up(get_speed_up_seconds_when_feeding(-age), true);
                self.play_eating_sound(ambient_sound);
                return InteractionResult::SUCCESS;
            }
        }

        InteractionResult::PASS
    }
}

#[must_use]
pub fn get_dye_color_from_item(item: &pumpkin_data::item::Item) -> Option<u8> {
    match item.registry_key {
        "white_dye" => Some(0),
        "orange_dye" => Some(1),
        "magenta_dye" => Some(2),
        "light_blue_dye" => Some(3),
        "yellow_dye" => Some(4),
        "lime_dye" => Some(5),
        "pink_dye" => Some(6),
        "gray_dye" => Some(7),
        "light_gray_dye" => Some(8),
        "cyan_dye" => Some(9),
        "purple_dye" => Some(10),
        "blue_dye" => Some(11),
        "brown_dye" => Some(12),
        "green_dye" => Some(13),
        "red_dye" => Some(14),
        "black_dye" => Some(15),
        _ => None,
    }
}

/// Vanilla `DyeColor.getMixedColor`: what crafting the two dyes together makes, or either
/// one at random when no recipe mixes them.
#[must_use]
pub fn get_mixed_color(world: &World, color1: DyeColor, color2: DyeColor) -> DyeColor {
    find_color_mix_in_recipes(world, color1, color2).unwrap_or_else(|| {
        if world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_bool()
        {
            color1
        } else {
            color2
        }
    })
}

/// Vanilla `DyeColor.findColorMixInRecipes`.
fn find_color_mix_in_recipes(
    world: &World,
    color1: DyeColor,
    color2: DyeColor,
) -> Option<DyeColor> {
    let dye_color = |stack: &ItemStack| stack.get_data_component::<DyeImpl>().map(|dye| dye.color);
    // Vanilla `findAll(DataComponents.DYE, color)`.
    let dye_items = |color: DyeColor| {
        (0..=u16::MAX)
            .filter_map(Item::from_id)
            .map(|item| ItemStack::new(1, item))
            .filter(move |stack| dye_color(stack) == Some(color))
            .collect::<Vec<_>>()
    };
    let dye1_items = dye_items(color1);
    let dye2_items = dye_items(color2);
    let server = world.server.upgrade();
    let provider = server
        .as_ref()
        .map(|server| &*server.recipe_manager as &dyn RecipeProvider);
    for dye1 in &dye1_items {
        for dye2 in &dye2_items {
            let input = CraftingInventory::new(2, 1);
            input.set_stack(0, dye1.clone());
            input.set_stack(1, dye2.clone());
            if let Some(result) = match_crafting_recipe(&input, provider) {
                let id = result
                    .item_id
                    .strip_prefix("minecraft:")
                    .unwrap_or(&result.item_id);
                if let Some(color) =
                    Item::from_registry_key(id).and_then(|item| dye_color(&ItemStack::new(1, item)))
                {
                    return Some(color);
                }
            }
        }
    }
    None
}

#[must_use]
pub const fn get_wool_item_for_color(color: u8) -> &'static pumpkin_data::item::Item {
    match color {
        0 => &pumpkin_data::item::Item::WHITE_WOOL,
        1 => &pumpkin_data::item::Item::ORANGE_WOOL,
        2 => &pumpkin_data::item::Item::MAGENTA_WOOL,
        3 => &pumpkin_data::item::Item::LIGHT_BLUE_WOOL,
        4 => &pumpkin_data::item::Item::YELLOW_WOOL,
        5 => &pumpkin_data::item::Item::LIME_WOOL,
        6 => &pumpkin_data::item::Item::PINK_WOOL,
        7 => &pumpkin_data::item::Item::GRAY_WOOL,
        8 => &pumpkin_data::item::Item::LIGHT_GRAY_WOOL,
        9 => &pumpkin_data::item::Item::CYAN_WOOL,
        10 => &pumpkin_data::item::Item::PURPLE_WOOL,
        11 => &pumpkin_data::item::Item::BLUE_WOOL,
        12 => &pumpkin_data::item::Item::BROWN_WOOL,
        13 => &pumpkin_data::item::Item::GREEN_WOOL,
        14 => &pumpkin_data::item::Item::RED_WOOL,
        _ => &pumpkin_data::item::Item::BLACK_WOOL,
    }
}

#[must_use]
pub fn get_carpet_color_from_item(item: &pumpkin_data::item::Item) -> Option<u8> {
    match item.registry_key {
        "white_carpet" => Some(0),
        "orange_carpet" => Some(1),
        "magenta_carpet" => Some(2),
        "light_blue_carpet" => Some(3),
        "yellow_carpet" => Some(4),
        "lime_carpet" => Some(5),
        "pink_carpet" => Some(6),
        "gray_carpet" => Some(7),
        "light_gray_carpet" => Some(8),
        "cyan_carpet" => Some(9),
        "purple_carpet" => Some(10),
        "blue_carpet" => Some(11),
        "brown_carpet" => Some(12),
        "green_carpet" => Some(13),
        "red_carpet" => Some(14),
        "black_carpet" => Some(15),
        _ => None,
    }
}

/// The base `Animal.canMate`: another animal of exactly this class, both in love.
#[must_use]
pub fn animal_can_mate(animal: &MobEntity, partner: &dyn Mob) -> bool {
    let entity = &animal.living_entity.entity;
    let partner_entity = partner.get_entity();
    partner_entity.entity_id != entity.entity_id
        && partner_entity.entity_type == entity.entity_type
        && animal.is_in_love()
        && partner.get_mob_entity().is_in_love()
}

/// Vanilla `AbstractHorse.canParent`.
#[must_use]
pub fn horse_can_parent(horse: &dyn Mob, is_tamed: bool) -> bool {
    let mob_entity = horse.get_mob_entity();
    let living = &mob_entity.living_entity;
    !living.entity.has_passengers()
        && !living.entity.has_vehicle()
        && is_tamed
        && living.entity.age.load(std::sync::atomic::Ordering::Relaxed) >= 0
        && living.health.load() >= living.get_max_health()
        && mob_entity.is_in_love()
}
