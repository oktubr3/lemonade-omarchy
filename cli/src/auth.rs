// Login nativo: OAuth de Google (loopback + PKCE) → Firebase Identity Toolkit.
//
// Flujo idéntico en espíritu al de gcloud/gh: se abre el browser, Google
// redirige a un servidor efímero en 127.0.0.1 con el authorization code,
// y el code se canjea localmente. PKCE hace inútil un code interceptado.
// El id_token de Google se convierte en sesión de Firebase vía
// accounts:signInWithIdp con la misma API key que usa la web app.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use crate::config::Config;
use crate::paths;

#[derive(Serialize, Deserialize)]
pub struct TokenStore {
    pub refresh_token: String,
    pub id_token: String,
    /// epoch segundos en que vence id_token
    pub expires_at: u64,
    pub email: String,
    pub uid: String,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn tokens_path() -> std::path::PathBuf {
    paths::state_dir().join("tokens.json")
}

impl TokenStore {
    pub fn load() -> Option<Self> {
        let raw = std::fs::read_to_string(tokens_path()).ok()?;
        serde_json::from_str(&raw).ok()
    }

    fn save(&self) -> Result<(), String> {
        let raw = serde_json::to_string(self).map_err(|e| e.to_string())?;
        paths::write_private(&tokens_path(), &raw)
    }
}

/// Devuelve un id_token de Firebase válido, renovando si hace falta.
pub fn get_id_token(cfg: &Config) -> Result<String, String> {
    let mut ts = TokenStore::load().ok_or("sin sesión — corré: lemonade login")?;

    if ts.expires_at > now() + 120 {
        return Ok(ts.id_token);
    }

    // Renovar contra securetoken (el refresh token no expira salvo revocación).
    let url = format!(
        "https://securetoken.googleapis.com/v1/token?key={}",
        cfg.api_key
    );
    let resp: serde_json::Value = ureq::post(&url)
        .send_form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &ts.refresh_token),
        ])
        .map_err(|e| format!("renovando sesión: {}", short_http_err(e)))?
        .into_json()
        .map_err(|e| format!("respuesta de securetoken ilegible: {e}"))?;

    let id_token = resp["id_token"]
        .as_str()
        .ok_or("securetoken no devolvió id_token — corré: lemonade login")?
        .to_string();
    let expires_in: u64 = resp["expires_in"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600);
    if let Some(rt) = resp["refresh_token"].as_str() {
        ts.refresh_token = rt.to_string();
    }
    ts.id_token = id_token.clone();
    ts.expires_at = now() + expires_in;
    ts.save()?;
    Ok(id_token)
}

pub fn logout() -> Result<(), String> {
    match std::fs::remove_file(tokens_path()) {
        Ok(()) => {
            eprintln!("Sesión cerrada (tokens borrados).");
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("No había sesión.");
            Ok(())
        }
        Err(e) => Err(format!("borrando tokens: {e}")),
    }
}

pub fn login() -> Result<(), String> {
    let cfg = Config::load()?;

    // --- PKCE ---
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).map_err(|e| format!("sin entropía: {e}"))?;
    let verifier = URL_SAFE_NO_PAD.encode(buf);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    let mut state_buf = [0u8; 16];
    getrandom::getrandom(&mut state_buf).map_err(|e| format!("sin entropía: {e}"))?;
    let state = URL_SAFE_NO_PAD.encode(state_buf);

    // --- servidor loopback efímero ---
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind loopback: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth\
         ?client_id={}&redirect_uri={}&response_type=code\
         &scope=openid%20email&code_challenge={}&code_challenge_method=S256\
         &state={}&prompt=select_account",
        urlencode(&cfg.oauth_client_id),
        urlencode(&redirect_uri),
        challenge,
        state,
    );

    eprintln!("Abriendo el browser para iniciar sesión con Google…");
    let opened = std::process::Command::new("xdg-open")
        .arg(&auth_url)
        .spawn()
        .is_ok();
    if !opened {
        eprintln!("No pude abrir el browser. Entrá manualmente a:\n{auth_url}");
    }

    // --- esperar el redirect (una sola conexión con code) ---
    let code = wait_for_code(&listener, &state)?;

    // --- code → tokens de Google ---
    let resp: serde_json::Value = ureq::post("https://oauth2.googleapis.com/token")
        .send_form(&[
            ("code", code.as_str()),
            ("client_id", cfg.oauth_client_id.as_str()),
            ("client_secret", cfg.oauth_client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ])
        .map_err(|e| format!("canjeando el code con Google: {}", short_http_err(e)))?
        .into_json()
        .map_err(|e| format!("respuesta de Google ilegible: {e}"))?;

    let google_id_token = resp["id_token"]
        .as_str()
        .ok_or("Google no devolvió id_token")?;

    // --- id_token de Google → sesión de Firebase ---
    let url = format!(
        "https://identitytoolkit.googleapis.com/v1/accounts:signInWithIdp?key={}",
        cfg.api_key
    );
    let resp: serde_json::Value = ureq::post(&url)
        .send_json(serde_json::json!({
            "postBody": format!("id_token={google_id_token}&providerId=google.com"),
            "requestUri": redirect_uri,
            "returnSecureToken": true
        }))
        .map_err(|e| format!("signInWithIdp: {}", short_http_err(e)))?
        .into_json()
        .map_err(|e| format!("respuesta de Firebase ilegible: {e}"))?;

    let expires_in: u64 = resp["expiresIn"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600);
    let ts = TokenStore {
        refresh_token: resp["refreshToken"]
            .as_str()
            .ok_or("Firebase no devolvió refreshToken")?
            .to_string(),
        id_token: resp["idToken"]
            .as_str()
            .ok_or("Firebase no devolvió idToken")?
            .to_string(),
        expires_at: now() + expires_in,
        email: resp["email"].as_str().unwrap_or("?").to_string(),
        uid: resp["localId"]
            .as_str()
            .ok_or("Firebase no devolvió localId")?
            .to_string(),
    };
    ts.save()?;
    eprintln!("Sesión iniciada como {}.", ts.email);
    Ok(())
}

fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String, String> {
    // El browser puede pedir /favicon.ico etc.: atender hasta encontrar ?code=
    for stream in listener.incoming() {
        let mut stream = stream.map_err(|e| format!("accept: {e}"))?;
        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .map_err(|e| format!("leyendo request: {e}"))?;

        // "GET /?code=...&state=... HTTP/1.1"
        let path = request_line.split_whitespace().nth(1).unwrap_or("/");
        let query = path.splitn(2, '?').nth(1).unwrap_or("");
        let mut code = None;
        let mut state = None;
        let mut error = None;
        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            match (kv.next(), kv.next()) {
                (Some("code"), Some(v)) => code = Some(urldecode(v)),
                (Some("state"), Some(v)) => state = Some(urldecode(v)),
                (Some("error"), Some(v)) => error = Some(urldecode(v)),
                _ => {}
            }
        }

        let (status, body) = if code.is_some() && state.as_deref() == Some(expected_state) {
            ("200 OK", "<html><body style='font-family:sans-serif;text-align:center;padding-top:4em'><h2>🍋 Lemonade</h2><p>Sesión iniciada. Ya podés cerrar esta pestaña.</p></body></html>")
        } else if error.is_some() {
            ("200 OK", "<html><body><p>Login cancelado.</p></body></html>")
        } else {
            ("404 Not Found", "")
        };
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.flush();

        if let Some(err) = error {
            return Err(format!("Google devolvió error: {err}"));
        }
        if let Some(code) = code {
            if state.as_deref() != Some(expected_state) {
                return Err("state inválido en el redirect (posible CSRF) — reintentá".into());
            }
            return Ok(code);
        }
        // otra request (favicon…): seguir esperando
    }
    Err("el listener cerró sin recibir el code".into())
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Errores HTTP sin volcar el body completo (puede traer tokens en URLs de error).
pub fn short_http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(t) => format!("red: {t}"),
    }
}
