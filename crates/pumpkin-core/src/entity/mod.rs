use crate::item::interaction_result::InteractionResult;
use crate::{
    command::argument_types::entity_anchor::{EntityAnchor, EntityAnchorExt},
    entity::{item::ItemEntity, projectile_deflection::ProjectileDeflectionType},
    net::{ClientPlatform, bedrock::BedrockClient, java::JavaClient},
    server::Server,
    world::{
        World,
        game_event::DynamicGameEventListener,
        portal::{NetherPortal, PortalProcessor, PortalType, SourcePortalInfo, TeleportTransition},
    },
};
use arc_swap::ArcSwap;
use bytes::BufMut;
use crossbeam::atomic::AtomicCell;
use inside_block_effect::InsideBlockEffectApplier;
use living::LivingEntity;
use player::{PassengerRemount, Player};
use pumpkin_data::BlockState;
use pumpkin_data::biome::Biome;
use pumpkin_data::block_state::PistonBehavior;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::dimension::Dimension;
use pumpkin_data::entity::EntityStatus;
use pumpkin_data::fluid::{Fluid, FluidState};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::tracked_data;
use pumpkin_data::{Block, BlockDirection};
use pumpkin_data::{
    block_properties::{Facing, HorizontalFacing},
    damage::DamageType,
    entity::{EntityPose, EntityType},
    sound::{Sound, SoundCategory},
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::bedrock::client::{CAddActor, CSetActorMotion};
use pumpkin_protocol::codec::var_long::VarLong;
use pumpkin_protocol::{
    PositionFlag,
    bedrock::client::set_actor_data::{
        CSetActorData, MetadataValue, PropertySyncData, SyncedActorDataList, entity_data_flag,
        entity_data_key,
    },
    codec::var_int::VarInt,
    codec::var_ulong::VarULong,
    java::client::play::{
        CEntityVelocity, CPlayerPosition, CRemoveMobEffect, CSetEntityMetadata, CSetPassengers,
        CSpawnEntity, CTeleportEntity, Metadata, MetadataSerializer,
    },
};
use pumpkin_util::Hand;
use pumpkin_util::math::quaternion::Quaternionf;
use pumpkin_util::math::vector3::Axis;
use pumpkin_util::math::{
    boundingbox::{AttachmentPoints, BoundingBox, EntityAttachments, EntityDimensions},
    get_section_cord, pack_degrees,
    position::{BlockPos, chunk_section_from_pos, for_each_block_intersected_between},
    vector2::Vector2,
    vector3::Vector3,
    wrap_degrees,
};
use pumpkin_util::random::RandomImpl;
use pumpkin_util::text::TextComponent;
use pumpkin_util::text::hover::HoverEvent;
use pumpkin_util::version::JavaMinecraftVersion;
use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{
        AtomicBool, AtomicI32, AtomicU8, AtomicU32,
        Ordering::{self, Relaxed},
    },
};
use uuid::Uuid;

pub mod ageable;
pub mod ai;
pub mod area_effect_cloud;
pub mod attributes;
pub mod boss;
pub mod breath;
pub mod bucketable;
pub mod custom_sound;
pub mod damage_source;
pub mod decoration;
pub mod dismount_helper;
pub mod effect;
pub mod entity_reference;
pub mod experience_orb;
pub mod falling;
pub mod hunger;
pub mod inside_block_effect;
pub mod interaction;
pub mod item;
pub mod item_steerable;
pub mod leash;
pub mod leashable;
pub mod lightning;
pub mod living;
pub mod marker;
pub mod mob;
pub mod movement_emission;
pub mod passive;
pub mod player;
pub mod projectile;
pub mod projectile_deflection;
pub mod simple_container;
pub mod spawn_util;
pub mod synched_entity_data;
pub mod tnt;
pub mod r#type;
pub mod vehicle;
pub mod waypoint;

pub use lightning::LightningBoltEntity;

pub(crate) mod combat;
pub mod predicate;

/// The maximum number of scoreboard tags an entity can carry, matching Vanilla.
pub const MAX_SCOREBOARD_TAGS: usize = 1024;

/// Returns the [`EntityStatus`] that should be broadcast when the given
/// equipment slot breaks.
#[must_use]
pub const fn equipment_break_status(slot: &EquipmentSlot) -> EntityStatus {
    match slot {
        EquipmentSlot::MainHand(_) => EntityStatus::MainhandBreak,
        EquipmentSlot::OffHand(_) => EntityStatus::OffhandBreak,
        EquipmentSlot::Head(_) => EntityStatus::HeadBreak,
        EquipmentSlot::Chest(_) => EntityStatus::ChestBreak,
        EquipmentSlot::Legs(_) => EntityStatus::LegsBreak,
        EquipmentSlot::Feet(_) => EntityStatus::FeetBreak,
        EquipmentSlot::Body(_) => EntityStatus::BodyBreak,
        EquipmentSlot::Saddle(_) => EntityStatus::SaddleBreak,
    }
}

impl dyn EntityBase + '_ {
    /// Inherent on the trait object so both sides can be `&dyn EntityBase`.
    #[must_use]
    pub fn is_allied_to(&self, other: &dyn EntityBase) -> bool {
        self.get_entity().entity_id == other.get_entity().entity_id
            || self.considers_entity_as_ally(other)
            || other.considers_entity_as_ally(self)
    }
}

/// Vanilla `Entity.doTeamsAllowDamage`.
pub fn do_teams_allow_damage(attacker: &dyn EntityBase, target: &dyn EntityBase) -> bool {
    let Some(team) = attacker.get_team() else {
        return true;
    };
    match target.get_team() {
        // `PlayerTeam` option bit 0 is friendly fire.
        Some(other) if other.name == team.name => team.options & 1 != 0,
        _ => true,
    }
}

/// Vanilla `Entity.push(Entity)`: nudges the two entities apart.
pub fn push_apart(self_entity: &Entity, self_pushable: bool, entity: &dyn EntityBase) {
    let other_entity = entity.get_entity();

    if self_entity.no_physics.load(Ordering::Relaxed)
        || other_entity.no_physics.load(Ordering::Relaxed)
    {
        return;
    }

    // `isPassengerOfSameVehicle`: riders of one vehicle, or a rider and its mount.
    if self_entity.root_vehicle_id() == other_entity.root_vehicle_id() {
        return;
    }

    let mut dx = other_entity.pos.load().x - self_entity.pos.load().x;
    let mut dz = other_entity.pos.load().z - self_entity.pos.load().z;
    let mut d = dx.abs().max(dz.abs());
    if d >= f64::from(0.01f32) {
        d = d.sqrt();
        dx /= d;
        dz /= d;
        let mut d2 = 1.0 / d;
        if d2 > 1.0 {
            d2 = 1.0;
        }
        dx *= d2;
        dz *= d2;
        dx *= f64::from(0.05f32);
        dz *= f64::from(0.05f32);

        if !self_entity.has_passengers() && self_pushable {
            let mut vel = self_entity.velocity.load();
            vel.x -= dx;
            vel.z -= dz;
            self_entity.velocity.store(vel);
            self_entity.velocity_dirty.store(true, Ordering::SeqCst);
        }

        if !other_entity.has_passengers() && entity.is_pushable() {
            let mut vel = other_entity.velocity.load();
            vel.x += dx;
            vel.z += dz;
            other_entity.velocity.store(vel);
            other_entity.velocity_dirty.store(true, Ordering::SeqCst);
        }
    }
}

pub trait EntityBase: Send + Sync + std::any::Any {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.get_entity().write_nbt(nbt);
        if let Some(living) = self.get_living_entity() {
            living.write_living_nbt(nbt);
        } else {
            // Vanilla `saveWithoutId` writes `Air` for every entity; only living ones use it.
            nbt.put_short("Air", breath::MAX_AIR as i16);
        }
        if let Some(projectile) = self.get_projectile() {
            projectile.write_nbt(nbt);
        }
        self.write_custom_nbt(nbt);
        self.get_entity().write_passengers_nbt(nbt);
    }

    /// Vanilla `NbtPredicate.getEntityTagToCompare`: the save without `id`, which `/data`
    /// and `nbt=` selectors read.
    /// Vanilla `NbtPredicate.getEntityTagToCompare`: `saveWithoutId`, and for a player the
    /// held stack as `SelectedItem`.
    fn tag_to_compare(&self) -> NbtCompound {
        let mut nbt = NbtCompound::new();
        self.write_nbt(&mut nbt);
        nbt.child_tags.remove("id");
        if let Some(player) = self.get_player() {
            let selected = player.inventory().held_item();
            if !selected.is_empty() {
                let mut item = NbtCompound::new();
                selected.write_item_stack(&mut item);
                nbt.put_compound("SelectedItem", item);
            }
        }
        nbt
    }

    fn write_custom_nbt(&self, _nbt: &mut NbtCompound) {}

    /// Reads base, living and custom tags.
    fn read_nbt_non_mut(&self, nbt: &NbtCompound) {
        self.get_entity().read_nbt_non_mut(nbt);
        self.read_nbt_before_living(nbt);
        if let Some(living) = self.get_living_entity() {
            living.read_living_nbt_non_mut(nbt);
            let max_air = self
                .get_mob()
                .map_or(breath::MAX_AIR, mob::Mob::max_air_supply);
            living.breath.read_nbt(nbt, max_air);
        }
        if let Some(projectile) = self.get_projectile() {
            projectile.read_nbt(nbt);
        }
        self.read_custom_nbt(nbt);
    }

    fn read_custom_nbt(&self, _nbt: &NbtCompound) {}

    /// The part of a vanilla `readAdditionalSaveData` override that runs before
    /// `super.readAdditionalSaveData`, so before the living tags.
    fn read_nbt_before_living(&self, _nbt: &NbtCompound) {}
    /// Called every tick for this entity.
    ///
    /// The `caller` parameter is a reference to the entity that initiated the tick.
    /// This can be the same entity the method is being called on (`self`),
    /// but in some scenarios (e.g., interactions or events), it might be a different entity.
    ///
    /// The `server` parameter provides access to the game server instance.
    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        if let Some(living) = self.get_living_entity() {
            living.tick(caller, server);
        } else {
            self.get_entity().tick(caller, server);
        }
    }

    fn get_job_site_pos(&self) -> Option<pumpkin_util::math::position::BlockPos> {
        None
    }

    fn get_home_pos(&self) -> Option<pumpkin_util::math::position::BlockPos> {
        None
    }

    fn as_any(&self) -> &dyn std::any::Any
    where
        Self: Sized,
    {
        self
    }

    fn get_item_steerable(&self) -> Option<&dyn crate::entity::item_steerable::ItemSteerable> {
        None
    }

    /// Vanilla `TraceableEntity.getOwner`, as a runtime entity id.
    fn get_owner_id(&self) -> Option<i32> {
        self.get_projectile()
            .and_then(|projectile| projectile.get_owner_id(self.get_entity()))
    }

    /// The vanilla `Projectile` state of a projectile entity.
    fn get_projectile(&self) -> Option<&projectile::Projectile> {
        None
    }

    fn get_eye_pos(&self) -> Vector3<f64> {
        self.get_entity().get_eye_pos()
    }

    fn get_looking_vector(&self) -> Vector3<f64> {
        let entity = self.get_entity();
        Vector3::from_yaw_pitch(entity.yaw.load(), entity.pitch.load())
    }

    /// Vanilla `DataComponentGetter.get` on an entity, for loot `components` predicates.
    fn get_component(
        &self,
        _component: pumpkin_data::data_component::DataComponent,
    ) -> Option<Box<dyn pumpkin_data::data_component_impl::DataComponentImpl>> {
        None
    }

    fn init_data_tracker(&self) {
        let entity = self.get_entity();

        // If the internal age is negative, it's a baby
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;

        if is_baby {
            let mut bedrock_meta = SyncedActorDataList::new();
            bedrock_meta.set_flag(entity_data_key::FLAGS, entity_data_flag::BABY as u8, true);
            entity.set_synced_data(tracked_data::ageable_mob::DATA_BABY_ID, true);
            entity.send_bedrock_actor_data(&bedrock_meta);
        }
    }
    fn set_variant_name(&self, _name: &str) {}
    fn set_sound_variant_name(&self, _name: &str) {}

    fn teleport(
        &self,
        position: Vector3<f64>,
        yaw: Option<f32>,
        pitch: Option<f32>,
        world: Arc<World>,
    ) {
        // Vanilla `Entity.teleport(TeleportTransition)`, which needs the shared entity.
        let entity = self.get_entity();
        if let Some(this) = entity.world.load().get_entity_by_id(entity.entity_id) {
            teleport_transition(
                &this,
                &TeleportTarget {
                    world,
                    position,
                    yaw,
                    pitch,
                    keep_velocity: false,
                    as_passenger: false,
                },
            );
        } else {
            entity.teleport(position, yaw, pitch, &world);
        }
    }

    fn is_pushed_by_fluids(&self) -> bool {
        true
    }

    /// Vanilla `Entity.getBlockExplosionResistance`, for an explosion this entity causes.
    fn get_block_explosion_resistance(
        &self,
        _world: &crate::world::World,
        _pos: &BlockPos,
        _block: &pumpkin_data::Block,
        resistance: f32,
    ) -> f32 {
        resistance
    }

    /// Vanilla `Entity.shouldBlockExplode`, for an explosion this entity causes.
    fn should_block_explode(
        &self,
        _world: &crate::world::World,
        _pos: &BlockPos,
        _block: &pumpkin_data::Block,
        _power: f32,
    ) -> bool {
        true
    }

    /// Vanilla `Entity.ignoreExplosion`: whether `explosion` neither hurts nor pushes this entity.
    fn is_immune_to_explosion(&self, _explosion: &crate::world::explosion::Explosion) -> bool {
        false
    }

    /// Vanilla `getDefaultGravity`; see [`Self::current_gravity`].
    fn get_gravity(&self) -> f64 {
        0.0
    }

    /// Vanilla `Entity.getGravity`: none while the entity has `NoGravity`.
    fn current_gravity(&self) -> f64 {
        if self.get_entity().has_no_gravity() {
            0.0
        } else {
            self.get_gravity()
        }
    }

    /// Bedrock network Y above the feet. Endstone `getBaseOffset`.
    fn bedrock_y_offset(&self) -> f64 {
        0.0
    }

    fn bedrock_pos(&self) -> Vector3<f64> {
        self.get_entity()
            .pos
            .load()
            .add_raw(0.0, self.bedrock_y_offset(), 0.0)
    }

    /// Vanilla `getDimensions(pose)`: a sleeping living entity is a 0.2 cube, a mob uses
    /// its own `default_dimensions`, and living entities scale with the `scale`
    /// attribute.
    fn dimensions_for_pose(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        let Some(living) = self.get_living_entity() else {
            return entity.get_dimensions(pose);
        };
        if pose == EntityPose::Sleeping {
            return entity.get_dimensions(pose);
        }
        let Some(mob) = self.get_mob() else {
            return entity.get_dimensions(pose).scale(living.get_scale());
        };
        let dimensions = mob.default_dimensions(pose);
        if mob.has_fixed_dimensions(pose) {
            dimensions
        } else {
            dimensions.scale(living.get_scale())
        }
    }

    /// Vanilla `Entity.refreshDimensions`, for when something the dimensions depend on
    /// changed: baby state, the `scale` attribute, a mob's size or state.
    fn refresh_dimensions(&self) {
        let entity = self.get_entity();
        entity.apply_dimensions(self.dimensions_for_pose(entity.pose.load()));
    }

    /// Vanilla `Entity.setPose`, which refreshes the dimensions for the new pose.
    fn set_pose(&self, pose: EntityPose) {
        self.get_entity()
            .set_pose_with_dimensions(pose, self.dimensions_for_pose(pose));
    }

    fn get_mob(&self) -> Option<&dyn mob::Mob> {
        None
    }

    /// Vanilla `Entity.deflection`: how `projectile` bounces off this entity instead of
    /// hitting it.
    fn deflection(&self, _projectile: &dyn EntityBase) -> ProjectileDeflectionType {
        ProjectileDeflectionType::of_entity_type(self.get_entity())
    }

    /// Players are tracked by profile name, every other entity by its UUID.
    fn get_scoreboard_name(&self) -> String {
        self.get_player().map_or_else(
            || self.get_entity().entity_uuid.to_string(),
            |player| player.gameprofile.name.clone(),
        )
    }

    fn get_team(&self) -> Option<crate::world::scoreboard::Team> {
        if let Some(player) = self.get_player() {
            return player.get_team();
        }
        let world = self.get_entity().world.load();
        let scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if scoreboard.get_teams().is_empty() {
            return None;
        }
        scoreboard
            .get_entity_team(&self.get_scoreboard_name())
            .cloned()
    }

    /// The team name alone, which is all the ally checks need. Worth having because
    /// they run once per candidate of every target search, and a `Team` is expensive
    /// to clone.
    fn get_team_name(&self) -> Option<String> {
        if let Some(player) = self.get_player() {
            return player.get_team_name();
        }
        let world = self.get_entity().world.load();
        let scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if scoreboard.get_teams().is_empty() {
            return None;
        }
        scoreboard
            .get_entity_team(&self.get_scoreboard_name())
            .map(|team| team.name.clone())
    }

    fn considers_entity_as_ally(&self, other: &dyn EntityBase) -> bool {
        if self
            .get_mob()
            .is_some_and(|mob| mob.mob_considers_entity_as_ally(other))
        {
            return true;
        }
        if let Some(tamable) = self.get_mob().and_then(mob::Mob::as_tamable)
            && let Some(considered) = tamable.tamable_considers_entity_as_ally(other)
        {
            return considered;
        }
        let Some(team) = self.get_team_name() else {
            return false;
        };
        other.get_team_name().is_some_and(|other| other == team)
    }

    fn tick_in_void(&self, _dyn_self: &dyn EntityBase) {
        self.get_entity().remove();
    }

    /// Returns if damage was successful or not
    fn damage(&self, caller: &dyn EntityBase, amount: f32, damage_type: DamageType) -> bool {
        caller.damage_with_context(caller, amount, damage_type, None, None, None)
    }

    fn on_lightning_strike(
        &self,
        caller: &dyn EntityBase,
        lightning: &lightning::LightningBoltEntity,
    ) {
        // Vanilla `Entity.thunderHit`: one more fire tick, and 8 seconds only when that lands
        // on exactly 0. The bolt's fire blocks do the rest.
        let entity = self.get_entity();
        caller.set_remaining_fire_ticks(entity.fire_ticks.load(Ordering::Relaxed) + 1);
        if entity.fire_ticks.load(Ordering::Relaxed) == 0 {
            caller.set_on_fire_for(8.0);
        }
        let cause = lightning.get_cause();
        self.damage_with_context(
            caller,
            5.0,
            DamageType::LIGHTNING_BOLT,
            None,
            Some(lightning),
            cause.as_deref().map(|p| p as &dyn EntityBase),
        );
    }

    fn is_spectator(&self) -> bool {
        false
    }

    /// Vanilla `isIgnoringBlockTriggers`: pressure plates and tripwires don't see the entity.
    fn is_ignoring_block_triggers(&self) -> bool {
        false
    }

    /// Vanilla `canBeCollidedWith`: whether other movement treats this entity as solid.
    fn is_collidable(&self, _entity: Option<&dyn EntityBase>) -> bool {
        false
    }

    /// Vanilla `Entity.canCollideWith`: `caller` (this entity) can't move through `other`.
    fn can_collide_with(&self, caller: &dyn EntityBase, other: &dyn EntityBase) -> bool {
        other.is_collidable(Some(caller))
            && caller.get_entity().root_vehicle_id() != other.get_entity().root_vehicle_id()
    }

    /// Vanilla `isPickable`: `false` for `Entity`, "not removed" for `LivingEntity`.
    fn can_hit(&self) -> bool {
        self.get_living_entity().is_some() && !self.get_entity().is_removed()
    }

    /// Vanilla `canBeHitByProjectile`: alive and pickable.
    fn can_be_hit_by_projectile(&self) -> bool {
        let alive = self
            .get_living_entity()
            .map_or_else(|| self.get_entity().is_alive(), LivingEntity::is_alive);
        alive && self.can_hit()
    }

    fn is_flutterer(&self) -> bool {
        false
    }

    /// Vanilla `Entity.isSteppingCarefully`: sneaking, so blocks stepped on stay calm.
    fn is_stepping_carefully(&self) -> bool {
        self.get_entity().is_sneaking()
    }

    /// Vanilla `Entity.dampensVibrations`.
    fn dampens_vibrations(&self) -> bool {
        false
    }

    /// Vanilla `Entity.getSoundSource`.
    fn entity_sound_source(&self) -> SoundCategory {
        SoundCategory::Neutral
    }

    /// Vanilla `Entity.playSound(sound, volume, pitch)`: at this entity, unless it is silent.
    fn play_sound_at_self(&self, sound: Sound, volume: f32, pitch: f32) {
        let entity = self.get_entity();
        if entity.is_silent() {
            return;
        }
        entity.world.load().play_sound_fine(
            sound,
            self.entity_sound_source(),
            &entity.pos.load(),
            volume,
            pitch,
        );
    }

    /// Vanilla `Entity.getControllingPassenger`.
    fn controlling_passenger(&self) -> Option<Arc<dyn EntityBase>> {
        None
    }

    /// Vanilla `maxUpStep`: 0 for a plain entity, `LivingEntity`'s step height otherwise.
    fn max_up_step(&self) -> f32 {
        self.get_living_entity().map_or(0.0, |living| {
            living_max_up_step(living, self.controlling_passenger())
        })
    }

    /// Vanilla `Entity.getVehicleAttachmentPoint`: the point of this entity that sits on
    /// `vehicle`'s passenger point.
    fn vehicle_attachment_point(&self, _vehicle: &dyn EntityBase) -> Vector3<f64> {
        let entity = self.get_entity();
        entity
            .entity_dimension
            .load()
            .attachments
            .vehicle(entity.yaw.load())
    }

    /// Vanilla `Entity.getPassengerAttachmentPoint`.
    fn passenger_attachment_point(
        &self,
        passenger: &dyn EntityBase,
        dimensions: EntityDimensions,
        _scale: f32,
    ) -> Vector3<f64> {
        self.get_entity()
            .default_passenger_attachment_point(passenger, &dimensions.attachments)
    }

    /// Vanilla `Entity.positionRider(passenger, Entity::setPos)`; `caller` is this entity.
    fn position_rider(&self, caller: &dyn EntityBase, passenger: &dyn EntityBase) {
        set_rider_position(caller, passenger);
    }

    /// Vanilla `Entity.isClientAuthoritative`: a client moves this entity, because it is a
    /// player or a player steers it.
    fn is_client_authoritative(&self) -> bool {
        self.controlling_passenger()
            .is_some_and(|passenger| passenger.is_client_authoritative())
    }

    /// Vanilla `Entity.onAboveBubbleColumn`.
    fn on_above_bubble_column(&self, caller: &dyn EntityBase, drag_down: bool, pos: BlockPos) {
        Entity::handle_on_above_bubble_column(caller, drag_down, pos);
    }

    /// Vanilla `Entity.onInsideBubbleColumn`.
    fn on_inside_bubble_column(&self, caller: &dyn EntityBase, drag_down: bool) {
        Entity::handle_on_inside_bubble_column(caller, drag_down);
    }

    /// Vanilla `Entity.canSimulateMovement` on the server: a vehicle a player steers is only
    /// moved by that player's client.
    fn can_simulate_movement(&self) -> bool {
        !self.is_client_authoritative()
    }

    /// Vanilla `Entity.getMovementEmission`.
    fn movement_emission(&self) -> movement_emission::MovementEmission {
        movement_emission::MovementEmission::All
    }

    /// Vanilla `Entity.playStepSound`.
    fn play_step_sound(
        &self,
        caller: &dyn EntityBase,
        _pos: &BlockPos,
        _block: &'static Block,
        state: &'static BlockState,
    ) {
        self.get_entity().play_step_sound_default(caller, state);
    }

    /// Vanilla `Entity.nextStep`.
    fn next_step(&self) -> f32 {
        (self.get_entity().move_dist.load() as i32 + 1) as f32
    }

    /// Vanilla `Entity.waterSwimSound`.
    fn water_swim_sound(&self, caller: &dyn EntityBase) {
        self.get_entity().water_swim_sound_default(caller);
    }

    /// Vanilla `Entity.playSwimSound`.
    fn play_swim_sound(&self, caller: &dyn EntityBase, volume: f32) {
        self.get_entity().play_swim_sound_default(caller, volume);
    }

    /// Vanilla `Entity.getSwimSound`.
    fn swim_sound(&self) -> Sound {
        Sound::EntityGenericSwim
    }

    /// Vanilla `Entity.getSwimSplashSound`.
    fn swim_splash_sound(&self) -> Sound {
        Sound::EntityGenericSplash
    }

    /// Vanilla `Entity.getSwimHighSpeedSplashSound`.
    fn swim_high_speed_splash_sound(&self) -> Sound {
        Sound::EntityGenericSplash
    }

    /// Vanilla `Entity.isFlapping`.
    fn is_flapping(&self) -> bool {
        false
    }

    /// Vanilla `Entity.onFlap`.
    fn on_flap(&self) {}

    fn set_sprinting(&self, is_sprinting: bool) {
        if let Some(living) = self.get_living_entity() {
            living.set_sprinting(is_sprinting);
        } else {
            self.get_entity().set_sprinting(is_sprinting);
        }
    }

    fn get_block_speed_factor(&self) -> f32 {
        self.get_living_entity().map_or_else(
            || self.get_entity().get_block_speed_factor(),
            LivingEntity::get_block_speed_factor,
        )
    }

    /// Custom Y-axis velocity drag multiplier applied during `travel_in_air`.
    /// Bats return `Some(0.6)` to match vanilla's `travel()` override.
    fn get_y_velocity_drag(&self) -> Option<f64> {
        None
    }

    fn java_spawn_metadata(&self, version: JavaMinecraftVersion) -> Option<Box<[u8]>> {
        if version < JavaMinecraftVersion::V_1_9 {
            let entity = self.get_entity();
            let shared_flags = entity.flags.load(Ordering::Relaxed);
            return (shared_flags != 0).then(|| {
                // (0 << 5) | 0 = 0 (type: byte, index: 0 flags), value, 127 (terminator)
                Box::<[u8]>::from([0x00u8, shared_flags as u8, 127u8])
            });
        }
        self.get_mob().map_or_else(
            || {
                let entity = self.get_entity();
                let shared_flags = entity.flags.load(Ordering::Relaxed);
                (shared_flags != 0).then(|| {
                    let mut buf = Vec::new();
                    let _ = Metadata::new(
                        pumpkin_data::tracked_data::entity::DATA_SHARED_FLAGS_ID,
                        shared_flags,
                    )
                    .write(&mut buf, &version);
                    buf.put_u8(255);
                    buf.into_boxed_slice()
                })
            },
            |mob| mob.mob_java_spawn_metadata(version),
        )
    }

    fn send_bedrock_spawn_packet(&self, client: &BedrockClient) {
        let entity = self.get_entity();
        let runtime_id = entity.entity_id as u64;
        let identifier = self
            .get_mob()
            .and_then(mob::Mob::mob_bedrock_identifier)
            .unwrap_or(entity.entity_type.resource_name);
        // TODO: non-mob spawn metadata hook like `mob_bedrock_spawn_metadata` (falling block, TNT).
        let mut metadata = entity.bedrock_metadata();
        if let Some(mob) = self.get_mob()
            && let Some(mob_metadata) = mob.mob_bedrock_spawn_metadata()
        {
            metadata.0.extend(mob_metadata.0);
        }
        let packet = CAddActor {
            target_actor_id: VarLong(runtime_id as i64),
            target_runtime_id: VarULong(runtime_id),
            actor_type: identifier.to_string(),
            position: self.bedrock_pos().to_f32_lossy(),
            velocity: entity.velocity.load().to_f32_lossy(),
            rotation: Vector2::new(entity.pitch.load(), entity.yaw.load()),
            y_head_rotation: entity.head_yaw.load(),
            y_body_rotation: entity.body_yaw.load(),
            attributes_list: Vec::new(),
            actor_data: metadata,
            // TODO: Bedrock `climate_variant` property for pig/cow/chicken (`TemperatureVariantAnimal`)
            // empty -> client picks its own variant.
            synced_properties: PropertySyncData {
                int_entries_list: std::collections::HashMap::new(),
                float_entries_list: std::collections::HashMap::new(),
            },
            actor_links: Vec::new(),
        };
        if let Ok(data) = client.serialize_packet(&packet) {
            client.try_enqueue_packet(data);
        }
    }

    fn send_java_spawn_packet(&self, client: &JavaClient) {
        let entity = self.get_entity();
        let metadata = self.java_spawn_metadata(CURRENT_MC_VERSION);
        let spawn_packet = entity.create_spawn_packet();
        if let Ok(data) = client.serialize_packet(&spawn_packet) {
            client.try_enqueue_packet(data);
        }
        if let Some(meta) = metadata {
            let meta_packet = CSetEntityMetadata::new(entity.entity_id.into(), meta);
            if let Ok(meta_data) = client.serialize_packet(&meta_packet) {
                client.try_enqueue_packet(meta_data);
            }
        }
    }

    fn damage_with_context(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if let Some(living) = caller.get_living_entity() {
            return living.damage_with_context(
                caller,
                amount,
                damage_type,
                position,
                source,
                cause,
            );
        }
        false
    }

    /// Vanilla `Entity.interact` with the clicked `location` (relative to the entity), for
    /// the entities that care where they were clicked (the armor stand). Others ignore it.
    fn interact_at(
        &self,
        player: &Arc<Player>,
        hand: Hand,
        _location: Vector3<f64>,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        self.interact(player, hand, item_stack)
    }

    /// Vanilla `Entity.interact`: `player` right-clicks this entity with `item_stack`, the
    /// stack in `hand`.
    fn interact(
        &self,
        player: &Arc<Player>,
        hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        leash::entity_interact_by_id(self.get_entity(), player, hand, item_stack)
    }

    /// Vanilla `instanceof Leashable`: mobs (through the blanket `Mob` impl) and boats.
    fn is_leashable(&self) -> bool {
        false
    }

    /// Vanilla `Leashable.leashSnapDistance`.
    fn leash_snap_distance(&self) -> f64 {
        leashable::LEASH_TOO_FAR_DIST
    }

    /// Vanilla `Leashable.leashElasticDistance`.
    fn leash_elastic_distance(&self) -> f64 {
        leashable::LEASH_ELASTIC_DIST
    }

    /// Vanilla `Leashable.supportQuadLeash`.
    fn support_quad_leash(&self) -> bool {
        false
    }

    /// Vanilla `Entity.supportQuadLeashAsHolder`.
    fn support_quad_leash_as_holder(&self) -> bool {
        false
    }

    /// Vanilla `Leashable.whenLeashedTo`; `caller` is this entity.
    fn when_leashed_to(&self, caller: &dyn EntityBase, holder: &dyn EntityBase) {
        holder.notify_leash_holder(caller);
    }

    /// Vanilla `Leashable.leashTooFarBehaviour`: `dropLeash()`.
    fn leash_too_far_behaviour(&self, caller: &dyn EntityBase) {
        leashable::drop_leash(caller, true, true);
    }

    /// Vanilla `Leashable.closeRangeLeashBehaviour`.
    fn close_range_leash_behaviour(&self, _caller: &dyn EntityBase, _holder: &dyn EntityBase) {}

    /// Vanilla `Leashable.onElasticLeashPull`.
    fn on_elastic_leash_pull(&self, _caller: &dyn EntityBase) {
        self.get_entity().check_fall_distance_accumulation();
    }

    /// Vanilla `Entity.notifyLeashHolder`: `leashable` is leashed to this entity this tick.
    fn notify_leash_holder(&self, _leashable: &dyn EntityBase) {}

    /// Vanilla `Entity.notifyLeasheeRemoved`: `leashable`, held by this entity (`caller`),
    /// let go of it.
    fn notify_leashee_removed(&self, _caller: &dyn EntityBase, _leashable: &dyn EntityBase) {}

    /// Vanilla `Leashable.onLeashRemoved`.
    fn on_leash_removed(&self) {}

    /// A living entity's own vanilla `pushEntities` in place of `LivingEntity`'s, run when it
    /// returns `true` (the armor stand's).
    fn push_entities_override(&self, _caller: &dyn EntityBase) -> bool {
        false
    }

    /// Vanilla `Entity.isAttackable`: whether a player's attack reaches this entity at all.
    fn is_attackable(&self) -> bool {
        true
    }

    /// Vanilla `Entity.skipAttackInteraction`: whether this entity handles `source`'s attack
    /// itself, so that `Player.attack` does nothing more.
    fn skip_attack_interaction(&self, _source: &dyn EntityBase) -> bool {
        false
    }

    /// Vanilla `Projectile.deflect(AIM_DEFLECT, deflector, deflector, byAttack = true, power)`
    /// for a redirectable projectile attacked by `deflector`: whether it was deflected.
    /// Only the `redirectable_projectile` types implement it.
    fn deflect_by_attack(&self, _deflector: &dyn EntityBase, _power: f64) -> bool {
        false
    }

    /// Vanilla `Entity.igniteForSeconds`.
    fn set_on_fire_for(&self, seconds: f32) {
        self.set_on_fire_for_ticks((seconds * 20.0).floor() as u32);
    }

    /// Vanilla `Entity.igniteForTicks`, with `LivingEntity`'s `burning_time` scaling.
    fn set_on_fire_for_ticks(&self, ticks: u32) {
        let entity = self.get_entity();
        // `LivingEntity.igniteForTicks`: scaled by the `burning_time` attribute.
        let ticks = self.get_living_entity().map_or(ticks, |living| {
            (f64::from(ticks)
                * living.get_attribute_value(&pumpkin_data::attributes::Attributes::BURNING_TIME))
            .ceil() as u32
        });
        let mut event = crate::plugin::api::events::entity::entity_combust::EntityCombustEvent::new(
            entity.entity_id,
            ticks as f32 / 20.0,
        );
        if let Some(server) = entity.world.load().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
        }
        let ticks = ticks as i32;
        if entity.fire_ticks.load(Ordering::Relaxed) < ticks {
            self.set_remaining_fire_ticks(ticks);
        }
        entity.clear_freeze();
    }

    /// Vanilla `Entity.setRemainingFireTicks`.
    fn set_remaining_fire_ticks(&self, ticks: i32) {
        self.get_entity().fire_ticks.store(ticks, Ordering::Relaxed);
    }

    /// Vanilla `Entity.getFireImmuneTicks`: the fire ticks are reset to minus this once the
    /// entity stops burning.
    fn fire_immune_ticks(&self) -> i32 {
        0
    }

    /// Vanilla `Entity.playEntityOnFireExtinguishedSound`.
    fn play_entity_on_fire_extinguished_sound(&self) {
        let entity = self.get_entity();
        let pitch = {
            let mut random = entity.random();
            1.6 + (random.next_f32() - random.next_f32()) * 0.4
        };
        entity.world.load().play_sound_fine(
            Sound::EntityGenericExtinguishFire,
            self.entity_sound_source(),
            &entity.pos.load(),
            0.7,
            pitch,
        );
    }

    /// Vanilla `Entity.modifyPassengerFluidInteractionBox`: `None` keeps the passenger out of
    /// every fluid.
    fn modify_passenger_fluid_interaction_box(
        &self,
        passenger_box: BoundingBox,
    ) -> Option<BoundingBox> {
        Some(passenger_box)
    }

    /// Vanilla `Entity.isOnRails`.
    fn is_on_rails(&self) -> bool {
        false
    }

    /// Vanilla `Entity.shouldPlayLavaHurtSound`.
    fn should_play_lava_hurt_sound(&self) -> bool {
        true
    }

    /// Called when a player collides with an entity
    fn on_player_collision(&self, _player: &Arc<Player>) {}

    /// The part of a vanilla `addPassenger` override before `super.addPassenger`.
    fn before_add_passenger(&self) {}

    /// The part of a vanilla `addPassenger` override after `super.addPassenger`.
    fn after_add_passenger(&self) {}

    /// The part of a vanilla `removePassenger` override after `super.removePassenger`.
    fn after_remove_passenger(&self) {}

    fn is_passenger(&self) -> bool {
        self.get_entity().has_vehicle()
    }

    fn is_vehicle(&self) -> bool {
        self.get_entity().has_passengers()
    }

    /// The forward direction this vehicle places dismounting passengers by, where it isn't its
    /// facing (`AbstractMinecart.getDismountLocationForPassenger` uses `getMotionDirection`).
    fn dismount_forward(&self) -> Option<BlockDirection> {
        None
    }

    fn has_passenger(&self, other: &dyn EntityBase) -> bool {
        self.get_entity()
            .has_passenger(other.get_entity().entity_id)
    }

    fn move_entity(&self, caller: &dyn EntityBase, motion: Vector3<f64>) {
        self.get_entity().move_entity(caller, motion);
    }

    /// Vanilla `Entity.setPos`, with its overrides (`BlockAttachedEntity` re-derives its
    /// block and snaps to it). The inherent [`Entity::set_pos`] is the plain base version.
    fn set_position(&self, position: Vector3<f64>) {
        self.get_entity().set_pos(position);
    }

    /// Vanilla `PistonMovingBlockEntity.moveEntityByPiston`'s `entity.move(MoverType.PISTON, ..)`
    /// and what follows it, for the entities whose `move` override takes the piston too.
    fn move_by_piston(&self, caller: &dyn EntityBase, delta: Vector3<f64>) {
        self.get_entity().move_by_piston(caller, delta);
    }

    /// Vanilla `Entity.canUsePortal`.
    fn can_use_portal(&self, ignore_passenger: bool) -> bool {
        let entity = self.get_entity();
        (ignore_passenger || !entity.has_vehicle()) && entity.is_alive()
    }

    /// Vanilla `Entity.getDimensionChangingDelay`, with `Projectile`'s override (2).
    fn dimension_changing_delay(&self) -> u32 {
        if self.get_projectile().is_some() {
            return 2;
        }
        match self.get_entity().get_first_passenger() {
            Some(first) if first.get_player().is_some() => first.dimension_changing_delay(),
            _ => 300,
        }
    }

    /// Vanilla `Entity.canTeleport`: leaving the End, a player riding along must have seen
    /// the credits.
    fn can_teleport(&self, from: &World, to: &World) -> bool {
        self.get_entity().can_teleport_base(from, to)
    }

    /// Vanilla `Entity.getPistonPushReaction`: how a moving piston block treats this entity.
    /// The mapping onto [`PistonBehavior`] is `Normal` = `PUSH_PULL`, `PushOnly` = `PUSH`,
    /// `Destroy` = `POPPED`, `Block` = `IMMOVEABLE`, `Ignore` = `IGNORE_ENTITY`.
    fn piston_push_reaction(&self) -> PistonBehavior {
        PistonBehavior::Normal
    }

    /// Vanilla `Entity.getKnownMovement`: a player steering this entity knows its movement best.
    fn get_known_movement(&self) -> Vector3<f64> {
        let entity = self.get_entity();
        if entity.is_alive()
            && let Some(controller) = self.controlling_passenger()
            && controller.get_player().is_some()
        {
            return controller.get_known_movement();
        }
        entity.velocity.load()
    }

    /// Vanilla `Entity.checkFallDamage` and its overrides: adds the vertical move `ya` to the
    /// fall, and on landing hands the fall to the block below (`pos`, `state`).
    fn check_fall_damage(
        &self,
        caller: &dyn EntityBase,
        ya: f64,
        on_ground: bool,
        state: &'static BlockState,
        pos: BlockPos,
    ) {
        if let Some(living) = self.get_living_entity() {
            living.check_fall_damage(caller, ya, on_ground, state, pos);
        } else {
            self.get_entity()
                .check_fall_damage_default(caller, ya, on_ground, state, pos);
        }
    }

    /// Vanilla `Entity.causeFallDamage` and its overrides: whether the fall hurt this entity.
    fn cause_fall_damage(
        &self,
        caller: &dyn EntityBase,
        fall_distance: f64,
        damage_modifier: f32,
        damage_type: DamageType,
    ) -> bool {
        self.get_living_entity().map_or_else(
            || {
                self.get_entity().cause_fall_damage_default(
                    fall_distance,
                    damage_modifier,
                    damage_type,
                )
            },
            |living| living.cause_fall_damage(caller, fall_distance, damage_modifier, damage_type),
        )
    }

    fn is_pushable(&self) -> bool {
        false
    }

    fn push(&self, entity: &dyn EntityBase) {
        push_apart(self.get_entity(), self.is_pushable(), entity);
    }

    /// Vanilla `Entity.push(Vec3)` / `push(x, y, z)`, which subclasses can override.
    fn push_impulse(&self, impulse: Vector3<f64>) {
        self.get_entity().push_impulse(impulse);
    }

    #[allow(clippy::too_many_lines)]
    fn push_entities(&self, dyn_self: &dyn EntityBase) -> bool {
        let mut picked_up = false;
        let mut pushed = false;
        let self_entity = self.get_entity();
        let entity_bb = self_entity.bounding_box.load();

        if !self.is_pushable() {
            return false;
        }

        let world = self_entity.world.load();

        let is_rideable_minecart = self_entity.entity_type.id == EntityType::MINECART.id;
        let is_abstract_minecart = is_rideable_minecart
            || self_entity.entity_type.id == EntityType::CHEST_MINECART.id
            || self_entity.entity_type.id == EntityType::COMMAND_BLOCK_MINECART.id
            || self_entity.entity_type.id == EntityType::FURNACE_MINECART.id
            || self_entity.entity_type.id == EntityType::HOPPER_MINECART.id
            || self_entity.entity_type.id == EntityType::SPAWNER_MINECART.id
            || self_entity.entity_type.id == EntityType::TNT_MINECART.id;

        let is_minecart_fn = |id| -> bool {
            id == EntityType::MINECART.id
                || id == EntityType::CHEST_MINECART.id
                || id == EntityType::COMMAND_BLOCK_MINECART.id
                || id == EntityType::FURNACE_MINECART.id
                || id == EntityType::HOPPER_MINECART.id
                || id == EntityType::SPAWNER_MINECART.id
                || id == EntityType::TNT_MINECART.id
        };

        if is_abstract_minecart {
            let is_vehicle = self.is_vehicle();

            if is_rideable_minecart && !is_vehicle {
                let pickup_bb = entity_bb.expand(0.2, 0.0, 0.2);
                let other_entities = world.get_entities_at_box(&pickup_bb);

                for other in other_entities {
                    if other.get_entity().entity_id != self_entity.entity_id {
                        let other_type = other.get_entity().entity_type.id;
                        let is_iron_golem = other_type == EntityType::IRON_GOLEM.id;
                        let is_other_minecart = is_minecart_fn(other_type);

                        if !is_iron_golem
                            && !is_other_minecart
                            && !other.is_passenger()
                            && other.is_pushable()
                            && other.get_entity().riding_cooldown.load(Relaxed) == 0
                            && let Some(self_arc) = world.get_entity_by_id(self_entity.entity_id)
                        {
                            self_entity.add_passenger(self_arc, other.clone());
                            picked_up = true;
                            break;
                        }
                    }
                }
            }

            let push_bb = entity_bb.expand(1.0e-7, 1.0e-7, 1.0e-7);

            let other_entities = world.get_entities_at_box(&push_bb);
            for other in other_entities {
                if other.get_entity().entity_id != self_entity.entity_id {
                    let other_type = other.get_entity().entity_type.id;
                    let is_other_minecart = is_minecart_fn(other_type);
                    let is_iron_golem = other_type == EntityType::IRON_GOLEM.id;

                    if is_rideable_minecart {
                        if (is_iron_golem
                            || is_other_minecart
                            || is_vehicle
                            || !other.get_entity().has_vehicle())
                            && other.is_pushable()
                        {
                            dyn_self.push(other.as_ref());
                            pushed = true;
                        }
                    } else if !self.has_passenger(other.as_ref())
                        && other.is_pushable()
                        && is_other_minecart
                    {
                        dyn_self.push(other.as_ref());
                        pushed = true;
                    }
                }
            }

            let players = world.get_players_at_box(&push_bb);
            for player in players {
                if player.get_entity().entity_id != self_entity.entity_id && is_rideable_minecart {
                    dyn_self.push(player.as_ref());
                    pushed = true;
                    // Non-rideable minecarts (hoppers, chests) do not push players in vanilla.
                }
            }
        } else {
            let other_entities = world.get_entities_at_box(&entity_bb);
            for other in other_entities {
                if other.get_entity().entity_id != self_entity.entity_id {
                    dyn_self.push(other.as_ref());
                    pushed = true;
                }
            }

            let players = world.get_players_at_box(&entity_bb);
            for player in players {
                if player.get_entity().entity_id != self_entity.entity_id {
                    dyn_self.push(player.as_ref());
                    pushed = true;
                }
            }
        }

        picked_up && !pushed
    }

    fn on_hit(&self, _hit: crate::entity::projectile::ProjectileHit) {}

    fn set_paddle_state(&self, _left: bool, _right: bool) {}

    fn is_in_love(&self) -> bool {
        false
    }

    fn is_breeding_ready(&self) -> bool {
        false
    }

    fn reset_love(&self) {}

    fn set_breeding_cooldown(&self, _ticks: i32) {}

    fn is_panicking(&self) -> bool {
        false
    }

    fn get_entity(&self) -> &Entity;

    fn get_living_entity(&self) -> Option<&LivingEntity>;

    fn cast_any(&self) -> &dyn std::any::Any;

    /// The mob side of this entity, for the behaviors that read another mob's brain.
    fn as_mob_entity(&self) -> Option<&crate::entity::mob::MobEntity> {
        None
    }

    fn get_item_entity(&self) -> Option<&ItemEntity> {
        None
    }

    fn get_player(&self) -> Option<&Player> {
        None
    }

    /// Vanilla `Entity.getWeaponItem`: the item this entity attacks with, if any.
    fn get_weapon_item(&self) -> Option<ItemStack> {
        None
    }

    /// Should return the name of the entity without click or hover events.
    fn get_name(&self) -> TextComponent {
        let entity = self.get_entity();
        entity
            .custom_name
            .load()
            .as_ref()
            .clone()
            .unwrap_or(TextComponent::translate_cross(
                format!("entity.minecraft.{}", entity.entity_type.resource_name),
                format!("entity.minecraft.{}", entity.entity_type.resource_name),
                [],
            ))
    }

    fn get_display_name(&self) -> TextComponent {
        // TODO: team color
        let entity = self.get_entity();
        let mut name =
            entity
                .custom_name
                .load()
                .as_ref()
                .clone()
                .unwrap_or(TextComponent::translate_cross(
                    format!("entity.minecraft.{}", entity.entity_type.resource_name),
                    format!("entity.minecraft.{}", entity.entity_type.resource_name),
                    [],
                ));
        let name_clone = name.clone();
        name = name.hover_event(HoverEvent::show_entity(
            entity.entity_uuid.to_string(),
            entity.entity_type.resource_name.into(),
            Some(name_clone),
        ));
        name = name.insertion(entity.entity_uuid.to_string());
        name
    }

    /// Kills the Entity.
    /// Vanilla `Entity.kill`, and `LivingEntity.kill`'s generic kill damage.
    fn kill(&self, caller: &dyn EntityBase) {
        if self.get_living_entity().is_some() {
            caller.damage(caller, f32::MAX, DamageType::GENERIC_KILL);
        } else {
            let entity = self.get_entity();
            entity.remove();
            entity.world.load().game_event_from(
                Some(caller),
                pumpkin_data::game_event::GameEvent::EntityDie,
                entity.pos.load(),
            );
        }
    }

    fn get_experience_reward(&self, _killer: Option<&dyn EntityBase>) -> u32 {
        0
    }

    fn get_base_experience_reward(&self) -> u32 {
        0
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum RemovalReason {
    Killed,
    Discarded,
    UnloadedToChunk,
    UnloadedWithPlayer,
    ChangedDimension,
}

impl RemovalReason {
    #[must_use]
    pub const fn should_destroy(&self) -> bool {
        match self {
            Self::Killed | Self::Discarded => true,
            Self::UnloadedToChunk | Self::UnloadedWithPlayer | Self::ChangedDimension => false,
        }
    }

    #[must_use]
    pub const fn should_save(&self) -> bool {
        match self {
            Self::Killed | Self::Discarded | Self::UnloadedWithPlayer | Self::ChangedDimension => {
                false
            }
            Self::UnloadedToChunk => true,
        }
    }
}

// IMPORTANT: have that 1 and not 0 because fetch_add returns previous value and 0 would be invalid
static CURRENT_ID: AtomicI32 = AtomicI32::new(1);

/// Position, motion and packed rotation for spawn packets.
struct SpawnState {
    pos: Vector3<f64>,
    velocity: Vector3<f64>,
    pitch: u8,
    yaw: u8,
    head_yaw: u8,
}

/// Represents a non-living Entity (e.g. Item, Egg, Snowball...)
/// Vanilla `LeashData.delayedLeashInfo`: who or what a saved leash is tied to.
#[derive(Clone, Copy, Debug)]
pub enum DelayedLeash {
    Holder(uuid::Uuid),
    Knot(BlockPos),
}

/// Vanilla `Entity.DELTA_AFFECTED_BY_BLOCKS_BELOW_0_5`, a float.
pub const DELTA_AFFECTED_BY_BLOCKS_BELOW_0_5: f64 = 0.500_001_013_278_961_2;
/// The horizontal limit vanilla `Entity.load` clamps a saved position to.
const MAX_HORIZONTAL_POSITION: f64 = 3.000_051_2E7;
/// The vertical limit vanilla `Entity.load` clamps a saved position to.
const MAX_VERTICAL_POSITION: f64 = 2.0E7;
/// Vanilla `Entity.WATER_FLOW_SCALE`.
const WATER_FLOW_SCALE: f64 = 0.014;
/// Vanilla `Entity.LAVA_FAST_FLOW_SCALE`.
const LAVA_FAST_FLOW_SCALE: f64 = 0.007;
/// Vanilla `Entity.LAVA_SLOW_FLOW_SCALE`.
const LAVA_SLOW_FLOW_SCALE: f64 = 0.002_333_333_333_333_333_5;
/// The smallest push `CurrentAccumulator.applyTo` gives an entity that is nearly still.
const MIN_FLUID_CURRENT_IMPULSE: f64 = 0.004_500_000_000_000_000_5;

/// The `EntityFluidInteraction` slot of a fluid: 0 for `#water`, 1 for `#lava`.
fn fluid_current_index(fluid: &Fluid) -> Option<usize> {
    if fluid.has_tag(&tag::Fluid::MINECRAFT_WATER) {
        Some(0)
    } else if fluid.has_tag(&tag::Fluid::MINECRAFT_LAVA) {
        Some(1)
    } else {
        None
    }
}

/// Vanilla `EntityFluidInteraction.CurrentAccumulator`.
#[derive(Clone, Copy, Default)]
struct FluidCurrent {
    height: f64,
    accumulated: Vector3<f64>,
    count: i32,
}

impl FluidCurrent {
    /// Vanilla `CurrentAccumulator.applyTo`.
    fn apply_to(&self, entity: &Entity, scale: f64) {
        let old_movement = entity.velocity.load();
        if let Some(impulse) = self.impulse(
            entity.entity_type == &EntityType::PLAYER,
            old_movement,
            scale,
        ) {
            entity.velocity.store(old_movement + impulse);
        }
    }

    /// The push `applyTo` adds to `old_movement`, if any.
    fn impulse(
        &self,
        is_player: bool,
        old_movement: Vector3<f64>,
        scale: f64,
    ) -> Option<Vector3<f64>> {
        if self.count == 0 || self.accumulated.length_squared() < f64::from(1.0E-5f32) {
            return None;
        }
        let impulse = if is_player {
            self.accumulated * (1.0 / f64::from(self.count))
        } else {
            self.accumulated.normalize()
        };
        let impulse = impulse * scale;
        if old_movement.x.abs() < 0.003
            && old_movement.z.abs() < 0.003
            && impulse.length() < MIN_FLUID_CURRENT_IMPULSE
        {
            return Some(impulse.normalize() * MIN_FLUID_CURRENT_IMPULSE);
        }
        Some(impulse)
    }
}

/// Vanilla `Entity.Movement`: one move the entity made, swept for the blocks it passed.
#[derive(Clone, Copy, Debug)]
pub struct EntityMovement {
    pub from: Vector3<f64>,
    pub to: Vector3<f64>,
    /// The move before collisions, whose axis order the sweep follows.
    pub axis_dependent_original_movement: Option<Vector3<f64>>,
}

impl EntityMovement {
    #[must_use]
    pub const fn new(
        from: Vector3<f64>,
        to: Vector3<f64>,
        axis_dependent_original_movement: Option<Vector3<f64>>,
    ) -> Self {
        Self {
            from,
            to,
            axis_dependent_original_movement,
        }
    }
}

pub struct Entity {
    /// A unique identifier for the entity
    pub entity_id: i32,
    /// A persistent, unique identifier for the entity
    pub entity_uuid: uuid::Uuid,
    /// The type of entity (e.g., player, zombie, item)
    pub entity_type: &'static EntityType,
    /// The world in which the entity exists.
    /// Uses `ArcSwap` to allow atomic updates when changing dimensions.
    pub world: ArcSwap<World>,
    /// The entity's current position in the world
    pub pos: AtomicCell<Vector3<f64>>,
    /// The last known position of the entity.
    pub last_pos: AtomicCell<Vector3<f64>>,
    /// The last movement vector
    pub movement: AtomicCell<Vector3<f64>>,
    /// The entity's position rounded to the nearest block coordinates
    pub block_pos: AtomicCell<BlockPos>,
    /// The block supporting the entity
    pub supporting_block_pos: AtomicCell<Option<BlockPos>>,
    /// Vanilla `Entity.fallDistance`.
    pub fall_distance: AtomicCell<f64>,
    /// Vanilla `Entity.onGroundNoBlocks`: on the ground last check with no block found under it.
    pub on_ground_no_blocks: AtomicBool,
    /// The chunk coordinates of the entity's current position
    pub chunk_pos: AtomicCell<Vector2<i32>>,
    /// Indicates whether the entity is sneaking
    pub sneaking: AtomicBool,
    /// Indicates whether the entity is sprinting
    pub sprinting: AtomicBool,
    /// Indicates whether the entity is swimming
    pub swimming: AtomicBool,
    /// Indicates whether the entity is invisible
    pub invisible: AtomicBool,
    /// Indicates whether the entity is glowing
    pub glowing: AtomicBool,
    /// Indicates whether the entity is flying due to a fall
    pub fall_flying: AtomicBool,
    /// The entity's current velocity vector, aka knockback
    pub velocity: AtomicCell<Vector3<f64>>,
    /// Tracks a horizontal collision
    pub horizontal_collision: AtomicBool,
    /// Vanilla `verticalCollisionBelow`: the last move was stopped on its way down.
    pub vertical_collision_below: AtomicBool,
    /// Indicates whether the entity is on the ground (may not always be accurate).
    pub on_ground: AtomicBool,
    /// Indicates whether the entity is touching water
    pub touching_water: AtomicBool,
    /// Indicates the fluid height
    pub water_height: AtomicCell<f64>,
    /// Vanilla `fluidInteraction.isEyeInFluid(WATER)`, from the last fluid update.
    pub eye_in_water: AtomicBool,
    /// Vanilla `wasEyeInWater`: `eye_in_water` from before this tick's fluid update.
    pub was_eye_in_water: AtomicBool,
    /// Indicates whether the entity is touching lava
    pub touching_lava: AtomicBool,
    /// Indicates the fluid height
    pub lava_height: AtomicCell<f64>,
    /// The entity's yaw rotation (horizontal rotation) ← →
    pub yaw: AtomicCell<f32>,
    /// The entity's head yaw rotation (horizontal rotation of the head)
    pub head_yaw: AtomicCell<f32>,
    /// The entity's body yaw rotation (horizontal rotation of the body)
    pub body_yaw: AtomicCell<f32>,
    /// The entity's pitch rotation (vertical rotation) ↑ ↓
    pub pitch: AtomicCell<f32>,
    /// The entity's current pose (e.g., standing, sitting, swimming).
    pub pose: AtomicCell<EntityPose>,
    /// The bounding box of an entity (hitbox)
    pub bounding_box: AtomicCell<BoundingBox>,
    ///The size (width and height) of the bounding box
    pub entity_dimension: AtomicCell<EntityDimensions>,
    /// Vanilla `permanentlyInvulnerable`, saved as `Invulnerable`.
    pub invulnerable: AtomicBool,
    /// Vanilla `invulnerableTime`: ticks of full invulnerability left, counted down in
    /// `commonTick` and saved as `invulnerable_time`.
    pub invulnerable_time: AtomicI32,
    /// List of damage types this entity is immune to
    pub damage_immunities: std::sync::Mutex<Vec<DamageType>>,
    // Whether the entity is immune to fire (to disable visual fire and fire damage)
    pub fire_immune: AtomicBool,
    pub fire_ticks: AtomicI32,
    pub has_visual_fire: AtomicBool,
    /// The number of ticks the entity has been frozen (in powder snow)
    /// Max is 140 ticks (7 seconds). Increases by 1/tick in powder snow, decreases by 2/tick outside.
    pub frozen_ticks: AtomicI32,
    /// Set during block-collision processing when the entity is touching powder snow.
    pub is_in_powder_snow: AtomicBool,
    /// True if the entity was in powder snow during the previous tick.
    pub was_in_powder_snow: AtomicBool,
    pub removal_reason: AtomicCell<Option<RemovalReason>>,
    // The passengers that entity has
    pub passengers: std::sync::Mutex<Vec<Arc<dyn EntityBase>>>,
    /// The vehicle that entity is in
    pub vehicle: std::sync::Mutex<Option<Arc<dyn EntityBase>>>,
    /// The entity this entity is attached/leashed to (if any)
    pub leashed_to: std::sync::Mutex<Option<Arc<dyn EntityBase>>>,
    /// Vanilla `LeashData.delayedLeashInfo`: a saved leash waiting for its holder to load.
    pub delayed_leash: std::sync::Mutex<Option<DelayedLeash>>,
    /// Vanilla `LeashData.angularMomentum`: 0 whenever the leash data is created, reset to 0
    /// when the leash is dropped.
    pub leash_angular_momentum: AtomicCell<f64>,
    /// Cooldown before entity can mount again after dismounting
    pub riding_cooldown: AtomicI32,
    /// Vanilla `AgeableMob.age`: negative while a baby, counting up to 0. Mobs whose baby
    /// state is their own flag (zombies, piglins, zoglins) keep it negative as well.
    pub age: AtomicI32,
    /// Vanilla `Entity.tickCount`: ticks since this entity object was created or loaded.
    pub tick_count: AtomicI32,
    /// Vanilla `Entity.firstTick`: cleared at the end of the first base tick.
    pub first_tick: AtomicBool,
    /// Whether this world's waypoint manager tracks the entity as a waypoint, so `set_pos` only
    /// enters the manager for transmitters (vanilla passes it nothing else).
    pub waypoint_tracked: AtomicBool,
    /// Vanilla `pistonDeltas` and `pistonDeltasGameTime`.
    piston_deltas: std::sync::Mutex<([f64; 3], i64)>,
    /// Vanilla `movementThisTick`: the moves made since the last block-effect sweep.
    movement_this_tick: std::sync::Mutex<Vec<EntityMovement>>,
    /// Vanilla `finalMovementsThisTick`: the moves the last sweep covered.
    final_movements_this_tick: std::sync::Mutex<Vec<EntityMovement>>,
    /// Vanilla `Entity.moveDist`, `flyDist` and `nextStep`, for step sounds.
    pub move_dist: AtomicCell<f32>,
    pub fly_dist: AtomicCell<f32>,
    pub next_step: AtomicCell<f32>,
    /// Vanilla `Entity.crystalSoundIntensity` and `lastCrystalSoundPlayTick`.
    pub crystal_sound_intensity: AtomicCell<f32>,
    pub last_crystal_sound_play_tick: AtomicI32,

    pub current_biome: ArcSwap<&'static Biome>,
    pub last_biome_update_pos: AtomicCell<BlockPos>,

    pub portal_cooldown: AtomicU32,

    pub portal_manager: std::sync::Mutex<Option<PortalProcessor>>,
    /// Custom name for the entity
    pub custom_name: ArcSwap<Option<TextComponent>>,
    /// Indicates whether the entity's custom name is visible
    pub custom_name_visible: AtomicBool,
    pub silent: AtomicBool,
    pub has_no_gravity: AtomicBool,
    /// Scoreboard tags attached to this entity, managed with `/tag`.
    /// Vanilla allows at most [`MAX_SCOREBOARD_TAGS`] tags per entity.
    pub scoreboard_tags: std::sync::Mutex<HashSet<String>>,
    /// The data send in the Entity Spawn packet
    pub data: AtomicI32,
    /// Stores entity boolean flags (on fire, sneaking, invisible, glowing, etc.)
    pub flags: std::sync::atomic::AtomicI8,
    /// Stores Bedrock-specific entity boolean flags (bit 0-63)
    pub bedrock_flags: std::sync::atomic::AtomicI64,
    /// Stores more Bedrock-specific entity boolean flags (bit 0-63)
    pub bedrock_flags_two: std::sync::atomic::AtomicI64,
    /// If true, the entity bypasses physics, collisions, and block effects (e.g. spectator, markers, display entities)
    pub no_physics: AtomicBool,
    pub synched_data: synched_entity_data::SynchedEntityData,
    /// Multiplies movement for one tick before being reset
    pub movement_multiplier: AtomicCell<Vector3<f64>>,
    /// Vanilla `needsSync`: tracker resyncs position and velocity of entities
    pub velocity_dirty: AtomicBool,
    /// Vanilla `syncPosition`: the tracker sends the position on its next tick.
    pub sync_position: AtomicBool,
    /// Vanilla `syncVelocity` (`markHurt`): the velocity also goes to the entity's own client.
    /// `velocity_dirty` alone only updates other viewers, so a pushed player is never sent its
    /// own stale velocity.
    pub sync_velocity: AtomicBool,
    /// Set when an Entity is to be removed but could still be referenced
    pub removed: AtomicBool,
    /// The last sent yaw value (encoded as u8) for change detection
    pub last_sent_yaw: AtomicU8,
    /// The last sent pitch value (encoded as u8) for change detection
    pub last_sent_pitch: AtomicU8,
    /// Delta base of the move packets. Vanilla `ServerEntity.positionCodec`.
    pub last_sent_pos: AtomicCell<Vector3<f64>>,
    /// Cache for the last sent head yaw byte
    pub last_sent_head_yaw: AtomicU8,
    /// Velocity last put on the wire. Vanilla `ServerEntity.lastSentMovement`.
    pub last_sent_velocity: AtomicCell<Vector3<f64>>,
    /// Persistent custom data container for plugins
    pub custom_data: std::sync::Mutex<NbtCompound>,
    /// Vanilla `customData`, saved as `data`.
    pub entity_custom_data: std::sync::Mutex<NbtCompound>,
    /// Vanilla `hasGlowingTag`, saved as `Glowing`; the glowing effect is separate.
    pub has_glowing_tag: AtomicBool,
    /// When the entity joined its current chunk section: vanilla `EntitySection` keeps its
    /// entities in the order they entered, which box queries return them in.
    pub section_order: std::sync::atomic::AtomicU64,
    /// Vanilla `Entity.random`: the entity's own `RandomSource`, behind [`Self::random`].
    random: std::sync::Mutex<pumpkin_util::random::legacy_rand::LegacyRand>,
    /// The listeners vanilla's `updateDynamicGameEventListener` overrides hand out (the
    /// allay's and the warden's), set once by the entity's constructor.
    pub dynamic_game_event_listeners: std::sync::OnceLock<Box<[DynamicGameEventListener]>>,
}

impl Entity {
    pub fn new(
        world: Arc<World>,
        position: Vector3<f64>,
        entity_type: &'static EntityType,
    ) -> Self {
        Self::from_uuid(Uuid::new_v4(), world, position, entity_type)
    }

    pub fn reserve_ids(count: i32) -> i32 {
        CURRENT_ID.fetch_add(count, Relaxed)
    }

    pub fn from_uuid(
        entity_uuid: uuid::Uuid,
        world: Arc<World>,
        position: Vector3<f64>,
        entity_type: &'static EntityType,
    ) -> Self {
        Self::from_uuid_with_id(
            CURRENT_ID.fetch_add(1, Relaxed),
            entity_uuid,
            world,
            position,
            entity_type,
        )
    }

    #[expect(clippy::too_many_lines, reason = "one initializer per entity field")]
    pub fn from_uuid_with_id(
        entity_id: i32,
        entity_uuid: uuid::Uuid,
        world: Arc<World>,
        position: Vector3<f64>,
        entity_type: &'static EntityType,
    ) -> Self {
        let floor_x = position.x.floor() as i32;
        let floor_y = position.y.floor() as i32;
        let floor_z = position.z.floor() as i32;

        let bounding_box_size = Self::type_dimensions(entity_type);

        let current_biome = world.get_zoomed_biome(&BlockPos::new(floor_x, floor_y, floor_z));

        Self {
            entity_id,
            entity_uuid,
            entity_type,
            on_ground: AtomicBool::new(false),
            touching_water: AtomicBool::new(false),
            water_height: AtomicCell::new(0.0),
            eye_in_water: AtomicBool::new(false),
            was_eye_in_water: AtomicBool::new(false),
            touching_lava: AtomicBool::new(false),
            lava_height: AtomicCell::new(0.0),
            horizontal_collision: AtomicBool::new(false),
            vertical_collision_below: AtomicBool::new(false),
            pos: AtomicCell::new(position),
            last_pos: AtomicCell::new(position),
            movement: AtomicCell::new(Vector3::default()),
            block_pos: AtomicCell::new(BlockPos(Vector3::new(floor_x, floor_y, floor_z))),
            supporting_block_pos: AtomicCell::new(None),
            fall_distance: AtomicCell::new(0.0),
            on_ground_no_blocks: AtomicBool::new(false),
            chunk_pos: AtomicCell::new(Vector2::new(
                get_section_cord(floor_x),
                get_section_cord(floor_z),
            )),
            sneaking: AtomicBool::new(false),
            swimming: AtomicBool::new(false),
            invisible: AtomicBool::new(false),
            glowing: AtomicBool::new(false),
            world: ArcSwap::new(world),
            sprinting: AtomicBool::new(false),
            fall_flying: AtomicBool::new(false),
            yaw: AtomicCell::new(0.0),
            head_yaw: AtomicCell::new(0.0),
            body_yaw: AtomicCell::new(0.0),
            pitch: AtomicCell::new(0.0),
            velocity: AtomicCell::new(Vector3::new(0.0, 0.0, 0.0)),
            pose: AtomicCell::new(EntityPose::Standing),
            bounding_box: AtomicCell::new(BoundingBox::new_from_pos(
                position.x,
                position.y,
                position.z,
                &bounding_box_size,
            )),
            entity_dimension: AtomicCell::new(bounding_box_size),
            invulnerable: AtomicBool::new(false),
            invulnerable_time: AtomicI32::new(0),
            damage_immunities: std::sync::Mutex::new(Vec::new()),
            data: AtomicI32::new(0),
            flags: std::sync::atomic::AtomicI8::new(0),
            bedrock_flags: std::sync::atomic::AtomicI64::new(0),
            bedrock_flags_two: std::sync::atomic::AtomicI64::new(0),
            fire_immune: AtomicBool::new(false),
            fire_ticks: AtomicI32::new(0),
            has_visual_fire: AtomicBool::new(false),
            frozen_ticks: AtomicI32::new(0),
            is_in_powder_snow: AtomicBool::new(false),
            was_in_powder_snow: AtomicBool::new(false),
            removal_reason: AtomicCell::new(None),
            passengers: std::sync::Mutex::new(Vec::new()),
            vehicle: std::sync::Mutex::new(None),
            leashed_to: std::sync::Mutex::new(None),
            delayed_leash: std::sync::Mutex::new(None),
            leash_angular_momentum: AtomicCell::new(0.0),

            riding_cooldown: AtomicI32::new(0),
            age: AtomicI32::new(0),
            tick_count: AtomicI32::new(0),
            first_tick: AtomicBool::new(true),
            waypoint_tracked: AtomicBool::new(false),
            movement_this_tick: std::sync::Mutex::new(Vec::new()),
            piston_deltas: std::sync::Mutex::new(([0.0; 3], 0)),
            final_movements_this_tick: std::sync::Mutex::new(Vec::new()),
            move_dist: AtomicCell::new(0.0),
            fly_dist: AtomicCell::new(0.0),
            next_step: AtomicCell::new(1.0),
            crystal_sound_intensity: AtomicCell::new(0.0),
            last_crystal_sound_play_tick: AtomicI32::new(0),
            current_biome: ArcSwap::new(Arc::new(current_biome)),
            last_biome_update_pos: AtomicCell::new(BlockPos::new(floor_x, floor_y, floor_z)),
            portal_cooldown: AtomicU32::new(0),
            portal_manager: std::sync::Mutex::new(None),
            custom_name: ArcSwap::new(Arc::new(None)),
            custom_name_visible: AtomicBool::new(false),
            silent: AtomicBool::new(false),
            has_no_gravity: AtomicBool::new(false),
            scoreboard_tags: std::sync::Mutex::new(HashSet::new()),
            no_physics: AtomicBool::new(false),
            synched_data: synched_entity_data::SynchedEntityData::for_entity_type(entity_type.id),
            movement_multiplier: AtomicCell::new(Vector3::default()),
            velocity_dirty: AtomicBool::new(true),
            sync_velocity: AtomicBool::new(false),
            sync_position: AtomicBool::new(false),
            removed: AtomicBool::new(false),
            last_sent_yaw: AtomicU8::new(0),
            last_sent_pitch: AtomicU8::new(0),
            last_sent_head_yaw: AtomicU8::new(0),
            last_sent_pos: AtomicCell::new(position),
            last_sent_velocity: AtomicCell::new(Vector3::default()),
            custom_data: std::sync::Mutex::new(NbtCompound::new()),
            entity_custom_data: std::sync::Mutex::new(NbtCompound::new()),
            has_glowing_tag: AtomicBool::new(false),
            section_order: std::sync::atomic::AtomicU64::new(next_section_order()),
            random: std::sync::Mutex::new(new_entity_random()),
            dynamic_game_event_listeners: std::sync::OnceLock::new(),
        }
    }

    /// Vanilla `Entity.updateDynamicGameEventListener`.
    pub fn update_dynamic_game_event_listener(&self, action: impl Fn(&DynamicGameEventListener)) {
        if let Some(listeners) = self.dynamic_game_event_listeners.get() {
            listeners.iter().for_each(action);
        }
    }

    /// Vanilla `Entity.push(x, y, z)`: adds an impulse that viewers pick up through
    /// `needsSync`.
    pub fn push_impulse(&self, impulse: Vector3<f64>) {
        if impulse.x.is_finite() && impulse.y.is_finite() && impulse.z.is_finite() {
            self.velocity.store(self.velocity.load() + impulse);
            self.velocity_dirty.store(true, Ordering::SeqCst);
        }
    }

    pub fn add_velocity(&self, velocity: Vector3<f64>) {
        self.set_velocity(self.velocity.load() + velocity);
    }

    pub fn set_velocity(&self, velocity: Vector3<f64>) {
        self.velocity.store(velocity);
        self.send_velocity();
    }

    /// Updates the world reference for this entity.
    /// Called when the entity changes dimensions (e.g., through a nether portal).
    pub fn set_world(&self, world: Arc<World>) {
        let block_pos = self.block_pos.load();
        let biome = world.get_zoomed_biome(&block_pos);
        self.current_biome.store(Arc::new(biome));
        self.last_biome_update_pos.store(block_pos);
        self.world.store(world);
    }

    pub fn bedrock_metadata(&self) -> SyncedActorDataList {
        if self.bedrock_flags.load(Ordering::Relaxed) == 0 {
            self.bedrock_flags.fetch_or(
                (1i64 << entity_data_flag::HAS_GRAVITY)
                    | (1i64 << entity_data_flag::CLIMB)
                    | (1i64 << entity_data_flag::HAS_COLLISION)
                    | (1i64 << entity_data_flag::BREATHING),
                Ordering::Relaxed,
            );
        }

        let mut metadata = SyncedActorDataList::new();
        metadata.set(
            entity_data_key::WIDTH,
            MetadataValue::Float(self.entity_type.dimension[0]),
        );
        metadata.set(
            entity_data_key::HEIGHT,
            MetadataValue::Float(self.entity_type.dimension[1]),
        );
        metadata.set(entity_data_key::SCALE, MetadataValue::Float(1.0));
        metadata.set(
            entity_data_key::FLAGS,
            MetadataValue::Int64(self.bedrock_flags.load(Ordering::Relaxed)),
        );
        metadata.set(
            entity_data_key::FLAGS_TWO,
            MetadataValue::Int64(self.bedrock_flags_two.load(Ordering::Relaxed)),
        );

        if let Some(name) = &**self.custom_name.load() {
            metadata.set(
                entity_data_key::NAME,
                MetadataValue::String(name.clone().get_text()),
            );
            if self.custom_name_visible.load(Ordering::Relaxed) {
                metadata.set_flag(
                    entity_data_key::FLAGS,
                    entity_data_flag::SHOW_NAME as u8,
                    true,
                );
                metadata.set_flag(
                    entity_data_key::FLAGS,
                    entity_data_flag::ALWAYS_SHOW_NAME as u8,
                    true,
                );
            }
        }

        metadata
    }

    /// Vanilla `Entity.getDefaultPassengerAttachmentPoint`: the passenger point for the
    /// passenger's place on this vehicle (the last point for any later place).
    #[must_use]
    pub fn default_passenger_attachment_point(
        &self,
        passenger: &dyn EntityBase,
        attachments: &EntityAttachments,
    ) -> Vector3<f64> {
        let id = passenger.get_entity().entity_id;
        let index = self
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .position(|p| p.get_entity().entity_id == id)
            .unwrap_or(0);
        attachments.passenger_clamped(index, self.yaw.load())
    }

    /// The type's default size, vanilla `EntityType.getDimensions`.
    #[must_use]
    pub const fn type_dimensions(entity_type: &EntityType) -> EntityDimensions {
        EntityDimensions::new(
            entity_type.dimension[0],
            entity_type.dimension[1],
            entity_type.eye_height,
        )
        .with_attachments(
            AttachmentPoints::Points(entity_type.passenger_attachments),
            AttachmentPoints::Points(entity_type.vehicle_attachments),
        )
    }

    /// Stores the baby age (see [`Self::age`]) without `AgeableMob.setAge`'s side effects.
    pub fn set_age(&self, age: i32) {
        self.age.store(age, Relaxed);
    }

    /// Adds a scoreboard tag to this entity.
    ///
    /// Returns `false` if the entity already has the tag or already carries
    /// [`MAX_SCOREBOARD_TAGS`] tags.
    pub fn add_scoreboard_tag(&self, tag: &str) -> bool {
        let mut tags = self
            .scoreboard_tags
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tags.len() < MAX_SCOREBOARD_TAGS && tags.insert(tag.to_owned())
    }

    /// Removes a scoreboard tag from this entity.
    ///
    /// Returns `false` if the entity did not have the tag.
    pub fn remove_scoreboard_tag(&self, tag: &str) -> bool {
        self.scoreboard_tags
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(tag)
    }

    /// Sets a custom name for the entity, typically used with nametags
    pub fn set_custom_name(&self, name: TextComponent) {
        self.custom_name.store(Arc::new(Some(name.clone())));
        let mut bedrock_meta = SyncedActorDataList::new();
        bedrock_meta.set(
            entity_data_key::NAME,
            MetadataValue::String(name.clone().get_text()),
        );
        let visible = self.custom_name_visible.load(Ordering::Relaxed);
        bedrock_meta.set_flag(
            entity_data_key::FLAGS,
            entity_data_flag::SHOW_NAME as u8,
            visible,
        );
        bedrock_meta.set_flag(
            entity_data_key::FLAGS,
            entity_data_flag::ALWAYS_SHOW_NAME as u8,
            visible,
        );
        self.set_synced_data(tracked_data::entity::DATA_CUSTOM_NAME, Some(name));
        self.send_bedrock_actor_data(&bedrock_meta);
    }

    pub fn set_custom_name_visible(&self, visible: bool) {
        self.custom_name_visible.store(visible, Ordering::Relaxed);
        let mut bedrock_meta = SyncedActorDataList::new();
        if let Some(name) = &**self.custom_name.load() {
            bedrock_meta.set(
                entity_data_key::NAME,
                MetadataValue::String(name.clone().get_text()),
            );
        }
        bedrock_meta.set_flag(
            entity_data_key::FLAGS,
            entity_data_flag::SHOW_NAME as u8,
            visible,
        );
        bedrock_meta.set_flag(
            entity_data_key::FLAGS,
            entity_data_flag::ALWAYS_SHOW_NAME as u8,
            visible,
        );
        self.set_synced_data(tracked_data::entity::DATA_CUSTOM_NAME_VISIBLE, visible);
        self.send_bedrock_actor_data(&bedrock_meta);
    }

    pub fn is_silent(&self) -> bool {
        self.silent.load(Ordering::Relaxed)
    }

    pub fn set_silent(&self, silent: bool) {
        self.silent.store(silent, Ordering::Relaxed);
        self.set_synced_data(tracked_data::entity::DATA_SILENT, silent);
    }

    pub fn has_no_gravity(&self) -> bool {
        self.has_no_gravity.load(Ordering::Relaxed)
    }

    pub fn set_has_no_gravity(&self, no_gravity: bool) {
        self.has_no_gravity.store(no_gravity, Ordering::Relaxed);
        self.set_synced_data(tracked_data::entity::DATA_NO_GRAVITY, no_gravity);
    }

    /// Vanilla `hurtMarked` path: immediate, to watchers and self.
    pub fn send_velocity(&self) {
        let velocity = self.velocity.load();
        self.last_sent_velocity.store(velocity);
        self.world
            .load()
            .send_to_tracking_players_and_self_editioned(
                self,
                &CEntityVelocity::new(self.entity_id.into(), velocity),
                &CSetActorMotion {
                    target_runtime_id: VarULong(self.entity_id as u64),
                    motion: Vector3::new(velocity.x as f32, velocity.y as f32, velocity.z as f32),
                    tick: VarULong(0),
                },
            );
    }

    #[must_use]
    pub fn get_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        use pumpkin_data::entity_dimensions as dims;
        if pose == EntityPose::Sleeping {
            return dims::LIVING_ENTITY_SLEEPING_DIMENSIONS;
        }
        if self.entity_type == &EntityType::PLAYER {
            // Vanilla `Avatar.POSES`.
            match pose {
                EntityPose::FallFlying => return dims::AVATAR_POSES_FALL_FLYING,
                EntityPose::Swimming => return dims::AVATAR_POSES_SWIMMING,
                EntityPose::SpinAttack => return dims::AVATAR_POSES_SPIN_ATTACK,
                EntityPose::Crouching => return dims::AVATAR_POSES_CROUCHING,
                EntityPose::Dying => return dims::AVATAR_POSES_DYING,
                _ => return dims::AVATAR_POSES_STANDING,
            }
        }
        Self::type_dimensions(self.entity_type)
    }

    /// Whether the entity is a baby. Pumpkin keeps that as a negative age for every
    /// mob that has babies.
    #[must_use]
    pub fn is_baby(&self) -> bool {
        self.age.load(Ordering::Relaxed) < 0
    }

    /// Stores new dimensions and rebuilds the bounding box around the current position,
    /// the core of vanilla `Entity.refreshDimensions`.
    /// Vanilla `Entity.refreshDimensions`: a small non-player entity that grows is moved out
    /// of the blocks it would grow into.
    pub fn apply_dimensions(&self, dimension: EntityDimensions) {
        let old_dimension = self.entity_dimension.load();
        let position = self.pos.load();
        self.bounding_box.store(BoundingBox::new_from_pos(
            position.x, position.y, position.z, &dimension,
        ));
        self.entity_dimension.store(dimension);
        let is_small = dimension.width <= 4.0 && dimension.height <= 4.0;
        if !self.first_tick.load(Ordering::Relaxed)
            && !self.no_physics.load(Ordering::Relaxed)
            && is_small
            && (dimension.width > old_dimension.width || dimension.height > old_dimension.height)
            && self.entity_type != &EntityType::PLAYER
        {
            self.fudge_position_after_size_change(old_dimension, dimension);
        }
    }

    /// Vanilla `Entity.fudgePositionAfterSizeChange`.
    fn fudge_position_after_size_change(
        &self,
        previous: EntityDimensions,
        new: EntityDimensions,
    ) -> bool {
        let world = self.world.load();
        let Some(caller) = world.get_entity_by_id(self.entity_id) else {
            return false;
        };
        let old_center = self.pos.load() + Vector3::new(0.0, f64::from(previous.height) / 2.0, 0.0);
        let width_delta = f64::from((new.width - previous.width).max(0.0)) + 1.0E-6;
        let height_delta = f64::from((new.height - previous.height).max(0.0)) + 1.0E-6;
        let free_position = |allowed_height: f64, size_y: f64| {
            // `AABB.ofSize(oldCenter, widthDelta, allowedHeight, widthDelta)`.
            let half = Vector3::new(width_delta / 2.0, allowed_height / 2.0, width_delta / 2.0);
            let allowed = BoundingBox::new(old_center - half, old_center + half);
            let size = Vector3::new(f64::from(new.width), size_y, f64::from(new.width));
            let search_area = allowed.expand(size.x, size.y, size.z);
            let mut collisions = world.get_block_collisions(search_area, caller.as_ref()).0;
            let border = world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            collisions.retain(|shape| border.contains_box(shape));
            drop(border);
            find_free_position(allowed, &collisions, old_center, size)
        };
        if let Some(free) = free_position(height_delta, f64::from(new.height)) {
            self.set_pos(free + Vector3::new(0.0, -f64::from(new.height) / 2.0, 0.0));
            return true;
        }
        if new.width > previous.width
            && new.height > previous.height
            && let Some(free) = free_position(1.0E-6, f64::from(previous.height))
        {
            self.set_pos(free + Vector3::new(0.0, -f64::from(previous.height) / 2.0 + 1.0E-6, 0.0));
            return true;
        }
        false
    }

    pub fn get_eye_height(&self) -> f64 {
        f64::from(self.entity_dimension.load().eye_height)
    }

    /// Vanilla `Entity.getRandom()`. Draw into locals and drop the guard before calling on.
    pub fn random(
        &self,
    ) -> std::sync::MutexGuard<'_, pumpkin_util::random::legacy_rand::LegacyRand> {
        self.random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Updates the entity's position, block position, and chunk position.
    ///
    /// This function calculates the new position, block position, and chunk position based on the provided coordinates. If any of these values change, the corresponding fields are updated.
    pub fn set_pos(&self, new_position: Vector3<f64>) {
        let pos = self.pos.load();
        if pos != new_position {
            self.pos.store(new_position);
            self.bounding_box.store(BoundingBox::new_from_pos(
                new_position.x,
                new_position.y,
                new_position.z,
                &self.entity_dimension.load(),
            ));

            let floor_x = new_position.x.floor() as i32;
            let floor_y = new_position.y.floor() as i32;
            let floor_z = new_position.z.floor() as i32;

            let block_pos = self.block_pos.load();
            let block_pos_vec = block_pos.0;
            if floor_x != block_pos_vec.x
                || floor_y != block_pos_vec.y
                || floor_z != block_pos_vec.z
            {
                let new_block_pos = Vector3::new(floor_x, floor_y, floor_z);
                let new_bp = BlockPos(new_block_pos);
                self.block_pos.store(new_bp);
                // Vanilla `EntityInLevelCallback.onMove`'s `onSectionChange`: the entity
                // leaves its old section and goes to the end of the new one.
                if chunk_section_from_pos(&block_pos) != chunk_section_from_pos(&new_bp) {
                    self.section_order
                        .store(next_section_order(), Ordering::Relaxed);
                    let world = self.world.load();
                    self.update_dynamic_game_event_listener(|listener| {
                        listener.on_section_change(&world);
                    });
                }

                let world = self.world.load();
                let biome = world.get_zoomed_biome(&new_bp);
                self.current_biome.store(Arc::new(biome));
                self.last_biome_update_pos.store(new_bp);

                let chunk_pos = self.chunk_pos.load();
                if get_section_cord(floor_x) != chunk_pos.x
                    || get_section_cord(floor_z) != chunk_pos.y
                {
                    self.chunk_pos.store(Vector2::new(
                        get_section_cord(new_block_pos.x),
                        get_section_cord(new_block_pos.z),
                    ));
                }
            }

            // The end of vanilla `setPosRaw`: on any position change, once past the first tick
            // and not removed, `updateWaypoint` for a transmitter and `updatePlayer` for a
            // receiving player.
            if !self.first_tick.load(Ordering::Relaxed) && !self.is_removed() {
                let world = self.world.load();
                world.waypoint_manager.on_entity_moved(&world, self);
            }
        }
    }

    /// Returns entity rotation as vector
    pub fn rotation(&self) -> Vector3<f32> {
        let pitch_rad = self.pitch.load().to_radians();
        let yaw_rad = -self.yaw.load().to_radians();

        let cos_yaw = yaw_rad.cos();
        let sin_yaw = yaw_rad.sin();
        let cos_pitch = pitch_rad.cos();
        let sin_pitch = pitch_rad.sin();

        Vector3::new(sin_yaw * cos_pitch, -sin_pitch, cos_yaw * cos_pitch)
    }

    /// Vanilla `Entity.lookAt(EntityAnchorArgument.Anchor, Vec3)`: turns the body and head
    /// to face `target` from the given anchor.
    pub fn look_at(&self, anchor: EntityAnchor, target: Vector3<f64>) {
        let delta = target.sub(&anchor.position_at_entity(self));
        #[expect(
            clippy::imprecise_flops,
            reason = "vanilla's Math.sqrt rounds like this, hypot doesn't"
        )]
        let horizontal = (delta.x * delta.x + delta.z * delta.z).sqrt();
        // `* 180.0F / (float) Math.PI`, left to right in doubles.
        let to_degrees =
            |radians: f64| radians * f64::from(180.0f32) / f64::from(std::f32::consts::PI);
        self.set_pitch(wrap_degrees(
            (-to_degrees(pumpkin_util::math::atan2(delta.y, horizontal))) as f32,
        ));
        let yaw =
            wrap_degrees(to_degrees(pumpkin_util::math::atan2(delta.z, delta.x)) as f32 - 90.0);
        self.yaw.store(yaw);
        self.head_yaw.store(yaw);
    }

    /// Vanilla `Entity.isSupportedBy`: whether `pos` is this entity's main supporting block.
    /// Reads `mainSupportingBlockPos` as is, without [`Self::get_supporting_block_pos`]'s
    /// on-ground check.
    pub fn is_supported_by(&self, pos: BlockPos) -> bool {
        self.supporting_block_pos.load() == Some(pos)
    }

    /// Returns the block position of the block the (non-player) entity is standing on, if any.
    pub fn get_supporting_block_pos(&self) -> Option<BlockPos> {
        // Check if the entity is on the ground
        if !self.on_ground.load(Ordering::Relaxed) {
            return None;
        }

        self.supporting_block_pos.load()
    }

    /// Vanilla `Entity.checkSupportingBlock`.
    pub fn check_supporting_block(
        &self,
        caller: &dyn EntityBase,
        on_ground: bool,
        movement: Option<Vector3<f64>>,
    ) {
        if !on_ground {
            self.on_ground_no_blocks.store(false, Ordering::Relaxed);
            self.supporting_block_pos.store(None);
            return;
        }
        let bb = self.bounding_box.load();
        let test_area = BoundingBox::new(
            Vector3::new(bb.min.x, bb.min.y - 1.0e-6, bb.min.z),
            Vector3::new(bb.max.x, bb.min.y, bb.max.z),
        );
        let mut supporting = self.find_supporting_block(caller, test_area);
        if supporting.is_some() || self.on_ground_no_blocks.load(Ordering::Relaxed) {
            self.supporting_block_pos.store(supporting);
        } else if let Some(movement) = movement {
            supporting = self.find_supporting_block(
                caller,
                test_area.shift(Vector3::new(-movement.x, 0.0, -movement.z)),
            );
            self.supporting_block_pos.store(supporting);
        }
        self.on_ground_no_blocks
            .store(supporting.is_none(), Ordering::Relaxed);
    }

    /// Vanilla `CollisionGetter.findSupportingBlock`: the colliding block whose centre is
    /// closest to the entity, ties going to the greatest position (y, then z, then x).
    fn find_supporting_block(
        &self,
        caller: &dyn EntityBase,
        area: BoundingBox,
    ) -> Option<BlockPos> {
        let pos = self.pos.load();
        let (_, positions) = self.world.load().get_block_collisions(area, caller);
        let mut main: Option<(BlockPos, f64)> = None;
        for (_, block_pos) in positions {
            let center = block_pos.to_centered_f64();
            let distance = center.squared_distance_to_vec(&pos);
            let better = match main {
                None => true,
                Some((main_pos, main_distance)) => {
                    distance < main_distance
                        || (distance == main_distance
                            && (main_pos.0.y, main_pos.0.z, main_pos.0.x)
                                < (block_pos.0.y, block_pos.0.z, block_pos.0.x))
                }
            };
            if better {
                main = Some((block_pos, distance));
            }
        }
        main.map(|(pos, _)| pos)
    }

    fn adjust_movement_for_collisions(
        &self,
        movement: Vector3<f64>,
        caller: &dyn EntityBase,
    ) -> Vector3<f64> {
        // A server-side move is authoritative, so vanilla resets both flags even for a
        // move of nothing. `collide` still reads the ground state from before the move.
        let was_on_ground = self.on_ground.swap(false, Ordering::SeqCst);
        self.horizontal_collision.store(false, Ordering::SeqCst);
        if movement.length_squared() == 0.0 {
            return movement;
        }

        let bounding_box = self.bounding_box.load();
        let world = self.world.load();
        let max_up_step = caller.max_up_step();
        let entity_colliders = world.get_entity_collisions(
            caller,
            bounding_box
                .stretch(movement)
                .stretch(Vector3::new(0.0, f64::from(max_up_step), 0.0)),
        );
        let movement_step = collide_with_shapes(
            movement,
            &bounding_box,
            &self.collect_colliders(caller, &entity_colliders, bounding_box.stretch(movement)),
        );

        // Vanilla `Entity.collide`'s step up.
        #[expect(
            clippy::float_cmp,
            reason = "vanilla compares the collided movement exactly"
        )]
        let (x_collision, y_collision, z_collision) = (
            movement.x != movement_step.x,
            movement.y != movement_step.y,
            movement.z != movement_step.z,
        );
        let on_ground_after_collision = y_collision && movement.y < 0.0;
        let mut resolved = movement_step;
        if max_up_step > 0.0
            && (on_ground_after_collision || was_on_ground)
            && (x_collision || z_collision)
        {
            let grounded = if on_ground_after_collision {
                bounding_box.shift(Vector3::new(0.0, movement_step.y, 0.0))
            } else {
                bounding_box
            };
            let mut step_up =
                grounded.stretch(Vector3::new(movement.x, f64::from(max_up_step), movement.z));
            if !on_ground_after_collision {
                step_up = step_up.stretch(Vector3::new(0.0, f64::from(-1.0e-5f32), 0.0));
            }
            let colliders = self.collect_colliders(caller, &entity_colliders, step_up);
            let candidates = collect_candidate_step_up_heights(
                &grounded,
                &colliders,
                max_up_step,
                movement_step.y as f32,
            );
            for candidate in candidates {
                let step_from_ground = collide_with_shapes(
                    Vector3::new(movement.x, f64::from(candidate), movement.z),
                    &grounded,
                    &colliders,
                );
                if step_from_ground.horizontal_length_squared()
                    > movement_step.horizontal_length_squared()
                {
                    let distance_to_ground = bounding_box.min.y - grounded.min.y;
                    resolved = step_from_ground - Vector3::new(0.0, distance_to_ground, 0.0);
                    break;
                }
            }
        }

        // `Entity.move`'s flags, from the final movement: `verticalCollisionBelow` and the
        // horizontal collision (`Mth.equal`: closer than `1.0E-5F`).
        #[expect(
            clippy::float_cmp,
            reason = "vanilla compares the collided movement exactly"
        )]
        let collided_below = movement.y != resolved.y && movement.y < 0.0;
        self.on_ground.store(collided_below, Ordering::SeqCst);
        let differs = |a: f64, b: f64| (b - a).abs() >= f64::from(1.0e-5f32);
        self.horizontal_collision.store(
            differs(movement.x, resolved.x) || differs(movement.z, resolved.z),
            Ordering::SeqCst,
        );

        resolved
    }

    /// Vanilla `Entity.collectCollidersIgnoringWorldBorder`: the entity colliders, the world
    /// border when the entity is close to it, and the blocks in `bounding_box`.
    fn collect_colliders(
        &self,
        caller: &dyn EntityBase,
        entity_colliders: &[BoundingBox],
        bounding_box: BoundingBox,
    ) -> Vec<BoundingBox> {
        let world = self.world.load();
        let mut colliders = entity_colliders.to_vec();
        let pos = self.pos.load();
        let border = world
            .worldborder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if border.is_inside_close_to_border(pos.x, pos.z, &bounding_box) {
            colliders.extend(border.collision_shapes());
        }
        drop(border);
        colliders.extend(world.get_block_collisions(bounding_box, caller).0);
        colliders
    }

    /// Applies knockback to the entity, following vanilla Minecraft's mechanics.
    /// `LivingEntity.takeKnockback()`
    /// This function calculates the entity's new velocity based on the specified knockback strength and direction.
    ///
    /// Knockback resistance is not applied here, because it is a `LivingEntity`
    /// attribute and this is an `Entity` method. Callers modelling vanilla's
    /// `LivingEntity.knockback` scale `strength` with
    /// `combat::knockback_after_resistance` first; callers modelling vanilla's raw
    /// `Entity.push` (such as the ender dragon) pass `strength` unscaled.
    pub fn apply_knockback(&self, strength: f64, mut x: f64, mut z: f64) {
        if strength <= 0.0 {
            return;
        }

        self.velocity_dirty.store(true, Ordering::SeqCst);

        // This has some vanilla magic

        // `xd * xd + zd * zd < 1.0E-5F`, rounded like vanilla, no fused multiply-add.
        while x * x + z * z < f64::from(1.0e-5f32) {
            let mut random = self.random();
            x = (random.next_f64() - random.next_f64()) * 0.01;
            z = (random.next_f64() - random.next_f64()) * 0.01;
        }

        let var8 = Vector3::new(x, 0.0, z).normalize() * strength;

        let velocity = self.velocity.load();

        self.velocity.store(Vector3::new(
            velocity.x / 2.0 - var8.x,
            if self.on_ground.load(Relaxed) {
                (velocity.y / 2.0 + strength).min(0.4)
            } else {
                velocity.y
            },
            velocity.z / 2.0 - var8.z,
        ));
    }

    // Part of LivingEntity.tickMovement() in yarn

    pub fn check_zero_velo(&self) {
        let mut motion = self.velocity.load();

        if self.entity_type == &EntityType::PLAYER {
            if motion.horizontal_length_squared() < 9.0E-6 {
                motion.x = 0.0;

                motion.z = 0.0;
            }
        } else {
            if motion.x.abs() < 0.003 {
                motion.x = 0.0;
            }

            if motion.z.abs() < 0.003 {
                motion.z = 0.0;
            }
        }

        if motion.y.abs() < 0.003 {
            motion.y = 0.0;
        }

        self.velocity.store(motion);
    }

    #[expect(dead_code)]
    const fn tick_block_underneath() {
        // let world = self.world.read();

        // let (pos, block, state) = self.get_block_with_y_offset(0.2);

        // world
        //     .block_registry
        //     .on_stepped_on(&world, caller, pos, block, state)
        //     ;

        // TODO: Add this to on_stepped_on

        /*


        if self.on_ground.load(Ordering::SeqCst) {


            let (_pos, block, state) = self.get_block_with_y_offset(0.2);


            if let Some(live) = living {


                if block == Block::CAMPFIRE


                    || block == Block::SOUL_CAMPFIRE


                        && CampfireLikeProperties::from_state_id(state.id).r#signal_fire


                {


                    let _ = live.damage(1.0, DamageType::CAMPFIRE);


                }





                if block == Block::MAGMA_BLOCK {


                    let _ = live.damage(1.0, DamageType::HOT_FLOOR);


                }


            }


        }


        */
    }

    /// Vanilla `Entity.isInWall` (for the caller's suffocation damage) followed by
    /// `applyEffectsFromBlocks`.
    pub fn tick_block_collisions(&self, caller: &dyn EntityBase) -> bool {
        let suffocating = self.is_in_wall();
        self.apply_effects_from_blocks(caller);
        suffocating
    }

    /// Vanilla `Entity.addMovementThisTick`: past 100 moves the first two merge.
    fn add_movement_this_tick(&self, movement: EntityMovement) {
        let mut movements = self
            .movement_this_tick
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if movements.len() >= 100 {
            let first = movements.remove(0);
            let second = movements.remove(0);
            movements.insert(0, EntityMovement::new(first.from, second.to, None));
        }
        movements.push(movement);
    }

    /// The movement recording of vanilla `Entity.move`, for a move made outside
    /// [`Self::move_entity`], such as a player's own movement packet.
    pub fn record_movement(&self, from: Vector3<f64>, to: Vector3<f64>, delta: Vector3<f64>) {
        self.add_movement_this_tick(EntityMovement::new(from, to, Some(delta)));
    }

    /// The moves recorded so far this tick, for [`Self::set_movement_this_tick`].
    pub fn movement_this_tick(&self) -> Vec<EntityMovement> {
        self.movement_this_tick
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Puts back moves saved with [`Self::movement_this_tick`], dropping any recorded since.
    pub fn set_movement_this_tick(&self, movements: Vec<EntityMovement>) {
        *self
            .movement_this_tick
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = movements;
    }

    /// Vanilla `Entity.removeLatestMovementRecording`.
    pub fn remove_latest_movement_recording(&self) {
        self.movement_this_tick
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop();
    }

    /// Vanilla `Entity.clearMovementThisTick`.
    pub fn clear_movement_this_tick(&self) {
        self.movement_this_tick
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// Vanilla `Entity.applyEffectsFromBlocks()`: sweeps this tick's moves, or the move from
    /// the old position when there were none.
    pub fn apply_effects_from_blocks(&self, caller: &dyn EntityBase) {
        let mut movements = std::mem::take(
            &mut *self
                .movement_this_tick
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let pos = self.pos.load();
        match movements.last() {
            None => movements.push(EntityMovement::new(self.last_pos.load(), pos, None)),
            Some(last) if last.to.squared_distance_to_vec(&pos) > f64::from(9.999_999_4E-11f32) => {
                movements.push(EntityMovement::new(last.to, pos, None));
            }
            Some(_) => {}
        }
        (*self
            .final_movements_this_tick
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
        .clone_from(&movements);
        self.apply_effects_from_movements(caller, &movements);
    }

    /// Vanilla `Entity.applyEffectsFromBlocksForLastMovements`.
    pub fn apply_effects_from_blocks_for_last_movements(&self, caller: &dyn EntityBase) {
        let movements = self
            .final_movements_this_tick
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        self.apply_effects_from_movements(caller, &movements);
    }

    /// Vanilla `Entity.applyEffectsFromBlocks(Vec3, Vec3)`.
    pub fn apply_effects_from_blocks_between(
        &self,
        caller: &dyn EntityBase,
        from: Vector3<f64>,
        to: Vector3<f64>,
    ) {
        self.apply_effects_from_movements(caller, &[EntityMovement::new(from, to, None)]);
    }

    /// Vanilla `Entity.applyEffectsFromBlocks(List)`.
    fn apply_effects_from_movements(&self, caller: &dyn EntityBase, movements: &[EntityMovement]) {
        if !self.is_affected_by_blocks() {
            return;
        }
        let world = self.world.load_full();
        if self.on_ground.load(Ordering::Relaxed) {
            let effect_pos = self.get_on_pos_legacy();
            let (block, state) = world.get_block_and_state(&effect_pos);
            world
                .block_registry
                .on_entity_step(block, &world, caller, &effect_pos, state);
        }

        let was_on_fire = self.is_on_fire();
        let was_freezing = self.is_freezing();
        let previous_fire_ticks = self.fire_ticks.load(Ordering::Relaxed);
        let effect_applier = InsideBlockEffectApplier::default();
        self.check_inside_blocks(caller, movements, &effect_applier);
        effect_applier.apply_and_clear(caller);
        if self.is_in_rain() {
            self.extinguish();
        }

        if (was_on_fire && !self.is_on_fire()) || (was_freezing && !self.is_freezing()) {
            caller.play_entity_on_fire_extinguished_sound();
        }

        let ignited_this_tick = self.fire_ticks.load(Ordering::Relaxed) > previous_fire_ticks;
        if !self.is_on_fire() && !ignited_this_tick {
            caller.set_remaining_fire_ticks(-caller.fire_immune_ticks());
        }
    }

    /// Vanilla `Entity.checkInsideBlocks(List, StepBasedCollector)`.
    fn check_inside_blocks(
        &self,
        caller: &dyn EntityBase,
        movements: &[EntityMovement],
        effect_applier: &InsideBlockEffectApplier,
    ) {
        if !self.is_affected_by_blocks() {
            return;
        }
        let mut visited_blocks = HashSet::new();
        for movement in movements {
            let mut pos = movement.from;
            let delta = movement.to - movement.from;
            let mut max_movement_iterations = 16;
            if let Some(original) = movement.axis_dependent_original_movement
                && delta.length_squared() > 0.0
            {
                for axis in Axis::step_order(original) {
                    let axis_move = delta.get_axis(axis);
                    if axis_move != 0.0 {
                        let mut to = pos;
                        to.set_axis(axis, pos.get_axis(axis) + axis_move);
                        max_movement_iterations -= self.check_inside_blocks_between(
                            caller,
                            pos,
                            to,
                            effect_applier,
                            &mut visited_blocks,
                            max_movement_iterations,
                        );
                        pos = to;
                    }
                }
            } else {
                max_movement_iterations -= self.check_inside_blocks_between(
                    caller,
                    movement.from,
                    movement.to,
                    effect_applier,
                    &mut visited_blocks,
                    16,
                );
            }

            if max_movement_iterations <= 0 {
                self.check_inside_blocks_between(
                    caller,
                    movement.to,
                    movement.to,
                    effect_applier,
                    &mut visited_blocks,
                    1,
                );
            }
        }
    }

    /// Vanilla `Entity.checkInsideBlocks(Vec3, Vec3, ...)`: returns the sweep steps it used.
    fn check_inside_blocks_between(
        &self,
        caller: &dyn EntityBase,
        from: Vector3<f64>,
        to: Vector3<f64>,
        effect_applier: &InsideBlockEffectApplier,
        visited_blocks: &mut HashSet<i64>,
        max_movement_iterations: i32,
    ) -> i32 {
        let deflate = f64::from(1.0E-5f32);
        let target_box = BoundingBox::new_from_pos(to.x, to.y, to.z, &self.entity_dimension.load())
            .expand(-deflate, -deflate, -deflate);
        let moved_far =
            from.squared_distance_to_vec(&to) > 0.999_990_000_000_252_6 * 0.999_990_000_000_252_6;
        let world = self.world.load_full();
        let server = world.server.upgrade();
        let full_block = BoundingBox::full_block();
        let mut iterations = 0;
        for_each_block_intersected_between(from, to, &target_box, |pos, iteration| {
            if !inside_block_effect::is_alive(caller) || iteration >= max_movement_iterations {
                return false;
            }
            iterations = iteration;
            let (block, state) = world.get_block_and_state(&pos);
            if state.is_air() {
                return true;
            }
            let shape = if block == &Block::POWDER_SNOW {
                crate::block::blocks::powder_snow::inside_collision_shape_for_entity(caller, &pos)
            } else {
                world
                    .block_registry
                    .get_inside_collision_shape(block, &world, state, &pos)
            };
            // `intersectShape == Shapes.block()`.
            let inside_block = (shape.min == full_block.min && shape.max == full_block.max)
                || self.collided_with_shape_moving_from(from, to, [shape.at_pos(pos)]);
            let (fluid, fluid_state) = world.get_fluid_and_fluid_state(&pos);
            let inside_fluid = self.collided_with_fluid(fluid, &fluid_state, &pos, from, to);
            if (inside_block || inside_fluid) && visited_blocks.insert(pos.as_long()) {
                if inside_block && let Some(server) = &server {
                    let is_precise =
                        moved_far || target_box.intersects(&BoundingBox::from_block(&pos));
                    effect_applier.advance_step(iteration);
                    world.block_registry.on_entity_collision(
                        block,
                        &world,
                        caller,
                        &pos,
                        state,
                        server,
                        effect_applier,
                        is_precise,
                    );
                    // Vanilla `onInsideBlock`, which only `ServerPlayer` overrides.
                    if let Some(player) = caller.get_player() {
                        player.trigger_advancement(
                            crate::entity::player::advancement::trigger::AdvancementTrigger::EnterBlock {
                                block,
                            },
                        );
                    }
                }
                if inside_fluid {
                    effect_applier.advance_step(iteration);
                    world
                        .block_registry
                        .on_entity_collision_fluid(fluid, caller, effect_applier);
                }
            }
            true
        });
        iterations + 1
    }

    pub fn update_last_pos(&self) -> Vector3<f64> {
        let pos = self.pos.load();
        let old = self.last_pos.load();
        self.movement.store(pos - old);
        self.last_pos.store(pos);
        old
    }

    // updateWaterState() in yarn

    /// Vanilla `Entity.updateFluidInteraction`. The fluids' own effects come from the
    /// block sweep after the move.
    fn update_fluid_state(&self, caller: &dyn EntityBase) {
        self.update_fluid_interaction(caller);
    }

    /// Vanilla `Entity.updateFluidInteraction`, with `EntityFluidInteraction.update`: the
    /// fluid heights, eyes and current push. Returns whether the entity is in any fluid.
    fn update_fluid_interaction(&self, caller: &dyn EntityBase) -> bool {
        let pushed = caller.is_pushed_by_fluids();
        let world = self.world.load();
        // Index 0 is `#water`, 1 is `#lava`.
        let mut height = [0.0f64; 2];
        let mut eyes_inside = [false; 2];
        let mut current = [FluidCurrent::default(); 2];
        let mut in_any_fluid = false;

        if let Some(fluid_box) = self.fluid_interaction_box() {
            let x0 = fluid_box.min.x.floor() as i32;
            let y0 = fluid_box.min.y.floor() as i32;
            let z0 = fluid_box.min.z.floor() as i32;
            let x1 = fluid_box.max.x.ceil() as i32 - 1;
            let y1 = fluid_box.max.y.ceil() as i32 - 1;
            let z1 = fluid_box.max.z.ceil() as i32 - 1;
            // `hasFluidAndLoaded`: every chunk around the box has to be loaded.
            let loaded = (get_section_cord(z0 - 1)..=get_section_cord(z1 + 1)).all(|chunk_z| {
                (get_section_cord(x0 - 1)..=get_section_cord(x1 + 1)).all(|chunk_x| {
                    world
                        .level
                        .read_chunk_sync(&Vector2::new(chunk_x, chunk_z), |_| ())
                        .is_some()
                })
            });
            if loaded {
                let entity_y = self.bounding_box.load().min.y;
                let pos = self.pos.load();
                let eye_block_x = pos.x.floor() as i32;
                let eye_y = self.get_eye_y();
                let eye_block_z = pos.z.floor() as i32;
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        for z in z0..=z1 {
                            let block_pos = BlockPos::new(x, y, z);
                            let (fluid, state) = world.get_fluid_and_fluid_state(&block_pos);
                            if state.is_empty {
                                continue;
                            }
                            let fluid_bottom = f64::from(y);
                            let fluid_top = fluid_bottom
                                + f64::from(world.get_fluid_height(&block_pos, fluid, &state));
                            if fluid_top < fluid_box.min.y {
                                continue;
                            }
                            in_any_fluid = true;
                            let Some(i) = fluid_current_index(fluid) else {
                                continue;
                            };
                            if x == eye_block_x && z == eye_block_z && eye_y >= fluid_bottom {
                                let fluid_top_for_camera = fluid_bottom
                                    + f64::from(Self::fluid_height_for_camera(
                                        &world, &block_pos, fluid, &state,
                                    ));
                                if eye_y <= fluid_top_for_camera {
                                    eyes_inside[i] = true;
                                }
                            }
                            height[i] = height[i].max(fluid_top - entity_y);
                            if pushed {
                                let mut flow = world.get_fluid_velocity(block_pos, fluid, &state);
                                current[i].height = height[i].max(current[i].height);
                                if current[i].height < 0.4 {
                                    flow = flow * current[i].height;
                                }
                                current[i].accumulated += flow;
                                current[i].count += 1;
                            }
                        }
                    }
                }
            }
        }

        let in_water = height[0] > 0.0;
        let in_lava = height[1] > 0.0;
        self.water_height.store(height[0]);
        self.lava_height.store(height[1]);
        self.eye_in_water.store(eyes_inside[0], Ordering::Relaxed);
        if in_water {
            self.reset_fall_distance();
            if !self.touching_water.load(Ordering::SeqCst)
                && !self.first_tick.load(Ordering::Relaxed)
            {
                self.do_water_splash_effect(caller);
            }
        }
        self.touching_water.store(in_water, Ordering::SeqCst);
        self.touching_lava.store(in_lava, Ordering::SeqCst);

        if pushed {
            if in_water {
                current[0].apply_to(self, WATER_FLOW_SCALE);
            }
            if in_lava {
                let lava_flow_scale = if world.dimension.fast_lava {
                    LAVA_FAST_FLOW_SCALE
                } else {
                    LAVA_SLOW_FLOW_SCALE
                };
                current[1].apply_to(self, lava_flow_scale);
            }
        }
        in_any_fluid
    }

    /// Vanilla `Entity.getFluidInteractionBox`: `None` when a vehicle keeps its passenger out
    /// of the fluid.
    fn fluid_interaction_box(&self) -> Option<BoundingBox> {
        let fluid_box = self.bounding_box.load().expand(-0.001, -0.001, -0.001);
        let vehicle = self
            .vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        vehicle.map_or(Some(fluid_box), |vehicle| {
            vehicle.modify_passenger_fluid_interaction_box(fluid_box)
        })
    }

    /// Vanilla `FluidState.getHeightForCamera`: a source under a sturdy face fills its block.
    fn fluid_height_for_camera(
        world: &World,
        pos: &BlockPos,
        fluid: &Fluid,
        state: &FluidState,
    ) -> f32 {
        if state.is_source
            && world
                .get_block_state(&pos.up())
                .is_side_solid(BlockDirection::Down)
        {
            return 1.0;
        }
        world.get_fluid_height(pos, fluid, state)
    }

    fn get_pos_with_y_offset(
        &self,
        offset: f64,
    ) -> (
        BlockPos,
        Option<&'static Block>,
        Option<&'static BlockState>,
    ) {
        if let Some(mut supporting_block) = self.supporting_block_pos.load() {
            if offset > f64::from(1.0e-5f32) {
                // Vanilla `Entity.getOnPos(offset)`: fences (for offsets up to 0.5), walls and
                // fence gates are tall, so the entity stands on them, not on the block below.
                let (block, state) = self.world.load().get_block_and_state(&supporting_block);
                if (offset <= 0.5 && block.has_tag(&tag::Block::MINECRAFT_FENCES))
                    || block.has_tag(&tag::Block::MINECRAFT_WALLS)
                    || block.has_tag(&tag::Block::MINECRAFT_FENCE_GATES)
                {
                    return (supporting_block, Some(block), Some(state));
                }

                supporting_block.0.y = (self.pos.load().y - offset).floor() as i32;
            }

            return (supporting_block, None, None);
        }

        let mut block_pos = self.block_pos.load();

        block_pos.0.y = (self.pos.load().y - offset).floor() as i32;

        (block_pos, None, None)
    }

    fn get_block_with_y_offset(
        &self,
        offset: f64,
    ) -> (BlockPos, &'static Block, &'static BlockState) {
        let (pos, block, state) = self.get_pos_with_y_offset(offset);

        if let (Some(b), Some(s)) = (block, state) {
            (pos, b, s)
        } else {
            let (b, s) = self.world.load().get_block_and_state(&pos);

            (pos, b, s)
        }
    }

    /// Vanilla `Entity.getOnPosLegacy`: the block 0.2 under the feet.
    #[must_use]
    pub fn get_on_pos_legacy(&self) -> BlockPos {
        self.get_pos_with_y_offset(f64::from(0.2f32)).0
    }

    /// Vanilla `Entity.resetFallDistance`, with `ServerPlayer`'s `fall_from_height` trigger.
    pub fn reset_fall_distance(&self) {
        // A player only has a fall start to report while it is falling.
        if self.fall_distance.swap(0.0) != 0.0
            && self.entity_type == &EntityType::PLAYER
            && let Some(player) = self.world.load().get_player_by_id(self.entity_id)
        {
            player.track_fall_reset();
        }
    }

    /// Vanilla `Entity.checkFallDistanceAccumulation`: a slow fall counts as at most a block.
    pub fn check_fall_distance_accumulation(&self) {
        if self.velocity.load().y > -0.5 && self.fall_distance.load() > 1.0 {
            self.fall_distance.store(1.0);
        }
    }

    /// The start of vanilla `Entity.move`: a move of a block or more that passes through a
    /// block resetting falls (a ladder, a cobweb, water...) ends the fall.
    pub fn reset_fall_on_long_move(&self, from: Vector3<f64>, movement: Vector3<f64>) {
        let movement_length = movement.length_squared();
        if self.fall_distance.load() == 0.0 || movement_length < 1.0 {
            return;
        }
        let check_distance = movement_length.sqrt().min(8.0);
        let to = from + movement.normalize() * check_distance;
        let hit = self
            .world
            .load()
            .clip(&pumpkin_world::clip::ClipContext::new(
                from,
                to,
                pumpkin_world::clip::ClipBlock::FallDamageResetting,
                pumpkin_world::clip::ClipFluid::Water,
            ));
        if !hit.is_miss {
            self.reset_fall_distance();
        }
    }

    /// Vanilla `Entity.doCheckFallDamage`: `checkFallDamage` for a move the client made,
    /// unless the entity is next to an unloaded chunk.
    pub fn do_check_fall_damage(
        &self,
        caller: &dyn EntityBase,
        movement: Vector3<f64>,
        on_ground: bool,
    ) {
        if self.touching_unloaded_chunk() {
            return;
        }
        self.check_supporting_block(caller, on_ground, Some(movement));
        let pos = self.get_on_pos_legacy();
        let state = self.world.load().get_block_state(&pos);
        caller.check_fall_damage(caller, movement.y, on_ground, state, pos);
    }

    /// Vanilla `Entity.touchingUnloadedChunk`: a chunk within a block of the box isn't loaded.
    #[must_use]
    pub fn touching_unloaded_chunk(&self) -> bool {
        let bounding_box = self.bounding_box.load().expand(1.0, 1.0, 1.0);
        let world = self.world.load();
        // `Level.hasChunksAt(x0, z0, x1, z1)`.
        let (x0, x1) = (
            get_section_cord(bounding_box.min.x.floor() as i32),
            get_section_cord(bounding_box.max.x.ceil() as i32),
        );
        let (z0, z1) = (
            get_section_cord(bounding_box.min.z.floor() as i32),
            get_section_cord(bounding_box.max.z.ceil() as i32),
        );
        !(x0..=x1).all(|chunk_x| {
            (z0..=z1).all(|chunk_z| {
                world
                    .level
                    .read_chunk_sync(&Vector2::new(chunk_x, chunk_z), |_| ())
                    .is_some()
            })
        })
    }

    /// Vanilla `Entity.checkFallDamage`.
    pub fn check_fall_damage_default(
        &self,
        caller: &dyn EntityBase,
        ya: f64,
        on_ground: bool,
        state: &'static BlockState,
        pos: BlockPos,
    ) {
        if !self.touching_water.load(Ordering::SeqCst) && ya < 0.0 {
            // `fallDistance -= (float)ya`.
            self.fall_distance
                .store(self.fall_distance.load() - f64::from(ya as f32));
        }
        if on_ground {
            let fall_distance = self.fall_distance.load();
            if fall_distance > 0.0 {
                let world = self.world.load();
                world
                    .block_registry
                    .fall_on(&world, state, &pos, caller, fall_distance);
                // `Context.of(this, mainSupportingBlockPos.map(getBlockState).orElse(onState))`.
                let supporting_state = self
                    .get_supporting_block_pos()
                    .map_or(state, |supporting| world.get_block_state(&supporting));
                world.game_event(
                    pumpkin_data::game_event::GameEvent::HitGround,
                    self.pos.load(),
                    crate::world::game_event::GameEventContext::of(
                        Some(caller),
                        Some(supporting_state),
                    ),
                );
            }
            self.reset_fall_distance();
        }
    }

    /// Vanilla `Entity.causeFallDamage`: no damage, but the passengers take the fall.
    pub fn cause_fall_damage_default(
        &self,
        fall_distance: f64,
        damage_modifier: f32,
        damage_type: DamageType,
    ) -> bool {
        if self
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_FALL_DAMAGE_IMMUNE)
        {
            return false;
        }
        self.propagate_fall_to_passengers(fall_distance, damage_modifier, damage_type);
        false
    }

    /// Vanilla `Entity.propagateFallToPassengers`.
    pub fn propagate_fall_to_passengers(
        &self,
        fall_distance: f64,
        damage_modifier: f32,
        damage_type: DamageType,
    ) {
        let passengers = self
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for passenger in passengers {
            passenger.cause_fall_damage(
                passenger.as_ref(),
                fall_distance,
                damage_modifier,
                damage_type,
            );
        }
    }

    /// Vanilla `getBlockStateOnLegacy`: the block 0.2 under the feet.
    pub fn get_block_state_on_legacy(&self) -> &'static BlockState {
        self.get_block_with_y_offset(f64::from(0.2f32)).2
    }

    // Entity.updateVelocity in yarn

    pub(crate) fn update_velocity_from_input(&self, movement_input: Vector3<f64>, speed: f64) {
        let final_input = self.movement_input_to_velocity(movement_input, speed);

        self.velocity.store(self.velocity.load() + final_input);
    }

    /// Vanilla `Entity.getInputVector`: the input, capped at length 1, scaled by `speed` and
    /// turned by the yaw through the `Mth` sine table. `speed` is a float in vanilla.
    fn movement_input_to_velocity(&self, movement_input: Vector3<f64>, speed: f64) -> Vector3<f64> {
        let length = movement_input.length_squared();
        if length < 1.0E-7 {
            return Vector3::default();
        }
        let input = if length > 1.0 {
            // `Vec3.normalize`.
            let norm = length.sqrt();
            if norm < f64::from(1.0E-5f32) {
                Vector3::default()
            } else {
                Vector3::new(
                    movement_input.x / norm,
                    movement_input.y / norm,
                    movement_input.z / norm,
                )
            }
        } else {
            movement_input
        };
        let speed = f64::from(speed as f32);
        let movement = Vector3::new(input.x * speed, input.y * speed, input.z * speed);
        let yaw_radians = self.yaw.load() * (std::f64::consts::PI / 180.0) as f32;
        let sin = f64::from(pumpkin_util::math::sin(yaw_radians));
        let cos = f64::from(pumpkin_util::math::cos(yaw_radians));
        Vector3::new(
            movement.x * cos - movement.z * sin,
            movement.y,
            movement.z * cos + movement.x * sin,
        )
    }

    /// Vanilla `Entity.getOnPos()`.
    #[must_use]
    pub fn get_on_pos(&self) -> BlockPos {
        self.get_pos_with_y_offset(f64::from(1.0e-5f32)).0
    }

    #[must_use]
    pub fn get_block_pos_below_that_affects_my_movement(&self) -> BlockPos {
        self.get_pos_with_y_offset(DELTA_AFFECTED_BY_BLOCKS_BELOW_0_5)
            .0
    }

    #[must_use]
    #[expect(clippy::float_cmp)]
    pub fn get_block_speed_factor(&self) -> f32 {
        let world = self.world.load();
        let (block, _state) = world.get_block_and_state(&self.block_pos.load());
        let speed_factor_here = block.get_speed_factor();
        if block != &Block::WATER && block != &Block::BUBBLE_COLUMN {
            if speed_factor_here == 1.0 {
                let below_pos = self.get_block_pos_below_that_affects_my_movement();
                let (below_block, _below_state) = world.get_block_and_state(&below_pos);
                below_block.get_speed_factor()
            } else {
                speed_factor_here
            }
        } else {
            speed_factor_here
        }
    }

    #[expect(clippy::float_cmp)]
    fn get_jump_velocity_multiplier(&self) -> f32 {
        let f = self
            .world
            .load()
            .get_block(&self.block_pos.load())
            .jump_velocity_multiplier;

        let g = self
            .get_block_with_y_offset(DELTA_AFFECTED_BY_BLOCKS_BELOW_0_5)
            .1
            .jump_velocity_multiplier;

        if f == 1f32 { g } else { f }
    }

    pub fn move_pos(&self, delta: Vector3<f64>) {
        self.set_pos(self.pos.load() + delta);
    }

    // Move by a delta, adjust for collisions, and send

    // Does not send movement. That must be done separately
    pub fn move_entity(&self, caller: &dyn EntityBase, motion: Vector3<f64>) {
        self.move_with_type(caller, motion, false);
    }

    /// Vanilla `PistonMovingBlockEntity.moveEntityByPiston`: `move(MoverType.PISTON)`, then
    /// the blocks along just that move.
    pub fn move_by_piston(&self, caller: &dyn EntityBase, delta: Vector3<f64>) {
        let previous_pos = self.pos.load();
        self.move_with_type(caller, delta, true);
        self.apply_effects_from_blocks_between(caller, previous_pos, self.pos.load());
        self.remove_latest_movement_recording();
    }

    /// Vanilla `Entity.limitPistonMovement`: pistons move an entity at most 0.51 per axis
    /// in a game tick, along one axis per move.
    fn limit_piston_movement(&self, movement: Vector3<f64>) -> Vector3<f64> {
        if movement.length_squared() <= 1.0E-7 {
            return movement;
        }
        let game_time = self.world.load().get_world_age();
        let mut piston = self
            .piston_deltas
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if piston.1 != game_time {
            *piston = ([0.0; 3], game_time);
        }
        let mut restrict = |index: usize, amount: f64| {
            let min = (amount + piston.0[index]).clamp(-0.51, 0.51);
            let amount = min - piston.0[index];
            piston.0[index] = min;
            amount
        };
        let threshold = f64::from(1.0E-5f32);
        if movement.x != 0.0 {
            let x = restrict(0, movement.x);
            if x.abs() <= threshold {
                Vector3::default()
            } else {
                Vector3::new(x, 0.0, 0.0)
            }
        } else if movement.y != 0.0 {
            let y = restrict(1, movement.y);
            if y.abs() <= threshold {
                Vector3::default()
            } else {
                Vector3::new(0.0, y, 0.0)
            }
        } else if movement.z != 0.0 {
            let z = restrict(2, movement.z);
            if z.abs() <= threshold {
                Vector3::default()
            } else {
                Vector3::new(0.0, 0.0, z)
            }
        } else {
            Vector3::default()
        }
    }

    /// Vanilla `Entity.move`; `piston` is `MoverType.PISTON`.
    fn move_with_type(&self, caller: &dyn EntityBase, mut motion: Vector3<f64>, piston: bool) {
        // A player only moves here in the server's own copy of its travel, which
        // `Player::tick_server_travel` puts back where the client says the player is.
        let is_player = caller.get_player().is_some();

        // `Player.tick` sets `noPhysics` for spectators. `onGround` is left as it was.
        if self.no_physics.load(Ordering::Relaxed) || caller.is_spectator() {
            self.move_pos(motion);
            self.horizontal_collision.store(false, Ordering::Relaxed);
            self.vertical_collision_below
                .store(false, Ordering::Relaxed);
            return;
        }

        if piston {
            motion = self.limit_piston_movement(motion);
            if motion == Vector3::default() {
                return;
            }
        }

        let movement_multiplier = self.movement_multiplier.swap(Vector3::default());

        if movement_multiplier.length_squared() > 1.0e-7 {
            // A piston's push isn't slowed by cobwebs or powder snow.
            if !piston {
                motion = motion.multiply(
                    movement_multiplier.x,
                    movement_multiplier.y,
                    movement_multiplier.z,
                );
            }

            self.velocity.store(Vector3::default());
        }

        let final_move = self.adjust_movement_for_collisions(motion, caller);
        #[expect(
            clippy::float_cmp,
            reason = "vanilla compares the collided movement exactly"
        )]
        let vertical_collision = motion.y != final_move.y;
        self.vertical_collision_below
            .store(vertical_collision && motion.y < 0.0, Ordering::Relaxed);

        // Vanilla `Entity.move` leaves out a negligible move unless collisions cut it short.
        let movement_length = final_move.length_squared();
        if movement_length > 1.0E-7 || motion.length_squared() - movement_length < 1.0E-7 {
            let pos = self.pos.load();
            self.reset_fall_on_long_move(pos, final_move);
            let new_position = pos + final_move;
            self.add_movement_this_tick(EntityMovement::new(pos, new_position, Some(motion)));
            self.set_pos(new_position);
        }

        // Vanilla `setOnGroundWithMovement`: `move` runs it when the entity moved vertically
        // or, as for every server-side mob here, when this side is authoritative.
        self.check_supporting_block(
            caller,
            self.on_ground.load(Ordering::SeqCst),
            Some(final_move),
        );

        // The block under the feet, as it was before the fall damage could change it.
        let effect_pos = self.get_on_pos_legacy();
        let effect_state = self.world.load().get_block_state(&effect_pos);
        // `checkFallDamage` only runs on the authoritative side, the client for a player or a
        // vehicle a player steers.
        if !caller.is_client_authoritative() {
            caller.check_fall_damage(
                caller,
                final_move.y,
                self.on_ground.load(Ordering::SeqCst),
                effect_state,
                effect_pos,
            );
        }

        if self.is_removed() {
            return;
        }

        if caller.can_simulate_movement() {
            self.restitute_movement_after_collisions(caller, effect_state, motion, final_move);
        }

        // A player's steps are played from its movement packets instead.
        if !is_player {
            self.apply_movement_emission_and_play_sound(
                caller,
                final_move,
                effect_pos,
                effect_state,
            );
        }

        self.apply_block_speed_factor(caller);
    }

    /// The end of vanilla `Entity.move`: the block's speed factor slows the horizontal velocity.
    pub fn apply_block_speed_factor(&self, caller: &dyn EntityBase) {
        let block_speed_factor = f64::from(caller.get_block_speed_factor());
        let velocity = self.velocity.load();
        self.velocity.store(Vector3::new(
            velocity.x * block_speed_factor,
            velocity.y,
            velocity.z * block_speed_factor,
        ));
    }

    /// Vanilla `restituteMovementAfterCollisions`: stops (or bounces) the movement along
    /// each axis the move of `delta` collided on, ending up at `movement`.
    fn restitute_movement_after_collisions(
        &self,
        caller: &dyn EntityBase,
        effect_state: &'static BlockState,
        delta: Vector3<f64>,
        movement: Vector3<f64>,
    ) {
        // `Mth.equal`.
        let x_collision = (movement.x - delta.x).abs() >= f64::from(1.0e-5f32);
        let z_collision = (movement.z - delta.z).abs() >= f64::from(1.0e-5f32);
        let vertical_collision = delta.y.abs() > 0.0 && delta.y != movement.y;
        let vertical_collision_below = vertical_collision && delta.y < 0.0;
        if !vertical_collision && !x_collision && !z_collision {
            return;
        }
        let effect_block = Block::from_state_id(effect_state.id);
        let living = caller.get_living_entity();
        let suppressing_bounce = self.is_sneaking();
        // `getEntityBounciness`: the BOUNCINESS attribute for living entities.
        let mut restitution = if suppressing_bounce {
            0.0
        } else {
            living.map_or(0.0, |living| {
                living.get_attribute_value(&pumpkin_data::attributes::Attributes::BOUNCINESS)
            })
        };
        let current = self.velocity.load();
        let mut after = current;
        if x_collision {
            after.x = -current.x * restitution;
        }
        if z_collision {
            after.z = -current.z * restitution;
        }
        let mut bounced = restitution > 0.0 && (x_collision || z_collision);
        if vertical_collision {
            let gravity = living.map_or_else(
                || caller.current_gravity(),
                |living| living.get_effective_gravity(caller),
            );
            if vertical_collision_below {
                restitution = if -current.y > gravity
                    && !suppressing_bounce
                    && !effect_block.has_tag(&tag::Block::MINECRAFT_SUPPRESSES_BOUNCE)
                {
                    // `getBlockBounciness`: non-living entities keep 80% of it.
                    let mut block_bounciness = crate::block::bounce_restitution(effect_block);
                    if living.is_none() {
                        block_bounciness *= 0.8;
                    }
                    restitution.max(f64::from(block_bounciness))
                } else {
                    0.0
                };
            }
            let (gravity_compensation, effective_drag) = if restitution > 0.0 {
                let portion_with_movement = movement.y / current.y;
                let air_drag = living.map_or(0.98f32, |living| living.get_air_drag(caller));
                bounced = true;
                (
                    portion_with_movement * gravity,
                    1.0 + portion_with_movement * (f64::from(air_drag) - 1.0),
                )
            } else {
                (0.0, 1.0)
            };
            after.y = (gravity_compensation - current.y) * effective_drag * restitution;
        }
        if bounced {
            let world = self.world.load_full();
            let pos = self.get_block_pos_below_that_affects_my_movement();
            world
                .block_registry
                .bounce_on(&world, effect_block, &pos, caller);
            world.game_event_from(
                Some(caller),
                pumpkin_data::game_event::GameEvent::Bounce,
                self.pos.load(),
            );
            self.sync_position.store(true, Ordering::Relaxed);
        }
        self.velocity.store(after);
    }

    /// Vanilla `Entity.moveTowardsClosestSpace`: pushed out through the nearest open side,
    /// trying north, south, west, east and up in that order.
    pub fn push_out_of_blocks(&self, center_pos: Vector3<f64>) {
        let block_pos = BlockPos::floored_v(center_pos);
        let delta = center_pos.sub(&block_pos.0.to_f64());
        let world = self.world.load();
        let mut closest = f64::MAX;
        let mut direction = BlockDirection::Up;

        for dir in [
            BlockDirection::North,
            BlockDirection::South,
            BlockDirection::West,
            BlockDirection::East,
            BlockDirection::Up,
        ] {
            let neighbor_pos = block_pos.offset(dir.to_offset());
            let mut shapes = world
                .get_block_state(&neighbor_pos)
                .get_block_collision_shapes_at(&neighbor_pos);
            // `isCollisionShapeFullBlock`.
            let full_block = BoundingBox::full_block();
            let is_full_block = shapes
                .next()
                .is_some_and(|shape| shape.min == full_block.min && shape.max == full_block.max)
                && shapes.next().is_none();
            if is_full_block {
                continue;
            }
            let component = delta.get_axis(dir.to_axis().into());
            let oriented_delta = if dir.positive() {
                1.0 - component
            } else {
                component
            };
            if oriented_delta < closest {
                closest = oriented_delta;
                direction = dir;
            }
        }

        // `float speed = this.random.nextFloat() * 0.2F + 0.1F`, in float.
        let speed = f64::from(self.random().next_f32() * 0.2 + 0.1);
        let step = if direction.positive() { 1.0 } else { -1.0 };
        let mut velo = self.velocity.load() * 0.75;
        velo.set_axis(direction.to_axis().into(), step * speed);
        self.velocity.store(velo);
    }

    /// Vanilla `Entity.ejectPassengers`: each passenger stops riding, the last first.
    pub fn eject_passengers(&self) {
        let passengers = self
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for passenger in passengers.iter().rev() {
            self.remove_passenger(passenger.get_entity().entity_id);
        }
    }

    /// The base of vanilla `Entity.canTeleport` (`EntityBase::can_teleport`'s default), for
    /// overrides that call `super`.
    pub fn can_teleport_base(&self, from: &World, to: &World) -> bool {
        if from.dimension == Dimension::THE_END && to.dimension == Dimension::OVERWORLD {
            return !self
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .any(|passenger| {
                    passenger
                        .get_player()
                        .is_some_and(|player| !player.seen_credits.load(Ordering::Relaxed))
                });
        }
        true
    }

    /// Vanilla `Entity.handlePortal`: the destination is searched for, and the entity changes
    /// place, inside its own tick.
    pub(crate) fn handle_portal(&self, caller: &dyn EntityBase) {
        // `processPortalCooldown`.
        if self.portal_cooldown.load(Ordering::Relaxed) > 0 {
            self.portal_cooldown.fetch_sub(1, Ordering::Relaxed);
        }
        let world = self.world.load_full();
        // Never held across the destination search: its chunk wait (`block_on_tick`) runs other
        // pool jobs on this thread, and any of them locking this mutex (a teleport of this
        // entity, a tick of it as a new passenger) would deadlock on this very thread.
        let mut guard = self
            .portal_manager
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(processor) = guard.as_mut() else {
            return;
        };
        if processor.process_portal_teleportation(&world, caller, caller.can_use_portal(false)) {
            // `setPortalCooldown`.
            self.portal_cooldown
                .store(caller.dimension_changing_delay(), Ordering::Relaxed);
            // Vanilla reads `portalProcess` once and acts on it: the search runs on this tick's
            // copy, so nothing touched while it waits changes where the entity goes.
            let processor = processor.clone();
            drop(guard);
            let transition = processor.get_portal_destination(&world, caller);
            if let Some(transition) = transition
                && let Some(this) = world.get_entity_by_id(self.entity_id)
            {
                teleport_to_portal_destination(&this, &world, transition);
            }
        } else if processor.has_expired() {
            *guard = None;
        }
    }

    /// Vanilla `Entity.setAsInsidePortal`, for the portal block of `portal`'s type.
    pub fn try_use_portal(
        &self,
        caller: &dyn EntityBase,
        portal: PortalType,
        portal_world: &World,
        pos: BlockPos,
    ) {
        let mut portal_event =
            crate::plugin::api::events::entity::entity_portal::EntityPortalEvent::new(
                self.entity_id,
                pos,
            );
        if let Some(server) = self.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut portal_event);
        }
        if portal_event.cancelled {
            return;
        }

        // Vanilla `setAsInsidePortal`: a portal touched during the cooldown restarts it.
        if self.portal_cooldown.load(Ordering::Relaxed) > 0 {
            self.portal_cooldown
                .store(caller.dimension_changing_delay(), Ordering::Relaxed);
            return;
        }

        let Some(server) = portal_world.server.upgrade() else {
            return;
        };

        if (portal_world.dimension == Dimension::THE_NETHER && !server.basic_config.allow_nether)
            || (portal_world.dimension == Dimension::THE_END && !server.basic_config.allow_end)
        {
            return;
        }

        let mut manager = self
            .portal_manager
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let world = self.world.load();
        // `isSamePortal`: a processor of the other portal is replaced.
        if manager
            .as_ref()
            .is_none_or(|manager| !manager.is_same_portal(portal))
        {
            let mut new_manager = PortalProcessor::new(portal, pos);

            let (block, state) = world.get_block_and_state(&pos);
            let source_axis = (block == &pumpkin_data::Block::NETHER_PORTAL).then(|| {
                let props = <pumpkin_data::block_properties::NetherPortalLikeProperties as pumpkin_data::block_properties::BlockProperties>::from_state_id(state.id, block);
                props.axis
            });

            if let Some(axis) = source_axis
                && let Some(portal) = NetherPortal::get_on_axis(&world, &pos, axis)
                && portal.was_already_valid()
            {
                new_manager.set_source_portal(SourcePortalInfo {
                    lower_corner: portal.lower_corner(),
                    axis: portal.axis(),
                    width: portal.width(),
                    height: portal.height(),
                });
            }

            *manager = Some(new_manager);
        } else if let Some(manager) = manager.as_mut()
            && !manager.inside_portal_this_tick
        {
            // The first portal block touched this tick is the entry.
            manager.entry_position = pos;
            manager.inside_portal_this_tick = true;
            if manager.source_portal.is_none() {
                let (block, state) = world.get_block_and_state(&pos);
                if block == &pumpkin_data::Block::NETHER_PORTAL {
                    let props = <pumpkin_data::block_properties::NetherPortalLikeProperties as pumpkin_data::block_properties::BlockProperties>::from_state_id(state.id, block);
                    if let Some(portal) = NetherPortal::get_on_axis(&world, &pos, props.axis)
                        && portal.was_already_valid()
                    {
                        manager.set_source_portal(SourcePortalInfo {
                            lower_corner: portal.lower_corner(),
                            axis: portal.axis(),
                            width: portal.width(),
                            height: portal.height(),
                        });
                    }
                }
            }
        }
    }

    /// Vanilla `Entity.clearFire`: never raises negative (post-fire grace) ticks to 0.
    pub fn extinguish(&self) {
        let ticks = self.fire_ticks.load(Ordering::Relaxed);
        self.fire_ticks.store(ticks.min(0), Ordering::Relaxed);
    }

    /// Vanilla `Entity.extinguishFire`.
    pub fn extinguish_fire(caller: &dyn EntityBase) {
        let entity = caller.get_entity();
        if entity.is_on_fire() {
            caller.play_entity_on_fire_extinguished_sound();
        }
        entity.extinguish();
    }

    /// Vanilla `Entity.lavaIgnite`.
    pub fn lava_ignite(caller: &dyn EntityBase) {
        if !caller.get_entity().is_fire_immune() {
            caller.set_on_fire_for(15.0);
        }
    }

    /// Vanilla `Entity.lavaHurt`.
    pub fn lava_hurt(caller: &dyn EntityBase) {
        let entity = caller.get_entity();
        if entity.is_fire_immune() {
            return;
        }
        if caller.damage(caller, 4.0, DamageType::LAVA)
            && caller.should_play_lava_hurt_sound()
            && !entity.is_silent()
        {
            let pitch = 2.0 + entity.random().next_f32() * 0.4;
            entity.world.load().play_sound_fine(
                Sound::EntityGenericBurn,
                caller.entity_sound_source(),
                &entity.pos.load(),
                0.4,
                pitch,
            );
        }
    }

    /// Vanilla `Entity.isInLava`: never on the entity's first tick.
    #[must_use]
    pub fn is_in_lava(&self) -> bool {
        !self.first_tick.load(Ordering::Relaxed) && self.touching_lava.load(Ordering::Relaxed)
    }

    /// Vanilla `Entity.handleOnAboveBubbleColumn`: the column's top pushes hard and sprays.
    pub fn handle_on_above_bubble_column(caller: &dyn EntityBase, drag_down: bool, pos: BlockPos) {
        let entity = caller.get_entity();
        if caller.can_simulate_movement() {
            let movement = entity.velocity.load();
            let y = if drag_down {
                (movement.y - 0.03).max(-0.9)
            } else {
                (movement.y + 0.1).min(1.8)
            };
            entity
                .velocity
                .store(Vector3::new(movement.x, y, movement.z));
        }
        Self::send_bubble_column_particles(&entity.world.load(), pos);
    }

    /// Vanilla `Entity.sendBubbleColumnParticles`, drawing from the level random.
    fn send_bubble_column_particles(world: &World, pos: BlockPos) {
        let (x, y, z) = (
            f64::from(pos.0.x),
            f64::from(pos.0.y + 1),
            f64::from(pos.0.z),
        );
        for _ in 0..2 {
            let (splash_x, splash_z, bubble_x, bubble_z) = {
                let mut random = world
                    .random
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (
                    random.next_f64(),
                    random.next_f64(),
                    random.next_f64(),
                    random.next_f64(),
                )
            };
            world.spawn_particles(
                pumpkin_data::particle::Particle::Splash,
                Vector3::new(x + splash_x, y, z + splash_z),
                1,
                Vector3::new(0.0, 0.0, 0.0),
                1.0,
            );
            world.spawn_particles(
                pumpkin_data::particle::Particle::Bubble,
                Vector3::new(x + bubble_x, y, z + bubble_z),
                1,
                Vector3::new(0.0, 0.01, 0.0),
                0.2,
            );
        }
    }

    /// Vanilla `Entity.handleOnInsideBubbleColumn`.
    pub fn handle_on_inside_bubble_column(caller: &dyn EntityBase, drag_down: bool) {
        let entity = caller.get_entity();
        if caller.can_simulate_movement() {
            let movement = entity.velocity.load();
            let y = if drag_down {
                (movement.y - 0.03).max(-0.3)
            } else {
                (movement.y + 0.06).min(0.7)
            };
            entity
                .velocity
                .store(Vector3::new(movement.x, y, movement.z));
        }
        entity.reset_fall_distance();
    }

    /// Vanilla `Entity.clearFreeze`.
    pub fn clear_freeze(&self) {
        self.set_frozen_ticks(0);
    }

    /// Vanilla `Entity.isFreezing`.
    #[must_use]
    pub fn is_freezing(&self) -> bool {
        self.frozen_ticks.load(Ordering::Relaxed) > 0
    }

    /// Maximum freeze ticks (7 seconds at 20 tps)
    pub const MAX_FROZEN_TICKS: i32 = 140;

    /// Freeze damage is dealt every 40 ticks when fully frozen
    const FREEZE_DAMAGE_INTERVAL: i32 = 40;

    /// Check if the entity is currently in powder snow.
    ///
    /// The flag is reset at the start of each tick and set while processing
    /// block collisions for the current tick.
    pub fn is_in_powder_snow(&self) -> bool {
        self.is_in_powder_snow.load(Ordering::Relaxed)
    }

    /// Check if this entity type is immune to freezing
    pub fn is_freeze_immune(&self) -> bool {
        self.entity_type
            .has_tag(&tag::EntityType::MINECRAFT_FREEZE_IMMUNE_ENTITY_TYPES)
    }

    /// Mirrors vanilla `LivingEntity#canFreeze`: spectators and entities wearing
    /// freeze-immune wearables (e.g. leather armor) cannot freeze.
    fn can_freeze(&self, caller: &dyn EntityBase) -> bool {
        if caller.is_spectator()
            || self.is_freeze_immune()
            || caller.get_mob().is_some_and(|mob| !mob.can_freeze())
        {
            return false;
        }

        let Some(living) = caller.get_living_entity() else {
            return true;
        };

        if let Ok(equipment) = living.entity_equipment.try_lock() {
            for (slot, stack) in &equipment.equipment {
                if (*slot == EquipmentSlot::HEAD
                    || *slot == EquipmentSlot::CHEST
                    || *slot == EquipmentSlot::LEGS
                    || *slot == EquipmentSlot::FEET)
                    && stack
                        .get_item()
                        .has_tag(&tag::Item::MINECRAFT_FREEZE_IMMUNE_WEARABLES)
                {
                    return false;
                }
            }
        }

        true
    }

    /// The freezing part of vanilla `LivingEntity.aiStep`: thaws when out of powder snow (the
    /// `FREEZE` inside-block effect does the freezing), and hurts once fully frozen.
    pub fn tick_frozen(&self, caller: &dyn EntityBase) {
        let can_freeze = self.can_freeze(caller);
        if !self.is_in_powder_snow() || !can_freeze {
            let ticks = self.frozen_ticks.load(Ordering::Relaxed);
            let thawed = (ticks - 2).max(0);
            if thawed != ticks {
                self.set_frozen_ticks(thawed);
            }
        }

        if self.tick_count.load(Ordering::Relaxed) % Self::FREEZE_DAMAGE_INTERVAL == 0
            && self.get_frozen_ticks() >= Self::MAX_FROZEN_TICKS
            && can_freeze
        {
            caller.damage(caller, 1.0, DamageType::FREEZE);
        }
    }

    /// Vanilla `Entity.setTicksFrozen`, which doesn't clamp.
    pub fn set_frozen_ticks(&self, ticks: i32) {
        let new_frozen_ticks = ticks;
        self.frozen_ticks.store(new_frozen_ticks, Ordering::Relaxed);
        let mut bedrock_meta = SyncedActorDataList::new();
        bedrock_meta.set(
            entity_data_key::FREEZING_EFFECT_STRENGTH,
            MetadataValue::Float(new_frozen_ticks as f32),
        );
        self.set_synced_data(
            tracked_data::entity::DATA_TICKS_FROZEN,
            VarInt(new_frozen_ticks),
        );
        self.send_bedrock_actor_data(&bedrock_meta);
    }

    /// Returns the number of ticks the entity has been frozen.
    pub fn get_frozen_ticks(&self) -> i32 {
        self.frozen_ticks.load(Ordering::Relaxed)
    }

    /// Sets the `Entity` yaw & pitch rotation
    pub fn set_rotation(&self, yaw: f32, pitch: f32) {
        // TODO
        self.yaw.store(yaw);
        // Java players have no separate head-yaw packet from the client- look yaw is also head yaw.
        // Bedrock `MovePlayer` / `MoveActorDelta` need this field, not only `yaw`.
        self.head_yaw.store(yaw);
        self.set_pitch(pitch);
    }

    /// Vanilla `Entity.setXRot`: ignores non-finite values, wraps then clamps.
    pub fn set_pitch(&self, pitch: f32) {
        if pitch.is_finite() {
            self.pitch.store((pitch % 360.0).clamp(-90.0, 90.0));
        }
    }

    /// Removes the `Entity` from their current `World`
    pub fn remove(&self) {
        self.world.load().remove_entity(self);
    }

    /// Vanilla `ClientboundAddEntityPacket(entity, serverEntity)`: tracker, not live.
    /// Players live: tracker skips `send_changes` for them.
    fn spawn_state(&self) -> SpawnState {
        if self.entity_type == &EntityType::PLAYER {
            SpawnState {
                pos: self.pos.load(),
                velocity: self.velocity.load(),
                pitch: pack_degrees(self.pitch.load()),
                yaw: pack_degrees(self.yaw.load()),
                head_yaw: pack_degrees(self.head_yaw.load()),
            }
        } else {
            SpawnState {
                pos: self.last_sent_pos.load(),
                velocity: self.last_sent_velocity.load(),
                pitch: self.last_sent_pitch.load(Relaxed),
                yaw: self.last_sent_yaw.load(Relaxed),
                head_yaw: self.last_sent_head_yaw.load(Relaxed),
            }
        }
    }

    pub fn create_spawn_packet(&self) -> CSpawnEntity {
        let spawn = self.spawn_state();
        CSpawnEntity::new_packed(
            VarInt(self.entity_id),
            self.entity_uuid,
            VarInt(i32::from(self.entity_type.id)),
            spawn.pos,
            spawn.pitch,
            spawn.yaw,
            spawn.head_yaw, // todo: head_yaw and yaw are swapped, find out why
            self.data.load(Relaxed).into(),
            spawn.velocity,
        )
    }

    pub fn width(&self) -> f32 {
        self.entity_dimension.load().width
    }

    pub fn height(&self) -> f32 {
        self.entity_dimension.load().height
    }

    /// Applies knockback to the entity, following vanilla Minecraft's mechanics.
    ///
    /// This function calculates the entity's new velocity based on the specified knockback strength and direction.
    pub fn knockback(&self, strength: f64, x: f64, z: f64) {
        // This has some vanilla magic
        let mut x = x;
        let mut z = z;
        // `xd * xd + zd * zd < 1.0E-5F`, rounded like vanilla.
        while x * x + z * z < f64::from(1.0e-5f32) {
            let mut random = self.random();
            x = (random.next_f64() - random.next_f64()) * 0.01;
            z = (random.next_f64() - random.next_f64()) * 0.01;
        }

        let var8 = Vector3::new(x, 0.0, z).normalize() * strength;
        let velocity = self.velocity.load();
        self.velocity.store(Vector3::new(
            velocity.x / 2.0 - var8.x,
            if self.on_ground.load(Relaxed) {
                (velocity.y / 2.0 + strength).min(0.4)
            } else {
                velocity.y
            },
            velocity.z / 2.0 - var8.z,
        ));
    }

    pub fn set_sneaking(&self, sneaking: bool) {
        //assert!(self.sneaking.load(Relaxed) != sneaking);
        self.sneaking.store(sneaking, Relaxed);
        self.set_flag(Flag::Sneaking, sneaking);
    }
    pub fn is_sneaking(&self) -> bool {
        self.sneaking.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_swimming(&self) -> bool {
        self.swimming.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_visually_swimming(&self) -> bool {
        self.pose.load() == EntityPose::Swimming
    }

    #[must_use]
    pub fn is_in_water(&self) -> bool {
        self.touching_water.load(Ordering::Relaxed)
    }

    /// Vanilla `Entity.isInLiquid`.
    #[must_use]
    pub fn is_in_liquid(&self) -> bool {
        self.is_in_water() || self.is_in_lava()
    }

    /// Vanilla `Entity.collidedWithShapeMovingFrom`: whether this entity's box, moved
    /// from `from` to `to`, touches any of `boxes`.
    pub fn collided_with_shape_moving_from(
        &self,
        from: Vector3<f64>,
        to: Vector3<f64>,
        boxes: impl IntoIterator<Item = BoundingBox>,
    ) -> bool {
        BoundingBox::new_from_pos(from.x, from.y, from.z, &self.entity_dimension.load())
            .collided_along_vector(to - from, boxes)
    }

    /// Vanilla `Entity.collidedWithFluid`, with `FluidState.getAABB`.
    pub fn collided_with_fluid(
        &self,
        fluid: &Fluid,
        fluid_state: &FluidState,
        pos: &BlockPos,
        from: Vector3<f64>,
        to: Vector3<f64>,
    ) -> bool {
        if fluid_state.is_empty {
            return false;
        }
        let height = self.world.load().get_fluid_height(pos, fluid, fluid_state);
        let min = pos.0.to_f64();
        let fluid_aabb = BoundingBox::new(
            min,
            Vector3::new(min.x + 1.0, min.y + f64::from(height), min.z + 1.0),
        );
        self.collided_with_shape_moving_from(from, to, [fluid_aabb])
    }

    /// Vanilla `Entity.isInRain`.
    pub fn is_in_rain(&self) -> bool {
        let world = self.world.load();
        let pos = self.block_pos.load();
        world.is_raining_at(&pos)
            || world.is_raining_at(&BlockPos::new(
                pos.0.x,
                self.bounding_box.load().max.y.floor() as i32,
                pos.0.z,
            ))
    }

    pub fn is_in_water_or_rain(&self) -> bool {
        self.is_in_water() || self.is_in_rain()
    }

    /// Vanilla `Entity.isEyeInFluid(WATER)`, from the last fluid update.
    #[must_use]
    pub fn is_submerged_in_water(&self) -> bool {
        self.eye_in_water.load(Ordering::Relaxed)
    }

    /// Vanilla `Entity.isUnderWater`, which reads the eyes from the previous fluid update.
    #[must_use]
    pub fn is_under_water(&self) -> bool {
        self.was_eye_in_water.load(Ordering::Relaxed) && self.is_in_water()
    }

    #[must_use]
    pub fn is_visually_crawling(&self) -> bool {
        self.is_visually_swimming() && !self.is_in_water()
    }

    pub fn set_swimming(&self, swimming: bool) {
        if self.swimming.load(Ordering::Relaxed) != swimming {
            let mut event =
                crate::plugin::api::events::entity::entity_toggle_swim::EntityToggleSwimEvent::new(
                    self.entity_id,
                    swimming,
                );
            if let Some(server) = self.world.load().server.upgrade() {
                server.plugin_manager.fire_blocking(&server, &mut event);
            }
            if event.cancelled {
                return;
            }
            self.swimming.store(event.is_swimming, Relaxed);
            self.set_flag(Flag::Swimming, event.is_swimming);
        }
    }

    /// Vanilla `Entity.updateSwimming`: a sprinting entity swims while under water.
    pub fn update_swimming_base(&self) {
        let sprinting = self.is_sprinting();
        let passenger = self.has_vehicle();
        if self.is_swimming() {
            self.set_swimming(sprinting && self.is_in_water() && !passenger);
        } else {
            let in_water_block = sprinting
                && self.is_under_water()
                && !passenger
                && self
                    .world
                    .load()
                    .get_fluid(&self.block_pos.load())
                    .has_tag(&tag::Fluid::MINECRAFT_WATER);
            self.set_swimming(in_water_block);
        }
    }

    /// Sets whether the entity is invisible and sends updated metadata.
    pub fn set_invisible(&self, invisible: bool) {
        if self.invisible.load(Ordering::Relaxed) != invisible {
            self.invisible.store(invisible, Relaxed);
            self.set_flag(Flag::Invisible, invisible);
        }
    }

    /// Sets whether the entity is glowing and sends updated metadata.
    pub fn set_glowing(&self, glowing: bool) {
        if self.glowing.load(Ordering::Relaxed) != glowing {
            self.glowing.store(glowing, Ordering::Relaxed);
            self.set_flag(Flag::Glowing, glowing);
        }
    }

    /// Vanilla `Entity.setSharedFlagOnFire`: the flag also stays set while `HasVisualFire` is.
    pub fn set_on_fire(&self, on_fire: bool) {
        let value = on_fire || self.has_visual_fire.load(Ordering::Relaxed);
        let mask = 1i8 << Flag::OnFire as u8;
        if (self.flags.load(Ordering::Relaxed) & mask != 0) != value {
            self.set_flag(Flag::OnFire, value);
        }
    }

    /// Vanilla `Entity.hasVisualFire`, the permanent flames set by NBT.
    pub fn set_has_visual_fire(&self, visual_fire: bool) {
        self.has_visual_fire.store(visual_fire, Ordering::Relaxed);
        self.set_on_fire(self.fire_ticks.load(Ordering::Relaxed) > 0);
    }

    /// Vanilla `Entity.isOnFire` on the server.
    #[must_use]
    pub fn is_on_fire(&self) -> bool {
        !self.is_fire_immune() && self.fire_ticks.load(Ordering::Relaxed) > 0
    }

    /// Vanilla `Entity.getLookAngle`.
    #[must_use]
    pub fn get_look_angle(&self) -> Vector3<f64> {
        Self::calculate_view_vector(self.pitch.load(), self.yaw.load())
    }

    /// Vanilla `Entity.getForward`.
    #[must_use]
    pub fn get_forward(&self) -> Vector3<f64> {
        Self::direction_from_rotation(self.pitch.load(), self.yaw.load())
    }

    /// Vanilla `Vec3.directionFromRotation`, in vanilla's float trigonometry.
    #[must_use]
    pub fn direction_from_rotation(rot_x: f32, rot_y: f32) -> Vector3<f64> {
        let to_radians = (std::f64::consts::PI / 180.0) as f32;
        let pi = std::f64::consts::PI as f32;
        let y_cos = pumpkin_util::math::cos(-rot_y * to_radians - pi);
        let y_sin = pumpkin_util::math::sin(-rot_y * to_radians - pi);
        let x_cos = -pumpkin_util::math::cos(-rot_x * to_radians);
        let x_sin = pumpkin_util::math::sin(-rot_x * to_radians);
        Vector3::new(
            f64::from(y_sin * x_cos),
            f64::from(x_sin),
            f64::from(y_cos * x_cos),
        )
    }

    /// Vanilla `Entity.calculateViewVector`, in vanilla's float trigonometry.
    #[must_use]
    pub fn calculate_view_vector(x_rot: f32, y_rot: f32) -> Vector3<f64> {
        let x_rot = x_rot * (std::f64::consts::PI / 180.0) as f32;
        let y_rot = -y_rot * (std::f64::consts::PI / 180.0) as f32;
        let y_cos = pumpkin_util::math::cos(y_rot);
        let y_sin = pumpkin_util::math::sin(y_rot);
        let x_cos = pumpkin_util::math::cos(x_rot);
        let x_sin = pumpkin_util::math::sin(x_rot);
        Vector3::new(
            f64::from(y_sin * x_cos),
            f64::from(-x_sin),
            f64::from(y_cos * x_cos),
        )
    }

    /// Vanilla `Entity.getLookQuaternion`.
    #[must_use]
    pub fn get_look_quaternion(&self) -> Quaternionf {
        Self::calculate_view_quaternion(self.pitch.load(), self.yaw.load())
    }

    /// Vanilla `Entity.calculateViewQuaternion`.
    #[must_use]
    pub fn calculate_view_quaternion(x_rot: f32, y_rot: f32) -> Quaternionf {
        let to_radians = (std::f64::consts::PI / 180.0) as f32;
        Quaternionf::rotation_y(-y_rot * to_radians).rotate_x(x_rot * to_radians)
    }

    pub fn get_horizontal_facing(&self) -> HorizontalFacing {
        let yaw = self.yaw.load();
        // Use vanilla's formula: floor(angle / 90.0 + 0.5) & 3
        let quarter_turns = ((yaw / 90.0) + 0.5).floor() as i32 & 3;
        match quarter_turns {
            0 => HorizontalFacing::South,
            1 => HorizontalFacing::West,
            2 => HorizontalFacing::North,
            _ => HorizontalFacing::East,
        }
    }

    pub fn get_rotation_16(&self) -> u8 {
        let adjusted_yaw = self.yaw.load().rem_euclid(360.0);

        ((adjusted_yaw / 22.5).round() as u8) % 16
    }

    pub fn get_flipped_rotation_16(&self) -> u8 {
        (self.get_rotation_16() + 8) % 16
    }

    pub fn get_facing(&self) -> Facing {
        let pitch = self.pitch.load().to_radians();
        let yaw = -self.yaw.load().to_radians();

        let (sin_p, cos_p) = pitch.sin_cos();
        let (sin_y, cos_y) = yaw.sin_cos();

        let x = sin_y * cos_p;
        let y = -sin_p;
        let z = cos_y * cos_p;

        let ax = x.abs();
        let ay = y.abs();
        let az = z.abs();

        if ax > ay && ax > az {
            if x > 0.0 { Facing::East } else { Facing::West }
        } else if ay > ax && ay > az {
            if y > 0.0 { Facing::Up } else { Facing::Down }
        } else if z > 0.0 {
            Facing::South
        } else {
            Facing::North
        }
    }

    pub fn get_entity_facing_order(&self) -> [Facing; 6] {
        let pitch = self.pitch.load().to_radians();
        let yaw = -self.yaw.load().to_radians();

        let sin_p = pitch.sin();
        let cos_p = pitch.cos();
        let sin_y = yaw.sin();
        let cos_y = yaw.cos();

        let east_west = if sin_y > 0.0 {
            Facing::East
        } else {
            Facing::West
        };
        let up_down = if sin_p < 0.0 {
            Facing::Up
        } else {
            Facing::Down
        };
        let south_north = if cos_y > 0.0 {
            Facing::South
        } else {
            Facing::North
        };

        let x_axis = sin_y.abs();
        let y_axis = sin_p.abs();
        let z_axis = cos_y.abs();
        let x_weight = x_axis * cos_p;
        let z_weight = z_axis * cos_p;

        let (first, second, third) = if x_axis > z_axis {
            if y_axis > x_weight {
                (up_down, east_west, south_north)
            } else if z_weight > y_axis {
                (east_west, south_north, up_down)
            } else {
                (east_west, up_down, south_north)
            }
        } else if y_axis > z_weight {
            (up_down, south_north, east_west)
        } else if x_weight > y_axis {
            (south_north, east_west, up_down)
        } else {
            (south_north, up_down, east_west)
        };

        [
            first,
            second,
            third,
            third.opposite(),
            second.opposite(),
            first.opposite(),
        ]
    }

    pub fn set_sprinting(&self, sprinting: bool) {
        //assert!(self.sprinting.load(Relaxed) != sprinting);
        self.sprinting.store(sprinting, Relaxed);
        self.set_flag(Flag::Sprinting, sprinting);
    }

    pub fn is_sprinting(&self) -> bool {
        self.sprinting.load(Ordering::Relaxed)
    }
    pub fn check_fall_flying(&self) -> bool {
        !self.on_ground.load(Relaxed)
    }

    pub fn set_fall_flying(&self, fall_flying: bool) {
        assert_ne!(self.fall_flying.load(Relaxed), fall_flying);
        self.fall_flying.store(fall_flying, Relaxed);
        self.set_flag(Flag::FallFlying, fall_flying);
    }
    pub fn is_fall_flying(&self) -> bool {
        self.fall_flying.load(Ordering::Relaxed)
    }

    fn set_flag(&self, flag: Flag, value: bool) {
        let index = flag as u8;
        let mask = (1i8).wrapping_shl(index as u32);
        let new_je_flags = if value {
            self.flags.fetch_or(mask, Ordering::Relaxed) | mask
        } else {
            self.flags.fetch_and(!mask, Ordering::Relaxed) & !mask
        };

        self.set_synced_data(tracked_data::entity::DATA_SHARED_FLAGS_ID, new_je_flags);

        if let Some(bedrock_flag) = flag.to_bedrock() {
            let (key, index) = if bedrock_flag >= 64 {
                (entity_data_key::FLAGS_TWO, (bedrock_flag - 64) as u8)
            } else {
                (entity_data_key::FLAGS, bedrock_flag as u8)
            };

            if value {
                let mask = 1i64 << index;
                if key == entity_data_key::FLAGS {
                    self.bedrock_flags.fetch_or(mask, Ordering::Relaxed);
                } else {
                    self.bedrock_flags_two.fetch_or(mask, Ordering::Relaxed);
                }
            } else {
                let mask = !(1i64 << index);
                if key == entity_data_key::FLAGS {
                    self.bedrock_flags.fetch_and(mask, Ordering::Relaxed);
                } else {
                    self.bedrock_flags_two.fetch_and(mask, Ordering::Relaxed);
                }
            }

            let world = self.world.load();
            let chunk_pos = self.chunk_pos.load();
            let mut metadata = SyncedActorDataList(std::collections::HashMap::new());
            metadata.set(
                entity_data_key::FLAGS,
                MetadataValue::Int64(self.bedrock_flags.load(Ordering::Relaxed)),
            );
            metadata.set(
                entity_data_key::FLAGS_TWO,
                MetadataValue::Int64(self.bedrock_flags_two.load(Ordering::Relaxed)),
            );
            let packet = CSetActorData {
                target_runtime_id: VarULong(self.entity_id as u64),
                actor_data: metadata,
                synced_properties: PropertySyncData {
                    int_entries_list: std::collections::HashMap::new(),
                    float_entries_list: std::collections::HashMap::new(),
                },
                tick: VarULong(0),
            };
            world.broadcast_to_chunk_bedrock(chunk_pos, &packet);
        }
    }

    /// Plays sound at this entity's position with the entity's sound category
    pub fn play_sound(&self, sound: Sound) {
        self.world
            .load()
            .play_sound(sound, SoundCategory::Neutral, &self.pos.load());
    }

    pub fn set_synced_data<T: MetadataSerializer + Clone + Send + Sync + 'static>(
        &self,
        tracked: pumpkin_data::tracked_data::TrackedData,
        value: T,
    ) -> bool {
        // Sent by the tracker at end of tick, vanilla `sendDirtyEntityData`.
        self.synched_data.set(tracked, value)
    }

    pub fn send_bedrock_actor_data(&self, bedrock_meta: &SyncedActorDataList) {
        let world = self.world.load();
        let players = world.players.load();
        let mut bedrock_recipients = Vec::new();

        if let Some(tracked) = world.entity_tracker.get_tracked_entity(self.entity_id) {
            if !tracked.has_bedrock_watchers() && self.entity_type != &EntityType::PLAYER {
                return;
            }
            for player in players.iter() {
                if (tracked.seen_by.contains(&player.gameprofile.id)
                    || player.entity_id() == self.entity_id)
                    && let ClientPlatform::Bedrock(client) = player.client.as_ref()
                {
                    bedrock_recipients.push(client);
                }
            }
        } else {
            let chunk_pos = self.chunk_pos.load();
            for player in players.iter() {
                if player
                    .watched_section
                    .load()
                    .is_within_distance(chunk_pos.x, chunk_pos.y)
                    && let ClientPlatform::Bedrock(client) = player.client.as_ref()
                {
                    bedrock_recipients.push(client);
                }
            }
        }

        let packet = CSetActorData {
            target_runtime_id: VarULong(self.entity_id as u64),
            actor_data: SyncedActorDataList(bedrock_meta.0.clone()),
            synced_properties: PropertySyncData {
                int_entries_list: std::collections::HashMap::new(),
                float_entries_list: std::collections::HashMap::new(),
            },
            tick: VarULong(0),
        };
        for recipient in bedrock_recipients {
            if let Ok(packet_data) = recipient.serialize_packet(&packet) {
                recipient.try_enqueue_packet(packet_data);
            }
        }
    }

    /// The Java players that should receive entity data for this entity: everyone
    /// tracking it, plus the entity itself when it is a player. Until the entity is
    /// tracked, every player watching its chunk stands in for the tracker.
    fn java_metadata_recipients<'a>(
        &self,
        world: &World,
        players: &'a [Arc<Player>],
    ) -> Vec<&'a Arc<Player>> {
        let mut java_recipients = Vec::new();

        if let Some(tracked) = world.entity_tracker.get_tracked_entity(self.entity_id) {
            for player in players {
                if (tracked.seen_by.contains(&player.gameprofile.id)
                    || player.entity_id() == self.entity_id)
                    && let ClientPlatform::Java(_) = player.client.as_ref()
                {
                    java_recipients.push(player);
                }
            }
        } else {
            let chunk_pos = self.chunk_pos.load();
            for player in players {
                // A player that just changed world is in the new world's player list but
                // not in its tracker yet, and its watched section still points at the
                // world it left, so it has to be matched on its entity id instead.
                if (player.entity_id() == self.entity_id
                    || player
                        .watched_section
                        .load()
                        .is_within_distance(chunk_pos.x, chunk_pos.y))
                    && let ClientPlatform::Java(_) = player.client.as_ref()
                {
                    java_recipients.push(player);
                }
            }
        }

        java_recipients
    }

    /// Sends the given metadata entries to this entity's viewers, and the matching
    /// actor data to the Bedrock ones when `bedrock_meta` is given.
    pub fn send_meta_data<T: MetadataSerializer>(
        &self,
        meta: &[Metadata<T>],
        bedrock_meta: Option<&SyncedActorDataList>,
    ) {
        let world = self.world.load();
        let players = world.players.load();

        let java_recipients = self.java_metadata_recipients(&world, &players);

        let recipients_by_version =
            World::collect_java_recipients_by_version(java_recipients.into_iter());

        for (version, recipients) in recipients_by_version {
            let mut buf = Vec::new();
            for m in meta {
                let _ = m.write(&mut buf, &version);
            }
            if buf.is_empty() {
                continue;
            }
            buf.put_u8(255);
            let packet = CSetEntityMetadata::new(self.entity_id.into(), buf.into());
            if let Ok(packet_data) = JavaClient::serialize_packet_for_version(&packet, version) {
                for recipient in recipients {
                    recipient.try_enqueue_packet(packet_data.clone());
                }
            }
        }

        if let Some(bedrock_meta) = bedrock_meta {
            self.send_bedrock_actor_data(bedrock_meta);
        }
    }

    /// Sends the synced values that changed since the last call to this entity's
    /// viewers, and does nothing when none did.
    pub fn send_dirty_entity_data(&self) {
        if !self.synched_data.is_dirty() {
            return;
        }

        let world = self.world.load();
        let players = world.players.load();

        // Unwatched, skip the player scan.
        if self.entity_type != &EntityType::PLAYER
            && world
                .entity_tracker
                .get_tracked_entity(self.entity_id)
                .is_some_and(|tracked| tracked.seen_by.is_empty())
        {
            self.synched_data.clear_dirty();
            return;
        }

        let java_recipients = self.java_metadata_recipients(&world, &players);

        if java_recipients.is_empty() {
            // Vanilla `packDirty` clears even without recipients.
            self.synched_data.clear_dirty();
            return;
        }

        let recipients_by_version =
            World::collect_java_recipients_by_version(java_recipients.into_iter());

        for (version, recipients) in recipients_by_version {
            if let Some(buf) = self.synched_data.pack_dirty_for_version(&version) {
                let packet = CSetEntityMetadata::new(self.entity_id.into(), buf);
                if let Ok(packet_data) = JavaClient::serialize_packet_for_version(&packet, version)
                {
                    for recipient in recipients {
                        recipient.try_enqueue_packet(packet_data.clone());
                    }
                }
            }
        }
        self.synched_data.clear_dirty();
    }

    /// Resends every non-default synced value of this entity to its viewers, the entity
    /// itself included when it is a player.
    ///
    /// Clients discard the entity data they cached whenever the entity is re-created on
    /// their side (a respawn or a dimension change re-creates the local player, and a
    /// spawn packet resets the entity for everyone else), so the values have to be pushed
    /// again even though nothing became dirty on our side.
    pub fn refresh_synced_data(&self) {
        let world = self.world.load();
        let players = world.players.load();

        let java_recipients = self.java_metadata_recipients(&world, &players);

        if java_recipients.is_empty() {
            return;
        }

        let recipients_by_version =
            World::collect_java_recipients_by_version(java_recipients.into_iter());

        for (version, recipients) in recipients_by_version {
            if let Some(buf) = self
                .synched_data
                .get_non_default_values_for_version(&version)
            {
                let packet = CSetEntityMetadata::new(self.entity_id.into(), buf);
                if let Ok(packet_data) = JavaClient::serialize_packet_for_version(&packet, version)
                {
                    for recipient in recipients {
                        recipient.try_enqueue_packet(packet_data.clone());
                    }
                }
            }
        }
        self.synched_data.clear_dirty();
    }

    /// Vanilla `Entity.setPose` with the dimensions the owning entity has in that pose,
    /// which only `EntityBase::set_pose` can work out. Call that instead.
    pub fn set_pose_with_dimensions(&self, pose: EntityPose, dimension: EntityDimensions) {
        if self.pose.load() == pose {
            return;
        }

        let mut pose_event =
            crate::plugin::api::events::entity::entity_pose_change::EntityPoseChangeEvent::new(
                self.entity_id,
                (pose as u8).to_string(),
            );
        if let Some(server) = self.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut pose_event);
            if pose_event.cancelled {
                return;
            }
        }

        self.pose.store(pose);
        self.apply_dimensions(dimension);
        let pose = pose as i32;
        let mut bedrock_meta = SyncedActorDataList::new();
        bedrock_meta.set(entity_data_key::POSE_INDEX, MetadataValue::Int(pose));
        bedrock_meta.set(
            entity_data_key::WIDTH,
            MetadataValue::Float(dimension.width),
        );
        bedrock_meta.set(
            entity_data_key::HEIGHT,
            MetadataValue::Float(dimension.height),
        );
        self.set_synced_data(tracked_data::entity::DATA_POSE, VarInt(pose));
        self.send_bedrock_actor_data(&bedrock_meta);
    }

    /// Vanilla `Entity.isInvulnerable`: permanently or for a while.
    #[must_use]
    pub fn is_invulnerable(&self) -> bool {
        self.invulnerable.load(Ordering::Relaxed)
            || self.invulnerable_time.load(Ordering::Relaxed) > 0
    }

    /// Vanilla `Entity.commonTick`, which the world runs before each entity's tick.
    pub fn common_tick(&self) {
        let invulnerable_time = self.invulnerable_time.load(Ordering::Relaxed);
        if invulnerable_time > 0 {
            self.invulnerable_time
                .store(invulnerable_time - 1, Ordering::Relaxed);
        }
        self.tick_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Vanilla `Entity.fireImmune`: the entity type.
    pub fn is_fire_immune(&self) -> bool {
        self.entity_type.fire_immune || self.fire_immune.load(Ordering::Relaxed)
    }

    /// Vanilla `Entity.isInvulnerableToBase`. Driven by the damage type tags, so datapacks
    /// decide what bypasses invulnerability or counts as fire and fall damage.
    /// `cause` is the attacker (vanilla `DamageSource.getEntity`).
    pub fn is_invulnerable_to(
        &self,
        damage_type: &DamageType,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        let bypasses = damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_INVULNERABILITY);
        let creative_cause = cause
            .and_then(EntityBase::get_player)
            .is_some_and(Player::is_creative);

        self.removed.load(Ordering::SeqCst)
            || (self.is_invulnerable() && !bypasses && !creative_cause)
            || (damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FIRE) && self.is_fire_immune())
            || (damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FALL)
                && self
                    .entity_type
                    .has_tag(&tag::EntityType::MINECRAFT_FALL_DAMAGE_IMMUNE))
            // Plugin immunities, which never block damage that bypasses invulnerability.
            || (!bypasses
                && self
                    .damage_immunities
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .contains(damage_type))
    }

    /// Sets if the entity is invulnerable to a specific damage type
    pub fn set_damage_immunity(&self, damage_type: DamageType, immune: bool) {
        let mut immunities = self
            .damage_immunities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if immune {
            if !immunities.contains(&damage_type) {
                immunities.push(damage_type);
            }
        } else {
            // retain is cleaner than finding index and removing
            immunities.retain(|dt| dt != &damage_type);
        }
    }

    /// Sets if the entity is invulnerable to all damage types (except `GENERIC_KILL` and `OUT_OF_WORLD`)
    pub fn set_invulnerable(&self, invulnerable: bool) {
        self.invulnerable.store(invulnerable, Relaxed);
    }

    /// Vanilla `Entity.teleportTo(x, y, z)`: `snapTo` the position (with a new rotation where
    /// given), then every rider follows (`teleportPassengers`). The tracker sends the move.
    pub fn teleport(
        &self,
        position: Vector3<f64>,
        yaw: Option<f32>,
        pitch: Option<f32>,
        world: &World,
    ) {
        self.snap_to(position, yaw, pitch);
        let Some(this) = world.get_entity_by_id(self.entity_id) else {
            return;
        };
        let mut stack = vec![this];
        while let Some(vehicle) = stack.pop() {
            let passengers = vehicle
                .get_entity()
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            for passenger in passengers {
                // `positionRider(passenger, Entity::snapTo)`.
                vehicle.position_rider(vehicle.as_ref(), passenger.as_ref());
                let passenger_entity = passenger.get_entity();
                passenger_entity.last_pos.store(passenger_entity.pos.load());
                stack.push(passenger);
            }
        }
    }

    /// Vanilla `Entity.snapTo`: the position and rotation, as the old position too.
    pub fn snap_to(&self, position: Vector3<f64>, yaw: Option<f32>, pitch: Option<f32>) {
        if let Some(yaw) = yaw
            && yaw.is_finite()
        {
            self.yaw.store(yaw);
        }
        if let Some(pitch) = pitch {
            self.set_pitch(pitch);
        }
        self.set_pos(position);
        self.last_pos.store(position);
    }

    /// Vanilla `Entity.teleportSetPosition`: the destination, with no move left to sweep.
    pub fn teleport_set_position(&self, target: &TeleportTarget) {
        let yaw = target.yaw.unwrap_or_else(|| self.yaw.load());
        let pitch = target.pitch.unwrap_or_else(|| self.pitch.load());
        self.set_pos(target.position);
        // `setYRot` and `setYHeadRot` ignore a non-finite yaw, `setXRot` wraps and clamps.
        if yaw.is_finite() {
            self.yaw.store(yaw);
            self.head_yaw.store(yaw);
        }
        self.set_pitch(pitch);
        self.last_pos.store(target.position);
        if !target.keep_velocity {
            self.velocity.store(Vector3::default());
        }
        self.clear_movement_this_tick();
    }

    /// Vanilla `Entity.sendTeleportTransitionToPlayers`: the players riding along see the
    /// teleport, and so does everyone tracking an entity without regular position updates.
    fn send_teleport_transition_to_players(
        &self,
        caller: &dyn EntityBase,
        world: &World,
        target: &TeleportTarget,
    ) {
        let controller_id = caller
            .controlling_passenger()
            .map(|controller| controller.get_entity().entity_id);
        let mut passengers = Vec::new();
        self.collect_indirect_passengers(&mut passengers);
        let on_ground = self.on_ground.load(Ordering::Relaxed);
        let current = CTeleportEntity::new(
            self.entity_id.into(),
            self.pos.load(),
            self.velocity.load(),
            self.yaw.load(),
            self.pitch.load(),
            &[],
            on_ground,
        );
        let mut relatives = Vec::new();
        if target.keep_velocity {
            relatives.extend([
                PositionFlag::DeltaX,
                PositionFlag::DeltaY,
                PositionFlag::DeltaZ,
            ]);
        }
        if target.yaw.is_none() {
            relatives.push(PositionFlag::YRot);
        }
        if target.pitch.is_none() {
            relatives.push(PositionFlag::XRot);
        }
        let transition = CTeleportEntity::new(
            self.entity_id.into(),
            target.position,
            Vector3::default(),
            target.yaw.unwrap_or_default(),
            target.pitch.unwrap_or_default(),
            &relatives,
            on_ground,
        );
        for passenger in &passengers {
            if let Some(player) = passenger.get_player() {
                if controller_id == Some(player.entity_id()) {
                    player.try_send_client_packet(&transition);
                } else {
                    player.try_send_client_packet(&current);
                }
            }
        }
        // `!type.hasUpdateInterval()`.
        if self.entity_type.update_interval == i32::MAX as u32 {
            let passenger_ids: Vec<i32> = passengers
                .iter()
                .map(|passenger| passenger.get_entity().entity_id)
                .collect();
            world.send_to_tracking_players_filtered(self, &current, |player| {
                !passenger_ids.contains(&player.entity_id())
            });
        }
    }

    /// Vanilla `getIndirectPassengersStream`: every passenger, then its own, depth first.
    fn collect_indirect_passengers(&self, out: &mut Vec<Arc<dyn EntityBase>>) {
        let passengers = self
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for passenger in passengers {
            out.push(passenger.clone());
            passenger.get_entity().collect_indirect_passengers(out);
        }
    }

    pub fn get_eye_pos(&self) -> Vector3<f64> {
        let pos = self.pos.load();
        Vector3::new(
            pos.x,
            pos.y + f64::from(self.entity_dimension.load().eye_height),
            pos.z,
        )
    }

    /// Vanilla `LivingEntity.hasLineOfSight`: no colliding block between the two eye positions.
    #[must_use]
    pub fn has_line_of_sight(&self, other: &Self) -> bool {
        let world = self.world.load_full();
        if !Arc::ptr_eq(&world, &other.world.load()) {
            return false;
        }
        let from = self.get_eye_pos();
        let to = other.get_eye_pos();
        if to.squared_distance_to_vec(&from).sqrt() > 128.0 {
            return false;
        }
        // TODO: vanilla clips with CollisionContext.of(this); Pumpkin's clip has no entity context.
        world
            .clip(&pumpkin_world::clip::ClipContext::new(
                from,
                to,
                pumpkin_world::clip::ClipBlock::Collider,
                pumpkin_world::clip::ClipFluid::None,
            ))
            .is_miss
    }

    pub fn get_eye_y(&self) -> f64 {
        self.pos.load().y + f64::from(self.entity_dimension.load().eye_height)
    }

    /// Vanilla `Entity.getRandomX`.
    pub fn get_random_x(&self, spread: f64) -> f64 {
        self.pos.load().x
            + f64::from(self.width()) * ((2.0 * self.random().next_f64() - 1.0) * spread)
    }

    /// Vanilla `Entity.getRandomY()`.
    pub fn get_random_y(&self) -> f64 {
        self.pos.load().y + f64::from(self.height()) * self.random().next_f64()
    }

    /// Vanilla `Entity.getRandomY(double)`.
    pub fn get_random_y_spread(&self, spread: f64) -> f64 {
        self.pos.load().y
            + f64::from(self.height()) * ((2.0 * self.random().next_f64() - 1.0) * spread)
    }

    /// Vanilla `Entity.getRandomZ`.
    pub fn get_random_z(&self, spread: f64) -> f64 {
        self.pos.load().z
            + f64::from(self.width()) * ((2.0 * self.random().next_f64() - 1.0) * spread)
    }

    pub fn is_removed(&self) -> bool {
        self.removal_reason.load().is_some()
    }

    /// Vanilla `Entity.setRemoved`'s bookkeeping for an entity leaving without being destroyed
    /// (`UNLOADED_TO_CHUNK`, `UNLOADED_WITH_PLAYER`): the first reason sticks. The world's own
    /// lists are the caller's business.
    pub fn set_removed_reason(&self, reason: RemovalReason) {
        let _ = self.removal_reason.compare_exchange(None, Some(reason));
        self.removed.store(true, Ordering::Release);
    }

    pub fn is_alive(&self) -> bool {
        !self.is_removed()
    }

    #[must_use]
    pub fn is_affected_by_blocks(&self) -> bool {
        !self.is_removed() && !self.no_physics.load(Ordering::Relaxed)
    }

    #[must_use]
    /// Vanilla `Entity.isInWall`: a suffocating block's collision shape at eye level.
    pub fn is_in_wall(&self) -> bool {
        if self.no_physics.load(Ordering::Relaxed) {
            return false;
        }

        let eye_pos = self.get_eye_pos();
        // `AABB.ofSize(eye, width * 0.8F, 1.0E-6, width * 0.8F)`.
        let half_width = f64::from(self.entity_dimension.load().width * 0.8) / 2.0;
        let half_height = 1.0e-6 / 2.0;
        let eye_bb = BoundingBox::new(
            Vector3::new(
                eye_pos.x - half_width,
                eye_pos.y - half_height,
                eye_pos.z - half_width,
            ),
            Vector3::new(
                eye_pos.x + half_width,
                eye_pos.y + half_height,
                eye_pos.z + half_width,
            ),
        );
        let world = self.world.load();
        BlockPos::iterate(
            BlockPos::floored_v(eye_bb.min),
            BlockPos::floored_v(eye_bb.max),
        )
        .any(|pos| {
            let state = world.get_block_state(&pos);
            !state.is_air()
                && state.is_suffocating()
                && state
                    .get_block_collision_shapes_at(&pos)
                    .any(|shape| shape.at_pos(pos).intersects(&eye_bb))
        })
    }

    /// The entity holding this one's leash (vanilla `getLeashHolder` on the server).
    #[must_use]
    pub fn leash_holder(&self) -> Option<Arc<dyn EntityBase>> {
        self.leashed_to
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Whether a saved leash is waiting for its holder (vanilla `LeashData.delayedLeashInfo`).
    #[must_use]
    pub fn delayed_leash_is_some(&self) -> bool {
        self.delayed_leash
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// Vanilla `Entity.setYRot`: a non-finite rotation is discarded.
    pub fn set_y_rot(&self, y_rot: f32) {
        if y_rot.is_finite() {
            self.yaw.store(y_rot);
        }
    }

    /// Vanilla `writeLeashData`: a knot's block position, or the holder's UUID.
    pub fn write_leash_nbt(&self, nbt: &mut NbtCompound) {
        let holder = self
            .leashed_to
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let leash = match holder {
            Some(holder) => holder
                .cast_any()
                .downcast_ref::<crate::entity::decoration::leash_knot::LeashKnotEntity>()
                .map_or_else(
                    || DelayedLeash::Holder(holder.get_entity().entity_uuid),
                    |knot| DelayedLeash::Knot(knot.block_pos()),
                ),
            None => match *self
                .delayed_leash
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
            {
                Some(delayed) => delayed,
                None => return,
            },
        };
        match leash {
            DelayedLeash::Knot(pos) => {
                nbt.put("leash", NbtTag::IntArray(vec![pos.0.x, pos.0.y, pos.0.z]));
            }
            DelayedLeash::Holder(uuid) => {
                let mut holder = NbtCompound::new();
                holder.put_uuid("UUID", uuid);
                nbt.put("leash", NbtTag::Compound(holder));
            }
        }
    }

    /// Vanilla `readLeashData`.
    pub fn read_leash_nbt(&self, caller: &dyn EntityBase, nbt: &NbtCompound) {
        let leash = match nbt.get("leash") {
            Some(NbtTag::IntArray(pos)) => match pos.as_slice() {
                &[x, y, z] => Some(DelayedLeash::Knot(BlockPos::new(x, y, z))),
                _ => None,
            },
            Some(NbtTag::Compound(holder)) => holder.get_uuid("UUID").map(DelayedLeash::Holder),
            _ => None,
        };
        if leash.is_none() {
            // `if (getLeashData() != null && newLeashData == null) removeLeash()`.
            leashable::drop_leash(caller, true, false);
        } else {
            // `setLeashData(newLeashData)` replaces the data outright: the current holder goes
            // without an event, packet or notify, and the new data starts without momentum.
            self.leashed_to
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            self.leash_angular_momentum.store(0.0);
        }
        *self
            .delayed_leash
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = leash;
    }

    /// Vanilla `Entity.spawnAtLocation`: drops `stack` where the entity stands.
    pub fn spawn_at_location(&self, stack: ItemStack) {
        if !stack.is_empty() {
            self.world.load_full().spawn_item_at(self.pos.load(), stack);
        }
    }

    pub fn has_passengers(&self) -> bool {
        !self
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    pub fn has_passenger(&self, id: i32) -> bool {
        self.passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|passenger| passenger.get_entity().entity_id == id)
    }

    pub fn has_vehicle(&self) -> bool {
        self.vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// Vanilla `Entity.getFirstPassenger`.
    pub fn get_first_passenger(&self) -> Option<Arc<dyn EntityBase>> {
        self.passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .first()
            .cloned()
    }

    /// Vanilla `Entity.hasMovedHorizontallyRecently`, from the movement measured at
    /// the start of this entity's last tick (vanilla's `lastKnownSpeed`).
    pub fn has_moved_horizontally_recently(&self) -> bool {
        self.movement.load().horizontal_length() > f64::from(1.0E-5f32)
    }

    /// Vanilla `getRootVehicle` for an entity that rides: the bottom of its stack.
    #[must_use]
    pub fn root_vehicle(&self) -> Option<Arc<dyn EntityBase>> {
        let mut root = self.get_vehicle()?;
        while let Some(vehicle) = root.get_entity().get_vehicle() {
            root = vehicle;
        }
        Some(root)
    }

    /// The id of vanilla's `getRootVehicle`: the bottom of the stack this entity rides in.
    #[must_use]
    pub fn root_vehicle_id(&self) -> i32 {
        let mut root = self.get_vehicle();
        let mut id = self.entity_id;
        while let Some(vehicle) = root {
            id = vehicle.get_entity().entity_id;
            root = vehicle.get_entity().get_vehicle();
        }
        id
    }

    pub fn get_vehicle(&self) -> Option<Arc<dyn EntityBase>> {
        self.vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Vanilla `Entity.getControlledVehicle`: the vehicle this entity rides when it is that vehicle's
    /// controlling passenger.
    #[must_use]
    pub fn get_controlled_vehicle(&self) -> Option<Arc<dyn EntityBase>> {
        let vehicle = self.get_vehicle()?;
        vehicle
            .controlling_passenger()
            .is_some_and(|c| c.get_entity().entity_id == self.entity_id)
            .then_some(vehicle)
    }

    pub fn is_leashed(&self) -> bool {
        self.leashed_to
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// Vanilla `startRiding(vehicle)` once its checks passed: mounts `passenger` on this
    /// vehicle, with the `entity_mount` game event.
    pub fn add_passenger(&self, vehicle: Arc<dyn EntityBase>, passenger: Arc<dyn EntityBase>) {
        self.mount_passenger(vehicle, passenger, true);
    }

    /// Vanilla `startRiding(vehicle, true, false)`, as entity loading mounts passengers: no
    /// game event.
    pub fn add_passenger_without_event(
        &self,
        vehicle: Arc<dyn EntityBase>,
        passenger: Arc<dyn EntityBase>,
    ) {
        self.mount_passenger(vehicle, passenger, false);
    }

    /// The end of vanilla `startRiding` and `addPassenger`: a player takes the front seat from
    /// anything but another player.
    fn mount_passenger(
        &self,
        vehicle: Arc<dyn EntityBase>,
        passenger: Arc<dyn EntityBase>,
        send_event: bool,
    ) {
        let mut mount_event =
            crate::plugin::api::events::entity::entity_mount::EntityMountEvent::new(
                passenger.get_entity().entity_id,
                self.entity_id,
            );
        let mut vehicle_enter =
            crate::plugin::api::events::vehicle::vehicle_enter::VehicleEnterEvent::new(
                self.entity_id,
                passenger.get_entity().entity_id,
            );
        if let Some(server) = self.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut mount_event);
            server
                .plugin_manager
                .fire_blocking(&server, &mut vehicle_enter);
        }
        if mount_event.cancelled || vehicle_enter.cancelled {
            return;
        }
        vehicle.before_add_passenger();
        let vehicle_hooks = Arc::clone(&vehicle);

        passenger.set_pose(EntityPose::Standing);
        let passenger_entity = passenger.get_entity();
        *passenger_entity
            .vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(vehicle);

        let mut passengers = self
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let event_source = Arc::clone(&passenger);
        if passenger.get_player().is_some()
            && passengers
                .first()
                .is_some_and(|first| first.get_player().is_none())
        {
            passengers.insert(0, passenger);
        } else {
            passengers.push(passenger);
        }

        let passenger_ids: Vec<VarInt> = passengers
            .iter()
            .map(|p| VarInt(p.get_entity().entity_id))
            .collect();

        drop(passengers);

        let world = self.world.load();
        let passengers_packet = CSetPassengers::new(VarInt(self.entity_id), &passenger_ids);
        let rider = event_source.get_player();
        // Everyone watching the vehicle, the rider too, learns of the mount from the
        // vehicle's tracker (`ServerEntity.sendChanges`, `TrackedEntity::send_passengers`).
        // The end of `addPassenger`, before `startRiding`'s `entity_mount` game event.
        vehicle_hooks.after_add_passenger();
        if send_event {
            world.game_event_from(
                Some(event_source.as_ref()),
                pumpkin_data::game_event::GameEvent::EntityMount,
                self.pos.load(),
            );
        }
        // `Mob.startRiding`: a mob that mounts lets go of its leash. A passenger restored from
        // NBT still has its leash delayed, so it keeps it.
        if event_source.get_mob().is_some() && event_source.get_entity().is_leashed() {
            leashable::drop_leash(event_source.as_ref(), true, true);
        }
        if let Some(player) = rider {
            // `ServerPlayer.startRiding`: the rider is placed and told; the tracker's
            // passenger list follows for everyone watching the vehicle, the rider again too.
            player.start_riding_packets(&vehicle_hooks, &passengers_packet);
        }
    }

    /// The vehicle's `removePassenger` override (`EntityBase::after_remove_passenger`), run
    /// on every dismount path.
    fn after_remove_passenger_hook(&self) {
        if let Some(vehicle) = self.world.load().get_entity_by_id(self.entity_id) {
            vehicle.after_remove_passenger();
        }
    }

    pub fn remove_passenger(&self, passenger_id: i32) {
        self.remove_passenger_internal(passenger_id, true, true);
    }

    pub fn remove_passenger_before_teleport(&self, passenger_id: i32) {
        self.remove_passenger_internal(passenger_id, false, true);
    }

    /// The dismount vanilla `setRemoved` forces: plugins are told but can't cancel it,
    /// since a removed entity can't keep riding or carrying anything.
    pub fn remove_passenger_on_removal(&self, passenger_id: i32) {
        self.remove_passenger_internal(passenger_id, true, false);
    }

    #[allow(clippy::too_many_lines)]
    fn remove_passenger_internal(&self, passenger_id: i32, reposition: bool, cancellable: bool) {
        let mut dismount_event =
            crate::plugin::api::events::entity::entity_dismount::EntityDismountEvent::new(
                passenger_id,
                self.entity_id,
            );
        let mut vehicle_exit =
            crate::plugin::api::events::vehicle::vehicle_exit::VehicleExitEvent::new(
                self.entity_id,
                passenger_id,
            );
        if let Some(server) = self.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut dismount_event);
            server
                .plugin_manager
                .fire_blocking(&server, &mut vehicle_exit);
        }
        if cancellable && (dismount_event.cancelled || vehicle_exit.cancelled) {
            return;
        }

        let (removed_passenger, passenger_ids) = {
            let mut passengers = self
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let removed_passenger = passengers
                .iter()
                .position(|p| p.get_entity().entity_id == passenger_id)
                .map(|idx| {
                    let passenger = passengers.remove(idx);
                    *passenger
                        .get_entity()
                        .vehicle
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                    passenger
                });

            let passenger_ids: Vec<VarInt> = passengers
                .iter()
                .map(|p| VarInt(p.get_entity().entity_id))
                .collect();
            (removed_passenger, passenger_ids)
        };

        if let Some(passenger) = removed_passenger {
            let vehicle_box = self.bounding_box.load();
            let passenger_entity = passenger.get_entity();

            // Pre-allocate teleport ID and block movement packets BEFORE sending
            // CSetPassengers. This prevents a race condition where the client receives
            // the dismount packet, sends stale position packets from the old riding
            // position, and the server processes them before the teleport arrives. A removed
            // passenger isn't placed (below), so it gets no teleport id either.
            let teleport_id = if reposition
                && !passenger_entity.is_removed()
                && let Some(player) = passenger.get_player()
            {
                let id = player
                    .teleport_id_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    + 1;
                // Use fallback position as placeholder — updated below with real position
                let placeholder =
                    Vector3::new(self.pos.load().x, vehicle_box.max.y, self.pos.load().z);
                *player
                    .awaiting_teleport
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some((id.into(), placeholder));
                Some(id)
            } else {
                None
            };

            // Vanilla `removePassenger`: boardingCooldown = 60 (prevents immediate re-mount);
            // `Player.removeVehicle` sets a player's back to 0.
            let cooldown = if passenger.get_player().is_some() {
                0
            } else {
                60
            };
            passenger_entity.riding_cooldown.store(cooldown, Relaxed);
            // `oldVehicle.removePassenger(this)`, before the dismount game event.
            self.after_remove_passenger_hook();
            // `removeVehicle`: a passenger that is only unloaded or changing dimension leaves
            // quietly.
            if passenger_entity
                .removal_reason
                .load()
                .is_none_or(|reason| reason.should_destroy())
            {
                self.world.load().game_event_from(
                    Some(passenger.as_ref()),
                    pumpkin_data::game_event::GameEvent::EntityDismount,
                    self.pos.load(),
                );
            }

            // `ServerPlayer.removeVehicle` tells the dismounting player directly; everyone
            // watching the vehicle, the player too, gets the list from the vehicle's tracker.
            let world = self.world.load();
            let passengers_packet = CSetPassengers::new(VarInt(self.entity_id), &passenger_ids);
            if let Some(player) = passenger.get_player() {
                // `ServerPlayer.removeVehicle`: the effects of a living vehicle go from the
                // player's view, then the passenger list.
                if let Some(vehicle) = world.get_entity_by_id(self.entity_id)
                    && let Some(living) = vehicle.get_living_entity()
                {
                    let effects: Vec<_> = living
                        .active_effects
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .keys()
                        .copied()
                        .collect();
                    for effect in effects {
                        player.try_send_client_packet(&CRemoveMobEffect::new(
                            self.entity_id.into(),
                            VarInt(i32::from(effect.id)),
                        ));
                    }
                }
                player.try_send_client_packet(&passengers_packet);
            }

            // `LivingEntity.dismountVehicle`: a removed entity stays where it is.
            if !reposition || passenger_entity.is_removed() {
                return;
            }

            // Calculate dismount directions and offsets (vanilla DismountHelper): forward is
            // the vehicle's facing (`getDirection`), or its `getMotionDirection` where its
            // `getDismountLocationForPassenger` uses that (minecarts).
            let forward_dir = world
                .get_entity_by_id(self.entity_id)
                .and_then(|vehicle| vehicle.dismount_forward())
                .unwrap_or_else(|| direction_from_yaw(self.yaw.load()));

            let get_step = |dir: BlockDirection| -> (i32, i32) {
                match dir {
                    BlockDirection::North => (0, -1),
                    BlockDirection::South => (0, 1),
                    BlockDirection::East => (1, 0),
                    BlockDirection::West => (-1, 0),
                    _ => (0, 0),
                }
            };

            let get_clockwise = |dir: BlockDirection| -> BlockDirection {
                match dir {
                    BlockDirection::North => BlockDirection::East,
                    BlockDirection::East => BlockDirection::South,
                    BlockDirection::South => BlockDirection::West,
                    BlockDirection::West => BlockDirection::North,
                    other => other,
                }
            };

            let get_opposite = |dir: BlockDirection| -> BlockDirection {
                match dir {
                    BlockDirection::North => BlockDirection::South,
                    BlockDirection::South => BlockDirection::North,
                    BlockDirection::East => BlockDirection::West,
                    BlockDirection::West => BlockDirection::East,
                    other => other,
                }
            };

            let right_dir = get_clockwise(forward_dir);
            let left_dir = get_opposite(right_dir);
            let back_dir = get_opposite(forward_dir);

            let (fx, fz) = get_step(forward_dir);
            let (rx, rz) = get_step(right_dir);
            let (lx, lz) = get_step(left_dir);
            let (bx, bz) = get_step(back_dir);

            let offsets = [
                (rx, rz),
                (lx, lz),
                (bx + rx, bz + rz),
                (bx + lx, bz + lz),
                (fx + rx, fz + rz),
                (fx + lx, fz + lz),
                (bx, bz),
                (fx, fz),
            ];

            let target_block_y = vehicle_box.max.y.floor() as i32;
            let below_pos = BlockPos(Vector3::new(
                self.pos.load().x.floor() as i32,
                target_block_y - 1,
                self.pos.load().z.floor() as i32,
            ));

            let below_state_id = world.get_block_state_id(&below_pos);
            // Vanilla: isWater checks specifically for water fluid, not any fluid
            let is_water = Fluid::from_state_id(below_state_id)
                .is_some_and(|f| f.id == Fluid::WATER.id || f.id == Fluid::FLOWING_WATER.id);

            let fallback_pos =
                Vector3::new(self.pos.load().x, vehicle_box.max.y, self.pos.load().z);

            let dismount_pos = if is_water {
                fallback_pos
            } else {
                // Vanilla checks Standing, Crouching, Swimming poses and their respective height checks
                let poses_and_heights = [
                    (EntityPose::Standing, vec![0, 1, -1]),
                    (EntityPose::Crouching, vec![0, 1, -1]),
                    (EntityPose::Swimming, vec![0, 1]),
                ];

                let vehicle_block_pos = self.block_pos.load();
                let mut found = None;

                'search: for (pose, y_offsets) in poses_and_heights {
                    let dims = passenger.dimensions_for_pose(pose);

                    for y_offset in y_offsets {
                        for &(ox, oz) in &offsets {
                            let target_block_x = vehicle_block_pos.0.x + ox;
                            let target_block_y = vehicle_block_pos.0.y + y_offset;
                            let target_block_z = vehicle_block_pos.0.z + oz;

                            let target_pos = BlockPos(Vector3::new(
                                target_block_x,
                                target_block_y,
                                target_block_z,
                            ));
                            let height = world.get_dismount_height(&target_pos);

                            if height.is_finite() && height < 1.0 {
                                let location = Vector3::new(
                                    f64::from(target_block_x) + 0.5,
                                    f64::from(target_block_y) + height,
                                    f64::from(target_block_z) + 0.5,
                                );

                                let bbox = BoundingBox::new_from_pos(
                                    location.x, location.y, location.z, &dims,
                                );
                                if world.is_space_empty(bbox) {
                                    found = Some((location, pose));
                                    break 'search;
                                }
                            }
                        }
                    }
                }

                if let Some((pos, pose)) = found {
                    if pose != EntityPose::Standing {
                        passenger.set_pose(pose);
                    }
                    pos
                } else {
                    // Try dismounting directly on top of the vehicle as fallback
                    let mut found_fallback = None;
                    let vehicle_top = vehicle_box.max.y;

                    let poses = [
                        EntityPose::Standing,
                        EntityPose::Crouching,
                        EntityPose::Swimming,
                    ];

                    for pose in poses {
                        let dims = passenger.dimensions_for_pose(pose);
                        let bbox = BoundingBox::new_from_pos(
                            self.pos.load().x,
                            vehicle_top,
                            self.pos.load().z,
                            &dims,
                        );
                        if world.is_space_empty(bbox) {
                            found_fallback = Some((
                                Vector3::new(self.pos.load().x, vehicle_top, self.pos.load().z),
                                pose,
                            ));
                            break;
                        }
                    }

                    if let Some((pos, pose)) = found_fallback {
                        if pose != EntityPose::Standing {
                            passenger.set_pose(pose);
                        }
                        pos
                    } else {
                        fallback_pos
                    }
                }
            };

            // Clean up any remaining reference to the dismounted passenger.
            passenger_entity.set_pos(dismount_pos);

            // Phase 2: Teleport to safety (unblocks movement)
            if let Some(player) = passenger.get_player() {
                if let Some(id) = teleport_id {
                    player.get_entity().set_pos(dismount_pos);
                    // Update awaiting_teleport with the real dismount position
                    *player
                        .awaiting_teleport
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some((id.into(), dismount_pos));
                    // Use send_client_packet so the teleport goes through
                    // the same packet queue as CSetPassengers, preserving send order.
                    // Vanilla uses DELTA | ROT flags: position absolute, delta/rotation relative.
                    // With rotation relative and yaw/pitch=0, the client preserves its current look.
                    player.try_send_client_packet(&CPlayerPosition::new(
                        id.into(),
                        dismount_pos,
                        Vector3::new(0.0, 0.0, 0.0),
                        0.0,
                        0.0,
                        vec![
                            PositionFlag::DeltaX,
                            PositionFlag::DeltaY,
                            PositionFlag::DeltaZ,
                            PositionFlag::YRot,
                            PositionFlag::XRot,
                        ],
                    ));
                }

                // Vanilla: setSneaking(false) after dismount via sneak input
                if passenger_entity.sneaking.load(Relaxed) {
                    passenger_entity.set_sneaking(false);
                }
            } else {
                passenger_entity.set_pos(dismount_pos);
            }
        }
    }

    pub fn check_out_of_world(&self, dyn_self: &dyn EntityBase) {
        if self.pos.load().y < f64::from(self.world.load().dimension.min_y) - 64.0 {
            dyn_self.tick_in_void(dyn_self);
        }
    }

    pub fn reset_state(&self) {
        self.pose.store(EntityPose::Standing);
        self.fall_flying.store(false, Relaxed);
        self.extinguish();
        self.set_on_fire(false);
    }

    pub fn slow_movement(&self, state: &BlockState, multiplier: Vector3<f64>) {
        match self.entity_type.id {
            v if v == EntityType::PLAYER.id => {
                if let Some(player_entity) = self.get_player()
                    && player_entity.is_flying()
                {
                    return;
                }
            }
            v if (v == EntityType::SPIDER.id || v == EntityType::CAVE_SPIDER.id)
                && Block::from_state_id(state.id).id == Block::COBWEB.id =>
            {
                return;
            }
            v if v == EntityType::WITHER.id => {
                return;
            }
            _ => {}
        }
        self.reset_fall_distance();
        self.movement_multiplier.store(multiplier);
    }

    pub fn set_custom_data(&self, namespace: &str, key: &str, value: NbtTag) {
        let mut custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut namespace_data = custom_data
            .child_tags
            .remove(namespace)
            .and_then(|tag| match tag {
                NbtTag::Compound(compound) => Some(compound),
                _ => None,
            })
            .unwrap_or_default();

        namespace_data.child_tags.insert(key.into(), value);
        custom_data
            .child_tags
            .insert(namespace.into(), NbtTag::Compound(namespace_data));
    }

    pub fn get_custom_data(&self, namespace: &str, key: &str) -> Option<NbtTag> {
        let custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        custom_data
            .get(namespace)?
            .extract_compound()?
            .get(key)
            .cloned()
    }

    pub fn remove_custom_data(&self, namespace: &str, key: &str) {
        let mut custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let Some(NbtTag::Compound(mut namespace_data)) = custom_data.child_tags.remove(namespace)
        else {
            return;
        };

        namespace_data.child_tags.remove(key);
        if !namespace_data.is_empty() {
            custom_data
                .child_tags
                .insert(namespace.into(), NbtTag::Compound(namespace_data));
        }
    }

    pub fn has_custom_data(&self, namespace: &str, key: &str) -> bool {
        self.get_custom_data(namespace, key).is_some()
    }
}

impl Entity {
    /// The `Pos` vanilla `saveWithoutId` writes: a passenger is saved over its vehicle, at its
    /// own height.
    fn saved_position(&self) -> Vector3<f64> {
        let mut position = self.pos.load();
        if let Some(vehicle) = self
            .vehicle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            let vehicle_pos = vehicle.get_entity().pos.load();
            position.x = vehicle_pos.x;
            position.z = vehicle_pos.z;
        }
        position
    }

    pub fn write_nbt(&self, nbt: &mut NbtCompound) {
        let position = self.saved_position();
        nbt.put_string(
            "id",
            format!("minecraft:{}", self.entity_type.resource_name),
        );
        nbt.put_uuid("UUID", self.entity_uuid);
        nbt.put(
            "Pos",
            NbtTag::List(vec![
                position.x.into(),
                position.y.into(),
                position.z.into(),
            ]),
        );
        let velocity = self.velocity.load();
        nbt.put(
            "Motion",
            NbtTag::List(vec![
                velocity.x.into(),
                velocity.y.into(),
                velocity.z.into(),
            ]),
        );
        nbt.put(
            "Rotation",
            NbtTag::List(vec![self.yaw.load().into(), self.pitch.load().into()]),
        );
        nbt.put_double("fall_distance", self.fall_distance.load());
        nbt.put_short("Fire", self.fire_ticks.load(Relaxed) as i16);
        nbt.put_bool("OnGround", self.on_ground.load(Relaxed));
        nbt.put_bool("Invulnerable", self.invulnerable.load(Relaxed));
        let invulnerable_time = self.invulnerable_time.load(Relaxed);
        if invulnerable_time > 0 {
            nbt.put_int("invulnerable_time", invulnerable_time);
        }
        if self.is_silent() {
            nbt.put_bool("Silent", true);
        }
        if self.has_no_gravity() {
            nbt.put_bool("NoGravity", true);
        }
        nbt.put_int("PortalCooldown", self.portal_cooldown.load(Relaxed) as i32);
        if self.has_visual_fire.load(Relaxed) {
            nbt.put_bool("HasVisualFire", true);
        }
        let ticks_frozen = self.frozen_ticks.load(Relaxed);
        if ticks_frozen > 0 {
            nbt.put_int("TicksFrozen", ticks_frozen);
        }
        if self.has_glowing_tag.load(Relaxed) {
            nbt.put_bool("Glowing", true);
        }
        if let Some(custom_name) = &**self.custom_name.load() {
            nbt.put(
                "CustomName",
                custom_name
                    .to_nbt_tag_for_version(&pumpkin_util::version::JavaMinecraftVersion::V_26_3),
            );
        }
        if self.custom_name_visible.load(Relaxed) {
            nbt.put_bool("CustomNameVisible", true);
        }

        let tags = self
            .scoreboard_tags
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !tags.is_empty() {
            nbt.put(
                "Tags",
                NbtTag::List(
                    tags.iter()
                        .map(|tag| NbtTag::String(tag.as_str().into()))
                        .collect(),
                ),
            );
        }

        let data = self
            .entity_custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !data.is_empty() {
            nbt.put_compound("data", data.clone());
        }
        drop(data);

        let custom_data = self
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !custom_data.is_empty() {
            nbt.put_compound("PumpkinCustomData", custom_data.clone());
        }

        // todo more...
    }

    /// The end of vanilla `Entity.saveWithoutId`: every passenger `saveAsPassenger` accepts,
    /// each with its own passengers.
    pub fn write_passengers_nbt(&self, nbt: &mut NbtCompound) {
        let passengers = self
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let list: Vec<NbtTag> = passengers
            .iter()
            .filter(|passenger| passenger.get_entity().should_save_as_passenger())
            .map(|passenger| {
                let mut passenger_nbt = NbtCompound::new();
                passenger.write_nbt(&mut passenger_nbt);
                NbtTag::Compound(passenger_nbt)
            })
            .collect();
        if !list.is_empty() {
            nbt.put("Passengers", NbtTag::List(list));
        }
    }

    /// Vanilla `Entity.saveAsPassenger`'s checks: not removed for good, and a type that can
    /// be saved (never a player).
    #[must_use]
    pub fn should_save_as_passenger(&self) -> bool {
        self.removal_reason
            .load()
            .is_none_or(|reason| reason.should_save())
            && self.entity_type != &EntityType::PLAYER
    }

    /// Vanilla `Entity.shouldBeSaved`: not removed for good, not a passenger (saved in its
    /// vehicle's `Passengers`), and not a vehicle carrying exactly one player (saved with that
    /// player, `ServerPlayer.saveParentVehicle`).
    #[must_use]
    pub fn should_be_saved(&self) -> bool {
        self.removal_reason
            .load()
            .is_none_or(|reason| reason.should_save())
            && !self.has_vehicle()
            && (!self.has_passengers() || !self.has_exactly_one_player_passenger())
    }

    /// Vanilla `Entity.getIndirectPassengers`: every passenger, then its own, depth first.
    #[must_use]
    pub fn indirect_passengers(&self) -> Vec<Arc<dyn EntityBase>> {
        let mut passengers = Vec::new();
        self.collect_indirect_passengers(&mut passengers);
        passengers
    }

    /// Vanilla `Entity.countPlayerPassengers`: the indirect passengers that are players.
    #[must_use]
    pub fn count_player_passengers(&self) -> usize {
        self.indirect_passengers()
            .iter()
            .filter(|passenger| passenger.get_player().is_some())
            .count()
    }

    /// Vanilla `Entity.hasExactlyOnePlayerPassenger`.
    #[must_use]
    pub fn has_exactly_one_player_passenger(&self) -> bool {
        self.count_player_passengers() == 1
    }

    /// Vanilla `Entity.load`'s `Glowing` and `data`.
    fn read_glowing_and_custom_data(&self, nbt: &NbtCompound) {
        if let Some(glowing) = nbt.get_bool("Glowing") {
            // `setGlowingTag`: the shared flag follows the tag on the server.
            self.has_glowing_tag.store(glowing, Relaxed);
            self.set_glowing(glowing);
        }
        if let Some(data) = nbt.get_compound("data") {
            *self
                .entity_custom_data
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = data.clone();
        }
    }

    /// The `Pos`, `Motion` and `Rotation` part of vanilla `Entity.load`: motion over 10 per
    /// axis is dropped, the position clamped to the world limits, and the head and body take
    /// the raw yaw before `setRot` wraps it.
    fn read_position_and_rotation(&self, nbt: &NbtCompound) {
        if let Some(position) = nbt.get_list("Pos")
            && position.len() >= 3
        {
            let x = position[0].extract_double().unwrap_or(0.0);
            let y = position[1].extract_double().unwrap_or(0.0);
            let z = position[2].extract_double().unwrap_or(0.0);
            let pos = Vector3::new(
                x.clamp(-MAX_HORIZONTAL_POSITION, MAX_HORIZONTAL_POSITION),
                y.clamp(-MAX_VERTICAL_POSITION, MAX_VERTICAL_POSITION),
                z.clamp(-MAX_HORIZONTAL_POSITION, MAX_HORIZONTAL_POSITION),
            );
            self.set_pos(pos);
            self.last_sent_pos.store(pos);
        }
        if let Some(velocity) = nbt.get_list("Motion")
            && velocity.len() >= 3
        {
            let component = |tag: &NbtTag| {
                let value = tag.extract_double().unwrap_or(0.0);
                if value.abs() > 10.0 { 0.0 } else { value }
            };
            self.velocity.store(Vector3::new(
                component(&velocity[0]),
                component(&velocity[1]),
                component(&velocity[2]),
            ));
            self.velocity_dirty.store(true, Ordering::Relaxed);
        }
        if let Some(rotation) = nbt.get_list("Rotation")
            && rotation.len() >= 2
        {
            let yaw = rotation[0].extract_float().unwrap_or(0.0);
            let pitch = rotation[1].extract_float().unwrap_or(0.0);
            if yaw.is_finite() {
                self.head_yaw.store(yaw);
                self.body_yaw.store(yaw);
                // `setRot`: both wrap (keeping their sign), the pitch is then clamped.
                self.yaw.store(yaw % 360.0);
            }
            self.set_pitch(pitch);
            let yaw = self.yaw.load();
            let yaw_byte = pack_degrees(yaw);
            self.last_sent_yaw.store(yaw_byte, Relaxed);
            self.last_sent_pitch
                .store(pack_degrees(self.pitch.load()), Relaxed);
            self.last_sent_head_yaw
                .store(pack_degrees(self.head_yaw.load()), Relaxed);
        }
    }

    /// The `Team` part of vanilla `Entity.load`: joins the team under the entity's UUID.
    fn read_team(&self, nbt: &NbtCompound) {
        let Some(team_name) = nbt.get_string("Team") else {
            return;
        };
        let world = self.world.load_full();
        let mut scoreboard = world.lock_scoreboard();
        if !scoreboard.add_player_to_team(&world, team_name, self.entity_uuid.to_string()) {
            tracing::warn!(
                "Unable to add entity to team \"{team_name}\" (that team probably doesn't exist)"
            );
        }
    }

    pub fn read_nbt_non_mut(&self, nbt: &NbtCompound) {
        self.read_position_and_rotation(nbt);
        // Only keys present are applied, so partial NBT leaves the rest untouched.
        // `FallDistance` is the float from before 1.21.5.
        if let Some(fall_distance) = nbt
            .get_double("fall_distance")
            .or_else(|| nbt.get_float("FallDistance").map(f64::from))
        {
            self.fall_distance.store(fall_distance);
        }
        if let Some(fire) = nbt.get_short("Fire") {
            self.fire_ticks.store(i32::from(fire), Relaxed);
        }
        if let Some(on_ground) = nbt.get_bool("OnGround") {
            self.on_ground.store(on_ground, Relaxed);
        }
        if let Some(invulnerable) = nbt.get_bool("Invulnerable") {
            self.invulnerable.store(invulnerable, Relaxed);
        }
        if let Some(invulnerable_time) = nbt.get_int("invulnerable_time") {
            self.invulnerable_time.store(invulnerable_time, Relaxed);
        }
        if let Some(silent) = nbt.get_bool("Silent") {
            self.set_silent(silent);
        }
        if let Some(no_gravity) = nbt.get_bool("NoGravity") {
            self.set_has_no_gravity(no_gravity);
        }
        if let Some(cooldown) = nbt.get_int("PortalCooldown") {
            self.portal_cooldown.store(cooldown as u32, Relaxed);
        }
        if let Some(visual_fire) = nbt.get_bool("HasVisualFire") {
            self.has_visual_fire.store(visual_fire, Relaxed);
        }
        if let Some(frozen) = nbt.get_int("TicksFrozen") {
            self.set_frozen_ticks(frozen);
        }
        self.read_glowing_and_custom_data(nbt);
        if let Some(name) = nbt.get("CustomName") {
            // `ComponentSerialization.CODEC`: a string is literal text.
            self.set_custom_name(TextComponent::from_nbt(name));
        }
        if let Some(visible) = nbt.get_bool("CustomNameVisible") {
            self.set_custom_name_visible(visible);
        }

        if let Some(tag_list) = nbt.get_list("Tags") {
            let mut tags = self
                .scoreboard_tags
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            tags.clear();
            // `sizeLimitedListOf(1024)`: a longer list fails to read at all.
            if tag_list.len() <= MAX_SCOREBOARD_TAGS {
                tags.extend(
                    tag_list
                        .iter()
                        .filter_map(|tag| tag.extract_string().map(str::to_owned)),
                );
            }
        }
        self.read_team(nbt);

        if let Some(custom_data) = nbt
            .get_compound("PumpkinCustomData")
            .or_else(|| nbt.get_compound("BukkitValues"))
        {
            let mut data = self
                .custom_data
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *data = custom_data.clone();
        }

        // todo more...
    }
}

impl EntityBase for Entity {
    /// Vanilla `Entity.baseTick`.
    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        self.update_last_pos();
        let riding_cooldown = self.riding_cooldown.load(Ordering::Relaxed);
        if riding_cooldown > 0 {
            self.riding_cooldown
                .store(riding_cooldown - 1, Ordering::Relaxed);
        }
        self.handle_portal(caller);
        // Recomputed during movement/block-collision handling in the same tick.
        let was_in_powder_snow = self.is_in_powder_snow.load(Ordering::Relaxed);
        self.was_in_powder_snow
            .store(was_in_powder_snow, Ordering::Relaxed);
        self.is_in_powder_snow.store(false, Ordering::Relaxed);
        self.was_eye_in_water
            .store(self.eye_in_water.load(Ordering::Relaxed), Ordering::Relaxed);
        self.update_fluid_state(caller);
        if let Some(mob) = caller.get_mob() {
            mob.update_swimming();
        } else if caller.get_player().is_none() {
            self.update_swimming_base();
        }

        let fire_ticks = self.fire_ticks.load(Ordering::Relaxed);
        if fire_ticks > 0 {
            if self.is_fire_immune() {
                self.extinguish();
            } else {
                if fire_ticks % 20 == 0 && !self.is_in_lava() {
                    caller.damage(caller, 1.0, DamageType::ON_FIRE);
                }
                caller.set_remaining_fire_ticks(self.fire_ticks.load(Ordering::Relaxed) - 1);
            }
        }

        if self.is_in_lava() {
            self.fall_distance.store(self.fall_distance.load() * 0.5);
        }

        self.check_out_of_world(caller);
        self.set_on_fire(self.fire_ticks.load(Ordering::Relaxed) > 0);
        self.first_tick.store(false, Ordering::Relaxed);
        if caller.is_leashable() {
            leashable::tick_leash(caller);
        }
    }

    fn get_entity(&self) -> &Entity {
        self
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl<T: EntityBase + ?Sized> NBTStorage for T {
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        EntityBase::write_nbt(self, nbt);
    }

    fn read_nbt_non_mut(&self, nbt: &NbtCompound) {
        EntityBase::read_nbt_non_mut(self, nbt);
    }
}

pub trait NBTStorage: Send + Sync {
    fn write_nbt(&self, _nbt: &mut NbtCompound) {}

    fn read_nbt(&mut self, nbt: &mut NbtCompound) {
        self.read_nbt_non_mut(nbt);
    }

    fn read_nbt_non_mut(&self, _nbt: &NbtCompound) {}
}

pub trait NBTStorageInit: Send + Sync + Sized {
    fn create_from_nbt(_nbt: &mut NbtCompound) -> Option<Self> {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Represents various entity flags that are sent in entity metadata.
///
/// These flags are used by the client to modify the rendering of entities based on their current state.
///
/// **Purpose:**
///
/// This enum provides a more type-safe and readable way to represent entity flags compared to using raw integer values.
pub enum Flag {
    /// Indicates if the entity is on fire.
    OnFire = 0,
    /// Indicates if the entity is sneaking.
    Sneaking = 1,
    /// Indicates if the entity is sprinting.
    Sprinting = 3,
    /// Indicates if the entity is swimming.
    Swimming = 4,
    /// Indicates if the entity is invisible.
    Invisible = 5,
    /// Indicates if the entity is glowing.
    Glowing = 6,
    /// Indicates if the entity is flying due to a fall.
    FallFlying = 7,
}

impl Flag {
    #[must_use]
    pub const fn to_bedrock(&self) -> Option<u32> {
        match self {
            Self::OnFire => Some(entity_data_flag::ON_FIRE),
            Self::Sneaking => Some(entity_data_flag::SNEAKING),
            Self::Sprinting => Some(entity_data_flag::SPRINTING),
            Self::Swimming => Some(entity_data_flag::SWIMMING),
            Self::Invisible => Some(entity_data_flag::INVISIBLE),
            Self::FallFlying => Some(entity_data_flag::GLIDING),
            Self::Glowing => None,
        }
    }
}

/// Vanilla `Entity.random = RandomSource.create()`, then the two `nextLong`s
/// `uuid = Mth.createInsecureUUID(this.random)` takes; Pumpkin keeps the UUID it was given.
fn new_entity_random() -> pumpkin_util::random::legacy_rand::LegacyRand {
    let mut random =
        pumpkin_util::random::legacy_rand::LegacyRand::from_seed(pumpkin_util::random::get_seed());
    random.next_i64();
    random.next_i64();
    random
}

/// The next stamp for [`Entity::section_order`].
fn next_section_order() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The order vanilla's entity box queries visit `entity` in.
///
/// `EntitySectionStorage.forEachAccessibleNonEmptySection` goes by section x, then by the
/// section keys `SectionPos.asLong` sorts for that x (z, then y, as their masked bits); each
/// `EntitySection` lists its entities in the order they entered.
#[must_use]
pub fn section_iteration_key(entity: &Entity) -> (i32, u32, u32, u64) {
    let section = entity.block_pos.load().0;
    let (x, y, z) = (section.x >> 4, section.y >> 4, section.z >> 4);
    (
        x,
        (z as u32) & 0x3F_FFFF,
        (y as u32) & 0xF_FFFF,
        entity.section_order.load(Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {

    #[test]
    fn find_free_position_steps_out_of_a_wall() {
        use super::find_free_position;
        use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};
        // A 1.0-wide box next to a wall block on its +x side can't have its centre past 0.5:
        // a centre already clear stays put, one in the wall is pulled back to 0.5.
        let center = Vector3::new(0.3, 0.5, 0.5);
        let half = Vector3::new(0.2, 0.5e-6, 0.2);
        let allowed = BoundingBox::new(center - half, center + half);
        let wall = BoundingBox::new(Vector3::new(1.0, 0.0, 0.0), Vector3::new(2.0, 1.0, 1.0));
        let free = find_free_position(allowed, &[wall], center, Vector3::new(1.0, 1.0, 1.0));
        assert_eq!(free.map(|p| p.x), Some(0.3));
        let free = find_free_position(
            BoundingBox::new(
                Vector3::new(0.4, 0.5, 0.3),
                Vector3::new(0.6, 0.500_000_5, 0.7),
            ),
            &[wall],
            Vector3::new(0.55, 0.5, 0.5),
            Vector3::new(1.0, 1.0, 1.0),
        );
        assert_eq!(free.map(|p| p.x), Some(0.5));
    }

    #[test]
    fn fluid_current_impulse_like_vanilla() {
        use super::{FluidCurrent, LAVA_SLOW_FLOW_SCALE, WATER_FLOW_SCALE};
        use pumpkin_util::math::vector3::Vector3;
        let current = FluidCurrent {
            height: 1.0,
            accumulated: Vector3::new(0.0, 0.0, 2.0),
            count: 2,
        };
        // A mob at rest in flowing water gets the full 0.014, not the 0.0045 floor.
        let impulse = current
            .impulse(false, Vector3::new(0.0, 0.0, 0.0), WATER_FLOW_SCALE)
            .unwrap_or_default();
        assert!((impulse.z - 0.014).abs() < 1e-12);
        // Slow lava is raised to the floor even while the entity moves vertically.
        let impulse = current
            .impulse(false, Vector3::new(0.0, -0.5, 0.0), LAVA_SLOW_FLOW_SCALE)
            .unwrap_or_default();
        assert!((impulse.z - 0.004_500_000_000_000_000_5).abs() < 1e-15);
        // A negligible current does nothing.
        let tiny = FluidCurrent {
            height: 1.0,
            accumulated: Vector3::new(0.0, 0.0, 0.001),
            count: 1,
        };
        assert!(
            tiny.impulse(false, Vector3::new(0.0, 0.0, 0.0), WATER_FLOW_SCALE)
                .is_none()
        );
    }
    use super::*;

    #[test]
    fn equipment_break_status_maps_all_slots() {
        // Status bytes from vanilla EntityEvent: mainhand=47, offhand=48,
        // head=49, chest=50, legs=51, feet=52, body=65, saddle=68.
        let cases: &[(&EquipmentSlot, u8)] = &[
            (&EquipmentSlot::MAIN_HAND, EntityStatus::MainhandBreak as u8),
            (&EquipmentSlot::OFF_HAND, EntityStatus::OffhandBreak as u8),
            (&EquipmentSlot::HEAD, EntityStatus::HeadBreak as u8),
            (&EquipmentSlot::CHEST, EntityStatus::ChestBreak as u8),
            (&EquipmentSlot::LEGS, EntityStatus::LegsBreak as u8),
            (&EquipmentSlot::FEET, EntityStatus::FeetBreak as u8),
            (&EquipmentSlot::BODY, EntityStatus::BodyBreak as u8),
            (&EquipmentSlot::SADDLE, EntityStatus::SaddleBreak as u8),
        ];
        for (i, (slot, expected)) in cases.iter().enumerate() {
            assert_eq!(
                equipment_break_status(slot) as u8,
                *expected,
                "status mismatch at index {i}"
            );
        }
    }
}

/// Vanilla `LivingEntity.getPassengerRidingPosition`, or plain `Entity`'s for a vehicle that
/// isn't living.
///
/// The vehicle's position plus its passenger point, measured at its current size for its
/// pose and given its size and age scale.
#[must_use]
pub fn passenger_riding_position(
    vehicle: &dyn EntityBase,
    passenger: &dyn EntityBase,
) -> Vector3<f64> {
    let entity = vehicle.get_entity();
    let (dimensions, scale) = vehicle.get_living_entity().map_or_else(
        || (entity.entity_dimension.load(), 1.0),
        |living| {
            let age_scale = vehicle.get_mob().map_or_else(
                || if entity.is_baby() { 0.5 } else { 1.0 },
                mob::Mob::age_scale,
            );
            (
                vehicle.dimensions_for_pose(entity.pose.load()),
                living.get_scale() * age_scale,
            )
        },
    );
    entity.pos.load() + vehicle.passenger_attachment_point(passenger, dimensions, scale)
}

/// The body of vanilla `Entity.positionRider`: the passenger's vehicle point onto the
/// vehicle's passenger point.
pub fn set_rider_position(vehicle: &dyn EntityBase, passenger: &dyn EntityBase) {
    let position = passenger_riding_position(vehicle, passenger);
    let offset = passenger.vehicle_attachment_point(vehicle);
    passenger.get_entity().set_pos(position - offset);
}

/// Vanilla `LivingEntity.maxUpStep`: the `step_height` attribute, at least a block while a
/// player steers.
#[must_use]
pub fn living_max_up_step(living: &LivingEntity, controller: Option<Arc<dyn EntityBase>>) -> f32 {
    let step =
        living.get_attribute_value(&pumpkin_data::attributes::Attributes::STEP_HEIGHT) as f32;
    if controller.is_some_and(|c| c.get_player().is_some()) {
        step.max(1.0)
    } else {
        step
    }
}

/// Vanilla `Entity.collideWithShapes`: each axis in `Direction.axisStepOrder`, the box moved
/// by what the earlier axes allowed, every shape clamping the move (`Shapes.collide`).
fn collide_with_shapes(
    movement: Vector3<f64>,
    bounding_box: &BoundingBox,
    shapes: &[BoundingBox],
) -> Vector3<f64> {
    if shapes.is_empty() {
        return movement;
    }
    let order = if movement.x.abs() < movement.z.abs() {
        [Axis::Y, Axis::Z, Axis::X]
    } else {
        [Axis::Y, Axis::X, Axis::Z]
    };
    let mut resolved = Vector3::new(0.0, 0.0, 0.0);
    for axis in order {
        let axis_movement = movement.get_axis(axis);
        if axis_movement == 0.0 {
            continue;
        }
        let moving = bounding_box.shift(resolved);
        let mut distance = axis_movement;
        for shape in shapes {
            if distance.abs() < 1.0e-7 {
                distance = 0.0;
                break;
            }
            distance = shape.collide_along(&moving, axis, distance);
        }
        resolved.set_axis(axis, distance);
    }
    resolved
}

/// Vanilla `Entity.collectCandidateStepUpHeights`: the colliders' Y coordinates relative to
/// the box's bottom, from 0 up to `max_step_height`, apart from the height already reached;
/// sorted.
fn collect_candidate_step_up_heights(
    bounding_box: &BoundingBox,
    colliders: &[BoundingBox],
    max_step_height: f32,
    step_height_to_skip: f32,
) -> Vec<f32> {
    let mut candidates: Vec<f32> = Vec::with_capacity(4);
    for collider in colliders {
        for coord in [collider.min.y, collider.max.y] {
            let relative = (coord - bounding_box.min.y) as f32;
            #[expect(clippy::float_cmp, reason = "vanilla skips the exact height")]
            let skip = relative == step_height_to_skip;
            if relative < 0.0 || skip {
                continue;
            }
            if relative > max_step_height {
                break;
            }
            if !candidates.contains(&relative) {
                candidates.push(relative);
            }
        }
    }
    candidates.sort_by(f32::total_cmp);
    candidates
}

/// Vanilla `Entity.blocksBuilding`: living entities (but marker armor stands), boats,
/// minecarts, falling blocks, primed TNT and end crystals.
#[must_use]
pub fn blocks_building(entity: &dyn EntityBase) -> bool {
    let entity_type = entity.get_entity().entity_type;
    if let Some(armor_stand) = entity
        .cast_any()
        .downcast_ref::<decoration::armor_stand::ArmorStandEntity>()
    {
        return !armor_stand.is_marker();
    }
    let name = entity_type.resource_name;
    entity.get_living_entity().is_some()
        || name.ends_with("_boat")
        || name.ends_with("_raft")
        || name.contains("minecart")
        || [
            &EntityType::FALLING_BLOCK,
            &EntityType::TNT,
            &EntityType::END_CRYSTAL,
        ]
        .contains(&entity_type)
}

/// Vanilla `Entity.teleportTo(x, y, z)` and `ServerPlayer`'s override, which teleports the
/// client like a command does.
pub fn teleport_to(entity: &dyn EntityBase, position: Vector3<f64>) {
    let base = entity.get_entity();
    let world = base.world.load_full();
    if let Some(player) = entity.get_player() {
        // `ServerPlayer.teleportTo(x, y, z)`: the client is moved, keeping its speed and look;
        // nothing dismounts.
        player.teleport_in_world(&TeleportTarget {
            world,
            position,
            yaw: None,
            pitch: None,
            keep_velocity: true,
            as_passenger: true,
        });
    } else {
        base.teleport(position, None, None, &world);
    }
}

/// Vanilla `Direction.fromYRot` (`Entity.getDirection`): the horizontal direction a yaw faces.
#[must_use]
pub fn direction_from_yaw(yaw: f32) -> BlockDirection {
    let wrapped_yaw = (yaw % 360.0 + 360.0) % 360.0;
    if !(45.0..315.0).contains(&wrapped_yaw) {
        BlockDirection::South
    } else if (45.0..135.0).contains(&wrapped_yaw) {
        BlockDirection::West
    } else if (135.0..225.0).contains(&wrapped_yaw) {
        BlockDirection::North
    } else {
        BlockDirection::East
    }
}

/// Vanilla `Entity.teleportTo(level, ...)`, as `/tp` and `/spreadplayers` use it.
///
/// A sleeping player wakes first (`ServerPlayer.teleportTo`: `stopSleepInBed(true, true)`),
/// then the teleport. Other teleports (pearls, gateways, plugins) don't wake.
pub fn command_teleport(
    entity: &dyn EntityBase,
    position: Vector3<f64>,
    yaw: Option<f32>,
    pitch: Option<f32>,
    world: Arc<World>,
) {
    if let Some(player) = entity.get_player()
        && player.is_sleeping()
    {
        player.stop_sleep_in_bed(true, true);
    }
    entity.teleport(position, yaw, pitch, world);
}

/// Vanilla `TeleportTransition`, as Pumpkin's teleports use it: an absolute position, and a
/// rotation that stays the entity's own where it is `None`.
#[derive(Clone)]
pub struct TeleportTarget {
    pub world: Arc<World>,
    pub position: Vector3<f64>,
    pub yaw: Option<f32>,
    pub pitch: Option<f32>,
    /// `Relative.DELTA`: keep the entity's speed instead of stopping it.
    pub keep_velocity: bool,
    /// Vanilla `asPassenger`: the entity keeps riding and nobody is sent the teleport.
    pub as_passenger: bool,
}

/// Vanilla `Entity.teleportToPortalDestination`.
pub fn teleport_to_portal_destination(
    entity: &Arc<dyn EntityBase>,
    world: &World,
    transition: TeleportTransition,
) {
    let new_world = transition.new_world;
    if !(world.is_allowed_to_enter_portal(&new_world)
        && (new_world.dimension == world.dimension || entity.can_teleport(world, &new_world)))
    {
        return;
    }
    // `ServerPlayer.teleport` first tells the player its respawn block is gone.
    if transition.missing_respawn_block
        && let Some(player) = entity.get_player()
    {
        player.try_send_client_packet(&pumpkin_protocol::java::client::play::CGameEvent::new(
            pumpkin_protocol::java::client::play::GameEvent::NoRespawnBlockAvailable,
            0.0,
        ));
    }
    teleport_transition(
        entity,
        &TeleportTarget {
            world: new_world,
            position: transition.position,
            yaw: transition.yaw,
            pitch: transition.pitch,
            keep_velocity: transition.keep_velocity,
            as_passenger: false,
        },
    );
}

/// Vanilla `Entity.teleport(TeleportTransition)`, and `ServerPlayer.teleport` for a player.
/// Returns the entity at its destination: a new one after a non-player's change of dimension.
pub fn teleport_transition(
    entity: &Arc<dyn EntityBase>,
    target: &TeleportTarget,
) -> Option<Arc<dyn EntityBase>> {
    let base = entity.get_entity();
    if entity.get_player().is_some() {
        let player = base.world.load().get_player_by_id(base.entity_id)?;
        return player.teleport_transition(target);
    }
    if base.is_removed() {
        return None;
    }
    let old_world = base.world.load_full();
    if !target.as_passenger
        && let Some(vehicle) = base.get_vehicle()
    {
        vehicle.get_entity().remove_passenger(base.entity_id);
    }
    if Arc::ptr_eq(&old_world, &target.world) {
        teleport_same_dimension(entity, &old_world, target);
        Some(entity.clone())
    } else {
        Some(teleport_cross_dimension(entity, &old_world, target))
    }
}

/// Vanilla `Entity.calculatePassengerTransition`: the passenger keeps its offset from the
/// vehicle, and its rotation relative to the vehicle's.
fn passenger_transition(
    vehicle: &Entity,
    passenger: &Entity,
    target: &TeleportTarget,
) -> TeleportTarget {
    let offset = passenger.pos.load() - vehicle.pos.load();
    TeleportTarget {
        world: target.world.clone(),
        position: target.position + offset,
        yaw: target
            .yaw
            .map(|yaw| yaw + (passenger.yaw.load() - vehicle.yaw.load())),
        pitch: target
            .pitch
            .map(|pitch| pitch + (passenger.pitch.load() - vehicle.pitch.load())),
        keep_velocity: target.keep_velocity,
        as_passenger: true,
    }
}

/// Vanilla `Entity.teleportSameDimension`.
fn teleport_same_dimension(entity: &Arc<dyn EntityBase>, world: &World, target: &TeleportTarget) {
    let base = entity.get_entity();
    let passengers = base
        .passengers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    for passenger in passengers {
        let transition = passenger_transition(base, passenger.get_entity(), target);
        teleport_transition(&passenger, &transition);
    }
    base.teleport_set_position(target);
    if !target.as_passenger {
        base.send_teleport_transition_to_players(entity.as_ref(), world, target);
    }
}

/// Vanilla `Entity.teleportCrossDimension`: a copy of the entity (`restoreFrom`) carries on
/// in the new dimension and the passengers board it again.
fn teleport_cross_dimension(
    entity: &Arc<dyn EntityBase>,
    old_world: &Arc<World>,
    target: &TeleportTarget,
) -> Arc<dyn EntityBase> {
    let base = entity.get_entity();
    let old_passengers = base
        .passengers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    base.eject_passengers();
    let mut new_passengers = Vec::with_capacity(old_passengers.len());
    for passenger in &old_passengers {
        let transition = passenger_transition(base, passenger.get_entity(), target);
        // A player's dimension change finishes later (`teleport_world`): it and the new
        // vehicle meet through its `passenger_remount`.
        let player = passenger.get_player();
        if let Some(player) = player {
            player.set_passenger_remount(PassengerRemount::Travelling);
        }
        if let Some(new_passenger) = teleport_transition(passenger, &transition) {
            new_passengers.push(new_passenger);
        } else if let Some(player) = player {
            player.set_passenger_remount(PassengerRemount::Idle);
        }
    }

    // `restoreFrom`: everything the entity saves, its portal cooldown and portal state.
    let new_entity = r#type::from_type(
        base.entity_type,
        base.pos.load(),
        &target.world,
        base.entity_uuid,
    );
    let mut nbt = NbtCompound::new();
    entity.write_nbt(&mut nbt);
    new_entity.read_nbt_non_mut(&nbt);
    let new_base = new_entity.get_entity();
    new_base.portal_cooldown.store(
        base.portal_cooldown.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
    *new_base
        .portal_manager
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = base
        .portal_manager
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();

    // `removeAfterChangingDimensions`: `setRemoved` first, then `removeLeash`. The entity is
    // untracked by then, so the unlink reaches nobody, as in vanilla.
    old_world.remove_entity(entity.as_ref());
    base.removal_reason
        .store(Some(RemovalReason::ChangedDimension));
    if entity.is_leashable() {
        leashable::drop_leash(entity.as_ref(), true, false);
    }

    new_base.velocity.store(base.velocity.load());
    new_base.yaw.store(base.yaw.load());
    new_base.pitch.store(base.pitch.load());
    new_base.teleport_set_position(target);
    new_entity.init_data_tracker();
    target.world.add_entity_silent(new_entity.clone());
    // `newPassenger.startRiding(newEntity, true, false)`.
    for new_passenger in new_passengers {
        if let Some(player) = new_passenger.get_player() {
            player.vehicle_ready(&new_passenger, &new_entity);
        } else {
            new_base.add_passenger_without_event(new_entity.clone(), new_passenger);
        }
    }
    new_entity
}

/// Vanilla `CollisionGetter.findFreePosition`'s geometry: the point of `allowed_centers` closest
/// to `preferred_center` where a box of `size` centred there touches none of `collisions`.
fn find_free_position(
    allowed_centers: BoundingBox,
    collisions: &[BoundingBox],
    preferred_center: Vector3<f64>,
    size: Vector3<f64>,
) -> Option<Vector3<f64>> {
    // `aabb.inflate(sizeX / 2, sizeY / 2, sizeZ / 2)`: where a centre would collide.
    let expanded: Vec<BoundingBox> = collisions
        .iter()
        .map(|shape| shape.expand(size.x / 2.0, size.y / 2.0, size.z / 2.0))
        .collect();
    // `Shapes.join(allowedCenters, expandedCollisions, ONLY_FIRST)` on the grid of every
    // box face inside the allowed box.
    let coords = |axis: Axis| {
        let (low, high) = (
            allowed_centers.min.get_axis(axis),
            allowed_centers.max.get_axis(axis),
        );
        let mut coords = vec![low, high];
        for shape in &expanded {
            for coord in [shape.min.get_axis(axis), shape.max.get_axis(axis)] {
                if coord > low && coord < high {
                    coords.push(coord);
                }
            }
        }
        coords.sort_by(f64::total_cmp);
        coords.dedup();
        coords
    };
    let (xs, ys, zs) = (coords(Axis::X), coords(Axis::Y), coords(Axis::Z));
    let mut closest: Option<(Vector3<f64>, f64)> = None;
    for x in xs.windows(2) {
        for y in ys.windows(2) {
            for z in zs.windows(2) {
                let mid = Vector3::new(
                    f64::midpoint(x[0], x[1]),
                    f64::midpoint(y[0], y[1]),
                    f64::midpoint(z[0], z[1]),
                );
                if expanded.iter().any(|shape| shape.contains(mid)) {
                    continue;
                }
                // `VoxelShape.closestPointTo`: the clamped point, the first box on a tie.
                let point = Vector3::new(
                    preferred_center.x.clamp(x[0], x[1]),
                    preferred_center.y.clamp(y[0], y[1]),
                    preferred_center.z.clamp(z[0], z[1]),
                );
                let distance = preferred_center.squared_distance_to_vec(&point);
                if closest.is_none_or(|(_, best)| distance < best) {
                    closest = Some((point, distance));
                }
            }
        }
    }
    closest.map(|(point, _)| point)
}
