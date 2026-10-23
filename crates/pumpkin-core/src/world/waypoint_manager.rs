//! Vanilla `ServerWaypointManager`: which living entities transmit a waypoint in a world, which
//! players receive them, and the connection between each pair.
//!
//! Vanilla keeps `HashSet`s and a `HashBasedTable` keyed by the objects, so the order across
//! different waypoints (or players) in one call is unspecified there; here it is the maps'
//! order. Per connection the order is vanilla's: connect before update, disconnect before a
//! remake.

use std::sync::atomic::Ordering;
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
pub(crate) struct Inner {
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

/// Vanilla `ServerWaypointManager`.
///
/// Lock order: apply -> scoreboard -> manager; the manager may take the scoreboard, never the
/// reverse. While the manager's mutex is held a connection may take a player's custom
/// scoreboard and the world scoreboard (`getTeam`, for the icon's team colour), so nothing may
/// take the manager while holding a scoreboard lock: the scoreboard queues its waypoint work
/// and `World::lock_scoreboard`'s guard runs it after unlocking the scoreboard, still holding
/// the world's apply lock (taken before the scoreboard, so a waiting change never holds it).
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
        if let Some(waypoint) = self
            .waypoints
            .remove(&waypoint_id)
            .and_then(|waypoint| waypoint.upgrade())
        {
            waypoint
                .get_entity()
                .waypoint_tracked
                .store(false, Ordering::Relaxed);
        }
    }

    /// Vanilla `trackWaypoint`.
    fn track_waypoint(&mut self, waypoint: &Arc<dyn EntityBase>) {
        self.waypoints
            .insert(waypoint.get_entity().entity_id, Arc::downgrade(waypoint));
        waypoint
            .get_entity()
            .waypoint_tracked
            .store(true, Ordering::Relaxed);
        for player in self.players() {
            self.create_connection(&player, waypoint, TeamColor::Lookup);
        }
    }
}

impl WaypointManager {
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
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
                // The receiver is gone without a `removePlayer`: drop its cell.
                inner.connections.remove(key);
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
                // The source is gone without an `untrackWaypoint`: drop its cell.
                inner.connections.remove(key);
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

    /// `remakeConnections` for a queued scoreboard change, with the team colour as it was at
    /// that change (a team switch remakes first without a team, then with the new one, as
    /// vanilla's immediate calls do).
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
    /// The manager is only entered for a tracked waypoint or a receiving player, as vanilla
    /// passes it nothing else; items and silent mobs never take its lock.
    pub fn on_entity_moved(&self, world: &super::World, entity: &crate::entity::Entity) {
        if entity.waypoint_tracked.load(Ordering::Relaxed) {
            self.update_waypoint(entity.entity_id);
        }
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

/// Sends a world scoreboard's queued events in order.
///
/// Each team packet goes to the world's players, each `updatePlayerWaypoint` / `updateTeamWaypoints` as a remake. Must be called
/// with no scoreboard lock held (see the lock order on [`WaypointManager`]).
pub fn apply_scoreboard_events(
    world: &super::World,
    events: Vec<super::scoreboard::ScoreboardEvent>,
) {
    use super::scoreboard::ScoreboardEvent;
    // Tests: stand in for a remake reaching the manager (the test world has no players).
    #[cfg(test)]
    if tests::APPLY_TAKES_MANAGER.load(Ordering::SeqCst) {
        drop(world.waypoint_manager.lock());
    }
    let server = world.server.upgrade();
    for event in events {
        match event {
            ScoreboardEvent::TeamPacket(packet) => packet.send(world),
            // `updatePlayerWaypoint`: the online player of that name, in its own level.
            ScoreboardEvent::Player(name, team_color) => {
                if let Some(player) = server.as_ref().and_then(|s| s.get_player_by_name(&name)) {
                    player
                        .world()
                        .waypoint_manager
                        .remake_connections_with_team_color(&as_entity(&player), team_color);
                }
            }
            // `updateTeamWaypoints`: in every level, each online player of the team.
            ScoreboardEvent::Team(names, team_color) => {
                let Some(server) = server.as_ref() else {
                    continue;
                };
                for level in server.worlds.load().iter() {
                    for name in &names {
                        if let Some(player) = server.get_player_by_name(name) {
                            level.waypoint_manager.remake_connections_with_team_color(
                                &as_entity(&player),
                                team_color,
                            );
                        }
                    }
                }
            }
        }
    }
}

/// [`World::lock_scoreboard`]'s guard.
///
/// Holds the world's apply lock (taken before the scoreboard) until the queued events are sent.
pub struct ScoreboardGuard<'a> {
    pub(super) world: &'a super::World,
    pub(super) apply: Option<std::sync::MutexGuard<'a, ()>>,
    pub(super) guard: Option<std::sync::MutexGuard<'a, super::scoreboard::Scoreboard>>,
}

impl std::ops::Deref for ScoreboardGuard<'_> {
    type Target = super::scoreboard::Scoreboard;

    #[expect(clippy::expect_used, reason = "the guard is only taken in drop")]
    fn deref(&self) -> &Self::Target {
        self.guard.as_deref().expect("held until drop")
    }
}

impl std::ops::DerefMut for ScoreboardGuard<'_> {
    #[expect(clippy::expect_used, reason = "the guard is only taken in drop")]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_deref_mut().expect("held until drop")
    }
}

impl Drop for ScoreboardGuard<'_> {
    fn drop(&mut self) {
        let apply = self.apply.take();
        let Some(mut guard) = self.guard.take() else {
            return;
        };
        let events = guard.take_events();
        // Apply, then the scoreboard (released here), then (in the remakes) the manager.
        drop(guard);
        if !events.is_empty() {
            apply_scoreboard_events(self.world, events);
        }
        drop(apply);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    /// Makes `apply_scoreboard_events` take the manager, as a remake for an online player does.
    pub(super) static APPLY_TAKES_MANAGER: AtomicBool = AtomicBool::new(false);

    /// Two overlapping team changes while a connection (holding the manager) asks for a team
    /// colour (the scoreboard) must all finish: the apply lock is taken before the scoreboard,
    /// so no change waits for it while holding the scoreboard.
    #[tokio::test]
    async fn overlapping_team_changes_and_a_colour_lookup_all_finish() {
        let (_folder, world) = crate::world::test_support::test_world();
        APPLY_TAKES_MANAGER.store(true, Ordering::SeqCst);
        let (done, finished) = mpsc::channel();
        let (held, manager_held) = mpsc::channel();
        let (go, lookup) = mpsc::channel::<()>();
        // A connection being made: the manager, then `getTeam`'s scoreboard.
        let mover = {
            let (world, done) = (world.clone(), done.clone());
            std::thread::spawn(move || {
                let manager = world.waypoint_manager.lock();
                held.send(()).unwrap();
                lookup.recv().unwrap();
                drop(
                    world
                        .scoreboard
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
                drop(manager);
                done.send("mover").unwrap();
            })
        };
        manager_held.recv().unwrap();
        let change = |name: &'static str| {
            let (world, done) = (world.clone(), done.clone());
            std::thread::spawn(move || {
                let mut scoreboard = world.lock_scoreboard();
                scoreboard.add_team(&NoTarget, Team::new(name.to_string()));
                drop(scoreboard);
                done.send(name).unwrap();
            })
        };
        // The first change applies (and waits for the manager), the second queues behind it,
        // then the connection asks for the scoreboard.
        let _first = change("a");
        std::thread::sleep(Duration::from_millis(200));
        let _second = change("b");
        std::thread::sleep(Duration::from_millis(200));
        go.send(()).unwrap();
        let mut finished_names = Vec::new();
        for _ in 0..3 {
            match finished.recv_timeout(Duration::from_secs(5)) {
                Ok(name) => finished_names.push(name),
                Err(_) => break,
            }
        }
        APPLY_TAKES_MANAGER.store(false, Ordering::SeqCst);
        finished_names.sort_unstable();
        assert_eq!(finished_names, ["a", "b", "mover"], "deadlocked");
        mover.join().unwrap();
    }

    use pumpkin_util::text::color::NamedColor;

    use crate::world::scoreboard::{NoTarget, Scoreboard, ScoreboardEvent, Team, TeamPacket};

    fn summary(events: &[ScoreboardEvent]) -> Vec<String> {
        events
            .iter()
            .map(|event| match event {
                ScoreboardEvent::TeamPacket(packet) => match packet {
                    TeamPacket::Create(team) => format!("create {}", team.name),
                    TeamPacket::Update(team) => format!("update {}", team.name),
                    TeamPacket::Remove(name) => format!("remove {name}"),
                    TeamPacket::AddPlayers(name, players) => format!("add {name} {players:?}"),
                    TeamPacket::RemovePlayers(name, players) => {
                        format!("leave {name} {players:?}")
                    }
                },
                ScoreboardEvent::Player(name, color) => format!("remake {name} {color:?}"),
                ScoreboardEvent::Team(names, color) => format!("remake {names:?} {color:?}"),
            })
            .collect()
    }

    /// A team change under the scoreboard lock must not enter the waypoint manager (whose
    /// connections take the scoreboard lock): with another thread holding the manager, the
    /// change still completes. Its team packets and remakes are queued in vanilla's
    /// interleaved order, each remake with the colour it had.
    #[tokio::test]
    async fn team_changes_never_take_the_manager_under_the_scoreboard_lock() {
        let (_folder, world) = crate::world::test_support::test_world();
        let manager = world.waypoint_manager.lock();
        let (done, finished) = mpsc::channel();
        let changer = {
            let world = world.clone();
            std::thread::spawn(move || {
                let mut scoreboard = world.lock_scoreboard();
                let mut red = Team::new("red".to_string());
                red.color = Some(NamedColor::Red);
                scoreboard.add_team(&NoTarget, red);
                scoreboard.add_team(&NoTarget, Team::new("plain".to_string()));
                scoreboard.add_player_to_team(&NoTarget, "plain", "Bob".to_string());
                scoreboard.add_player_to_team(&NoTarget, "red", "Ann".to_string());
                scoreboard.add_player_to_team(&NoTarget, "red", "Bob".to_string());
                let mut red = scoreboard.get_team("red").cloned().unwrap();
                red.color = Some(NamedColor::Gold);
                scoreboard.update_team(&NoTarget, red);
                scoreboard.remove_team(&NoTarget, "red");
                let events = scoreboard.take_events();
                done.send(()).unwrap();
                events
            })
        };
        let completed = finished.recv_timeout(Duration::from_secs(10)).is_ok();
        drop(manager);
        let events = changer.join().unwrap();
        assert!(completed, "a team change waited for the waypoint manager");
        assert_eq!(
            summary(&events),
            [
                "create red",
                "create plain",
                "add plain [\"Bob\"]",
                "remake Bob None",
                "add red [\"Ann\"]",
                "remake Ann Some(Red)",
                "leave plain [\"Bob\"]",
                "remake Bob None",
                "add red [\"Bob\"]",
                "remake Bob Some(Red)",
                "update red",
                "remake [\"Ann\", \"Bob\"] Some(Gold)",
                "remove red",
                "remake [\"Ann\", \"Bob\"] None",
            ]
        );
    }

    /// The guard sends what a change queued once it unlocks, so nothing stays behind.
    #[tokio::test]
    async fn the_guard_drains_the_queue() {
        let (_folder, world) = crate::world::test_support::test_world();
        let mut scoreboard = world.lock_scoreboard();
        scoreboard.add_team(&NoTarget, Team::new("t".to_string()));
        scoreboard.add_player_to_team(&NoTarget, "t", "Bob".to_string());
        drop(scoreboard);
        let mut scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(scoreboard.take_events().is_empty());
        assert!(scoreboard.get_team("t").is_some());
    }

    /// A player's custom scoreboard isn't a `ServerScoreboard`: it sends its team packets at
    /// once and queues nothing.
    #[test]
    fn custom_scoreboards_queue_nothing() {
        let mut scoreboard = Scoreboard::default();
        scoreboard.add_team(&NoTarget, Team::new("t".to_string()));
        scoreboard.add_player_to_team(&NoTarget, "t", "Bob".to_string());
        scoreboard.remove_team(&NoTarget, "t");
        assert!(scoreboard.take_events().is_empty());
    }
}
