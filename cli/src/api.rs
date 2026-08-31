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
    #[serde(default)]
    pub highlighted: bool,
}

pub struct EntryFull {
    pub password: String,
    pub username: String,
    pub title: String,
    pub url: String,
    pub notes: String,
    /// (label, valor descifrado, tipo)
    pub custom_fields: Vec<(String, String, String)>,
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
            highlighted: fields["highlighted"]["booleanValue"].as_bool().unwrap_or(false),
        });
    }

    entries.sort_by(|a, b| {
        b.highlighted
            .cmp(&a.highlighted)
            .then(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });

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
        title: resp["title"].as_str().unwrap_or("").to_string(),
        url: resp["url"].as_str().unwrap_or("").to_string(),
        notes: resp["notes"].as_str().unwrap_or("").to_string(),
        custom_fields: resp["customFields"]
            .as_array()
            .map(|fs| {
                fs.iter()
                    .map(|f| {
                        (
                            f["label"].as_str().unwrap_or("").to_string(),
                            f["value"].as_str().unwrap_or("").to_string(),
                            f["type"].as_str().unwrap_or("text").to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
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

/// Alterna el favorito (campo `highlighted`, mismo que la estrella de la web).
/// Devuelve el estado nuevo.
pub fn toggle_highlight(cfg: &Config, entry_id: &str) -> Result<bool, String> {
    let entries = list_entries(cfg, false)?;
    let entry = entries
        .iter()
        .find(|e| e.id == entry_id)
        .ok_or("entrada no encontrada en el listado")?;
    let new_state = !entry.highlighted;
    let mut fields = serde_json::Map::new();
    fields.insert("highlighted".into(), serde_json::json!(new_state));
    update_entry(cfg, entry_id, fields)?;
    Ok(new_state)
}

// --- notas seguras ---

pub struct NoteMeta {
    pub id: String,
    pub title: String,
}

pub fn list_notes(cfg: &Config) -> Result<Vec<NoteMeta>, String> {
    let resp = post_fn(cfg, "getSecureNotesHttp", serde_json::json!({}))?;
    Ok(resp["notes"]
        .as_array()
        .map(|ns| {
            ns.iter()
                .map(|n| NoteMeta {
                    id: n["id"].as_str().unwrap_or("").to_string(),
                    title: n["title"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Devuelve (título, contenido descifrado).
pub fn get_note(cfg: &Config, note_id: &str) -> Result<(String, String), String> {
    let resp = post_fn(cfg, "getSecureNoteHttp", serde_json::json!({ "noteId": note_id }))?;
    let n = &resp["note"];
    Ok((
        n["title"].as_str().unwrap_or("").to_string(),
        n["content"].as_str().unwrap_or("").to_string(),
    ))
}

pub fn create_note(cfg: &Config, title: &str, content: &str) -> Result<String, String> {
    let resp = post_fn(
        cfg,
        "createSecureNoteHttp",
        serde_json::json!({ "title": title, "content": content }),
    )?;
    Ok(resp["noteId"]
        .as_str()
        .or_else(|| resp["id"].as_str())
        .unwrap_or("?")
        .to_string())
}

pub fn update_note(
    cfg: &Config,
    note_id: &str,
    title: Option<&str>,
    content: Option<&str>,
) -> Result<(), String> {
    let mut body = serde_json::json!({ "noteId": note_id });
    if let Some(t) = title {
        body["title"] = serde_json::json!(t);
    }
    if let Some(c) = content {
        body["content"] = serde_json::json!(c);
    }
    post_fn(cfg, "updateSecureNoteHttp", body)?;
    Ok(())
}

pub fn delete_note(cfg: &Config, note_id: &str) -> Result<(), String> {
    post_fn(cfg, "deleteSecureNoteHttp", serde_json::json!({ "noteId": note_id }))?;
    Ok(())
}

// --- papelera ---

pub struct TrashEntry {
    pub id: String,
    pub title: String,
    pub username: String,
    pub deleted_at: String,
}

pub fn list_trash(cfg: &Config) -> Result<Vec<TrashEntry>, String> {
    let resp = post_fn(cfg, "getTrashEntriesHttp", serde_json::json!({}))?;
    Ok(resp["entries"]
        .as_array()
        .map(|es| {
            es.iter()
                .map(|e| TrashEntry {
                    id: e["id"].as_str().unwrap_or("").to_string(),
                    title: e["title"].as_str().unwrap_or("Sin nombre").to_string(),
                    username: e["username"].as_str().unwrap_or("").to_string(),
                    deleted_at: e["deletedAt"]["_seconds"]
                        .as_i64()
                        .map(|s| format!("{s}"))
                        .or_else(|| e["deletedAt"].as_str().map(String::from))
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default())
}

pub fn restore_entry(cfg: &Config, entry_id: &str) -> Result<(), String> {
    post_fn(cfg, "restorePasswordEntryHttp", serde_json::json!({ "entryId": entry_id }))?;
    refresh_cache_best_effort(cfg);
    Ok(())
}

pub fn purge_entry(cfg: &Config, entry_id: &str) -> Result<(), String> {
    post_fn(
        cfg,
        "permanentDeletePasswordEntryHttp",
        serde_json::json!({ "entryId": entry_id }),
    )?;
    Ok(())
}

// --- compartir a usuarios Lemonade ---

pub struct SystemUser {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
}

/// Búsqueda por email EXACTO (el server previene enumeración de usuarios).
pub fn find_user(cfg: &Config, email: &str) -> Result<Option<SystemUser>, String> {
    let resp = post_fn(cfg, "getSystemUsersHttp", serde_json::json!({ "searchQuery": email }))?;
    Ok(resp["users"].as_array().and_then(|us| {
        us.iter()
            .find(|u| u["email"].as_str().unwrap_or("").eq_ignore_ascii_case(email))
            .map(|u| SystemUser {
                user_id: u["userId"]
                    .as_str()
                    .or_else(|| u["id"].as_str())
                    .unwrap_or("")
                    .to_string(),
                email: u["email"].as_str().unwrap_or("").to_string(),
                display_name: u["displayName"].as_str().unwrap_or("").to_string(),
            })
    }))
}

pub fn share_to_user(cfg: &Config, entry_id: &str, to_user_id: &str) -> Result<(), String> {
    post_fn(
        cfg,
        "sharePasswordEntryHttp",
        serde_json::json!({ "entryId": entry_id, "toUserId": to_user_id }),
    )?;
    Ok(())
}

pub struct PendingShare {
    pub id: String,
    pub title: String,
    pub from_name: String,
    pub from_email: String,
}

pub fn pending_shares(cfg: &Config) -> Result<Vec<PendingShare>, String> {
    let resp = post_fn(cfg, "getPendingSharedPasswordsHttp", serde_json::json!({}))?;
    Ok(resp["pendingShares"]
        .as_array()
        .map(|ss| {
            ss.iter()
                .map(|s| PendingShare {
                    id: s["id"].as_str().unwrap_or("").to_string(),
                    title: s["title"].as_str().unwrap_or("Sin nombre").to_string(),
                    from_name: s["fromUserName"].as_str().unwrap_or("").to_string(),
                    from_email: s["fromUserEmail"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default())
}

pub fn accept_share(cfg: &Config, share_id: &str) -> Result<(), String> {
    post_fn(cfg, "acceptSharedPasswordHttp", serde_json::json!({ "shareId": share_id }))?;
    refresh_cache_best_effort(cfg);
    Ok(())
}

pub fn reject_share(cfg: &Config, share_id: &str) -> Result<(), String> {
    post_fn(cfg, "rejectSharedPasswordHttp", serde_json::json!({ "shareId": share_id }))?;
    Ok(())
}

// --- historial de contraseñas ---

pub struct HistoryItem {
    pub password: String,
    pub changed_at: String,
}

pub fn password_history(cfg: &Config, entry_id: &str) -> Result<Vec<HistoryItem>, String> {
    let resp = post_fn(cfg, "getPasswordHistoryHttp", serde_json::json!({ "entryId": entry_id }))?;
    Ok(resp["history"]
        .as_array()
        .map(|hs| {
            hs.iter()
                .map(|h| HistoryItem {
                    password: h["password"].as_str().unwrap_or("").to_string(),
                    changed_at: h["changedAt"].as_str().unwrap_or("¿?").to_string(),
                })
                .collect()
        })
        .unwrap_or_default())
}
