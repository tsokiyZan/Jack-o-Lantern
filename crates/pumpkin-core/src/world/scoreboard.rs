use pumpkin_util::fastutil::OpenHashKeys;
use rustc_hash::FxHashMap;

use pumpkin_data::scoreboard::ScoreboardDisplaySlot;
use pumpkin_protocol::{
    BClientPacket, ClientPacket, NumberFormat,
    bedrock::client::{
        remove_objective::CRemoveObjective as BRemoveObjective,
        set_display_objective::CSetDisplayObjective as BSetDisplayObjective,
        set_score::{CSetScore as BSetScore, ScoreEntry as BScoreEntry},
    },
    codec::var_int::VarInt,
    java::client::play::{
        CDisplayObjective, CResetScore, CSetPlayerTeam, CUpdateObjectives, CUpdateScore, Mode,
        RenderType, TeamMethod, TeamParameters,
    },
};
use pumpkin_util::text::{TextComponent, color::NamedColor};
use tracing::warn;

use super::World;
use crate::entity::player::Player;

pub trait ScoreboardTarget: Send + Sync {
    fn send_editioned<J: ClientPacket + Sync, B: BClientPacket + Sync>(
        &self,
        je_packet: &J,
        be_packet: &B,
    );
    fn send_je<J: ClientPacket + Sync>(&self, je_packet: &J);

    /// Vanilla `ServerScoreboard.updatePlayerWaypoint`: the online player of that name remakes
    /// its waypoint connections. `team_color` is its team's colour now.
    fn update_player_waypoint(&self, _player: &str, _team_color: Option<NamedColor>) {}

    /// Vanilla `ServerScoreboard.updateTeamWaypoints`.
    fn update_team_waypoints(&self, _players: &[String], _team_color: Option<NamedColor>) {}
}

impl ScoreboardTarget for World {
    fn send_editioned<J: ClientPacket + Sync, B: BClientPacket + Sync>(
        &self,
        je_packet: &J,
        be_packet: &B,
    ) {
        self.broadcast_editioned(je_packet, be_packet);
    }

    fn send_je<J: ClientPacket + Sync>(&self, je_packet: &J) {
        self.broadcast_packet_all(je_packet);
    }

    fn update_player_waypoint(&self, player: &str, team_color: Option<NamedColor>) {
        crate::world::waypoint_manager::update_player_waypoint(self, player, team_color);
    }

    fn update_team_waypoints(&self, players: &[String], team_color: Option<NamedColor>) {
        crate::world::waypoint_manager::update_team_waypoints(self, players, team_color);
    }
}

impl<T: ScoreboardTarget + ?Sized> ScoreboardTarget for &T {
    fn send_editioned<J: ClientPacket + Sync, B: BClientPacket + Sync>(
        &self,
        je_packet: &J,
        be_packet: &B,
    ) {
        (*self).send_editioned(je_packet, be_packet);
    }

    fn send_je<J: ClientPacket + Sync>(&self, je_packet: &J) {
        (*self).send_je(je_packet);
    }

    fn update_player_waypoint(&self, player: &str, team_color: Option<NamedColor>) {
        (*self).update_player_waypoint(player, team_color);
    }

    fn update_team_waypoints(&self, players: &[String], team_color: Option<NamedColor>) {
        (*self).update_team_waypoints(players, team_color);
    }
}

impl ScoreboardTarget for std::sync::Arc<World> {
    fn send_editioned<J: ClientPacket + Sync, B: BClientPacket + Sync>(
        &self,
        je_packet: &J,
        be_packet: &B,
    ) {
        self.broadcast_editioned(je_packet, be_packet);
    }

    fn send_je<J: ClientPacket + Sync>(&self, je_packet: &J) {
        self.broadcast_packet_all(je_packet);
    }

    fn update_player_waypoint(&self, player: &str, team_color: Option<NamedColor>) {
        crate::world::waypoint_manager::update_player_waypoint(self, player, team_color);
    }

    fn update_team_waypoints(&self, players: &[String], team_color: Option<NamedColor>) {
        crate::world::waypoint_manager::update_team_waypoints(self, players, team_color);
    }
}

impl ScoreboardTarget for Player {
    fn send_editioned<J: ClientPacket + Sync, B: BClientPacket + Sync>(
        &self,
        je_packet: &J,
        be_packet: &B,
    ) {
        self.try_enqueue_packet_editioned(je_packet, be_packet);
    }

    fn send_je<J: ClientPacket + Sync>(&self, je_packet: &J) {
        self.try_send_client_packet(je_packet);
    }
}

impl ScoreboardTarget for std::sync::Arc<Player> {
    fn send_editioned<J: ClientPacket + Sync, B: BClientPacket + Sync>(
        &self,
        je_packet: &J,
        be_packet: &B,
    ) {
        self.try_enqueue_packet_editioned(je_packet, be_packet);
    }

    fn send_je<J: ClientPacket + Sync>(&self, je_packet: &J) {
        self.try_send_client_packet(je_packet);
    }
}

pub struct NoTarget;

impl ScoreboardTarget for NoTarget {
    fn send_editioned<J: ClientPacket + Sync, B: BClientPacket + Sync>(
        &self,
        _je_packet: &J,
        _be_packet: &B,
    ) {
    }

    fn send_je<J: ClientPacket + Sync>(&self, _je_packet: &J) {}
}

/// `Scoreboard.playerScores`' initial size and load factor.
const PLAYER_SCORES_EXPECTED: usize = 16;
const PLAYER_SCORES_LOAD_FACTOR: f32 = 0.5;

#[derive(Clone, Debug)]
pub struct Scoreboard {
    objectives: FxHashMap<String, ScoreboardObjective>,
    display_slots: FxHashMap<ScoreboardDisplaySlot, String>,
    scores: FxHashMap<String, FxHashMap<String, ScoreboardScore>>,
    teams: FxHashMap<String, Team>,
    /// The holders vanilla keeps in `Scoreboard.playerScores`, in that map's order. A holder
    /// stays tracked after its objectives are removed, until its scores are reset.
    tracked_players: OpenHashKeys,
    /// The names in vanilla's `Scoreboard.teamsByName`, in that map's order.
    team_names: OpenHashKeys,
}

impl Default for Scoreboard {
    fn default() -> Self {
        Self {
            objectives: FxHashMap::default(),
            display_slots: FxHashMap::default(),
            scores: FxHashMap::default(),
            teams: FxHashMap::default(),
            tracked_players: OpenHashKeys::new(PLAYER_SCORES_EXPECTED, PLAYER_SCORES_LOAD_FACTOR),
            team_names: OpenHashKeys::default(),
        }
    }
}

impl Scoreboard {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Vanilla `Scoreboard.getTrackedPlayers`.
    pub fn get_tracked_players(&self) -> impl Iterator<Item = &str> {
        self.tracked_players.iter()
    }

    /// Vanilla `Scoreboard.getPlayerTeams`, in the order iterating it gives.
    pub fn get_player_teams(&self) -> impl Iterator<Item = &Team> {
        self.team_names
            .iter_rev()
            .filter_map(|name| self.teams.get(name))
    }

    #[must_use]
    pub const fn get_objectives(&self) -> &FxHashMap<String, ScoreboardObjective> {
        &self.objectives
    }

    #[must_use]
    pub fn get_objective(&self, name: &str) -> Option<&ScoreboardObjective> {
        self.objectives.get(name)
    }

    pub fn get_objective_mut(&mut self, name: &str) -> Option<&mut ScoreboardObjective> {
        self.objectives.get_mut(name)
    }

    #[must_use]
    pub const fn get_display_slots(&self) -> &FxHashMap<ScoreboardDisplaySlot, String> {
        &self.display_slots
    }

    #[must_use]
    pub fn get_display_objective(&self, slot: ScoreboardDisplaySlot) -> Option<&str> {
        self.display_slots.get(&slot).map(String::as_str)
    }

    #[must_use]
    pub const fn get_scores(&self) -> &FxHashMap<String, FxHashMap<String, ScoreboardScore>> {
        &self.scores
    }

    #[must_use]
    pub fn get_score(&self, entity_name: &str, objective_name: &str) -> Option<&ScoreboardScore> {
        self.scores.get(objective_name)?.get(entity_name)
    }

    #[must_use]
    pub fn get_score_value(&self, entity_name: &str, objective_name: &str) -> Option<i32> {
        self.get_score(entity_name, objective_name)
            .map(|s| s.value.0)
    }

    #[must_use]
    pub fn get_scores_for_objective(
        &self,
        objective_name: &str,
    ) -> Option<&FxHashMap<String, ScoreboardScore>> {
        self.scores.get(objective_name)
    }

    #[must_use]
    pub fn get_scores_for_entity(&self, entity_name: &str) -> FxHashMap<String, &ScoreboardScore> {
        let mut entity_scores = FxHashMap::default();
        for (obj_name, obj_scores) in &self.scores {
            if let Some(score) = obj_scores.get(entity_name) {
                entity_scores.insert(obj_name.clone(), score);
            }
        }
        entity_scores
    }

    #[must_use]
    pub const fn get_teams(&self) -> &FxHashMap<String, Team> {
        &self.teams
    }

    #[must_use]
    pub fn get_team(&self, name: &str) -> Option<&Team> {
        self.teams.get(name)
    }

    pub fn get_team_mut(&mut self, name: &str) -> Option<&mut Team> {
        self.teams.get_mut(name)
    }

    #[must_use]
    pub fn get_entity_team(&self, entity_name: &str) -> Option<&Team> {
        self.teams
            .values()
            .find(|team| team.players.iter().any(|p| p == entity_name))
    }

    pub fn add_objective(
        &mut self,
        target: &impl ScoreboardTarget,
        objective: ScoreboardObjective,
    ) {
        if self.objectives.contains_key(&objective.name) {
            warn!(
                "Tried to create an objective which already exists: {}",
                &objective.name
            );
            return;
        }

        let je_update = CUpdateObjectives::new(
            objective.name.clone(),
            Mode::Add,
            objective.display_name.clone(),
            objective.render_type,
            objective.number_format.clone(),
        );

        let be_update = BSetDisplayObjective {
            display_slot_name: "sidebar".to_string(), // Default to sidebar
            objective_name: objective.name.clone(),
            objective_display_name: objective.display_name.clone().get_text(),
            criteria_name: "dummy".to_string(),
            sort_order: VarInt(0),
        };

        target.send_editioned(&je_update, &be_update);

        self.objectives.insert(objective.name.clone(), objective);
    }

    pub fn update_objective(
        &mut self,
        target: &impl ScoreboardTarget,
        objective: ScoreboardObjective,
    ) {
        if !self.objectives.contains_key(&objective.name) {
            warn!(
                "Tried to update an objective which does not exist: {}",
                &objective.name
            );
            return;
        }

        let je_update = CUpdateObjectives::new(
            objective.name.clone(),
            Mode::Update,
            objective.display_name.clone(),
            objective.render_type,
            objective.number_format.clone(),
        );

        let be_update = BSetDisplayObjective {
            display_slot_name: "sidebar".to_string(),
            objective_name: objective.name.clone(),
            objective_display_name: objective.display_name.clone().get_text(),
            criteria_name: "dummy".to_string(),
            sort_order: VarInt(0),
        };

        target.send_editioned(&je_update, &be_update);

        self.objectives.insert(objective.name.clone(), objective);
    }

    pub fn set_display_objective(
        &mut self,
        target: &impl ScoreboardTarget,
        slot: ScoreboardDisplaySlot,
        objective_name: Option<&str>,
    ) {
        let slot_str = match slot {
            ScoreboardDisplaySlot::List => "list",
            ScoreboardDisplaySlot::BelowName => "belowname",
            _ => "sidebar",
        };

        let obj_name_str = objective_name.unwrap_or("");

        let display_name = objective_name
            .and_then(|name| self.objectives.get(name))
            .map_or_else(
                || obj_name_str.to_string(),
                |o| o.display_name.clone().get_text(),
            );

        let je_display = CDisplayObjective::new(slot, obj_name_str.to_string());
        let be_display = BSetDisplayObjective {
            display_slot_name: slot_str.to_string(),
            objective_name: obj_name_str.to_string(),
            objective_display_name: display_name,
            criteria_name: "dummy".to_string(),
            sort_order: VarInt(0),
        };

        target.send_editioned(&je_display, &be_display);

        if let Some(name) = objective_name {
            self.display_slots.insert(slot, name.to_string());
        } else {
            self.display_slots.remove(&slot);
        }
    }

    pub fn clear_display_objective(
        &mut self,
        target: &impl ScoreboardTarget,
        slot: ScoreboardDisplaySlot,
    ) {
        self.set_display_objective(target, slot, None);
    }

    pub fn remove_objective(&mut self, target: &impl ScoreboardTarget, name: &str) {
        if !self.objectives.contains_key(name) {
            warn!(
                "Tried to remove an objective which does not exist: {}",
                name
            );
            return;
        }

        let je_packet = CUpdateObjectives::new(
            name.to_string(),
            Mode::Remove,
            TextComponent::empty(),
            RenderType::Integer,
            None,
        );

        let be_packet = BRemoveObjective {
            objective_name: name.to_string(),
        };

        target.send_editioned(&je_packet, &be_packet);

        self.objectives.remove(name);
        self.scores.remove(name);
        self.display_slots.retain(|_, obj| obj != name);
    }

    pub fn update_score(&mut self, target: &impl ScoreboardTarget, score: ScoreboardScore) {
        if !self.objectives.contains_key(&score.objective_name) {
            warn!(
                "Tried to place a score into an objective which does not exist: {}",
                &score.objective_name
            );
            return;
        }

        let je_packet = CUpdateScore::new(
            score.entity_name.clone(),
            score.objective_name.clone(),
            score.value,
            score.display_name.clone(),
            score.number_format.clone(),
        );

        let be_packet = BSetScore {
            action: VarInt(0), // Change
            entries: vec![BScoreEntry {
                scoreboard_id: score.entity_name.as_ptr() as i64, // Internal ID
                objective_name: score.objective_name.clone(),
                score: score.value,
                entry_type: VarInt(3), // Fake player / Literal
                entity_unique_id: 0,
                custom_name: score.entity_name.clone(),
            }],
        };

        target.send_editioned(&je_packet, &be_packet);

        self.tracked_players.insert(&score.entity_name);
        self.scores
            .entry(score.objective_name.clone())
            .or_default()
            .insert(score.entity_name.clone(), score);
    }

    pub fn set_score_value(
        &mut self,
        target: &impl ScoreboardTarget,
        entity_name: impl Into<String>,
        objective_name: impl Into<String>,
        value: i32,
    ) {
        let entity_s = entity_name.into();
        let obj_s = objective_name.into();
        let existing = self.get_score(&entity_s, &obj_s).cloned();
        let score = ScoreboardScore {
            entity_name: entity_s,
            objective_name: obj_s,
            value: VarInt(value),
            display_name: existing.as_ref().and_then(|s| s.display_name.clone()),
            number_format: existing.as_ref().and_then(|s| s.number_format.clone()),
            locked: existing.as_ref().is_none_or(|s| s.locked),
        };
        self.update_score(target, score);
    }

    pub fn add_score(
        &mut self,
        target: &impl ScoreboardTarget,
        entity_name: impl Into<String>,
        objective_name: impl Into<String>,
        delta: i32,
    ) -> i32 {
        let entity_s = entity_name.into();
        let obj_s = objective_name.into();
        let current_val = self.get_score_value(&entity_s, &obj_s).unwrap_or(0);
        let new_val = current_val.wrapping_add(delta);
        self.set_score_value(target, entity_s, obj_s, new_val);
        new_val
    }

    pub fn remove_score(
        &mut self,
        target: &impl ScoreboardTarget,
        entity_name: &str,
        objective_name: &str,
    ) {
        let je_packet = CResetScore::new(entity_name.to_string(), Some(objective_name.to_string()));

        let be_packet = BSetScore {
            action: VarInt(1), // Remove
            entries: vec![BScoreEntry {
                scoreboard_id: entity_name.as_ptr() as i64,
                objective_name: objective_name.to_string(),
                score: VarInt(0),
                entry_type: VarInt(3),
                entity_unique_id: 0,
                custom_name: entity_name.to_string(),
            }],
        };

        target.send_editioned(&je_packet, &be_packet);

        if let Some(objective_scores) = self.scores.get_mut(objective_name) {
            objective_scores.remove(entity_name);
        }
        // `resetSinglePlayerScore` stops tracking a holder left without scores.
        if !self
            .scores
            .values()
            .any(|objective_scores| objective_scores.contains_key(entity_name))
        {
            self.tracked_players.remove(entity_name);
        }
    }

    pub fn reset_scores_for_entity(&mut self, target: &impl ScoreboardTarget, entity_name: &str) {
        let je_packet = CResetScore::new(entity_name.to_string(), None);

        let mut be_entries = Vec::new();
        for (obj_name, obj_scores) in &self.scores {
            if obj_scores.contains_key(entity_name) {
                be_entries.push(BScoreEntry {
                    scoreboard_id: entity_name.as_ptr() as i64,
                    objective_name: obj_name.clone(),
                    score: VarInt(0),
                    entry_type: VarInt(3),
                    entity_unique_id: 0,
                    custom_name: entity_name.to_string(),
                });
            }
        }

        let be_packet = BSetScore {
            action: VarInt(1), // Remove
            entries: be_entries,
        };

        target.send_editioned(&je_packet, &be_packet);

        for obj_scores in self.scores.values_mut() {
            obj_scores.remove(entity_name);
        }
        self.tracked_players.remove(entity_name);
    }

    pub fn add_team(&mut self, target: &impl ScoreboardTarget, team: Team) {
        if self.teams.contains_key(&team.name) {
            warn!("Tried to create Team which already exists, {}", team.name);
            return;
        }

        target.send_je(&CSetPlayerTeam {
            team_name: team.name.clone(),
            method: TeamMethod::Create,
            parameters: Some(team.parameters()),
            players: team.players.clone().into(),
        });

        self.team_names.insert(&team.name);
        self.teams.insert(team.name.clone(), team);
    }

    pub fn create_team(&mut self, target: &impl ScoreboardTarget, team: Team) {
        self.add_team(target, team);
    }

    pub fn update_team(&mut self, target: &impl ScoreboardTarget, team: Team) {
        if !self.teams.contains_key(&team.name) {
            warn!("Tried to update Team which does not exist, {}", team.name);
            return;
        }

        target.send_je(&CSetPlayerTeam {
            team_name: team.name.clone(),
            method: TeamMethod::Update,
            parameters: Some(team.parameters()),
            players: Box::new([]),
        });

        // `onTeamChanged`: the packet, then `updateTeamWaypoints`.
        let (players, color) = (team.players.clone(), team.color);
        self.teams.insert(team.name.clone(), team);
        target.update_team_waypoints(&players, color);
    }

    pub fn remove_team(&mut self, target: &impl ScoreboardTarget, name: &str) {
        if !self.teams.contains_key(name) {
            warn!("Tried to remove Team which does not exist, {}", name);
            return;
        }

        target.send_je(&CSetPlayerTeam {
            team_name: name.to_string(),
            method: TeamMethod::Remove,
            parameters: None,
            players: Box::new([]),
        });

        self.team_names.remove(name);
        // `onTeamRemoved`: the packet, then `updateTeamWaypoints` with the players no longer
        // on any team.
        if let Some(team) = self.teams.remove(name) {
            target.update_team_waypoints(&team.players, None);
        }
    }

    /// Vanilla `Scoreboard.addPlayerToTeam`: the player leaves its current team first. Returns
    /// whether it was added.
    pub fn add_player_to_team(
        &mut self,
        target: &impl ScoreboardTarget,
        team_name: &str,
        player: String,
    ) -> bool {
        if !self.teams.contains_key(team_name) {
            warn!(
                "Tried to add player to Team which does not exist, {}",
                team_name
            );
            return false;
        }

        self.remove_player_from_any_team(target, &player);

        target.send_je(&CSetPlayerTeam {
            team_name: team_name.to_string(),
            method: TeamMethod::AddPlayers,
            parameters: None,
            players: vec![player.clone()].into(),
        });

        // `ServerScoreboard.addPlayerToTeam`: the packet, then `updatePlayerWaypoint` (which
        // is handed the team's colour, so the order against the push doesn't matter).
        if let Some(team) = self.teams.get_mut(team_name) {
            target.update_player_waypoint(&player, team.color);
            team.players.push(player);
        }
        true
    }

    /// Vanilla `Scoreboard.removePlayerFromTeam(String)`: returns whether the player had a team.
    pub fn remove_player_from_any_team(
        &mut self,
        target: &impl ScoreboardTarget,
        player: &str,
    ) -> bool {
        let Some(team_name) = self.get_entity_team(player).map(|team| team.name.clone()) else {
            return false;
        };
        self.remove_player_from_team(target, &team_name, player);
        true
    }

    pub fn remove_player_from_team(
        &mut self,
        target: &impl ScoreboardTarget,
        team_name: &str,
        player: &str,
    ) {
        let Some(team) = self.teams.get_mut(team_name) else {
            warn!(
                "Tried to remove player from Team which does not exist, {}",
                team_name
            );
            return;
        };

        if !team.players.contains(&player.to_string()) {
            return;
        }

        target.send_je(&CSetPlayerTeam {
            team_name: team_name.to_string(),
            method: TeamMethod::RemovePlayers,
            parameters: None,
            players: vec![player.to_string()].into(),
        });

        team.players.retain(|p| p != player);
        // `ServerScoreboard.removePlayerFromTeam`: the packet, then `updatePlayerWaypoint`.
        target.update_player_waypoint(player, None);
    }

    pub fn clear_team_players(&mut self, target: &impl ScoreboardTarget, team_name: &str) {
        let Some(team) = self.teams.get_mut(team_name) else {
            warn!(
                "Tried to clear players from Team which does not exist, {}",
                team_name
            );
            return;
        };

        if team.players.is_empty() {
            return;
        }

        let players_to_remove = team.players.clone();
        target.send_je(&CSetPlayerTeam {
            team_name: team_name.to_string(),
            method: TeamMethod::RemovePlayers,
            parameters: None,
            players: players_to_remove.clone().into(),
        });

        team.players.clear();
        for player in &players_to_remove {
            target.update_player_waypoint(player, None);
        }
    }

    pub fn send_to_player(&self, player: &Player) {
        for objective in self.objectives.values() {
            let je_update = CUpdateObjectives::new(
                objective.name.clone(),
                Mode::Add,
                objective.display_name.clone(),
                objective.render_type,
                objective.number_format.clone(),
            );
            let be_update = BSetDisplayObjective {
                display_slot_name: "sidebar".to_string(),
                objective_name: objective.name.clone(),
                objective_display_name: objective.display_name.clone().get_text(),
                criteria_name: "dummy".to_string(),
                sort_order: VarInt(0),
            };
            player.try_enqueue_packet_editioned(&je_update, &be_update);
        }

        for (slot, objective_name) in &self.display_slots {
            let slot_str = match slot {
                ScoreboardDisplaySlot::List => "list",
                ScoreboardDisplaySlot::BelowName => "belowname",
                _ => "sidebar",
            };
            let display_name = self.objectives.get(objective_name).map_or_else(
                || objective_name.clone(),
                |o| o.display_name.clone().get_text(),
            );
            let je_display = CDisplayObjective::new(*slot, objective_name.clone());
            let be_display = BSetDisplayObjective {
                display_slot_name: slot_str.to_string(),
                objective_name: objective_name.clone(),
                objective_display_name: display_name,
                criteria_name: "dummy".to_string(),
                sort_order: VarInt(0),
            };
            player.try_enqueue_packet_editioned(&je_display, &be_display);
        }

        for objective_scores in self.scores.values() {
            for score in objective_scores.values() {
                let je_packet = CUpdateScore::new(
                    score.entity_name.clone(),
                    score.objective_name.clone(),
                    score.value,
                    score.display_name.clone(),
                    score.number_format.clone(),
                );
                let be_packet = BSetScore {
                    action: VarInt(0),
                    entries: vec![BScoreEntry {
                        scoreboard_id: score.entity_name.as_ptr() as i64,
                        objective_name: score.objective_name.clone(),
                        score: score.value,
                        entry_type: VarInt(3),
                        entity_unique_id: 0,
                        custom_name: score.entity_name.clone(),
                    }],
                };
                player.try_enqueue_packet_editioned(&je_packet, &be_packet);
            }
        }

        for team in self.teams.values() {
            let je_packet = CSetPlayerTeam {
                team_name: team.name.clone(),
                method: TeamMethod::Create,
                parameters: Some(team.parameters()),
                players: team.players.clone().into(),
            };
            player.try_send_client_packet(&je_packet);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreboardObjective {
    pub name: String,
    pub display_name: TextComponent,
    pub render_type: RenderType,
    pub number_format: Option<NumberFormat>,
    pub criterion: String,
}

impl ScoreboardObjective {
    /// Vanilla `Objective.getFormattedDisplayName`: the display name in square brackets,
    /// hovering the objective's name.
    #[must_use]
    pub fn formatted_display_name(&self) -> TextComponent {
        TextComponent::wrap_in_square_brackets(self.display_name.clone().hover_event(
            pumpkin_util::text::hover::HoverEvent::show_text(TextComponent::text(
                self.name.clone(),
            )),
        ))
    }

    #[must_use]
    pub fn new(
        name: impl Into<String>,
        display_name: TextComponent,
        render_type: RenderType,
        number_format: Option<NumberFormat>,
        criterion: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            display_name,
            render_type,
            number_format,
            criterion: criterion.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreboardScore {
    pub entity_name: String,
    pub objective_name: String,
    pub value: VarInt,
    pub display_name: Option<TextComponent>,
    pub number_format: Option<NumberFormat>,
    pub locked: bool,
}

impl ScoreboardScore {
    #[must_use]
    pub fn new(
        entity_name: impl Into<String>,
        objective_name: impl Into<String>,
        value: VarInt,
        display_name: Option<TextComponent>,
        number_format: Option<NumberFormat>,
    ) -> Self {
        Self {
            entity_name: entity_name.into(),
            objective_name: objective_name.into(),
            value,
            display_name,
            number_format,
            locked: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameTagVisibility {
    Always,
    Never,
    HideForOtherTeams,
    HideForOwnTeam,
}

impl NameTagVisibility {
    /// Vanilla `getDisplayName`.
    #[must_use]
    pub fn display_name(self) -> TextComponent {
        let key = format!("team.visibility.{}", self.to_str());
        TextComponent::translate_cross(key.clone(), key, [])
    }

    #[must_use]
    pub const fn to_str(&self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
            Self::HideForOtherTeams => "hideForOtherTeams",
            Self::HideForOwnTeam => "hideForOwnTeam",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionRule {
    Always,
    Never,
    PushOtherTeams,
    PushOwnTeam,
}

impl CollisionRule {
    /// Vanilla `getDisplayName`.
    #[must_use]
    pub fn display_name(self) -> TextComponent {
        let key = format!("team.collision.{}", self.to_str());
        TextComponent::translate_cross(key.clone(), key, [])
    }

    #[must_use]
    pub const fn to_str(&self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
            Self::PushOtherTeams => "pushOtherTeams",
            Self::PushOwnTeam => "pushOwnTeam",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Team {
    pub name: String,
    pub display_name: TextComponent,
    pub options: i8,
    pub nametag_visibility: NameTagVisibility,
    pub death_message_visibility: NameTagVisibility,
    pub collision_rule: CollisionRule,
    pub color: Option<NamedColor>,
    pub player_prefix: TextComponent,
    pub player_suffix: TextComponent,
    pub players: Vec<String>,
}

impl Team {
    pub const BIT_FRIENDLY_FIRE: i8 = 1;
    pub const BIT_SEE_INVISIBLES: i8 = 2;

    /// Vanilla `new PlayerTeam(scoreboard, name)`.
    #[must_use]
    pub fn new(name: String) -> Self {
        Self {
            display_name: TextComponent::text(name.clone()),
            name,
            options: Self::BIT_FRIENDLY_FIRE | Self::BIT_SEE_INVISIBLES,
            nametag_visibility: NameTagVisibility::Always,
            death_message_visibility: NameTagVisibility::Always,
            collision_rule: CollisionRule::Always,
            color: None,
            player_prefix: TextComponent::empty(),
            player_suffix: TextComponent::empty(),
            players: Vec::new(),
        }
    }

    /// Vanilla `PlayerTeam.getFormattedDisplayName`.
    #[must_use]
    pub fn formatted_display_name(&self) -> TextComponent {
        let name = self
            .display_name
            .clone()
            .insertion(self.name.clone())
            .hover_event(pumpkin_util::text::hover::HoverEvent::show_text(
                TextComponent::text(self.name.clone()),
            ))
            .wrap_in_square_brackets();
        match self.color {
            Some(color) => name.color_named(color),
            None => name,
        }
    }

    /// Vanilla `ClientboundSetPlayerTeamPacket.Parameters(PlayerTeam)`.
    #[must_use]
    pub fn parameters(&self) -> TeamParameters<'_> {
        TeamParameters {
            display_name: &self.display_name,
            options: self.options,
            nametag_visibility: self.nametag_visibility.to_str(),
            collision_rule: self.collision_rule.to_str(),
            color: self.color.map_or(-1, |color| color as i32),
            player_prefix: &self.player_prefix,
            player_suffix: &self.player_suffix,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BedrockSortOrder {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BedrockDisplaySlot {
    PlayerList,
    Sidebar,
    BelowName,
}

impl BedrockDisplaySlot {
    #[must_use]
    pub const fn to_str(&self) -> &'static str {
        match self {
            Self::PlayerList => "list",
            Self::Sidebar => "sidebar",
            Self::BelowName => "belowname",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BedrockObjective {
    pub name: String,
    pub display_name: String,
    pub sort_order: BedrockSortOrder,
}

#[derive(Clone, Debug, Default)]
pub struct BedrockScoreboard {
    pub objectives: FxHashMap<String, BedrockObjective>,
    pub display_slots: FxHashMap<BedrockDisplaySlot, String>,
    pub scores: FxHashMap<(String, String), i32>,
}

impl BedrockScoreboard {
    pub fn add_objective(&mut self, player: &Player, objective: BedrockObjective) {
        let be_update = BSetDisplayObjective {
            display_slot_name: "sidebar".to_string(),
            objective_name: objective.name.clone(),
            objective_display_name: objective.display_name.clone(),
            criteria_name: "dummy".to_string(),
            sort_order: VarInt(match objective.sort_order {
                BedrockSortOrder::Ascending => 0,
                BedrockSortOrder::Descending => 1,
            }),
        };
        player.try_enqueue_packet_editioned(
            &pumpkin_protocol::java::client::play::CUpdateObjectives::new(
                objective.name.clone(),
                pumpkin_protocol::java::client::play::Mode::Add,
                TextComponent::text(objective.display_name.clone()),
                pumpkin_protocol::java::client::play::RenderType::Integer,
                None,
            ),
            &be_update,
        );

        self.objectives.insert(objective.name.clone(), objective);
    }

    pub fn update_objective(&mut self, player: &Player, objective: BedrockObjective) {
        let be_update = BSetDisplayObjective {
            display_slot_name: "sidebar".to_string(),
            objective_name: objective.name.clone(),
            objective_display_name: objective.display_name.clone(),
            criteria_name: "dummy".to_string(),
            sort_order: VarInt(match objective.sort_order {
                BedrockSortOrder::Ascending => 0,
                BedrockSortOrder::Descending => 1,
            }),
        };
        player.try_enqueue_packet_editioned(
            &pumpkin_protocol::java::client::play::CUpdateObjectives::new(
                objective.name.clone(),
                pumpkin_protocol::java::client::play::Mode::Update,
                TextComponent::text(objective.display_name.clone()),
                pumpkin_protocol::java::client::play::RenderType::Integer,
                None,
            ),
            &be_update,
        );

        self.objectives.insert(objective.name.clone(), objective);
    }

    pub fn remove_objective(&mut self, player: &Player, name: &str) {
        let be_remove = BRemoveObjective {
            objective_name: name.to_string(),
        };
        player.try_enqueue_packet_editioned(
            &pumpkin_protocol::java::client::play::CUpdateObjectives::new(
                name.to_string(),
                pumpkin_protocol::java::client::play::Mode::Remove,
                TextComponent::text(""),
                pumpkin_protocol::java::client::play::RenderType::Integer,
                None,
            ),
            &be_remove,
        );
        self.objectives.remove(name);
        self.display_slots.retain(|_, v| v != name);
        self.scores.retain(|(_, obj), _| obj != name);
    }

    pub fn set_display_objective(
        &mut self,
        player: &Player,
        slot: BedrockDisplaySlot,
        objective_name: Option<&str>,
    ) {
        let obj_name_str = objective_name.unwrap_or("");
        let display_name = objective_name
            .and_then(|name| self.objectives.get(name))
            .map_or_else(|| obj_name_str.to_string(), |o| o.display_name.clone());

        let be_display = BSetDisplayObjective {
            display_slot_name: slot.to_str().to_string(),
            objective_name: obj_name_str.to_string(),
            objective_display_name: display_name,
            criteria_name: "dummy".to_string(),
            sort_order: VarInt(0),
        };
        player.try_enqueue_packet_editioned(
            &pumpkin_protocol::java::client::play::CDisplayObjective::new(
                match slot {
                    BedrockDisplaySlot::PlayerList => ScoreboardDisplaySlot::List,
                    BedrockDisplaySlot::Sidebar => ScoreboardDisplaySlot::Sidebar,
                    BedrockDisplaySlot::BelowName => ScoreboardDisplaySlot::BelowName,
                },
                obj_name_str.to_string(),
            ),
            &be_display,
        );

        if let Some(name) = objective_name {
            self.display_slots.insert(slot, name.to_string());
        } else {
            self.display_slots.remove(&slot);
        }
    }

    pub fn clear_display_objective(&mut self, player: &Player, slot: BedrockDisplaySlot) {
        self.set_display_objective(player, slot, None);
    }

    pub fn update_score(
        &mut self,
        player: &Player,
        entity_name: &str,
        objective_name: &str,
        value: i32,
    ) {
        let score = ScoreboardScore::new(
            entity_name.to_string(),
            objective_name.to_string(),
            VarInt(value),
            None,
            None,
        );
        let be_score = BSetScore {
            action: VarInt(0),
            entries: vec![],
        };
        player.try_enqueue_packet_editioned(
            &pumpkin_protocol::java::client::play::CUpdateScore::new(
                score.entity_name.clone(),
                score.objective_name.clone(),
                score.value,
                score.display_name.clone(),
                score.number_format,
            ),
            &be_score,
        );

        self.scores
            .insert((entity_name.to_string(), objective_name.to_string()), value);
    }

    pub fn add_score(
        &mut self,
        player: &Player,
        entity_name: impl Into<String>,
        objective_name: impl Into<String>,
        delta: i32,
    ) -> i32 {
        let entity_s = entity_name.into();
        let obj_s = objective_name.into();
        let current = self
            .scores
            .get(&(entity_s.clone(), obj_s.clone()))
            .copied()
            .unwrap_or(0);
        let new_val = current + delta;
        self.update_score(player, &entity_s, &obj_s, new_val);
        new_val
    }

    pub fn remove_score(&mut self, player: &Player, entity_name: &str, objective_name: &str) {
        let be_score = BSetScore {
            action: VarInt(1),
            entries: vec![],
        };
        player.try_enqueue_packet_editioned(
            &pumpkin_protocol::java::client::play::CResetScore::new(
                entity_name.to_string(),
                Some(objective_name.to_string()),
            ),
            &be_score,
        );
        self.scores
            .remove(&(entity_name.to_string(), objective_name.to_string()));
    }

    pub fn reset_scores_for_entity(&mut self, player: &Player, entity_name: &str) {
        let be_score = BSetScore {
            action: VarInt(1),
            entries: vec![],
        };
        player.try_enqueue_packet_editioned(
            &pumpkin_protocol::java::client::play::CResetScore::new(entity_name.to_string(), None),
            &be_score,
        );
        self.scores.retain(|(entity, _), _| entity != entity_name);
    }

    pub fn send_to_player(&self, player: &Player) {
        for objective in self.objectives.values() {
            let be_update = BSetDisplayObjective {
                display_slot_name: "sidebar".to_string(),
                objective_name: objective.name.clone(),
                objective_display_name: objective.display_name.clone(),
                criteria_name: "dummy".to_string(),
                sort_order: VarInt(match objective.sort_order {
                    BedrockSortOrder::Ascending => 0,
                    BedrockSortOrder::Descending => 1,
                }),
            };
            player.try_enqueue_packet_editioned(
                &pumpkin_protocol::java::client::play::CUpdateObjectives::new(
                    objective.name.clone(),
                    pumpkin_protocol::java::client::play::Mode::Add,
                    TextComponent::text(objective.display_name.clone()),
                    pumpkin_protocol::java::client::play::RenderType::Integer,
                    None,
                ),
                &be_update,
            );
        }

        for (slot, objective_name) in &self.display_slots {
            let display_name = self
                .objectives
                .get(objective_name)
                .map_or_else(|| objective_name.clone(), |o| o.display_name.clone());
            let be_display = BSetDisplayObjective {
                display_slot_name: slot.to_str().to_string(),
                objective_name: objective_name.clone(),
                objective_display_name: display_name,
                criteria_name: "dummy".to_string(),
                sort_order: VarInt(0),
            };
            player.try_enqueue_packet_editioned(
                &pumpkin_protocol::java::client::play::CDisplayObjective::new(
                    match slot {
                        BedrockDisplaySlot::PlayerList => ScoreboardDisplaySlot::List,
                        BedrockDisplaySlot::Sidebar => ScoreboardDisplaySlot::Sidebar,
                        BedrockDisplaySlot::BelowName => ScoreboardDisplaySlot::BelowName,
                    },
                    objective_name.clone(),
                ),
                &be_display,
            );
        }

        for ((entity_name, objective_name), value) in &self.scores {
            let score = ScoreboardScore::new(
                entity_name.clone(),
                objective_name.clone(),
                VarInt(*value),
                None,
                None,
            );
            let be_score = BSetScore {
                action: VarInt(0),
                entries: vec![],
            };
            player.try_enqueue_packet_editioned(
                &pumpkin_protocol::java::client::play::CUpdateScore::new(
                    score.entity_name,
                    score.objective_name,
                    score.value,
                    score.display_name,
                    score.number_format,
                ),
                &be_score,
            );
        }
    }
}

#[derive(Default, Debug)]
pub struct ScoreboardBuilder {
    scoreboard: Scoreboard,
}

impl ScoreboardBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn objective(mut self, name: impl Into<String>, display_name: TextComponent) -> Self {
        let obj = ScoreboardObjective::new(
            name.into(),
            display_name,
            RenderType::Integer,
            None,
            "dummy",
        );
        self.scoreboard.objectives.insert(obj.name.clone(), obj);
        self
    }

    #[must_use]
    pub fn objective_with_render(
        mut self,
        name: impl Into<String>,
        display_name: TextComponent,
        render_type: RenderType,
    ) -> Self {
        let obj = ScoreboardObjective::new(name.into(), display_name, render_type, None, "dummy");
        self.scoreboard.objectives.insert(obj.name.clone(), obj);
        self
    }

    #[must_use]
    pub fn display_slot(
        mut self,
        slot: ScoreboardDisplaySlot,
        objective_name: impl Into<String>,
    ) -> Self {
        self.scoreboard
            .display_slots
            .insert(slot, objective_name.into());
        self
    }

    #[must_use]
    pub fn score(
        mut self,
        entity_name: impl Into<String>,
        objective_name: impl Into<String>,
        value: i32,
    ) -> Self {
        let entity_s = entity_name.into();
        let obj_s = objective_name.into();
        let score =
            ScoreboardScore::new(entity_s.clone(), obj_s.clone(), VarInt(value), None, None);
        self.scoreboard
            .scores
            .entry(entity_s)
            .or_default()
            .insert(obj_s, score);
        self
    }

    #[must_use]
    pub fn team(mut self, team: Team) -> Self {
        self.scoreboard.team_names.insert(&team.name);
        self.scoreboard.teams.insert(team.name.clone(), team);
        self
    }

    #[must_use]
    pub fn build(self) -> Scoreboard {
        self.scoreboard
    }
}

#[derive(Default, Debug)]
pub struct BedrockScoreboardBuilder {
    scoreboard: BedrockScoreboard,
}

impl BedrockScoreboardBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn objective(
        mut self,
        name: impl Into<String>,
        display_name: impl Into<String>,
        sort_order: BedrockSortOrder,
    ) -> Self {
        let name = name.into();
        let obj = BedrockObjective {
            name: name.clone(),
            display_name: display_name.into(),
            sort_order,
        };
        self.scoreboard.objectives.insert(name, obj);
        self
    }

    #[must_use]
    pub fn display_slot(
        mut self,
        slot: BedrockDisplaySlot,
        objective_name: impl Into<String>,
    ) -> Self {
        self.scoreboard
            .display_slots
            .insert(slot, objective_name.into());
        self
    }

    #[must_use]
    pub fn score(
        mut self,
        entity_name: impl Into<String>,
        objective_name: impl Into<String>,
        value: i32,
    ) -> Self {
        self.scoreboard
            .scores
            .insert((entity_name.into(), objective_name.into()), value);
        self
    }

    #[must_use]
    pub fn build(self) -> BedrockScoreboard {
        self.scoreboard
    }
}
