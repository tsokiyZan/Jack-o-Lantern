use crate::enchantment::HurtAndBreak;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tracked_data;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::GameMode;
use pumpkin_util::math::vector3::Vector3;

use crate::entity::item::ItemEntity;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::player::Player;
use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity, equipment::RegionalDifficulty, skeleton::SkeletonEntityBase},
};
use crate::world::World;
use crate::world::loot::LootContextParameters;

/// Vanilla `BuiltInLootTables.BOGGED_SHEAR`.
const BOGGED_SHEAR_LOOT_TABLE: &str = "minecraft:shearing/bogged";

/// Vanilla `Bogged`.
pub struct BoggedSkeletonEntity {
    entity: Arc<SkeletonEntityBase>,
    sheared: AtomicBool,
}

impl BoggedSkeletonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = SkeletonEntityBase::new(entity);
        Arc::new(Self {
            entity,
            sheared: AtomicBool::new(false),
        })
    }

    #[must_use]
    pub fn is_sheared(&self) -> bool {
        self.sheared.load(Ordering::Relaxed)
    }

    pub fn set_sheared(&self, sheared: bool) {
        self.sheared.store(sheared, Ordering::Relaxed);
        self.get_entity()
            .set_synced_data(tracked_data::bogged::DATA_SHEARED, sheared);
    }

    /// Vanilla `Bogged.readyForShearing`.
    #[must_use]
    pub fn ready_for_shearing(&self) -> bool {
        !self.is_sheared()
    }

    /// Vanilla `Bogged.shear`.
    pub fn shear(&self, sound_category: SoundCategory, tool: &ItemStack) {
        let entity = self.get_entity();
        let world = entity.world.load();
        world.play_sound(Sound::EntityBoggedShear, sound_category, &entity.pos.load());
        self.spawn_sheared_mushrooms(&world, tool);
        self.set_sheared(true);
    }

    /// Vanilla `Bogged.spawnShearedMushrooms`: the shearing loot drops from the top of the
    /// bogged.
    fn spawn_sheared_mushrooms(&self, world: &Arc<World>, tool: &ItemStack) {
        let entity = self.get_entity();
        let Some(loot_table) = world.get_loot_table(BOGGED_SHEAR_LOOT_TABLE) else {
            return;
        };
        let pos = entity.pos.load();
        let params = LootContextParameters {
            this_entity: Some(entity.entity_type),
            position: Some(pos),
            tool: Some(tool.clone()),
            ..Default::default()
        };
        let drop_pos = pos.add(&Vector3::new(0.0, f64::from(entity.height()), 0.0));
        for stack in
            crate::world::loot::generate_loot_from_handle(&loot_table, rand::random(), &params)
        {
            if stack.is_empty() {
                continue;
            }
            world.spawn_entity(Arc::new(ItemEntity::new(
                Entity::new(world.clone(), drop_pos, &EntityType::ITEM),
                stack,
            )));
        }
    }
}

impl Mob for BoggedSkeletonEntity {
    /// Vanilla `rideTick`.
    fn mob_after_ride_tick(&self) {
        crate::entity::mob::turn_body_with_controlled_vehicle(self);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn on_equip_item(&self, _slot: &EquipmentSlot) {
        self.entity.reassess_weapon_goal(self);
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_bool("sheared", self.is_sheared());
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.entity.reassess_weapon_goal(self);
        self.set_sheared(nbt.get_bool("sheared").unwrap_or(false));
    }

    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        self.entity.finalize_spawn(self, world, group_data)
    }

    fn populate_default_equipment_slots(
        &self,
        world: &Arc<World>,
        difficulty: &RegionalDifficulty,
    ) {
        self.entity
            .populate_default_equipment_slots(world, difficulty);
    }

    fn wants_to_pick_up(
        &self,
        _brain: &crate::entity::ai::brain::Brain,
        stack: &pumpkin_data::item_stack::ItemStack,
    ) -> bool {
        super::wants_to_pick_up(self, stack)
    }

    fn get_preferred_weapon_type(&self) -> Option<&'static pumpkin_data::tag::Tag> {
        Some(&pumpkin_data::tag::Item::MINECRAFT_SKELETON_PREFERRED_WEAPONS)
    }

    /// Vanilla `Bogged.mobInteract`.
    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        if item_stack.item != &Item::SHEARS || !self.ready_for_shearing() {
            return self.get_mob_entity().mob_interact(player, item_stack);
        }
        self.shear(SoundCategory::Players, item_stack);
        let entity = self.get_entity();
        entity.world.load().game_event_from(
            Some(player.as_ref()),
            GameEvent::Shear,
            entity.pos.load(),
        );
        if player.gamemode.load() != GameMode::Creative {
            let _ = item_stack.hurt_and_break(&entity.world.load(), 1);
        }
        true
    }
}
