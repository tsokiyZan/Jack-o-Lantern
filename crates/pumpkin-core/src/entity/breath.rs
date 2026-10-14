//! Air supply. The shared part is vanilla's `LivingEntity.baseTick`; mobs that
//! breathe differently out of water add their own `handleAirSupply` through
//! [`Mob::handle_air_supply`].

use crate::entity::EntityBase;
use crate::entity::living::LivingEntity;
use crate::entity::{Entity, mob::Mob};
use pumpkin_data::Block;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::tag;
use pumpkin_data::tag::Taggable;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::position::BlockPos;
use rand::RngExt;
use std::sync::atomic::{AtomicI32, Ordering};

/// Vanilla `Entity.getMaxAirSupply`.
pub const MAX_AIR: i32 = 300;
/// The step in vanilla `LivingEntity.increaseAirSupply`.
pub const AIR_RECOVERY_RATE: i32 = 4;
/// Vanilla `LivingEntity.shouldTakeDrowningDamage`: air at or below this hurts.
const DROWNING_AIR: i32 = -20;
const DROWNING_DAMAGE: f32 = 2.0;

/// An entity's air supply. Like vanilla's `DATA_AIR_SUPPLY_ID`, it goes negative
/// while drowning and is reset to zero each time drowning damage is dealt.
pub struct BreathManager {
    pub air_supply: AtomicI32,
}

impl Default for BreathManager {
    fn default() -> Self {
        Self {
            air_supply: AtomicI32::new(MAX_AIR),
        }
    }
}

impl BreathManager {
    /// Vanilla `Entity.getMaxAirSupply`.
    #[must_use]
    pub fn max_air_supply(caller: &dyn EntityBase) -> i32 {
        caller.get_mob().map_or(MAX_AIR, Mob::max_air_supply)
    }

    /// Vanilla `MobEffectUtil.hasWaterBreathing`.
    #[must_use]
    pub fn has_water_breathing(living: &LivingEntity) -> bool {
        living.has_effect(&StatusEffect::WATER_BREATHING)
            || living.has_effect(&StatusEffect::CONDUIT_POWER)
            || living.has_effect(&StatusEffect::BREATH_OF_THE_NAUTILUS)
    }

    /// Vanilla `MobEffectUtil.shouldEffectsRefillAirsupply`.
    fn should_effects_refill_air_supply(living: &LivingEntity) -> bool {
        !living.has_effect(&StatusEffect::BREATH_OF_THE_NAUTILUS)
            || living.has_effect(&StatusEffect::WATER_BREATHING)
            || living.has_effect(&StatusEffect::CONDUIT_POWER)
    }

    /// Vanilla `LivingEntity.canBreatheUnderwater`.
    fn can_breathe_underwater(caller: &dyn EntityBase) -> bool {
        caller.get_mob().map_or_else(
            || {
                caller
                    .get_entity()
                    .entity_type
                    .has_tag(&tag::EntityType::MINECRAFT_CAN_BREATHE_UNDER_WATER)
            },
            Mob::can_breathe_underwater,
        )
    }

    /// Vanilla `LivingEntity.decreaseAirSupply`: Respiration (the oxygen bonus
    /// attribute) gives a chance to keep the air this tick.
    fn decrease_air_supply(living: &LivingEntity, current: i32) -> i32 {
        let oxygen_bonus = living.get_attribute_value(&Attributes::OXYGEN_BONUS);
        if oxygen_bonus > 0.0 && rand::rng().random::<f64>() >= 1.0 / (oxygen_bonus + 1.0) {
            current
        } else {
            current - 1
        }
    }

    /// Vanilla `LivingEntity.increaseAirSupply`.
    fn increase_air_supply(caller: &dyn EntityBase, current: i32) -> i32 {
        caller.get_mob().map_or_else(
            || (current + AIR_RECOVERY_RATE).min(MAX_AIR),
            |mob| mob.increase_air_supply(current),
        )
    }

    #[must_use]
    pub fn air(&self) -> i32 {
        self.air_supply.load(Ordering::Relaxed)
    }

    /// The air part of vanilla `LivingEntity.baseTick`, then the mob's own
    /// `handleAirSupply`, which vanilla runs right after `baseTick` with the air
    /// from before it.
    pub fn tick(&self, living: &LivingEntity, caller: &dyn EntityBase) {
        let entity = &living.entity;
        if !entity.is_alive() {
            return;
        }
        let pre_tick_air = self.air();
        let max_air = Self::max_air_supply(caller);

        if Self::is_eye_in_water(entity) && !Self::is_eye_in_bubble_column(entity) {
            let invulnerable_player = caller.get_player().is_some_and(|player| {
                player
                    .abilities
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .invulnerable
            });
            let can_drown = !Self::can_breathe_underwater(caller)
                && !Self::has_water_breathing(living)
                && !invulnerable_player;
            if can_drown {
                self.set_air(caller, Self::decrease_air_supply(living, self.air()));
                if self.air() <= DROWNING_AIR {
                    self.set_air(caller, 0);
                    entity.world.load().send_entity_status(
                        entity,
                        EntityStatus::DrownParticles,
                        None,
                    );
                    // Vanilla `Player.isInvulnerableTo`: the game rule spares players only.
                    let spared = caller.get_player().is_some()
                        && !entity
                            .world
                            .load()
                            .level_info
                            .load()
                            .game_rules
                            .drowning_damage;
                    if !spared {
                        living.damage(caller, DROWNING_DAMAGE, DamageType::DROWN);
                    }
                }
            } else if self.air() < max_air && Self::should_effects_refill_air_supply(living) {
                self.set_air(caller, Self::increase_air_supply(caller, self.air()));
            }
        } else if self.air() < max_air {
            self.set_air(caller, Self::increase_air_supply(caller, self.air()));
        }

        if let Some(mob) = caller.get_mob() {
            mob.handle_air_supply(pre_tick_air);
        }
    }

    /// The body shared by vanilla's `WaterAnimal`, `AgeableWaterCreature`,
    /// `Axolotl` and `Nautilus` versions of `handleAirSupply`: out of its element
    /// the mob counts down from `pre_tick_air` and takes `damage_type` damage at
    /// the drowning threshold; back in it, the air is set to `refill_to`.
    pub fn handle_air_out_of_water(
        mob: &dyn Mob,
        pre_tick_air: i32,
        in_its_element: bool,
        refill_to: i32,
        damage_type: DamageType,
    ) {
        let living = &mob.get_mob_entity().living_entity;
        let breath = &living.breath;
        if living.entity.is_alive() && !in_its_element {
            breath.set_air(mob, pre_tick_air - 1);
            if breath.air() <= DROWNING_AIR {
                breath.set_air(mob, 0);
                living.damage(mob, DROWNING_DAMAGE, damage_type);
            }
        } else {
            breath.set_air(mob, refill_to);
        }
    }

    /// Vanilla `WaterAnimal.handleAirSupply`, also used by `AgeableWaterCreature`
    /// (squid): drown on land, refill to 300 in water.
    pub fn water_animal_handle_air_supply(mob: &dyn Mob, pre_tick_air: i32) {
        let in_water = mob.get_entity().is_in_water();
        Self::handle_air_out_of_water(mob, pre_tick_air, in_water, MAX_AIR, DamageType::DROWN);
    }

    /// Applies an air change through `EntityAirChangeEvent`.
    fn set_air(&self, caller: &dyn EntityBase, new_air: i32) {
        if new_air == self.air() {
            return;
        }
        let entity = caller.get_entity();
        let mut new_air = new_air;
        if let Some(server) = entity.world.load().server.upgrade() {
            let mut event =
                crate::plugin::api::events::entity::entity_air_change::EntityAirChangeEvent::new(
                    entity.entity_id,
                    new_air,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
            new_air = event.amount.min(Self::max_air_supply(caller));
        }
        self.air_supply.store(new_air, Ordering::Relaxed);
        self.send_air_supply(entity);
    }

    /// Sets the air supply, at most the entity's maximum, through
    /// `EntityAirChangeEvent`.
    pub fn set_air_supply(&self, caller: &dyn EntityBase, air: i32) {
        self.set_air(caller, air.min(Self::max_air_supply(caller)));
    }

    /// Writes vanilla's `Air` tag.
    pub fn write_nbt(&self, nbt: &mut NbtCompound) {
        nbt.put_short(
            "Air",
            self.air().clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
        );
    }

    /// Reads vanilla's `Air` tag, defaulting to `max_air` like `Entity.load`.
    pub fn read_nbt(&self, nbt: &NbtCompound, max_air: i32) {
        let air = nbt
            .get_short("Air")
            .map(i32::from)
            .or_else(|| nbt.get_int("Air"))
            .unwrap_or(max_air);
        self.air_supply.store(air.min(max_air), Ordering::Relaxed);
    }

    /// Vanilla `Entity.isEyeInFluid(FluidTags.WATER)`.
    #[must_use]
    pub fn is_eye_in_water(entity: &Entity) -> bool {
        let pos = entity.pos.load();
        let eye_y = entity.get_eye_y();

        let bp = BlockPos::new(
            pos.x.floor() as i32,
            eye_y.floor() as i32,
            pos.z.floor() as i32,
        );
        let world = entity.world.load();

        let (fluid, state) = world.get_fluid_and_fluid_state(&bp);

        let mut in_water_fluid = fluid.has_tag(&tag::Fluid::MINECRAFT_WATER);

        if !in_water_fluid {
            let state_here = world.get_block_state(&bp);
            if !state_here.is_solid() {
                let above = BlockPos::new(bp.0.x, bp.0.y + 1, bp.0.z);
                let fluid_above_x = world.get_fluid(&above);
                if fluid_above_x.has_tag(&tag::Fluid::MINECRAFT_WATER) {
                    in_water_fluid = true;
                }
            }
        }

        if !in_water_fluid {
            return false;
        }

        let above = BlockPos::new(bp.0.x, bp.0.y + 1, bp.0.z);
        let fluid_above = world.get_fluid(&above);

        let surface_y = if fluid_above.has_tag(&tag::Fluid::MINECRAFT_WATER) {
            f64::from(bp.0.y as f32 + 1.0)
        } else {
            let height: f32 = if state.is_still {
                1.0
            } else {
                let lvl = i32::from(state.level);
                if lvl >= 8 {
                    1.0
                } else {
                    ((8 - lvl).clamp(1, 8) as f32) / 8.0
                }
            };
            f64::from(bp.0.y as f32 + height)
        };

        surface_y > eye_y
    }

    /// The bubble column check in vanilla `LivingEntity.baseTick`.
    fn is_eye_in_bubble_column(entity: &Entity) -> bool {
        let pos = entity.pos.load();
        let bp = BlockPos::new(
            pos.x.floor() as i32,
            entity.get_eye_y().floor() as i32,
            pos.z.floor() as i32,
        );
        entity.world.load().get_block(&bp) == &Block::BUBBLE_COLUMN
    }

    /// Syncs the air supply to clients. Java clients get the raw value, which is
    /// negative while drowning, as in vanilla.
    pub fn send_air_supply(&self, entity: &Entity) {
        let air = self.air();

        let mut bedrock_meta =
            pumpkin_protocol::bedrock::client::set_actor_data::SyncedActorDataList::new();
        bedrock_meta.set(
            pumpkin_protocol::bedrock::client::set_actor_data::entity_data_key::AIR_SUPPLY,
            pumpkin_protocol::bedrock::client::set_actor_data::MetadataValue::Short(
                air.clamp(0, i32::from(i16::MAX)) as i16,
            ),
        );

        entity.set_synced_data(
            pumpkin_data::tracked_data::entity::DATA_AIR_SUPPLY_ID,
            VarInt(air),
        );
        entity.send_bedrock_actor_data(&bedrock_meta);
    }

    /// Fills the air to the maximum, as a freshly created entity has.
    pub fn reset(&self, caller: &dyn EntityBase) {
        self.air_supply
            .store(Self::max_air_supply(caller), Ordering::Relaxed);
        self.send_air_supply(caller.get_entity());
    }
}
