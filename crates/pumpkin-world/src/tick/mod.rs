use std::str::FromStr;

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::{
    math::position::BlockPos,
    resource_location::{FromResourceLocation, ResourceLocation, ToResourceLocation},
};

pub mod level_ticks;
pub mod scheduler;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd)]
#[repr(i32)]
pub enum TickPriority {
    ExtremelyHigh = -3,
    VeryHigh = -2,
    High = -1,
    Normal = 0,
    Low = 1,
    VeryLow = 2,
    ExtremelyLow = 3,
}

impl TickPriority {
    #[must_use]
    pub const fn values() -> [Self; 7] {
        [
            Self::ExtremelyHigh,
            Self::VeryHigh,
            Self::High,
            Self::Normal,
            Self::Low,
            Self::VeryLow,
            Self::ExtremelyLow,
        ]
    }

    /// Vanilla `TickPriority.byValue`: out-of-range values clamp to the nearest end.
    #[must_use]
    pub const fn by_value(value: i32) -> Self {
        match value {
            ..=-3 => Self::ExtremelyHigh,
            -2 => Self::VeryHigh,
            -1 => Self::High,
            0 => Self::Normal,
            1 => Self::Low,
            2 => Self::VeryLow,
            3.. => Self::ExtremelyLow,
        }
    }
}

/// Vanilla `ScheduledTick`: a tick due at an absolute game time.
#[derive(Clone, Copy, Debug)]
pub struct ScheduledTick<T> {
    pub value: T,
    pub position: BlockPos,
    pub trigger_tick: i64,
    pub priority: TickPriority,
    pub sub_tick_order: i64,
}

impl<T> ScheduledTick<T> {
    /// Vanilla `ScheduledTick.DRAIN_ORDER`.
    #[must_use]
    pub fn drain_order(&self, other: &Self) -> std::cmp::Ordering {
        self.trigger_tick
            .cmp(&other.trigger_tick)
            .then_with(|| self.intra_tick_drain_order(other))
    }

    /// Vanilla `ScheduledTick.INTRA_TICK_DRAIN_ORDER`.
    #[must_use]
    pub fn intra_tick_drain_order(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| self.sub_tick_order.cmp(&other.sub_tick_order))
    }
}

impl<T: Copy> ScheduledTick<T> {
    /// Vanilla `ScheduledTick.toSavedTick`.
    #[must_use]
    pub const fn to_saved_tick(&self, current_tick: i64) -> SavedTick<T> {
        SavedTick {
            value: self.value,
            position: self.position,
            delay: (self.trigger_tick - current_tick) as i32,
            priority: self.priority,
        }
    }
}

/// Vanilla `SavedTick`: a tick stored relative to the game time it was saved or generated at.
#[derive(Clone, Copy, Debug)]
pub struct SavedTick<T> {
    pub value: T,
    pub position: BlockPos,
    pub delay: i32,
    pub priority: TickPriority,
}

impl<T: Copy> SavedTick<T> {
    /// Vanilla `SavedTick.unpack`.
    #[must_use]
    pub fn unpack(&self, current_tick: i64, sub_tick_order: i64) -> ScheduledTick<T> {
        ScheduledTick {
            value: self.value,
            position: self.position,
            trigger_tick: current_tick + i64::from(self.delay),
            priority: self.priority,
            sub_tick_order,
        }
    }
}

impl<T> SavedTick<T>
where
    T: ToResourceLocation,
{
    #[must_use]
    pub fn to_nbt_compound(&self) -> NbtCompound {
        let mut nbt = NbtCompound::new();
        nbt.put_string("i", self.value.to_resource_location());
        nbt.put_int("x", self.position.0.x);
        nbt.put_int("y", self.position.0.y);
        nbt.put_int("z", self.position.0.z);
        nbt.put_int("t", self.delay);
        nbt.put_int("p", self.priority as i32);
        nbt
    }
}

impl<T> SavedTick<T>
where
    T: FromResourceLocation,
{
    #[must_use]
    pub fn from_nbt_compound(nbt: &NbtCompound) -> Option<Self> {
        let res_loc = ResourceLocation::from_str(nbt.get_string("i")?).ok()?;
        let value = T::from_resource_location(&res_loc)?;
        let x = nbt.get_int("x")?;
        let y = nbt.get_int("y")?;
        let z = nbt.get_int("z")?;
        let delay = nbt.get_int("t")?;
        let priority = TickPriority::by_value(nbt.get_int("p")?);

        Some(Self {
            value,
            position: BlockPos::new(x, y, z),
            delay,
            priority,
        })
    }
}
