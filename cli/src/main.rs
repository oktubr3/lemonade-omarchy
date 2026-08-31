// lemonade — CLI nativo para Lemonade Password Manager
//
// Diseñado para el plugin de barra de Omarchy, pero usable solo.
// Seguridad:
//   - La contraseña nunca pasa por argv (visible en /proc/*/cmdline):
//     siempre stdin (wl-copy, wtype) o env (job de auto-clear).
//   - Tokens con permisos 0600 en ~/.local/state/lemonade/.
//   - El cache de listado solo guarda metadata (título/usuario/url), jamás
//     material cifrado ni descifrado.

mod api;
mod auth;
mod clip;
mod config;
mod paths;

use std::process::ExitCode;

const USAGE: &str = "\
lemonade — Lemonade Password Manager CLI

USO:
  lemonade login                 Iniciar sesión con Google (abre el browser)
  lemonade logout                Borrar tokens locales
  lemonade status                Estado de la sesión y del cache
  lemonade list [--json] [--refresh]
                                 Listar entradas (metadata). --refresh fuerza red
  lemonade copy <id> [--field password|username] [--clear <segs>]
                                 Copiar al clipboard (auto-clear, default 30s)
  lemonade totp <id>             Copiar el código TOTP vigente
  lemonade type <id> [--full] [--delay <ms>]
                                 Tipear la contraseña en la ventana enfocada.
                                 --full tipea usuario<Tab>contraseña";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");

    let result = match cmd {
        "login" => auth::login(),
        "logout" => auth::logout(),
        "status" => cmd_status(),
        "list" => cmd_list(&args[1..]),
        "copy" => cmd_copy(&args[1..]),
        "totp" => cmd_totp(&args[1..]),
        "type" => cmd_type(&args[1..]),
        "help" | "--help" | "-h" | "" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("comando desconocido: {other}\n\n{USAGE}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lemonade: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_status() -> Result<(), String> {
    let cfg = config::Config::load()?;
    match auth::TokenStore::load() {
        Some(ts) => {
            println!("Sesión: {} (uid {})", ts.email, ts.uid);
            match auth::get_id_token(&cfg) {
                Ok(_) => println!("Token: válido / renovable"),
                Err(e) => println!("Token: PROBLEMA — {e}"),
            }
        }
        None => println!("Sin sesión. Corré: lemonade login"),
    }
    match api::cache_age_secs() {
        Some(age) => println!("Cache de listado: {age}s de antigüedad"),
        None => println!("Cache de listado: vacío"),
    }
    Ok(())
}

fn cmd_list(args: &[String]) -> Result<(), String> {
    let json = args.iter().any(|a| a == "--json");
    let refresh = args.iter().any(|a| a == "--refresh");

    let cfg = config::Config::load()?;
    let entries = api::list_entries(&cfg, refresh)?;

    // Un solo write con EPIPE ignorado: `lemonade list | head` no debe paniquear.
    let mut out = String::new();
    if json {
        let items: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "title": e.title,
                    "username": e.username,
                    "url": e.url,
                    "has_totp": e.has_totp,
                })
            })
            .collect();
        out = serde_json::Value::Array(items).to_string();
        out.push('\n');
    } else {
        for e in &entries {
            let totp = if e.has_totp { " [TOTP]" } else { "" };
            out.push_str(&format!("{}\t{}\t{}{}\n", e.id, e.title, e.username, totp));
        }
    }
    use std::io::Write;
    let _ = std::io::stdout().write_all(out.as_bytes());
    Ok(())
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn require_id<'a>(args: &'a [String], cmd: &str) -> Result<&'a str, String> {
    args.iter()
        .find(|a| !a.starts_with("--"))
        .map(String::as_str)
        .ok_or_else(|| format!("falta el id de la entrada: lemonade {cmd} <id>"))
}

fn cmd_copy(args: &[String]) -> Result<(), String> {
    let id = require_id(args, "copy")?;
    let field = flag_value(args, "--field").unwrap_or("password");
    let clear: u64 = flag_value(args, "--clear")
        .map(|v| v.parse().map_err(|_| "--clear debe ser un número de segundos"))
        .transpose()?
        .unwrap_or(30);

    let cfg = config::Config::load()?;
    let entry = api::get_entry(&cfg, id)?;

    let value = match field {
        "password" => entry.password,
        "username" => entry.username,
        other => return Err(format!("--field inválido: {other} (password|username)")),
    };
    if value.is_empty() {
        return Err(format!("la entrada no tiene {field}"));
    }

    // El username no es secreto: copiar sin auto-clear.
    let clear = if field == "password" { Some(clear) } else { None };
    clip::copy(&value, clear)?;

    match clear {
        Some(s) => eprintln!("Contraseña copiada. El clipboard se limpia en {s}s."),
        None => eprintln!("Usuario copiado."),
    }
    Ok(())
}

fn cmd_totp(args: &[String]) -> Result<(), String> {
    let id = require_id(args, "totp")?;
    let cfg = config::Config::load()?;
    let totp = api::get_totp(&cfg, id)?;
    clip::copy(&totp.code, Some(totp.time_remaining.max(5)))?;
    eprintln!(
        "TOTP {} copiado (vence en {}s).",
        totp.code, totp.time_remaining
    );
    Ok(())
}

fn cmd_type(args: &[String]) -> Result<(), String> {
    let id = require_id(args, "type")?;
    let full = args.iter().any(|a| a == "--full");
    let delay_ms: u64 = flag_value(args, "--delay")
        .map(|v| v.parse().map_err(|_| "--delay debe ser milisegundos"))
        .transpose()?
        .unwrap_or(0);

    let cfg = config::Config::load()?;
    let entry = api::get_entry(&cfg, id)?;
    if entry.password.is_empty() {
        return Err("la entrada no tiene contraseña".into());
    }

    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }

    if full {
        if entry.username.is_empty() {
            return Err("la entrada no tiene usuario para --full".into());
        }
        clip::type_text(&entry.username)?;
        clip::type_key("Tab")?;
        clip::type_text(&entry.password)?;
    } else {
        clip::type_text(&entry.password)?;
    }
    Ok(())
}
