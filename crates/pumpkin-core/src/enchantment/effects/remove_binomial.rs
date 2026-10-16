use pumpkin_data::enchantment::LevelBasedValue;

#[derive(Clone, Debug, PartialEq)]
pub struct RemoveBinomial {
    pub chance: LevelBasedValue,
}

impl RemoveBinomial {
    #[must_use]
    pub const fn new(chance: LevelBasedValue) -> Self {
        Self { chance }
    }

    /// Vanilla `RemoveBinomial.process`.
    #[must_use]
    pub fn process(&self, level: i32, current_value: f32) -> f32 {
        pumpkin_data::enchantment::EnchantmentValueEffect::RemoveBinomial(self.chance.clone())
            .process(level, current_value)
    }
}
