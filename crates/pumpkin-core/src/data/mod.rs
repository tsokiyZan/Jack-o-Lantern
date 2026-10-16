use std::{env, fs, path::Path, sync::RwLock};

use serde::{Deserialize, Serialize};
use tracing::{debug, error, warn};

const DATA_FOLDER: &str = "data/";

pub mod op;

pub mod advancement_data;
pub mod banlist_serializer;
pub mod banned_ip;
pub mod banned_player;
pub mod datapack;
pub mod player_server;
pub mod usercache;
pub mod whitelist;

/// Moves what Pumpkin saved under its old offline UUID for `name` to the vanilla one.
///
/// Covers player data, advancements, and operator, whitelist and ban entries. Data already
/// saved under the vanilla UUID is kept.
pub fn migrate_legacy_offline_uuid(server: &crate::server::Server, name: &str, uuid: uuid::Uuid) {
    let legacy = crate::net::legacy_offline_uuid(name);
    let move_file = |dir: &Path, extension: &str| {
        let from = dir.join(format!("{legacy}.{extension}"));
        let to = dir.join(format!("{uuid}.{extension}"));
        if from.exists() && !to.exists() {
            match fs::rename(&from, &to) {
                Ok(()) => tracing::info!(
                    "Moved {} to the vanilla offline UUID of {name}",
                    from.display()
                ),
                Err(error) => warn!("Failed to move {}: {error}", from.display()),
            }
        }
    };
    move_file(server.player_data_storage.data_path(), "dat");
    move_file(&server.advancement_manager.advancement_path, "json");

    let mut ops = server
        .data
        .operator_config
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(op) = ops.ops.iter_mut().find(|op| op.uuid == legacy) {
        op.uuid = uuid;
        ops.save();
    }
    drop(ops);
    let mut whitelist = server
        .data
        .whitelist_config
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = whitelist
        .whitelist
        .iter_mut()
        .find(|entry| entry.uuid == legacy)
    {
        entry.uuid = uuid;
        whitelist.save();
    }
    drop(whitelist);
    let mut bans = server
        .data
        .banned_player_list
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = bans
        .banned_players
        .iter_mut()
        .find(|entry| entry.uuid == legacy)
    {
        entry.uuid = uuid;
        bans.save();
    }
}

pub struct VanillaData {
    pub banned_ip_list: RwLock<banned_ip::BannedIpList>,
    pub banned_player_list: RwLock<banned_player::BannedPlayerList>,
    pub operator_config: RwLock<op::OperatorConfig>,
    pub user_cache: RwLock<usercache::UserCache>,
    pub whitelist_config: RwLock<whitelist::WhitelistConfig>,
}

impl VanillaData {
    #[must_use]
    pub fn load() -> Self {
        Self {
            banned_ip_list: RwLock::new(banned_ip::BannedIpList::load()),
            banned_player_list: RwLock::new(banned_player::BannedPlayerList::load()),
            operator_config: RwLock::new(op::OperatorConfig::load()),
            user_cache: RwLock::new(usercache::UserCache::load()),
            whitelist_config: RwLock::new(whitelist::WhitelistConfig::load()),
        }
    }
}

pub trait LoadJSONConfiguration {
    #[must_use]
    fn load() -> Self
    where
        Self: Sized + Default + Serialize + for<'de> Deserialize<'de>,
    {
        let exe_dir = env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let data_dir = exe_dir.join(DATA_FOLDER);
        if !data_dir.exists() {
            debug!("creating new data root folder");
            let _ = fs::create_dir(&data_dir);
        }
        let path = data_dir.join(Self::get_path());

        let config = if path.exists() {
            let file_content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(err) => {
                    error!(
                        "Couldn't read configuration file at {}: {err}",
                        path.display()
                    );
                    return Self::default();
                }
            };

            match serde_json::from_str(&file_content) {
                Ok(c) => c,
                Err(err) => {
                    error!(
                        "Couldn't parse data config at {}. Reason: {err}. Falling back to default.",
                        path.display()
                    );
                    Self::default()
                }
            }
        } else {
            let content = Self::default();

            if let Ok(json_str) = serde_json::to_string_pretty(&content) {
                let _ = fs::write(&path, json_str);
            }

            content
        };

        config.validate();
        config
    }

    fn get_path() -> &'static Path;

    fn validate(&self);
}

pub trait SaveJSONConfiguration: LoadJSONConfiguration {
    fn save(&self)
    where
        Self: Sized + Default + Serialize + for<'de> Deserialize<'de>,
    {
        let exe_dir = env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let data_dir = exe_dir.join(DATA_FOLDER);
        if !data_dir.exists() {
            debug!("creating new data root folder");
            let _ = fs::create_dir(&data_dir);
        }
        let path = data_dir.join(Self::get_path());

        let content = match serde_json::to_string_pretty(self) {
            Ok(content) => content,
            Err(err) => {
                warn!(
                    "Couldn't serialize operator data config to {}. Reason: {err}",
                    path.display()
                );
                return;
            }
        };

        if let Err(err) = std::fs::write(&path, content) {
            warn!(
                "Couldn't write operator config to {}. Reason: {err}",
                path.display()
            );
        }
    }
}
