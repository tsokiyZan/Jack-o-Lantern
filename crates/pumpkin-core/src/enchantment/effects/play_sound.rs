use std::sync::Arc;

use pumpkin_data::sound::Sound;
use pumpkin_util::math::float_provider::FloatProvider;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::{RandomGenerator, get_seed, xoroshiro128::Xoroshiro};

use super::EnchantmentEntityEffectExt;
use crate::enchantment::EnchantedItemInUse;
use crate::entity::EntityBase;
use crate::world::World;

#[derive(Clone, Debug, PartialEq)]
pub struct PlaySound {
    pub sounds: &'static [Sound],
    pub volume: FloatProvider,
    pub pitch: FloatProvider,
}

impl PlaySound {
    #[must_use]
    pub const fn new(
        sounds: &'static [Sound],
        volume: FloatProvider,
        pitch: FloatProvider,
    ) -> Self {
        Self {
            sounds,
            volume,
            pitch,
        }
    }
}

impl EnchantmentEntityEffectExt for PlaySound {
    fn apply(
        &self,
        world: &Arc<World>,
        enchantment_level: i32,
        _item: &EnchantedItemInUse<'_>,
        entity: &dyn EntityBase,
        position: Vector3<f64>,
    ) {
        if entity.get_entity().is_silent() || self.sounds.is_empty() {
            return;
        }
        let index = (enchantment_level - 1).clamp(0, self.sounds.len() as i32 - 1) as usize;
        let mut random = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(get_seed()));
        let volume = self.volume.get(&mut random);
        let pitch = self.pitch.get(&mut random);
        world.play_sound_fine(
            self.sounds[index],
            entity.entity_sound_source(),
            &position,
            volume,
            pitch,
        );
    }
}
