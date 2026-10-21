use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::potion::Effect;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::random::RandomImpl;
use std::sync::Arc;

use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::mob::spawn::SpawnGroupData;
use crate::entity::mob::zombie::ZombieEntityBase;
use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
};
use crate::world::World;
use pumpkin_nbt::compound::NbtCompound;

pub struct HuskEntity {
    entity: Arc<ZombieEntityBase>,
}

impl HuskEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = ZombieEntityBase::with_can_break_doors(entity, false, BABY_DIMENSIONS);
        let zombie = Self { entity };
        Arc::new(zombie)
    }

    #[must_use]
    pub fn with_can_break_doors(entity: Entity, can_break_doors: bool) -> Arc<Self> {
        let entity =
            ZombieEntityBase::with_can_break_doors(entity, can_break_doors, BABY_DIMENSIONS);
        let zombie = Self { entity };
        Arc::new(zombie)
    }
}

/// Vanilla `Husk.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::HUSK_BABY_DIMENSIONS;

impl Mob for HuskEntity {
    /// Vanilla `Zombie.wantsToPickUp`.
    fn mob_wants_to_pick_up(&self, stack: &pumpkin_data::item_stack::ItemStack) -> bool {
        crate::entity::mob::zombie::ZombieEntityBase::zombie_wants_to_pick_up(self, stack)
    }

    crate::impl_zombie_mob!(entity);

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    /// Vanilla `Husk.finalizeSpawn`.
    fn finalize_spawn(
        &self,
        world: &Arc<World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<SpawnGroupData>,
    ) -> Option<SpawnGroupData> {
        let group_data = self
            .entity
            .finalize_zombie_spawn(self, world, group_data, false);
        let difficulty = RegionalDifficulty::at(world, self.get_entity().pos.load());
        // Rolled again on top of `Zombie.finalizeSpawn`'s.
        // `level.getRandom()`.
        let roll = world
            .random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next_f32();
        self.entity
            .mob_entity
            .set_can_pick_up_loot(roll < 0.55 * difficulty.special_multiplier);
        // TODO: `Husk.finalizeSpawn`'s camel husk jockey needs `CamelHusk` and the
        // `NATURAL` spawn reason.
        group_data
    }

    fn populate_default_equipment_slots(
        &self,
        world: &Arc<World>,
        difficulty: &RegionalDifficulty,
    ) {
        self.entity
            .populate_default_equipment_slots(world, difficulty);
    }

    /// Vanilla `Husk.doHurtTarget` after a hit lands: an empty-handed husk makes its
    /// target hungry, after `Zombie.doHurtTarget`'s fire.
    fn on_attack(&self, target: &dyn EntityBase) {
        self.entity.on_attack(self, target);
        let mob_entity = &self.entity.mob_entity;
        if !mob_entity
            .get_item_slot(&EquipmentSlot::MAIN_HAND)
            .is_empty()
        {
            return;
        }
        let Some(living) = target.get_living_entity() else {
            return;
        };
        let world = self.get_entity().world.load_full();
        let difficulty =
            RegionalDifficulty::at(&world, self.get_entity().pos.load()).effective_difficulty;
        living.add_effect(Effect {
            effect_type: &StatusEffect::HUNGER,
            duration: 140 * difficulty as i32,
            amplifier: 0,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        });
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.entity.write_zombie_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.entity.read_zombie_nbt(self, nbt);
    }
}
