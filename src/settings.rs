use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const FILE_PATH: &str = "./wg-gui.settings";

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Settings {
    #[serde(rename = "MultipleTunnels", default)]
    pub multiple_tunnels: bool,
    #[serde(rename = "StartOnTray", default)]
    pub start_on_tray: bool,
    #[serde(rename = "CheckUpdates", default)]
    pub check_updates: bool,
    #[serde(rename = "Debug", default)]
    pub debug: bool,
    #[serde(rename = "RememberConnectedServers", default)]
    pub remember_connected_servers: bool,
    #[serde(rename = "ConnectedServers", default)]
    pub connected_servers: HashMap<String, i64>,
    #[serde(rename = "CountryFlags", default)]
    pub country_flags: bool,
    #[serde(rename = "Passwordless", default)]
    pub passwordless: bool,
}

impl Settings {
    pub fn load() -> Settings {
        let data = match std::fs::read(FILE_PATH) {
            Ok(data) => data,
            Err(_) => return Settings::default(),
        };

        serde_json::from_slice(&data).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let data = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        std::fs::write(FILE_PATH, data).map_err(|e| e.to_string())?;

        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(FILE_PATH, std::fs::Permissions::from_mode(0o660));

        Ok(())
    }
}
