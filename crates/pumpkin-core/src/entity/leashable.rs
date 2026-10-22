//! Vanilla `Leashable`'s static functions.
//!
//! The spring math (constants, `Wrench`, `computeDampenedSpringInteraction`) and attaching,
//! dropping and finding leashes. The overridable `Leashable` methods are hooks on
//! `EntityBase`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_protocol::codec::var_long::VarLong;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::vector3::Vector3;

use super::{DelayedLeash, Entity, EntityBase};
use crate::plugin::api::events::entity::entity_unleash::EntityUnleashEvent;
use crate::plugin::api::events::player::player_leash_entity::PlayerLeashEntityEvent;
use crate::plugin::api::events::player::player_unleash_entity::PlayerUnleashEntityEvent;
use crate::world::World;

/// Vanilla `Leashable.LEASH_TOO_FAR_DIST`.
pub const LEASH_TOO_FAR_DIST: f64 = 12.0;
/// Vanilla `Leashable.LEASH_ELASTIC_DIST`.
pub const LEASH_ELASTIC_DIST: f64 = 6.0;
/// Vanilla `Leashable.MAXIMUM_ALLOWED_LEASHED_DIST`.
pub const MAXIMUM_ALLOWED_LEASHED_DIST: f64 = 16.0;
/// Vanilla `Leashable.AXIS_SPECIFIC_ELASTICITY`.
pub const AXIS_SPECIFIC_ELASTICITY: Vector3<f64> = Vector3::new(0.8, 0.2, 0.8);
/// Vanilla `Leashable.SPRING_DAMPENING` (a Java `float`).
pub const SPRING_DAMPENING: f32 = 0.7;
/// Vanilla `Leashable.TORSIONAL_ELASTICITY`.
pub const TORSIONAL_ELASTICITY: f64 = 10.0;
/// Vanilla `Leashable.STIFFNESS`.
pub const STIFFNESS: f64 = 0.11;
/// Vanilla `Leashable.ENTITY_ATTACHMENT_POINT`.
pub const ENTITY_ATTACHMENT_POINT: [Vector3<f64>; 1] = [Vector3::new(0.0, 0.5, 0.5)];
/// Vanilla `Leashable.LEASHER_ATTACHMENT_POINT`.
pub const LEASHER_ATTACHMENT_POINT: [Vector3<f64>; 1] = [Vector3::new(0.0, 0.5, 0.0)];
/// Vanilla `Leashable.SHARED_QUAD_ATTACHMENT_POINTS`.
pub const SHARED_QUAD_ATTACHMENT_POINTS: [Vector3<f64>; 4] = [
    Vector3::new(-0.5, 0.5, 0.5),
    Vector3::new(-0.5, 0.5, -0.5),
    Vector3::new(0.5, 0.5, -0.5),
    Vector3::new(0.5, 0.5, 0.5),
];

/// Vanilla `Leashable.Wrench`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wrench {
    pub force: Vector3<f64>,
    pub torque: f64,
}

impl Wrench {
    pub const ZERO: Self = Self {
        force: Vector3::new(0.0, 0.0, 0.0),
        torque: 0.0,
    };

    /// `leverArm.z * force.x - leverArm.x * force.z`.
    #[must_use]
    pub fn torque_from_force(lever_arm: Vector3<f64>, force: Vector3<f64>) -> f64 {
        lever_arm.z * force.x - lever_arm.x * force.z
    }

    /// Sums x, y, z and torque in list order (`ZERO` for an empty list).
    #[must_use]
    pub fn accumulate(wrenches: &[Self]) -> Self {
        if wrenches.is_empty() {
            return Self::ZERO;
        }
        let (mut x, mut y, mut z, mut t) = (0.0, 0.0, 0.0, 0.0);
        for wrench in wrenches {
            x += wrench.force.x;
            y += wrench.force.y;
            z += wrench.force.z;
            t += wrench.torque;
        }
        Self {
            force: Vector3::new(x, y, z),
            torque: t,
        }
    }

    #[must_use]
    pub fn scale(self, scale: f64) -> Self {
        Self {
            force: self.force * scale,
            torque: self.torque * scale,
        }
    }
}

/// Vanilla `computeDampenedSpringInteraction`.
#[must_use]
pub fn compute_dampened_spring_interaction(
    pivot: Vector3<f64>,
    object: Vector3<f64>,
    slack: f64,
    object_motion: Vector3<f64>,
    lever_arm: Vector3<f64>,
) -> Option<Wrench> {
    // Vec3.distanceTo: d = other - this, sqrt(dx*dx + dy*dy + dz*dz).
    let d = pivot - object;
    let distance = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
    if distance < slack {
        return None;
    }
    let mut displacement = (pivot - object).normalize() * (distance - slack);
    let torque = Wrench::torque_from_force(lever_arm, displacement);
    if object_motion.dot(&displacement) >= 0.0 {
        displacement = displacement * f64::from(0.3f32);
    }
    Some(Wrench {
        force: displacement,
        torque,
    })
}

/// Vanilla `Entity.canInteractWithLevel`: alive (`LivingEntity`'s health-aware `isAlive`), not
/// removed and not a spectator.
#[must_use]
pub fn can_interact_with_level(entity: &dyn EntityBase) -> bool {
    super::inside_block_effect::is_alive(entity)
        && !entity.get_entity().is_removed()
        && !entity.is_spectator()
}

/// Vanilla `Leashable.tickLeash`, run at the end of `Entity.baseTick` for a leashable `caller`.
pub fn tick_leash(caller: &dyn EntityBase) {
    let entity = caller.get_entity();
    // `leashData != null && leashData.delayedLeashInfo != null`.
    if entity.delayed_leash_is_some() {
        restore_leash_from_save(caller);
    }
    // `leashData != null && leashData.leashHolder != null`: `restoreLeashFromSave` sets the
    // holder on that same `LeashData`, so a restored leash ticks right away.
    let Some(holder) = entity.leash_holder() else {
        return;
    };
    let world = entity.world.load_full();
    if !can_interact_with_level(caller) || !can_interact_with_level(holder.as_ref()) {
        if world.level_info.load().game_rules.entity_drops {
            drop_leash(caller, true, true);
        } else {
            drop_leash(caller, true, false);
        }
    }

    // `entity.getLeashHolder()`, now possibly dropped above.
    let Some(holder) = entity.leash_holder() else {
        return;
    };
    let holder_entity = holder.get_entity();
    if !Arc::ptr_eq(&holder_entity.world.load_full(), &world) {
        return;
    }
    // Vanilla keeps `leashData` in a local: a leash dropped below still rotates this tick by
    // the momentum its detached `LeashData` holds.
    let mut momentum = entity.leash_angular_momentum.load();
    let distance_to = leash_distance_to(entity, holder_entity);
    caller.when_leashed_to(caller, holder.as_ref());
    if distance_to > caller.leash_snap_distance() {
        world.play_sound(
            Sound::ItemLeadBreak,
            SoundCategory::Neutral,
            &holder_entity.pos.load(),
        );
        caller.leash_too_far_behaviour(caller);
    } else if distance_to
        > caller.leash_elastic_distance()
            - f64::from(holder_entity.width())
            - f64::from(entity.width())
        && let Some(new_momentum) = check_elastic_interactions(caller, holder.as_ref(), momentum)
    {
        momentum = new_momentum;
        caller.on_elastic_leash_pull(caller);
    } else {
        caller.close_range_leash_behaviour(caller, holder.as_ref());
    }

    // While still leashed the `LeashData` is the entity's own, so a reset made by the
    // behaviours above (`Mob.resetAngularLeashMomentum`) shows here as in vanilla.
    let leashed = entity.is_leashed();
    if leashed {
        momentum = entity.leash_angular_momentum.load();
    }
    entity.set_y_rot((f64::from(entity.yaw.load()) - momentum) as f32);
    let next = momentum * f64::from(angular_friction(caller));
    if leashed {
        entity.leash_angular_momentum.store(next);
    }
}

/// Vanilla `Leashable.angularFriction`.
fn angular_friction(caller: &dyn EntityBase) -> f32 {
    let entity = caller.get_entity();
    if entity.on_ground.load(Ordering::Relaxed) {
        // `getBlockState(getBlockPosBelowThatAffectsMyMovement()).getBlock().getFriction()`.
        entity
            .get_block_with_y_offset(super::DELTA_AFFECTED_BY_BLOCKS_BELOW_0_5)
            .1
            .slipperiness
            * 0.91f32
    } else if entity.is_in_liquid() {
        0.8
    } else {
        0.91
    }
}

/// Vanilla `Leashable.checkElasticInteractions`: `None` when no attachment pulls (vanilla's
/// `false`), else the angular momentum plus `10 * torque`, already written back, with the pull
/// added to the velocity.
fn check_elastic_interactions(
    caller: &dyn EntityBase,
    holder: &dyn EntityBase,
    momentum: f64,
) -> Option<f64> {
    let quad_connection = holder.support_quad_leash_as_holder() && caller.support_quad_leash();
    let wrenches = if quad_connection {
        compute_elastic_interaction(
            caller,
            holder,
            &SHARED_QUAD_ATTACHMENT_POINTS,
            &SHARED_QUAD_ATTACHMENT_POINTS,
        )
    } else {
        compute_elastic_interaction(
            caller,
            holder,
            &ENTITY_ATTACHMENT_POINT,
            &LEASHER_ATTACHMENT_POINT,
        )
    };
    if wrenches.is_empty() {
        return None;
    }
    let result = Wrench::accumulate(&wrenches).scale(if quad_connection { 0.25 } else { 1.0 });
    let momentum = momentum + TORSIONAL_ELASTICITY * result.torque;
    let entity = caller.get_entity();
    entity.leash_angular_momentum.store(momentum);
    let relative_velocity_to_leasher = holder_movement(holder) - caller.get_known_movement();
    // `addDeltaMovement`: a finite change only, and no velocity packet of its own.
    let delta =
        multiply(result.force, AXIS_SPECIFIC_ELASTICITY) + relative_velocity_to_leasher * STIFFNESS;
    if delta.x.is_finite() && delta.y.is_finite() && delta.z.is_finite() {
        let velocity = entity.velocity.load() + delta;
        if velocity.x.is_finite() && velocity.y.is_finite() && velocity.z.is_finite() {
            entity.velocity.store(velocity);
        }
    }
    Some(momentum)
}

/// Vanilla `Leashable.getHolderMovement`: a `NoAI` mob is taken as still.
fn holder_movement(entity: &dyn EntityBase) -> Vector3<f64> {
    if entity
        .get_mob()
        .is_some_and(|mob| mob.get_mob_entity().is_no_ai())
    {
        Vector3::new(0.0, 0.0, 0.0)
    } else {
        entity.get_known_movement()
    }
}

/// Vanilla `Leashable.computeElasticInteraction` for `entity` leashed to `holder`.
fn compute_elastic_interaction(
    entity: &dyn EntityBase,
    holder: &dyn EntityBase,
    entity_attachment_points: &[Vector3<f64>],
    leasher_attachment_points: &[Vector3<f64>],
) -> Vec<Wrench> {
    let base = entity.get_entity();
    let holder_base = holder.get_entity();
    elastic_wrenches(
        &ElasticBody {
            pos: base.pos.load(),
            width: base.width(),
            height: base.height(),
            y_rot: base.yaw.load(),
        },
        &ElasticBody {
            pos: holder_base.pos.load(),
            width: holder_base.width(),
            height: holder_base.height(),
            y_rot: holder_base.yaw.load(),
        },
        entity.leash_elastic_distance(),
        // Vanilla reads the leashed entity's own movement through `getHolderMovement`.
        holder_movement(entity),
        entity_attachment_points,
        leasher_attachment_points,
    )
}

/// What `computeElasticInteraction` reads of an entity: `position()`, `getBbWidth()`,
/// `getBbHeight()` and `getYRot()` (degrees).
#[derive(Clone, Copy, Debug)]
struct ElasticBody {
    pos: Vector3<f64>,
    width: f32,
    height: f32,
    y_rot: f32,
}

/// The body of `computeElasticInteraction`, over plain values.
fn elastic_wrenches(
    entity: &ElasticBody,
    holder: &ElasticBody,
    slack_distance: f64,
    current_movement: Vector3<f64>,
    entity_attachment_points: &[Vector3<f64>],
    leasher_attachment_points: &[Vector3<f64>],
) -> Vec<Wrench> {
    // `getYRot() * (float)(Math.PI / 180.0)`, in float.
    let deg_to_rad = std::f64::consts::PI / 180.0;
    let entity_y_rot = entity.y_rot * deg_to_rad as f32;
    let entity_dimensions = Vector3::new(
        f64::from(entity.width),
        f64::from(entity.height),
        f64::from(entity.width),
    );
    let leash_holder_y_rot = holder.y_rot * deg_to_rad as f32;
    let leasher_dimensions = Vector3::new(
        f64::from(holder.width),
        f64::from(holder.height),
        f64::from(holder.width),
    );
    let mut wrenches = Vec::new();
    for (entity_point, leasher_point) in entity_attachment_points
        .iter()
        .zip(leasher_attachment_points)
    {
        let entity_attach_vector = multiply(*entity_point, entity_dimensions).y_rot(-entity_y_rot);
        let entity_attach_pos = entity.pos + entity_attach_vector;
        let leasher_attach_vector =
            multiply(*leasher_point, leasher_dimensions).y_rot(-leash_holder_y_rot);
        let leasher_attach_pos = holder.pos + leasher_attach_vector;
        if let Some(wrench) = compute_dampened_spring_interaction(
            leasher_attach_pos,
            entity_attach_pos,
            slack_distance,
            current_movement,
            entity_attach_vector,
        ) {
            wrenches.push(wrench);
        }
    }
    wrenches
}

/// Vanilla `Vec3.multiply(Vec3)`: per axis.
fn multiply(a: Vector3<f64>, b: Vector3<f64>) -> Vector3<f64> {
    Vector3::new(a.x * b.x, a.y * b.y, a.z * b.z)
}

/// The entity that holds `entity`'s leash, if any (vanilla `getLeashHolder` on the server).
#[must_use]
pub fn leash_holder(entity: &Entity) -> Option<Arc<dyn EntityBase>> {
    entity.leash_holder()
}

/// Vanilla `Leashable.leashableInArea(level, pos, test)`: `AABB.ofSize(pos, 32, 32, 32)`, every
/// leashable entity passing `test`, in `getEntitiesOfClass` order.
pub fn leashable_in_area(
    world: &World,
    pos: Vector3<f64>,
    test: impl Fn(&dyn EntityBase) -> bool,
) -> Vec<Arc<dyn EntityBase>> {
    let area = BoundingBox::new(
        Vector3::new(pos.x - 16.0, pos.y - 16.0, pos.z - 16.0),
        Vector3::new(pos.x + 16.0, pos.y + 16.0, pos.z + 16.0),
    );
    world
        .get_entities_at_box(&area)
        .into_iter()
        .filter(|candidate| candidate.is_leashable() && test(candidate.as_ref()))
        .collect()
}

/// Vanilla `Leashable.leashableInArea(entity, l -> l.getLeashHolder() == holder)`: around the
/// centre of `entity`'s box, the leashables held by the entity with id `holder_id`.
pub fn leashable_held_by_in_area(entity: &Entity, holder_id: i32) -> Vec<Arc<dyn EntityBase>> {
    let world = entity.world.load_full();
    leashable_in_area(
        &world,
        entity.bounding_box.load().get_center(),
        |candidate| {
            leash_holder(candidate.get_entity())
                .is_some_and(|holder| holder.get_entity().entity_id == holder_id)
        },
    )
}

/// Vanilla `Leashable.leashableLeashedTo(entity)`: `leashableInArea(entity, l ->
/// l.getLeashHolder() == entity)`, centred on the bounding box.
pub fn leashable_leashed_to(entity: &Entity) -> Vec<Arc<dyn EntityBase>> {
    leashable_held_by_in_area(entity, entity.entity_id)
}

/// Vanilla `Leashable.canBeLeashed`: `Mob`'s own answer, `true` for the other leashables.
#[must_use]
pub fn can_be_leashed(leashable: &dyn EntityBase) -> bool {
    leashable
        .get_mob()
        .is_none_or(crate::entity::mob::Mob::can_be_leashed)
}

/// Vanilla `Leashable.leashDistanceTo`: between the bounding box centres.
#[must_use]
pub fn leash_distance_to(leashable: &Entity, holder: &Entity) -> f64 {
    holder
        .bounding_box
        .load()
        .get_center()
        .squared_distance_to_vec(&leashable.bounding_box.load().get_center())
        .sqrt()
}

/// Vanilla `Leashable.canHaveALeashAttachedTo`: not itself, within `leashSnapDistance`, and
/// `canBeLeashed`.
#[must_use]
pub fn can_have_a_leash_attached_to(leashable: &dyn EntityBase, holder: &Entity) -> bool {
    let entity = leashable.get_entity();
    if entity.entity_id == holder.entity_id {
        return false;
    }
    if leash_distance_to(entity, holder) > leashable.leash_snap_distance() {
        return false;
    }
    can_be_leashed(leashable)
}

/// `ClientboundSetEntityLinkPacket(entity, holder)` and Bedrock's actor link; `None` is the
/// unlink.
#[must_use]
pub fn leash_link_packets(
    entity_id: i32,
    holder_id: Option<i32>,
) -> (
    pumpkin_protocol::java::client::play::CSetEntityLink,
    pumpkin_protocol::bedrock::client::CSetActorLink,
) {
    // `destEntity != null ? destEntity.getId() : 0`.
    let je_packet = pumpkin_protocol::java::client::play::CSetEntityLink::new(
        entity_id,
        holder_id.unwrap_or(0),
        true,
    );
    let be_packet = pumpkin_protocol::bedrock::client::CSetActorLink {
        link: pumpkin_protocol::bedrock::client::common::ActorLink {
            ridden_unique_id: VarLong(i64::from(entity_id)),
            rider_unique_id: VarLong(i64::from(holder_id.unwrap_or(-1))),
            // 1 is the leash link, 0 the unlink.
            link_type: u8::from(holder_id.is_some()),
            immediate: true,
            rider_initiated: false,
            vehicle_angular_velocity: 0.0,
        },
    };
    (je_packet, be_packet)
}

/// The leash link (`None`: the unlink) to the players tracking `entity`.
fn send_leash_link(entity: &Entity, holder_id: Option<i32>) {
    let (je_packet, be_packet) = leash_link_packets(entity.entity_id, holder_id);
    entity
        .world
        .load()
        .send_to_tracking_players_editioned(entity, &je_packet, &be_packet);
}

/// Vanilla `Leashable.dropLeash(entity, sendPacket, dropLead)`; `dropLeash()` is (true, true),
/// `removeLeash()` is (true, false).
///
/// The plugin events: `EntityUnleashEvent` before anything changes (a cancel keeps the
/// leash), `PlayerUnleashEntityEvent` when a player held it.
pub fn drop_leash(entity: &dyn EntityBase, send_packet: bool, drop_lead: bool) {
    let base = entity.get_entity();
    if !base.is_leashed() {
        return;
    }
    let world = base.world.load_full();
    if let Some(server) = world.server.upgrade() {
        let mut event = EntityUnleashEvent::new(base.entity_id, "unleashed".to_string());
        server.plugin_manager.fire_blocking(&server, &mut event);
        if event.cancelled {
            return;
        }
    }
    // `setLeashData(null)`: the holder and any delayed holder go.
    let Some(holder) = base
        .leashed_to
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        return;
    };
    *base
        .delayed_leash
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    base.leash_angular_momentum.store(0.0);
    entity.on_leash_removed();
    if drop_lead {
        base.spawn_at_location(ItemStack::new(1, &Item::LEAD));
    }
    if send_packet {
        send_leash_link(base, None);
    }
    if let Some(server) = world.server.upgrade()
        && let Some(player) = holder.get_player()
        && let Some(player) = world.get_player_by_uuid(player.gameprofile.id)
    {
        let mut event = PlayerUnleashEntityEvent {
            player,
            entity_id: base.entity_id,
            cancelled: false,
        };
        server.plugin_manager.fire_blocking(&server, &mut event);
    }
    holder.notify_leashee_removed(holder.as_ref(), entity);
}

/// Vanilla `Leashable.setLeashedTo(entity, holder, synch)`.
///
/// A player holder fires `PlayerLeashEntityEvent` first; a cancel leaves the leash as it was.
pub fn set_leashed_to(entity: &dyn EntityBase, holder: Arc<dyn EntityBase>, synch: bool) {
    let base = entity.get_entity();
    let holder_id = holder.get_entity().entity_id;
    // `Leashable.setLeashedTo(holder, synch)`: `this != holder`.
    if base.entity_id == holder_id {
        return;
    }
    let world = base.world.load_full();
    if let Some(server) = world.server.upgrade()
        && let Some(player) = holder.get_player()
        && let Some(player) = world.get_player_by_uuid(player.gameprofile.id)
    {
        let mut event = PlayerLeashEntityEvent {
            player,
            entity_id: base.entity_id,
            holder_id,
            cancelled: false,
        };
        server.plugin_manager.fire_blocking(&server, &mut event);
        if event.cancelled {
            return;
        }
    }
    let old_holder = base
        .leashed_to
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .replace(holder);
    // A new `LeashData` starts without momentum; an existing one keeps it.
    if old_holder.is_none() {
        base.leash_angular_momentum.store(0.0);
    }
    // `LeashData.setLeashHolder` clears the delayed holder.
    *base
        .delayed_leash
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    if let Some(old_holder) = old_holder
        && old_holder.get_entity().entity_id != holder_id
    {
        old_holder.notify_leashee_removed(old_holder.as_ref(), entity);
    }
    if synch {
        send_leash_link(base, Some(holder_id));
    }
    // `if (entity.isPassenger()) entity.stopRiding()`.
    if let Some(vehicle) = base.get_vehicle() {
        vehicle.get_entity().remove_passenger(base.entity_id);
    }
}

/// Vanilla `Leashable.restoreLeashFromSave`: attaches a saved leash once its holder is
/// around (a knot is found or made), and gives up with a dropped lead after 100 ticks.
pub fn restore_leash_from_save(entity: &dyn EntityBase) {
    let base = entity.get_entity();
    let Some(delayed) = *base
        .delayed_leash
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    else {
        return;
    };
    let world = base.world.load_full();
    let holder: Option<Arc<dyn EntityBase>> = match delayed {
        DelayedLeash::Holder(uuid) => world.get_entity_by_uuid(uuid),
        DelayedLeash::Knot(pos) => {
            crate::entity::decoration::leash_knot::LeashKnotEntity::get_or_create(&world, pos)
        }
    };
    if let Some(holder) = holder {
        // `setLeashHolder` clears it in vanilla; clearing it first as well keeps a plugin's
        // cancelled `PlayerLeashEntityEvent` from retrying every tick.
        *base
            .delayed_leash
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        set_leashed_to(entity, holder, true);
    } else if base.tick_count.load(Ordering::Relaxed) > 100 {
        base.spawn_at_location(ItemStack::new(1, &Item::LEAD));
        *base
            .delayed_leash
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values computed independently in Python (IEEE doubles, f32-rounded 0.3).
    const K: f64 = 0.300_000_011_920_928_96;

    fn v(x: f64, y: f64, z: f64) -> Vector3<f64> {
        Vector3::new(x, y, z)
    }

    #[test]
    fn torque_from_force_sign() {
        // lever (1,0,2), force (3,0,5): 2*3 - 1*5 = 1
        assert_eq!(
            Wrench::torque_from_force(v(1.0, 0.0, 2.0), v(3.0, 0.0, 5.0)),
            1.0
        );
        assert_eq!(
            Wrench::torque_from_force(v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)),
            1.0
        );
        assert_eq!(
            Wrench::torque_from_force(v(1.0, 0.0, 0.0), v(0.0, 0.0, 1.0)),
            -1.0
        );
    }

    #[test]
    fn accumulate_sums_in_order() {
        let a = Wrench {
            force: v(1.0, 2.0, 3.0),
            torque: 0.5,
        };
        let b = Wrench {
            force: v(-0.25, 4.0, 1.0),
            torque: 1.5,
        };
        let r = Wrench::accumulate(&[a, b]);
        assert_eq!(
            r,
            Wrench {
                force: v(0.75, 6.0, 4.0),
                torque: 2.0
            }
        );
        assert_eq!(Wrench::accumulate(&[]), Wrench::ZERO);
        assert_eq!(r.scale(0.25).force, v(0.1875, 1.5, 1.0));
        assert_eq!(r.scale(0.25).torque, 0.5);
    }

    #[test]
    fn spring_inside_slack_is_none() {
        let r = compute_dampened_spring_interaction(
            v(5.0, 0.0, 0.0),
            v(0.0, 0.0, 0.0),
            6.0,
            v(0.0, 0.0, 0.0),
            v(0.0, 0.0, 0.5),
        );
        assert_eq!(r, None);
    }

    #[test]
    fn spring_at_slack_has_zero_force() {
        // distance == slack is not "< slack": a zero-length displacement results.
        let r = compute_dampened_spring_interaction(
            v(6.0, 0.0, 0.0),
            v(0.0, 0.0, 0.0),
            6.0,
            v(0.0, 0.0, 0.0),
            v(0.0, 0.0, 0.5),
        )
        .expect("at slack is not None");
        assert_eq!(r.force, v(0.0, 0.0, 0.0));
    }

    #[test]
    fn spring_zero_motion_scales_by_f32_0_3() {
        // 2.0 * f64::from(0.3f32) == 0.6000000238418579
        let r = compute_dampened_spring_interaction(
            v(8.0, 0.0, 0.0),
            v(0.0, 0.0, 0.0),
            6.0,
            v(0.0, 0.0, 0.0),
            v(0.0, 0.0, 1.0),
        )
        .expect("past slack");
        assert_eq!(r.force.x.to_bits(), 0.600_000_023_841_857_9f64.to_bits());
        assert_eq!(r.force.x.to_bits(), (2.0 * K).to_bits());
        // torque uses the unscaled displacement: 1.0 * 2.0
        assert_eq!(r.torque, 2.0);
    }

    #[test]
    fn spring_opposing_motion_is_unscaled() {
        let r = compute_dampened_spring_interaction(
            v(8.0, 0.0, 0.0),
            v(0.0, 0.0, 0.0),
            6.0,
            v(-1.0, 0.0, 0.0),
            v(0.0, 0.0, 1.0),
        )
        .expect("past slack");
        assert_eq!(r.force, v(2.0, 0.0, 0.0));
        assert_eq!(r.torque, 2.0);
    }

    fn assert_wrenches(actual: &[Wrench], expected: &[([f64; 3], f64)]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?}");
        for (wrench, (force, torque)) in actual.iter().zip(expected) {
            assert_eq!(wrench.force.x.to_bits(), force[0].to_bits(), "{wrench:?}");
            assert_eq!(wrench.force.y.to_bits(), force[1].to_bits(), "{wrench:?}");
            assert_eq!(wrench.force.z.to_bits(), force[2].to_bits(), "{wrench:?}");
            assert_eq!(wrench.torque.to_bits(), torque.to_bits(), "{wrench:?}");
        }
    }

    // `elastic_wrenches` expectations come from an independent Python model of vanilla
    // `computeElasticInteraction` (Mth's 65536-entry float sine table, Java float rounding of
    // the yaw in radians, f32-rounded 0.3).

    #[test]
    fn elastic_single_attachment() {
        // A cow-sized entity yawed 30 degrees, leashed to a knot-sized holder.
        let wrenches = elastic_wrenches(
            &ElasticBody {
                pos: v(0.5, 0.0, 0.5),
                width: 0.9,
                height: 1.4,
                y_rot: 30.0,
            },
            &ElasticBody {
                pos: v(8.5, 1.0, 3.5),
                width: 0.375,
                height: 0.5,
                y_rot: 0.0,
            },
            6.0,
            v(0.1, 0.0, -0.05),
            &ENTITY_ATTACHMENT_POINT,
            &LEASHER_ATTACHMENT_POINT,
        );
        assert_wrenches(
            &wrenches,
            &[(
                [
                    0.755_298_495_925_716,
                    0.050_506_360_038_387_98,
                    0.239_703_451_569_916_51,
                ],
                1.160_892_871_262_981_4,
            )],
        );
    }

    #[test]
    fn elastic_quad_all_attachments_pull() {
        // A horse-sized entity under a ghast-sized holder, every corner past the slack.
        let wrenches = elastic_wrenches(
            &ElasticBody {
                pos: v(1.0, 0.0, -2.0),
                width: 1.396_484_4,
                height: 1.6,
                y_rot: -135.0,
            },
            &ElasticBody {
                pos: v(6.0, 4.0, 5.0),
                width: 4.0,
                height: 4.0,
                y_rot: 45.0,
            },
            6.0,
            v(0.0, 0.0, 0.0),
            &SHARED_QUAD_ATTACHMENT_POINTS,
            &SHARED_QUAD_ATTACHMENT_POINTS,
        );
        assert_wrenches(
            &wrenches,
            &[
                (
                    [
                        0.113_042_406_035_410_59,
                        0.496_367_351_891_543,
                        0.668_173_883_287_993_2,
                    ],
                    -2.199_324_438_622_803,
                ),
                (
                    [
                        0.358_671_260_618_644_4,
                        0.373_007_998_615_641_7,
                        0.228_413_235_337_646_76,
                    ],
                    1.180_582_612_760_147_8,
                ),
                (
                    [
                        1.365_017_847_136_288_6,
                        0.805_159_934_286_248_7,
                        1.083_890_131_745_567_8,
                    ],
                    3.567_673_198_778_185,
                ),
                (
                    [
                        0.807_709_772_070_857_7,
                        0.840_040_933_019_235,
                        1.747_245_580_350_663_5,
                    ],
                    -2.658_613_102_757_607,
                ),
            ],
        );
    }

    #[test]
    fn elastic_quad_skips_slack_attachments() {
        // Closer: two corners are inside the slack and give no wrench.
        let wrenches = elastic_wrenches(
            &ElasticBody {
                pos: v(1.0, 0.0, -2.0),
                width: 1.396_484_4,
                height: 1.6,
                y_rot: -135.0,
            },
            &ElasticBody {
                pos: v(4.5, 2.0, 2.5),
                width: 4.0,
                height: 4.0,
                y_rot: 45.0,
            },
            6.0,
            v(0.0, 0.0, 0.0),
            &SHARED_QUAD_ATTACHMENT_POINTS,
            &SHARED_QUAD_ATTACHMENT_POINTS,
        );
        assert_wrenches(
            &wrenches,
            &[
                (
                    [
                        0.758_039_055_867_086_8,
                        0.331_575_475_113_960_35,
                        0.466_292_058_009_500_4,
                    ],
                    1.534_821_315_777_165_3,
                ),
                (
                    [
                        0.391_878_561_821_951_7,
                        0.358_302_845_106_023_06,
                        0.931_112_099_688_103_8,
                    ],
                    -1.289_885_940_686_934_5,
                ),
            ],
        );
    }

    #[test]
    fn spring_oblique_matches_independent_values() {
        let pivot = v(3.0, 4.0, 5.5);
        let obj = v(0.5, -1.0, 0.25);
        let lever = v(0.5, 0.0, 1.5);
        let r = compute_dampened_spring_interaction(pivot, obj, 6.0, v(0.0, 0.0, 0.0), lever)
            .expect("past slack");
        assert_eq!(r.force.x.to_bits(), 0.163_216_776_510_724_17f64.to_bits());
        assert_eq!(r.force.y.to_bits(), 0.326_433_553_021_448_33f64.to_bits());
        assert_eq!(r.force.z.to_bits(), 0.342_755_230_672_520_7f64.to_bits());
        assert_eq!(r.torque.to_bits(), 0.244_825_155_037_608_6f64.to_bits());
    }
}
