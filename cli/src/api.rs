// Acceso a datos: Firestore REST para el listado (igual que la web app,
// que lee password_entries directo con las security rules como guardia),
// y Cloud Functions para todo lo que toque material cifrado — el server
// descifra el vault principal, el cliente jamás ve la clave.

use serde::{Deserialize, Serialize};

use crate::auth;
use crate::config::Config;
use crate::paths;

#[derive(Serialize, Deserialize)]
pub struct EntryMeta {
    pub id: String,
    pub title: String,
    pub username: String,
    pub url: String,
    pub has_totp: bool,
}

pub struct EntryFull {
    pub password: String,
    pub username: String,
}

pub struct TotpCode {
    pub code: String,
    pub time_remaining: u64,
}

fn cache_path() -> std::path::PathBuf {
    paths::cache_dir().join("entries.json")
}

pub fn cache_age_secs() -> Option<u64> {
    let meta = std::fs::metadata(cache_path()).ok()?;
    let modified = meta.modified().ok()?;
    modified.elapsed().ok().map(|d| d.as_secs())
}

/// Listado de entradas. Con cache metadata-only (0600) para que el panel
/// abra al instante; `refresh` fuerza el fetch y reescribe el cache.
pub fn list_entries(cfg: &Config, refresh: bool) -> Result<Vec<EntryMeta>, String> {
    if !refresh {
        if let Ok(raw) = std::fs::read_to_string(cache_path()) {
            if let Ok(entries) = serde_json::from_str::<Vec<EntryMeta>>(&raw) {
                return Ok(entries);
            }
        }
    }

    let token = auth::get_id_token(cfg)?;
    let uid = auth::TokenStore::load()
        .ok_or("sin sesión")?
        .uid;

    let url = format!(
        "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents:runQuery",
        cfg.project_id
    );
    let body = serde_json::json!({
        "structuredQuery": {
            "from": [{ "collectionId": "password_entries" }],
            "where": {
                "fieldFilter": {
                    "field": { "fieldPath": "userId" },
                    "op": "EQUAL",
                    "value": { "stringValue": uid }
                }
            }
        }
    });

    let resp: serde_json::Value = ureq::post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .map_err(|e| format!("consultando Firestore: {}", auth::short_http_err(e)))?
        .into_json()
        .map_err(|e| format!("respuesta de Firestore ilegible: {e}"))?;

    let mut entries: Vec<EntryMeta> = Vec::new();
    for row in resp.as_array().unwrap_or(&Vec::new()) {
        let Some(doc) = row.get("document") else { continue };
        let fields = &doc["fields"];

        // Mismo filtro post-query que la web app: docs pre-migración sin
        // "status" siguen siendo entradas activas.
        if field_str(fields, "status") == "deleted" {
            continue;
        }

        let id = doc["name"]
            .as_str()
            .and_then(|n| n.rsplit('/').next())
            .unwrap_or_default()
            .to_string();
        if id.is_empty() {
            continue;
        }

        let mut title = field_str(fields, "title");
        if title.is_empty() {
            title = field_str(fields, "name"); // legacy
        }
        if title.is_empty() {
            title = "Sin nombre".into();
        }

        entries.push(EntryMeta {
            id,
            title,
            username: field_str(fields, "username"),
            url: field_str(fields, "url"),
            has_totp: !fields["totpSecret"].is_null(),
        });
    }

    entries.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

    // Cache metadata-only; nunca material cifrado ni descifrado.
    if let Ok(raw) = serde_json::to_string(&entries) {
        let _ = paths::write_private(&cache_path(), &raw);
    }
    Ok(entries)
}

fn field_str(fields: &serde_json::Value, name: &str) -> String {
    fields[name]["stringValue"].as_str().unwrap_or("").to_string()
}

/// Pide la entrada descifrada al server (getPasswordEntryHttp).
pub fn get_entry(cfg: &Config, entry_id: &str) -> Result<EntryFull, String> {
    let token = auth::get_id_token(cfg)?;
    let resp: serde_json::Value = ureq::post(&format!("{}/getPasswordEntryHttp", cfg.functions_url))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "entryId": entry_id }))
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => "entrada no encontrada".to_string(),
            ureq::Error::Status(429, _) => "rate limit del server — esperá un minuto".to_string(),
            other => format!("pidiendo la entrada: {}", auth::short_http_err(other)),
        })?
        .into_json()
        .map_err(|e| format!("respuesta ilegible: {e}"))?;

    Ok(EntryFull {
        password: resp["password"].as_str().unwrap_or("").to_string(),
        username: resp["username"].as_str().unwrap_or("").to_string(),
    })
}

/// Código TOTP vigente (getTotpCodeHttp).
pub fn get_totp(cfg: &Config, entry_id: &str) -> Result<TotpCode, String> {
    let token = auth::get_id_token(cfg)?;
    let resp: serde_json::Value = ureq::post(&format!("{}/getTotpCodeHttp", cfg.functions_url))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "entryId": entry_id }))
        .map_err(|e| match e {
            ureq::Error::Status(400, _) => "la entrada no tiene TOTP configurado".to_string(),
            ureq::Error::Status(429, _) => "rate limit del server — esperá un minuto".to_string(),
            other => format!("pidiendo TOTP: {}", auth::short_http_err(other)),
        })?
        .into_json()
        .map_err(|e| format!("respuesta ilegible: {e}"))?;

    let code = resp["code"].as_str().unwrap_or("").to_string();
    if code.is_empty() {
        return Err("el server no devolvió código".into());
    }
    Ok(TotpCode {
        code,
        time_remaining: resp["timeRemaining"].as_u64().unwrap_or(30),
    })
}

// --- escritura ---

pub struct NewEntry {
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub notes: String,
}

fn post_fn(cfg: &Config, name: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let token = auth::get_id_token(cfg)?;
    ureq::post(&format!("{}/{name}", cfg.functions_url))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => "entrada no encontrada".to_string(),
            ureq::Error::Status(429, _) => "rate limit del server — esperá un minuto".to_string(),
            ureq::Error::Status(400, resp) => {
                let msg = resp
                    .into_json::<serde_json::Value>()
                    .ok()
                    .and_then(|v| v["error"].as_str().map(String::from))
                    .unwrap_or_else(|| "petición inválida".into());
                format!("el server rechazó la petición: {msg}")
            }
            other => format!("{name}: {}", auth::short_http_err(other)),
        })?
        .into_json()
        .map_err(|e| format!("respuesta ilegible: {e}"))
}

/// Refresca el cache tras una mutación; si falla no es fatal.
fn refresh_cache_best_effort(cfg: &Config) {
    let _ = list_entries(cfg, true);
}

pub fn create_entry(cfg: &Config, e: &NewEntry) -> Result<String, String> {
    let resp = post_fn(
        cfg,
        "createPasswordEntryHttp",
        serde_json::json!({
            "title": e.title,
            "username": e.username,
            "password": e.password,
            "url": e.url,
            "notes": e.notes,
        }),
    )?;
    let id = resp["entryId"]
        .as_str()
        .ok_or("el server no devolvió entryId")?
        .to_string();
    refresh_cache_best_effort(cfg);
    Ok(id)
}

/// Actualiza solo los campos presentes en `fields`
/// (permitidos por el server: title, username, password, url, notes).
pub fn update_entry(
    cfg: &Config,
    entry_id: &str,
    fields: serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    if fields.is_empty() {
        return Err("nada que actualizar".into());
    }
    let mut body = serde_json::Value::Object(fields);
    body["entryId"] = serde_json::json!(entry_id);
    post_fn(cfg, "updatePasswordEntryHttp", body)?;
    refresh_cache_best_effort(cfg);
    Ok(())
}

/// Borrado suave: la entrada va a la papelera de Lemonade
/// (recuperable desde la web app; se purga sola a los 30 días).
pub fn delete_entry(cfg: &Config, entry_id: &str) -> Result<(), String> {
    post_fn(cfg, "deletePasswordEntryHttp", serde_json::json!({ "entryId": entry_id }))?;
    refresh_cache_best_effort(cfg);
    Ok(())
}

/// Busca en el cache local el título de una entrada (para confirmaciones).
pub fn cached_title(entry_id: &str) -> Option<String> {
    let raw = std::fs::read_to_string(cache_path()).ok()?;
    let entries: Vec<EntryMeta> = serde_json::from_str(&raw).ok()?;
    entries.into_iter().find(|e| e.id == entry_id).map(|e| e.title)
}

/// Campo de metadata (username/url) desde el cache local — sin tocar el
/// server. Si el id no está en cache, refresca una vez y reintenta.
pub fn cached_field(cfg: &Config, entry_id: &str, field: &str) -> Result<String, String> {
    let lookup = |entries: &[EntryMeta]| -> Option<String> {
        entries.iter().find(|e| e.id == entry_id).map(|e| match field {
            "username" => e.username.clone(),
            "url" => e.url.clone(),
            _ => String::new(),
        })
    };
    if let Some(v) = lookup(&list_entries(cfg, false)?) {
        return Ok(v);
    }
    lookup(&list_entries(cfg, true)?).ok_or_else(|| "entrada no encontrada".to_string())
}
