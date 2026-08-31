// Env Vault — el compartimento zero-knowledge de Lemonade.
//
// A diferencia del vault principal (descifrado server-side), acá el server
// solo guarda blobs: la master password nunca sale de esta máquina y la
// clave se deriva localmente. Réplica exacta de la crypto de la web app
// (src/utils/cryptoWorker.js + src/stores/envVault.js):
//
//   root     = PBKDF2-SHA256(password, salt_utf8, iteraciones, 32 bytes)
//   encKey   = HKDF-SHA256(salt=32 ceros, ikm=root, info="lemonade-enc-v1")
//   verifier = hex(HKDF-SHA256(salt=32 ceros, ikm=root, info="lemonade-ver-v1"))
//   blob     = { encrypted: hex(AES-256-GCM ct||tag), iv: hex(12 bytes) }
//
// Solo soporta vaults con verifierVersion 3 (HKDF). Los v1/v2 migran solos
// al desbloquearse una vez en la web app.
//
// La master password se pide oculta en cada invocación y muere con el
// proceso: nunca se cachea, nunca toca disco.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;

use crate::auth;
use crate::config::Config;
use crate::input;

pub struct VaultKey {
    key: Aes256Gcm,
}

struct VaultSettings {
    salt: String,
    password_hash: String,
    iterations: u32,
    verifier_version: u64,
}

fn firestore_get_doc(cfg: &Config, path: &str) -> Result<serde_json::Value, String> {
    let token = auth::get_id_token(cfg)?;
    let url = format!(
        "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents/{path}",
        cfg.project_id
    );
    ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => {
                "no hay Env Vault configurado (crealo en la web app)".to_string()
            }
            other => format!("leyendo Firestore: {}", auth::short_http_err(other)),
        })?
        .into_json()
        .map_err(|e| format!("respuesta ilegible: {e}"))
}

pub fn firestore_query(
    cfg: &Config,
    collection: &str,
    filters: &[(&str, &str)],
) -> Result<Vec<(String, serde_json::Value)>, String> {
    let token = auth::get_id_token(cfg)?;
    let url = format!(
        "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents:runQuery",
        cfg.project_id
    );

    let field_filters: Vec<serde_json::Value> = filters
        .iter()
        .map(|(f, v)| {
            serde_json::json!({
                "fieldFilter": {
                    "field": { "fieldPath": f },
                    "op": "EQUAL",
                    "value": { "stringValue": v }
                }
            })
        })
        .collect();
    let where_clause = if field_filters.len() == 1 {
        field_filters[0].clone()
    } else {
        serde_json::json!({ "compositeFilter": { "op": "AND", "filters": field_filters } })
    };

    let resp: serde_json::Value = ureq::post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({
            "structuredQuery": {
                "from": [{ "collectionId": collection }],
                "where": where_clause
            }
        }))
        .map_err(|e| format!("consultando {collection}: {}", auth::short_http_err(e)))?
        .into_json()
        .map_err(|e| format!("respuesta ilegible: {e}"))?;

    let mut out = Vec::new();
    for row in resp.as_array().unwrap_or(&Vec::new()) {
        let Some(doc) = row.get("document") else { continue };
        let id = doc["name"]
            .as_str()
            .and_then(|n| n.rsplit('/').next())
            .unwrap_or_default()
            .to_string();
        out.push((id, doc["fields"].clone()));
    }
    Ok(out)
}

fn load_settings(cfg: &Config) -> Result<VaultSettings, String> {
    let uid = auth::TokenStore::load().ok_or("sin sesión")?.uid;
    let doc = firestore_get_doc(cfg, &format!("env_vault_settings/{uid}"))?;
    let f = &doc["fields"];
    let iterations = f["kdfIterations"]["integerValue"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600_000);
    Ok(VaultSettings {
        salt: f["salt"]["stringValue"].as_str().unwrap_or("").to_string(),
        password_hash: f["passwordHash"]["stringValue"].as_str().unwrap_or("").to_string(),
        iterations,
        verifier_version: f["verifierVersion"]["integerValue"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1),
    })
}

fn derive(password: &str, salt: &str, iterations: u32) -> ([u8; 32], String) {
    let mut root = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt.as_bytes(), iterations, &mut root);

    let hk = Hkdf::<Sha256>::new(Some(&[0u8; 32]), &root);
    let mut enc = [0u8; 32];
    hk.expand(b"lemonade-enc-v1", &mut enc).unwrap();
    let mut ver = [0u8; 32];
    hk.expand(b"lemonade-ver-v1", &mut ver).unwrap();
    (enc, hex::encode(ver))
}

/// Pide la master password, deriva y verifica contra el hash almacenado.
pub fn unlock(cfg: &Config) -> Result<VaultKey, String> {
    let settings = load_settings(cfg)?;
    if settings.salt.is_empty() || settings.password_hash.is_empty() {
        return Err("el Env Vault no está inicializado (configuralo en la web app)".into());
    }
    if settings.verifier_version < 3 {
        return Err(
            "tu vault usa un formato de verifier viejo — desbloquealo una vez en la web app \
             para que migre a v3 y después volvé acá"
                .into(),
        );
    }

    let password = input::prompt_hidden("Master password del Env Vault")?;
    if password.is_empty() {
        return Err("cancelado".into());
    }

    eprintln!("Derivando clave ({} iteraciones)…", settings.iterations);
    let (enc, verifier) = derive(&password, &settings.salt, settings.iterations);

    if verifier != settings.password_hash {
        return Err("master password incorrecta".into());
    }

    Ok(VaultKey {
        key: Aes256Gcm::new_from_slice(&enc).map_err(|e| e.to_string())?,
    })
}

impl VaultKey {
    /// Descifra un blob { encrypted: hex(ct||tag), iv: hex(12B) }.
    pub fn decrypt_blob(&self, blob: &serde_json::Value) -> Result<String, String> {
        let ct_hex = blob["encrypted"].as_str().ok_or("blob sin campo encrypted")?;
        let iv_hex = blob["iv"].as_str().ok_or("blob sin campo iv")?;
        let ct = hex::decode(ct_hex).map_err(|e| format!("hex inválido: {e}"))?;
        let iv = hex::decode(iv_hex).map_err(|e| format!("iv inválido: {e}"))?;
        if iv.len() != 12 {
            return Err("iv de largo inesperado".into());
        }
        let plain = self
            .key
            .decrypt(Nonce::from_slice(&iv), ct.as_ref())
            .map_err(|_| "no se pudo descifrar (¿blob corrupto?)".to_string())?;
        String::from_utf8(plain).map_err(|e| format!("contenido no-UTF8: {e}"))
    }
}

/// Convierte fields Firestore de un blob mapValue { encrypted, iv } a JSON plano.
pub fn map_blob(fields: &serde_json::Value) -> serde_json::Value {
    let m = &fields["mapValue"]["fields"];
    serde_json::json!({
        "encrypted": m["encrypted"]["stringValue"].as_str().unwrap_or(""),
        "iv": m["iv"]["stringValue"].as_str().unwrap_or(""),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Vector generado con WebCrypto (node), réplica del cryptoWorker de la web.
    #[test]
    fn crypto_matches_webcrypto() {
        let (enc, verifier) = derive("test-master-2026", "a1b2c3d4e5f60718293a4b5c6d7e8f90", 10000);
        assert_eq!(
            verifier,
            "30c8d8aeb31e54f71c7305c4e64e48d83a970f89c73e40aa61f25df9933e5f37"
        );
        let vk = VaultKey {
            key: Aes256Gcm::new_from_slice(&enc).unwrap(),
        };
        let blob = serde_json::json!({
            "encrypted": "53547fbd4ef1e3b7fa437983260be0af57fbeac8a77e4783675d4316a347426080fbf3c405edce",
            "iv": "070707070707070707070707"
        });
        assert_eq!(vk.decrypt_blob(&blob).unwrap(), "SECRETO=hola-mundo-🍋");
    }
}
