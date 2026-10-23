use crate::entity::breath::BreathManager;
use pumpkin_data::data_component_impl::{SALMON_VARIANTS, SalmonSizeImpl};
use pumpkin_data::entity::EntityPose;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::random::RandomImpl;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use pumpkin_data::sound::Sound;

use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
    passive::{
        fish,
        schooling_fish::{self, SchoolingFish},
    },
};

/// Represents a Salmon, a passive fish of rivers and cold oceans.
///
/// Wiki: <https://minecraft.wiki/w/Salmon>
/// Vanilla `Salmon.getMaxSchoolSize`.
const MAX_SCHOOL_SIZE: i32 = 5;

/// Vanilla `Salmon.Variant`: the size, its id in `SALMON_VARIANTS` order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SalmonSize {
    Small = 0,
    Medium = 1,
    Large = 2,
}

impl SalmonSize {
    /// Vanilla `Salmon.Variant.DEFAULT`.
    pub const DEFAULT: Self = Self::Medium;

    /// Vanilla `Variant.BY_ID`, `ByIdMap.continuous` with `CLAMP`.
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            i32::MIN..=0 => Self::Small,
            1 => Self::Medium,
            _ => Self::Large,
        }
    }

    /// Vanilla `StringRepresentable.fromEnum`: the serialized name, or `None`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        SALMON_VARIANTS
            .iter()
            .position(|variant| *variant == name)
            .map(|id| Self::from_id(id as i32))
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        SALMON_VARIANTS[self as usize]
    }

    /// Vanilla `Salmon.Variant.boundingBoxScale`.
    #[must_use]
    pub const fn bounding_box_scale(self) -> f32 {
        match self {
            Self::Small => 0.5,
            Self::Medium => 1.0,
            Self::Large => 1.5,
        }
    }
}

pub struct SalmonEntity {
    pub mob_entity: MobEntity,
    pub school: SchoolingFish,
    pub from_bucket: fish::FromBucket,
    /// Vanilla `DATA_TYPE`.
    size: AtomicU8,
}

impl SalmonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        fish::init(&mob_entity);
        schooling_fish::init(&mob_entity);
        Arc::new(Self {
            mob_entity,
            school: SchoolingFish::new(MAX_SCHOOL_SIZE),
            from_bucket: fish::FromBucket::default(),
            size: AtomicU8::new(SalmonSize::DEFAULT as u8),
        })
    }

    #[must_use]
    pub fn get_size(&self) -> SalmonSize {
        SalmonSize::from_id(i32::from(self.size.load(Ordering::Relaxed)))
    }

    /// Vanilla `Salmon.setVariant`; `onSyncedDataUpdated` refreshes the dimensions.
    pub fn set_size(&self, size: SalmonSize) {
        self.size.store(size as u8, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::salmon::DATA_TYPE,
            VarInt(size as i32),
        );
        self.refresh_dimensions();
    }

    /// Vanilla `Salmon.saveToBucketTag`: the default bucket data, then
    /// `copyFrom(SALMON_SIZE)`.
    fn save_to_bucket_tag(&self, bucket: &mut ItemStack) {
        let tag = crate::entity::bucketable::save_default_data_to_bucket_tag(self, bucket);
        bucket.set_data_component(pumpkin_data::data_component_impl::BucketEntityDataImpl {
            nbt: Some(tag),
        });
        bucket.set_data_component(SalmonSizeImpl {
            value: std::borrow::Cow::Borrowed(self.get_size().name()),
        });
    }
}

impl Mob for SalmonEntity {
    /// Vanilla `Salmon.finalizeSpawn`: the size from `WeightedList` SMALL 30, MEDIUM 50,
    /// LARGE 15 on the salmon's own random, then `AbstractSchoolingFish.finalizeSpawn`.
    fn finalize_spawn(
        &self,
        _world: &Arc<crate::world::World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        let selection = self.get_entity().random().next_bounded_i32(95);
        self.set_size(if selection < 30 {
            SalmonSize::Small
        } else if selection < 80 {
            SalmonSize::Medium
        } else {
            SalmonSize::Large
        });
        self.mob_entity.finalize_spawn_base();
        Some(schooling_fish::finalize_school_spawn(
            &self.school,
            group_data,
        ))
    }

    /// Vanilla `Salmon.getDefaultDimensions`: scaled by the size.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        self.get_entity()
            .get_dimensions(pose)
            .scale(self.age_scale())
            .scale(self.get_size().bounding_box_scale())
    }

    /// Vanilla `Salmon.applyImplicitComponents`: `SALMON_SIZE`.
    fn mob_apply_item_components(&self, item: &ItemStack) -> bool {
        let Some(component) = item.get_data_component::<SalmonSizeImpl>() else {
            return false;
        };
        if let Some(size) = SalmonSize::from_name(&component.value) {
            self.set_size(size);
        }
        true
    }

    fn mob_init_data_tracker(&self) {
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::salmon::DATA_TYPE,
            VarInt(self.get_size() as i32),
        );
    }

    /// Vanilla `WaterAnimal.checkSpawnObstruction`: water doesn't obstruct it.
    fn check_spawn_obstruction(&self, world: &crate::world::World) -> bool {
        let entity = self.get_entity();
        world.is_unobstructed(Some(entity), &entity.bounding_box.load())
    }

    fn set_from_bucket(&self, from_bucket: bool) {
        self.from_bucket.set(self, from_bucket);
    }

    fn requires_custom_persistence(&self) -> bool {
        self.from_bucket.requires_custom_persistence(self)
    }

    fn remove_when_far_away(&self, _distance_sq: f64) -> bool {
        self.from_bucket.remove_when_far_away(self)
    }

    fn mob_write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.from_bucket.write_nbt(nbt);
        // Vanilla `Salmon.addAdditionalSaveData`.
        nbt.put_string("type", self.get_size().name().to_string());
    }

    fn mob_read_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.from_bucket.read_nbt(self, nbt);
        // Vanilla `Salmon.readAdditionalSaveData`: a missing or unknown size is MEDIUM.
        self.set_size(
            nbt.get_string("type")
                .and_then(SalmonSize::from_name)
                .unwrap_or(SalmonSize::DEFAULT),
        );
    }

    /// Vanilla `AbstractFish.loadFromBucketTag`.
    fn load_from_bucket_tag(&self, tag: &pumpkin_nbt::compound::NbtCompound) {
        crate::entity::bucketable::load_default_data_from_bucket_tag(self, tag);
    }

    fn mob_interact(
        &self,
        player: &Arc<crate::entity::player::Player>,
        _hand: pumpkin_util::Hand,
        item_stack: &mut pumpkin_data::item_stack::ItemStack,
    ) -> crate::item::interaction_result::InteractionResult {
        crate::entity::bucketable::fish_mob_interact(
            self,
            player,
            item_stack,
            &pumpkin_data::item::Item::SALMON_BUCKET,
            |bucket| self.save_to_bucket_tag(bucket),
        )
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    /// Vanilla `WaterAnimal.getBaseExperienceReward`.
    fn get_base_experience_reward(&self) -> u32 {
        1 + self.get_entity().random().next_bounded_i32(3) as u32
    }

    /// Vanilla `WaterAnimal.handleAirSupply`.
    fn handle_air_supply(&self, pre_tick_air: i32) {
        BreathManager::water_animal_handle_air_supply(self, pre_tick_air);
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.school.tick(self);
    }

    /// Vanilla `AbstractFish.aiStep`.
    fn ai_step(&self, _caller: &dyn EntityBase) {
        fish::flop(self, Sound::EntitySalmonFlop);
    }

    fn as_schooling_fish(&self) -> Option<&SchoolingFish> {
        Some(&self.school)
    }

    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        fish::travel(self, caller)
    }

    fn mob_is_pushed_by_fluids(&self) -> bool {
        false
    }
}
