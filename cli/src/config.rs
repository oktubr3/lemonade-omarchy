// Config en ~/.config/lemonade/config.json — NO se commitea nunca.
//
// La API key de Firebase es pública por diseño (viaja en el bundle web),
// y el client ID de un OAuth "Desktop app" tampoco es un secreto real,
// pero igual viven acá afuera del repo: el repo publicable no debe
// contener ni un identificador del deployment personal.

use serde::Deserialize;

use crate::paths;

#[derive(Deserialize)]
pub struct Config {
    /// Firebase Web API key (VITE_FIREBASE_API_KEY del deployment).
    pub api_key: String,
    /// ID del proyecto Firebase (ej. passmanager-d2b6d).
    pub project_id: String,
    /// Base de Cloud Functions (ej. https://us-central1-<pid>.cloudfunctions.net).
    pub functions_url: String,
    /// OAuth client ID tipo "Desktop app" del MISMO proyecto GCP.
    pub oauth_client_id: String,
    /// Google exige client_secret para clientes Desktop (no es confidencial).
    pub oauth_client_secret: String,
}

impl Config {
    pub fn path() -> std::path::PathBuf {
        paths::config_dir().join("config.json")
    }

    pub fn load() -> Result<Self, String> {
        let path = Self::path();
        let raw = std::fs::read_to_string(&path).map_err(|_| {
            format!(
                "no existe {}.\nCrealo con:\n{}",
                path.display(),
                TEMPLATE
            )
        })?;
        serde_json::from_str(&raw).map_err(|e| format!("config inválida ({}): {e}", path.display()))
    }
}

pub const TEMPLATE: &str = r#"{
  "api_key": "<VITE_FIREBASE_API_KEY>",
  "project_id": "<VITE_FIREBASE_PROJECT_ID>",
  "functions_url": "https://us-central1-<project_id>.cloudfunctions.net",
  "oauth_client_id": "<id>.apps.googleusercontent.com",
  "oauth_client_secret": "<secret del cliente Desktop>"
}"#;
