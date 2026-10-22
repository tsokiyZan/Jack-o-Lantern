use std::collections::HashMap;

use pumpkin_util::identifier::Identifier;
use pumpkin_util::random::RandomImpl;
use pumpkin_util::random::xoroshiro128::Xoroshiro;
use pumpkin_world::world_info::data_files::RandomSequencesData;

/// Vanilla `RandomSequence`: one named `XoroshiroRandomSource`.
pub struct RandomSequence {
    rng: Xoroshiro,
}

impl RandomSequence {
    /// Vanilla `RandomSequence.createSequence`: without a key the seed is only mixed.
    #[must_use]
    pub fn new(seed: u64, key: Option<&Identifier>) -> Self {
        let rng = key.map_or_else(
            || Xoroshiro::from_seed(seed),
            |key| Xoroshiro::from_seed_and_key(seed, &key.to_string()),
        );
        Self { rng }
    }

    pub const fn random(&mut self) -> &mut Xoroshiro {
        &mut self.rng
    }

    /// Vanilla `Mth.randomBetweenInclusive`.
    pub fn random_between_inclusive(&mut self, min: i32, max: i32) -> i32 {
        self.rng.next_bounded_i32(max - min + 1) + min
    }
}

/// Persistent/runtime manager for server random sequences.
pub struct RandomSequences {
    salt: i32,
    include_world_seed: bool,
    include_sequence_id: bool,
    sequences: HashMap<String, RandomSequence>,
}

impl Default for RandomSequences {
    fn default() -> Self {
        Self::new()
    }
}

impl RandomSequences {
    #[must_use]
    pub fn new() -> Self {
        Self {
            salt: 0,
            include_world_seed: true,
            include_sequence_id: true,
            sequences: HashMap::new(),
        }
    }

    /// Vanilla `RandomSequences.createSequence`.
    fn create_sequence(
        key: &Identifier,
        world_seed: i64,
        salt: i32,
        include_world_seed: bool,
        include_sequence_id: bool,
    ) -> RandomSequence {
        let base_seed = if include_world_seed { world_seed } else { 0 };
        let seed = (base_seed ^ i64::from(salt)) as u64;
        RandomSequence::new(seed, include_sequence_id.then_some(key))
    }

    pub fn get_or_create(&mut self, key: &Identifier, world_seed: i64) -> &mut RandomSequence {
        let (salt, include_world_seed, include_sequence_id) =
            (self.salt, self.include_world_seed, self.include_sequence_id);
        self.sequences.entry(key.to_string()).or_insert_with(|| {
            Self::create_sequence(
                key,
                world_seed,
                salt,
                include_world_seed,
                include_sequence_id,
            )
        })
    }

    /// Vanilla `RandomSequences.get`.
    pub fn get(&mut self, key: &Identifier, world_seed: i64) -> &mut Xoroshiro {
        self.get_or_create(key, world_seed).random()
    }

    pub fn reset(&mut self, key: &Identifier, world_seed: i64) {
        let sequence = Self::create_sequence(
            key,
            world_seed,
            self.salt,
            self.include_world_seed,
            self.include_sequence_id,
        );
        self.sequences.insert(key.to_string(), sequence);
    }

    pub fn reset_with_options(
        &mut self,
        key: &Identifier,
        world_seed: i64,
        salt: i32,
        include_world_seed: bool,
        include_sequence_id: bool,
    ) {
        let sequence = Self::create_sequence(
            key,
            world_seed,
            salt,
            include_world_seed,
            include_sequence_id,
        );
        self.sequences.insert(key.to_string(), sequence);
    }

    pub fn clear(&mut self) -> usize {
        let count = self.sequences.len();
        self.sequences.clear();
        count
    }

    pub const fn set_seed_defaults(
        &mut self,
        salt: i32,
        include_world_seed: bool,
        include_sequence_id: bool,
    ) {
        self.salt = salt;
        self.include_world_seed = include_world_seed;
        self.include_sequence_id = include_sequence_id;
    }

    #[must_use]
    pub fn get_sequence_keys(&self) -> Vec<String> {
        self.sequences.keys().cloned().collect()
    }
}

impl From<RandomSequencesData> for RandomSequences {
    fn from(data: RandomSequencesData) -> Self {
        Self {
            salt: data.salt,
            include_world_seed: data.include_world_seed,
            include_sequence_id: data.include_sequence_id,
            sequences: data
                .sequences
                .into_iter()
                .map(|(key, (lo, hi))| {
                    (
                        key,
                        RandomSequence {
                            rng: Xoroshiro::from_state(lo, hi),
                        },
                    )
                })
                .collect(),
        }
    }
}

impl From<&RandomSequences> for RandomSequencesData {
    fn from(sequences: &RandomSequences) -> Self {
        Self {
            salt: sequences.salt,
            include_world_seed: sequences.include_world_seed,
            include_sequence_id: sequences.include_sequence_id,
            sequences: sequences
                .sequences
                .iter()
                .map(|(key, sequence)| (key.clone(), sequence.rng.state()))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Identifier {
        Identifier::parse("minecraft:test").unwrap()
    }

    #[test]
    fn save_and_load_continue_the_stream() {
        let mut sequences = RandomSequences::new();
        sequences.get(&key(), 12345).next_i64();
        let data = RandomSequencesData::from(&sequences);
        let mut loaded = RandomSequences::from(data);
        assert_eq!(
            sequences.get(&key(), 12345).next_i64(),
            loaded.get(&key(), 999).next_i64()
        );
    }
}
