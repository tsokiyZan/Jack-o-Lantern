//! Vanilla `ServerWaypointManager`: which living entities transmit a waypoint in a world, which
//! players receive them, and the connection between each pair.
//!
//! Vanilla keeps `HashSet`s and a `HashBasedTable` keyed by the objects, so the order across
//! different waypoints (or players) in one call is unspecified there; here it is the maps'
//! order. Per connection the order is vanilla's: connect before update, disconnect before a
//! remake.

use std::sync::{Arc, Weak};

use pumpkin_util::text::color::NamedColor;
use rustc_hash::FxHashMap;
use uuid::Uuid;

use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::entity::waypoint::{
    WaypointConnection, is_receiving_waypoints, is_transmitting_waypoint,
    make_waypoint_connection_with, team_color_of,
};

/// One cell of vanilla's `connections` table: the connection and the two objects it joins.
struct Cell {
    source: Weak<dyn EntityBase>,
    source_uuid: Uuid,
    receiver: Weak<Player>,
    connection: WaypointConnection,
}

#[derive(Default)]
struct Inner {
    /// Vanilla `waypoints`, by entity id.
    waypoints: FxHashMap<i32, Weak<dyn EntityBase>>,
    /// Vanilla `players`, by entity id.
    players: FxHashMap<i32, Weak<Player>>,
    /// Vanilla `connections`: (player id, waypoint id).
    connections: FxHashMap<(i32, i32), Cell>,
}

/// How a connection finds its source's team colour (`cloneAndAssignStyle`).
#[derive(Clone, Copy)]
enum TeamColor {
    /// Read it from the source (`getTeam().getColor()`).
    Lookup,
    /// The scoreboard is changing it and passes the new one (it holds its own lock).
    Given(Option<NamedColor>),
}

#[derive(Default)]
pub struct WaypointManager {
    inner: std::sync::Mutex<Inner>,
}

/// Vanilla `isLocatorBarEnabledFor`: the `locator_bar` rule of the player's world.
fn is_locator_bar_enabled_for(player: &Player) -> bool {
    player.world().level_info.load().game_rules.locator_bar
}

fn as_entity(player: &Arc<Player>) -> Arc<dyn EntityBase> {
    player.clone() as Arc<dyn EntityBase>
}

impl Inner {
    fn players(&self) -> Vec<Arc<Player>> {
        self.players.values().filter_map(Weak::upgrade).collect()
    }

    /// Vanilla `createConnection`.
    fn create_connection(
        &mut self,
        player: &Arc<Player>,
        waypoint: &Arc<dyn EntityBase>,
        team_color: TeamColor,
    ) {
        let player_id = player.entity_id();
        let waypoint_id = waypoint.get_entity().entity_id;
        if player_id == waypoint_id || !is_locator_bar_enabled_for(player) {
            return;
        }
        let made = make_waypoint_connection_with(waypoint.as_ref(), player, || match team_color {
            TeamColor::Lookup => team_color_of(waypoint.as_ref()),
            TeamColor::Given(color) => color,
        });
        let key = (player_id, waypoint_id);
        match made {
            Some(connection) => {
                let source_uuid = waypoint.get_entity().entity_uuid;
                connection.connect(waypoint.get_entity(), player);
                self.connections.insert(
                    key,
                    Cell {
                        source: Arc::downgrade(waypoint),
                        source_uuid,
                        receiver: Arc::downgrade(player),
                        connection,
                    },
                );
            }
            None => {
                if let Some(old) = self.connections.remove(&key) {
                    old.connection.disconnect(old.source_uuid, player);
                }
            }
        }
    }

    /// Vanilla `updateConnection` for the connection at `key`.
    fn update_connection(
        &mut self,
        player: &Arc<Player>,
        waypoint: &Arc<dyn EntityBase>,
        key: (i32, i32),
    ) {
        if key.0 == key.1 || !is_locator_bar_enabled_for(player) {
            return;
        }
        let Some(cell) = self.connections.get_mut(&key) else {
            return;
        };
        if !cell.connection.is_broken(waypoint.as_ref(), player) {
            cell.connection.update(waypoint.get_entity(), player);
            return;
        }
        if let Some(connection) = make_waypoint_connection_with(waypoint.as_ref(), player, || {
            team_color_of(waypoint.as_ref())
        }) {
            connection.connect(waypoint.get_entity(), player);
            cell.connection = connection;
        } else {
            cell.connection.disconnect(cell.source_uuid, player);
            self.connections.remove(&key);
        }
    }

    /// Vanilla `untrackWaypoint`.
    fn untrack_waypoint(&mut self, waypoint_id: i32) {
        let keys: Vec<_> = self
            .connections
            .keys()
            .filter(|key| key.1 == waypoint_id)
            .copied()
            .collect();
        for key in keys {
            if let Some(cell) = self.connections.remove(&key)
                && let Some(receiver) = cell.receiver.upgrade()
            {
                cell.connection.disconnect(cell.source_uuid, &receiver);
            }
        }
        self.waypoints.remove(&waypoint_id);
    }

    /// Vanilla `trackWaypoint`.
    fn track_waypoint(&mut self, waypoint: &Arc<dyn EntityBase>) {
        self.waypoints
            .insert(waypoint.get_entity().entity_id, Arc::downgrade(waypoint));
        for player in self.players() {
            self.create_connection(&player, waypoint, TeamColor::Lookup);
        }
    }
}

impl WaypointManager {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Vanilla `trackWaypoint`.
    pub fn track_waypoint(&self, waypoint: &Arc<dyn EntityBase>) {
        self.lock().track_waypoint(waypoint);
    }

    /// Vanilla `updateWaypoint` for the tracked waypoint with this entity id. Like the vanilla
    /// caller, only while it is transmitting.
    pub fn update_waypoint(&self, waypoint_id: i32) {
        let mut inner = self.lock();
        let Some(waypoint) = inner.waypoints.get(&waypoint_id).and_then(Weak::upgrade) else {
            return;
        };
        if !is_transmitting_waypoint(waypoint.as_ref()) {
            return;
        }
        let connected: Vec<(i32, i32)> = inner
            .connections
            .keys()
            .filter(|key| key.1 == waypoint_id)
            .copied()
            .collect();
        for key in &connected {
            let Some(player) = inner
                .connections
                .get(key)
                .and_then(|c| c.receiver.upgrade())
            else {
                continue;
            };
            inner.update_connection(&player, &waypoint, *key);
        }
        // `Sets.difference(players, connected)`, read as the loop goes: a player whose
        // connection the loop above dropped gets another try.
        for player in inner.players() {
            if !inner
                .connections
                .contains_key(&(player.entity_id(), waypoint_id))
            {
                inner.create_connection(&player, &waypoint, TeamColor::Lookup);
            }
        }
    }

    /// Vanilla `untrackWaypoint`.
    pub fn untrack_waypoint(&self, waypoint_id: i32) {
        self.lock().untrack_waypoint(waypoint_id);
    }

    /// Vanilla `addPlayer`.
    pub fn add_player(&self, player: &Arc<Player>) {
        let mut inner = self.lock();
        inner
            .players
            .insert(player.entity_id(), Arc::downgrade(player));
        let waypoints: Vec<_> = inner.waypoints.values().filter_map(Weak::upgrade).collect();
        for waypoint in waypoints {
            inner.create_connection(player, &waypoint, TeamColor::Lookup);
        }
        let as_waypoint = as_entity(player);
        if is_transmitting_waypoint(as_waypoint.as_ref()) {
            inner.track_waypoint(&as_waypoint);
        }
    }

    /// Vanilla `updatePlayer`.
    pub fn update_player(&self, player: &Arc<Player>) {
        let mut inner = self.lock();
        let player_id = player.entity_id();
        let connected: Vec<(i32, i32)> = inner
            .connections
            .keys()
            .filter(|key| key.0 == player_id)
            .copied()
            .collect();
        for key in &connected {
            let Some(waypoint) = inner.connections.get(key).and_then(|c| c.source.upgrade()) else {
                continue;
            };
            inner.update_connection(player, &waypoint, *key);
        }
        let waypoints: Vec<_> = inner
            .waypoints
            .iter()
            .filter_map(|(id, waypoint)| Some((*id, waypoint.upgrade()?)))
            .collect();
        for (id, waypoint) in waypoints {
            if !inner.connections.contains_key(&(player_id, id)) {
                inner.create_connection(player, &waypoint, TeamColor::Lookup);
            }
        }
    }

    /// Vanilla `removePlayer`.
    pub fn remove_player(&self, player: &Player) {
        let mut inner = self.lock();
        let player_id = player.entity_id();
        let keys: Vec<_> = inner
            .connections
            .keys()
            .filter(|key| key.0 == player_id)
            .copied()
            .collect();
        for key in keys {
            if let Some(cell) = inner.connections.remove(&key) {
                cell.connection.disconnect(cell.source_uuid, player);
            }
        }
        inner.untrack_waypoint(player_id);
        inner.players.remove(&player_id);
    }

    /// Vanilla `breakAllConnections`.
    pub fn break_all_connections(&self) {
        let mut inner = self.lock();
        for (_, cell) in inner.connections.drain() {
            if let Some(receiver) = cell.receiver.upgrade() {
                cell.connection.disconnect(cell.source_uuid, &receiver);
            }
        }
    }

    /// Vanilla `remakeConnections`.
    pub fn remake_connections(&self, waypoint: &Arc<dyn EntityBase>) {
        let mut inner = self.lock();
        for player in inner.players() {
            inner.create_connection(&player, waypoint, TeamColor::Lookup);
        }
    }

    /// `remakeConnections` from the scoreboard, which holds its lock and passes the waypoint's
    /// new team colour.
    pub fn remake_connections_with_team_color(
        &self,
        waypoint: &Arc<dyn EntityBase>,
        team_color: Option<NamedColor>,
    ) {
        let mut inner = self.lock();
        for player in inner.players() {
            inner.create_connection(&player, waypoint, TeamColor::Given(team_color));
        }
    }

    /// Vanilla `transmitters`.
    pub fn transmitters(&self) -> Vec<Arc<dyn EntityBase>> {
        self.lock()
            .waypoints
            .values()
            .filter_map(Weak::upgrade)
            .collect()
    }

    /// The hook at the end of vanilla `Entity.setPosRaw`: once past its first tick and not
    /// removed, a transmitter updates its waypoint and a receiving player its connections.
    pub fn on_entity_moved(&self, world: &super::World, entity: &crate::entity::Entity) {
        self.update_waypoint(entity.entity_id);
        if entity.entity_type == &pumpkin_data::entity::EntityType::PLAYER
            && let Some(player) = world.get_player_by_id(entity.entity_id)
            && is_receiving_waypoints(&player)
        {
            self.update_player(&player);
        }
    }

    /// The waypoint part of vanilla `ServerLevel.EntityCallbacks.onTrackingStart` for a
    /// player: `addPlayer` when it receives, then `trackWaypoint` when it transmits.
    pub fn on_player_tracking_start(&self, player: &Arc<Player>) {
        if is_receiving_waypoints(player) {
            self.add_player(player);
        }
        let as_waypoint = as_entity(player);
        if is_transmitting_waypoint(as_waypoint.as_ref()) {
            self.track_waypoint(&as_waypoint);
        }
    }

    /// `onCreated` / `onTrackingStart` for any other entity: track it when it transmits.
    pub fn on_entity_added(&self, entity: &Arc<dyn EntityBase>) {
        if is_transmitting_waypoint(entity.as_ref()) {
            self.track_waypoint(entity);
        }
    }
}

/// Vanilla `ServerScoreboard.updatePlayerWaypoint`: the online player with that name remakes
/// its connections in its own level. `team_color` is its team's colour after the change.
pub fn update_player_waypoint(world: &super::World, name: &str, team_color: Option<NamedColor>) {
    let Some(server) = world.server.upgrade() else {
        return;
    };
    if let Some(player) = server.get_player_by_name(name) {
        player
            .world()
            .waypoint_manager
            .remake_connections_with_team_color(&as_entity(&player), team_color);
    }
}

/// Vanilla `ServerScoreboard.updateTeamWaypoints`: in every level, each online player of the
/// team remakes its connections with that level's players.
pub fn update_team_waypoints(
    world: &super::World,
    names: &[String],
    team_color: Option<NamedColor>,
) {
    let Some(server) = world.server.upgrade() else {
        return;
    };
    for level in server.worlds.load().iter() {
        for name in names {
            if let Some(player) = server.get_player_by_name(name) {
                level
                    .waypoint_manager
                    .remake_connections_with_team_color(&as_entity(&player), team_color);
            }
        }
    }
}
