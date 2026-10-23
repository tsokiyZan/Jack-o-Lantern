use crate::entity::breath::BreathManager;
use pumpkin_data::data_component_impl::{
    TROPICAL_FISH_PATTERNS, TropicalFishBaseColorImpl, TropicalFishPatternColorImpl,
    TropicalFishPatternImpl,
};
use pumpkin_data::dye_color::DyeColor;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::random::RandomImpl;
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use pumpkin_data::sound::Sound;

use crate::entity::{
    Entity, EntityBase,
    mob::{Mob, MobEntity},
    passive::{
        fish,
        schooling_fish::{self, DEFAULT_MAX_SCHOOL_SIZE, SchoolingFish},
    },
};

/// Represents a Tropical Fish, a passive fish of warm oceans.
///
/// Wiki: <https://minecraft.wiki/w/Tropical_Fish>
pub struct TropicalFishEntity {
    pub mob_entity: MobEntity,
    pub school: SchoolingFish,
    pub from_bucket: fish::FromBucket,
    /// Vanilla `DATA_ID_TYPE_VARIANT`: pattern, base colour and pattern colour packed.
    packed_variant: AtomicI32,
    /// Vanilla `isSchool`: false for a fish that rolled a rare variant.
    is_school: AtomicBool,
}

/// Vanilla `TropicalFish.Pattern.byId`: `ByIdMap.sparse` with KOB as the default.
fn pattern_by_id(packed_id: i32) -> i32 {
    TROPICAL_FISH_PATTERNS
        .iter()
        .find(|(id, _)| *id == packed_id)
        .map_or(0, |(id, _)| *id)
}

/// Vanilla `DyeColor.byId`: `ByIdMap.continuous` with `ZERO`, so WHITE when out of range.
fn dye_by_id(id: i32) -> DyeColor {
    u8::try_from(id)
        .ok()
        .and_then(DyeColor::by_id)
        .unwrap_or(DyeColor::White)
}

/// Vanilla `TropicalFish.packVariant`.
const fn pack_variant(pattern: i32, base_color: DyeColor, pattern_color: DyeColor) -> i32 {
    (pattern & 65535)
        | ((base_color.id() as i32 & 0xFF) << 16)
        | ((pattern_color.id() as i32 & 0xFF) << 24)
}

/// Vanilla `TropicalFish.getPattern(packedVariant)`.
fn get_pattern(packed: i32) -> i32 {
    pattern_by_id(packed & 65535)
}

/// Vanilla `TropicalFish.getBaseColor(packedVariant)`.
fn get_base_color(packed: i32) -> DyeColor {
    dye_by_id(packed >> 16 & 0xFF)
}

/// Vanilla `TropicalFish.getPatternColor(packedVariant)`.
fn get_pattern_color(packed: i32) -> DyeColor {
    dye_by_id(packed >> 24 & 0xFF)
}

/// Vanilla `TropicalFish.Variant(packedId).getPackedId()`: the packed variant with unknown
/// ids replaced by their defaults, as `Variant.CODEC` saves it.
fn normalize_variant(packed: i32) -> i32 {
    pack_variant(
        get_pattern(packed),
        get_base_color(packed),
        get_pattern_color(packed),
    )
}

/// The packed id of a pattern's serialized name.
fn pattern_from_name(name: &str) -> Option<i32> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    TROPICAL_FISH_PATTERNS
        .iter()
        .find(|(_, pattern)| *pattern == name)
        .map(|(id, _)| *id)
}

fn pattern_name(packed_id: i32) -> &'static str {
    TROPICAL_FISH_PATTERNS
        .iter()
        .find(|(id, _)| *id == packed_id)
        .map_or("kob", |(_, name)| name)
}

/// Vanilla `TropicalFish.COMMON_VARIANTS`.
const COMMON_VARIANTS: [(&str, DyeColor, DyeColor); 22] = [
    ("stripey", DyeColor::Orange, DyeColor::Gray),
    ("flopper", DyeColor::Gray, DyeColor::Gray),
    ("flopper", DyeColor::Gray, DyeColor::Blue),
    ("clayfish", DyeColor::White, DyeColor::Gray),
    ("sunstreak", DyeColor::Blue, DyeColor::Gray),
    ("kob", DyeColor::Orange, DyeColor::White),
    ("spotty", DyeColor::Pink, DyeColor::LightBlue),
    ("blockfish", DyeColor::Purple, DyeColor::Yellow),
    ("clayfish", DyeColor::White, DyeColor::Red),
    ("spotty", DyeColor::White, DyeColor::Yellow),
    ("glitter", DyeColor::White, DyeColor::Gray),
    ("clayfish", DyeColor::White, DyeColor::Orange),
    ("dasher", DyeColor::Cyan, DyeColor::Pink),
    ("brinely", DyeColor::Lime, DyeColor::LightBlue),
    ("betty", DyeColor::Red, DyeColor::White),
    ("snooper", DyeColor::Gray, DyeColor::Red),
    ("blockfish", DyeColor::Red, DyeColor::White),
    ("flopper", DyeColor::White, DyeColor::Yellow),
    ("kob", DyeColor::Red, DyeColor::White),
    ("sunstreak", DyeColor::Gray, DyeColor::White),
    ("dasher", DyeColor::Cyan, DyeColor::Yellow),
    ("flopper", DyeColor::Yellow, DyeColor::Yellow),
];

/// Vanilla `TropicalFish.Pattern.values()`, in declaration order.
const PATTERN_ORDER: [&str; 12] = [
    "kob",
    "sunstreak",
    "snooper",
    "dasher",
    "brinely",
    "spotty",
    "flopper",
    "stripey",
    "glitter",
    "blockfish",
    "betty",
    "clayfish",
];

/// Vanilla `TropicalFish.DEFAULT_VARIANT`: KOB, white on white.
const DEFAULT_VARIANT: i32 = 0;

impl TropicalFishEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        fish::init(&mob_entity);
        schooling_fish::init(&mob_entity);
        Arc::new(Self {
            mob_entity,
            school: SchoolingFish::new(DEFAULT_MAX_SCHOOL_SIZE),
            from_bucket: fish::FromBucket::default(),
            packed_variant: AtomicI32::new(DEFAULT_VARIANT),
            is_school: AtomicBool::new(true),
        })
    }

    #[must_use]
    pub fn get_packed_variant(&self) -> i32 {
        self.packed_variant.load(Ordering::Relaxed)
    }

    /// Vanilla `TropicalFish.setPackedVariant`, synced as `DATA_ID_TYPE_VARIANT`.
    pub fn set_packed_variant(&self, packed: i32) {
        self.packed_variant.store(packed, Ordering::Relaxed);
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::tropical_fish::DATA_ID_TYPE_VARIANT,
            VarInt(packed),
        );
    }

    /// Vanilla `TropicalFish.setPattern`.
    fn set_pattern(&self, pattern: i32) {
        let base = self.get_packed_variant();
        self.set_packed_variant(pack_variant(
            pattern,
            get_base_color(base),
            get_pattern_color(base),
        ));
    }

    /// Vanilla `TropicalFish.setBaseColor`.
    fn set_base_color(&self, base_color: DyeColor) {
        let base = self.get_packed_variant();
        self.set_packed_variant(pack_variant(
            get_pattern(base),
            base_color,
            get_pattern_color(base),
        ));
    }

    /// Vanilla `TropicalFish.setPatternColor`.
    fn set_pattern_color(&self, pattern_color: DyeColor) {
        let base = self.get_packed_variant();
        self.set_packed_variant(pack_variant(
            get_pattern(base),
            get_base_color(base),
            pattern_color,
        ));
    }

    /// Vanilla `TropicalFish.saveToBucketTag`: `AbstractFish`'s, then `copyFrom` of the
    /// pattern, base colour and pattern colour.
    fn save_to_bucket_tag(&self, bucket: &mut ItemStack) {
        crate::entity::bucketable::save_fish_to_bucket_tag(self, bucket);
        let packed = self.get_packed_variant();
        bucket.set_data_component(TropicalFishPatternImpl {
            value: Cow::Borrowed(pattern_name(get_pattern(packed))),
        });
        bucket.set_data_component(TropicalFishBaseColorImpl {
            value: Cow::Borrowed(get_base_color(packed).name()),
        });
        bucket.set_data_component(TropicalFishPatternColorImpl {
            value: Cow::Borrowed(get_pattern_color(packed).name()),
        });
    }
}

impl Mob for TropicalFishEntity {
    /// Vanilla `TropicalFish.finalizeSpawn`: `AbstractSchoolingFish`'s, then the group's
    /// variant, or on `level.getRandom()` one of the common variants (90%, which starts a
    /// `TropicalFishGroupData` led by this fish) or a random pattern and colours (the fish
    /// then schools alone).
    fn finalize_spawn(
        &self,
        world: &Arc<crate::world::World>,
        _reason: crate::entity::mob::spawn::EntitySpawnReason,
        group_data: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        use crate::entity::mob::spawn::SpawnGroupData;
        self.mob_entity.finalize_spawn_base();
        let mut group_data = schooling_fish::finalize_school_spawn(&self.school, group_data);
        let variant = if let SpawnGroupData::School {
            tropical_variant: Some(variant),
            ..
        } = group_data
        {
            variant
        } else {
            let mut random = world
                .random
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if f64::from(random.next_f32()) < 0.9 {
                let (pattern, base_color, pattern_color) =
                    COMMON_VARIANTS[random.next_bounded_i32(22) as usize];
                let variant = pack_variant(
                    pattern_from_name(pattern).unwrap_or(0),
                    base_color,
                    pattern_color,
                );
                group_data = SpawnGroupData::School {
                    leader: None,
                    tropical_variant: Some(variant),
                };
                variant
            } else {
                self.is_school.store(false, Ordering::Relaxed);
                let pattern = PATTERN_ORDER[random.next_bounded_i32(12) as usize];
                let base_color = dye_by_id(random.next_bounded_i32(16));
                let pattern_color = dye_by_id(random.next_bounded_i32(16));
                pack_variant(
                    pattern_from_name(pattern).unwrap_or(0),
                    base_color,
                    pattern_color,
                )
            }
        };
        self.set_packed_variant(variant);
        Some(group_data)
    }

    /// Vanilla `TropicalFish.isMaxGroupSizeReached`.
    fn is_max_group_size_reached(&self, _group_size: i32) -> bool {
        !self.is_school.load(Ordering::Relaxed)
    }

    /// Vanilla `TropicalFish.applyImplicitComponents`: the pattern, then the base colour,
    /// then the pattern colour.
    fn mob_apply_item_components(&self, item: &ItemStack) -> bool {
        let pattern = item.get_data_component::<TropicalFishPatternImpl>();
        if let Some(pattern) = pattern.and_then(|c| pattern_from_name(&c.value)) {
            self.set_pattern(pattern);
        }
        let base_color = item.get_data_component::<TropicalFishBaseColorImpl>();
        if let Some(color) = base_color.and_then(|c| DyeColor::by_name(&c.value)) {
            self.set_base_color(color);
        }
        let pattern_color = item.get_data_component::<TropicalFishPatternColorImpl>();
        if let Some(color) = pattern_color.and_then(|c| DyeColor::by_name(&c.value)) {
            self.set_pattern_color(color);
        }
        pattern.is_some() || base_color.is_some() || pattern_color.is_some()
    }

    fn mob_init_data_tracker(&self) {
        self.get_entity().set_synced_data(
            pumpkin_data::tracked_data::tropical_fish::DATA_ID_TYPE_VARIANT,
            VarInt(self.get_packed_variant()),
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
        // Vanilla `TropicalFish.addAdditionalSaveData`, through `Variant.CODEC`.
        nbt.put_int("Variant", normalize_variant(self.get_packed_variant()));
    }

    fn mob_read_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        self.from_bucket.read_nbt(self, nbt);
        // Vanilla `TropicalFish.readAdditionalSaveData`: missing is `DEFAULT_VARIANT`.
        self.set_packed_variant(
            nbt.get_int("Variant")
                .map_or(DEFAULT_VARIANT, normalize_variant),
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
            &pumpkin_data::item::Item::TROPICAL_FISH_BUCKET,
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
        fish::flop(self, Sound::EntityTropicalFishFlop);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_like_vanilla() {
        // Vanilla 26.3 saved these for betty lime/black and glitter red/cyan.
        assert_eq!(
            pack_variant(
                pattern_from_name("betty").unwrap(),
                DyeColor::Lime,
                DyeColor::Black
            ),
            251_986_945
        );
        assert_eq!(
            pack_variant(
                pattern_from_name("glitter").unwrap(),
                DyeColor::Red,
                DyeColor::Cyan
            ),
            151_912_961
        );
    }

    #[test]
    fn unknown_ids_normalize_to_defaults() {
        // An unknown pattern is KOB and an unknown colour WHITE (`ByIdMap` defaults).
        let packed = 7 | (20 << 16) | (5 << 24);
        assert_eq!(
            normalize_variant(packed),
            pack_variant(0, DyeColor::White, DyeColor::Lime)
        );
        assert_eq!(
            get_pattern(1 | (5 << 8)),
            pattern_from_name("clayfish").unwrap()
        );
    }
}
