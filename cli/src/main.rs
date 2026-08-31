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
mod input;
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
  lemonade copy <id> [--field password|username|url] [--clear <segs>]
                                 Copiar al clipboard (auto-clear, default 30s)
  lemonade totp <id>             Copiar el código TOTP vigente
  lemonade type <id> [--full] [--delay <ms>]
                                 Tipear la contraseña en la ventana enfocada.
                                 --full tipea usuario<Tab>contraseña
  lemonade add [--title T] [--username U] [--url URL] [--notes N]
               [--generate [largo]]
                                 Crear una entrada. Pregunta lo que falte;
                                 --generate crea la contraseña y la copia
  lemonade edit <id> [--title T] [--username U] [--url URL] [--notes N]
                     [--password] [--generate [largo]]
                                 Editar. Sin flags es interactivo
                                 (Enter conserva el valor actual)
  lemonade rm <id> [--yes]       Mandar a la papelera (recuperable en la web)
  lemonade generate [largo]      Generar contraseña y copiarla (default 20)";

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
        "add" => cmd_add(&args[1..]),
        "edit" => cmd_edit(&args[1..]),
        "rm" | "delete" => cmd_rm(&args[1..]),
        "generate" | "gen" => cmd_generate(&args[1..]),
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

    // username y url son metadata: salen del cache, sin viaje al server.
    let value = match field {
        "password" => api::get_entry(&cfg, id)?.password,
        "username" | "url" => api::cached_field(&cfg, id, field)?,
        other => return Err(format!("--field inválido: {other} (password|username|url)")),
    };
    if value.is_empty() {
        return Err(format!("la entrada no tiene {field}"));
    }

    // Solo la contraseña es secreta: el resto se copia sin auto-clear.
    let clear = if field == "password" { Some(clear) } else { None };
    clip::copy(&value, clear)?;

    match field {
        "password" => eprintln!("Contraseña copiada. El clipboard se limpia en {}s.", clear.unwrap()),
        "username" => eprintln!("Usuario copiado."),
        _ => eprintln!("URL copiada."),
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

/// --generate opcionalmente con largo: "--generate" o "--generate 32".
fn generate_flag(args: &[String]) -> Result<Option<usize>, String> {
    let Some(i) = args.iter().position(|a| a == "--generate") else {
        return Ok(None);
    };
    let len = match args.get(i + 1) {
        Some(v) if !v.starts_with("--") => v
            .parse()
            .map_err(|_| "el largo de --generate debe ser un número".to_string())?,
        _ => 20,
    };
    if !(4..=500).contains(&len) {
        return Err("el largo debe estar entre 4 y 500".into());
    }
    Ok(Some(len))
}

fn cmd_add(args: &[String]) -> Result<(), String> {
    let cfg = config::Config::load()?;

    let interactive = input::has_tty();
    let title = match flag_value(args, "--title") {
        Some(t) => t.to_string(),
        None if interactive => input::prompt("Título", None)?,
        None => return Err("falta --title (no hay terminal para preguntar)".into()),
    };
    if title.is_empty() {
        return Err("el título es obligatorio".into());
    }
    // Los opcionales solo se preguntan si hay terminal; sin ella quedan vacíos.
    let username = match flag_value(args, "--username") {
        Some(u) => u.to_string(),
        None if interactive => input::prompt("Usuario (opcional)", Some(""))?,
        None => String::new(),
    };
    let url = match flag_value(args, "--url") {
        Some(u) => u.to_string(),
        None if interactive => input::prompt("URL (opcional)", Some(""))?,
        None => String::new(),
    };
    let notes = flag_value(args, "--notes").unwrap_or("").to_string();

    let (password, generated) = match generate_flag(args)? {
        Some(len) => (input::generate_password(len)?, true),
        None => {
            let p1 = input::prompt_hidden("Contraseña")?;
            if p1.is_empty() {
                return Err("la contraseña es obligatoria (o usá --generate)".into());
            }
            let p2 = input::prompt_hidden("Repetir contraseña")?;
            if p1 != p2 {
                return Err("no coinciden".into());
            }
            (p1, false)
        }
    };

    let id = api::create_entry(
        &cfg,
        &api::NewEntry { title: title.clone(), username, password: password.clone(), url, notes },
    )?;

    if generated {
        clip::copy(&password, Some(30))?;
        eprintln!("Creada \"{title}\" ({id}). Contraseña generada y copiada — se limpia en 30s.");
    } else {
        eprintln!("Creada \"{title}\" ({id}).");
    }
    Ok(())
}

fn cmd_edit(args: &[String]) -> Result<(), String> {
    let id = require_id(args, "edit")?;
    let cfg = config::Config::load()?;

    let mut fields = serde_json::Map::new();
    let has_flags = args.iter().any(|a| a.starts_with("--"));

    if has_flags {
        for (flag, key) in [("--title", "title"), ("--username", "username"), ("--url", "url"), ("--notes", "notes")] {
            if let Some(v) = flag_value(args, flag) {
                fields.insert(key.into(), serde_json::json!(v));
            }
        }
        if let Some(len) = generate_flag(args)? {
            let p = input::generate_password(len)?;
            clip::copy(&p, Some(30))?;
            eprintln!("Contraseña nueva generada y copiada — se limpia en 30s.");
            fields.insert("password".into(), serde_json::json!(p));
        } else if args.iter().any(|a| a == "--password") {
            let p1 = input::prompt_hidden("Contraseña nueva")?;
            let p2 = input::prompt_hidden("Repetir")?;
            if p1 != p2 {
                return Err("no coinciden".into());
            }
            if !p1.is_empty() {
                fields.insert("password".into(), serde_json::json!(p1));
            }
        }
    } else {
        // Interactivo: metadata actual como default; Enter conserva.
        let metas = api::list_entries(&cfg, false)?;
        let meta = metas.iter().find(|e| e.id == id).ok_or("id no encontrado en el listado")?;
        eprintln!("Editando \"{}\" — Enter conserva el valor actual.", meta.title);

        let title = input::prompt("Título", Some(&meta.title))?;
        if !title.is_empty() && title != meta.title {
            fields.insert("title".into(), serde_json::json!(title));
        }
        let username = input::prompt("Usuario", Some(&meta.username))?;
        if username != meta.username {
            fields.insert("username".into(), serde_json::json!(username));
        }
        let url = input::prompt("URL", Some(&meta.url))?;
        if url != meta.url {
            fields.insert("url".into(), serde_json::json!(url));
        }
        if input::confirm("¿Cambiar la contraseña?")? {
            if input::confirm("¿Generarla automáticamente?")? {
                let p = input::generate_password(20)?;
                clip::copy(&p, Some(30))?;
                eprintln!("Generada y copiada — se limpia en 30s.");
                fields.insert("password".into(), serde_json::json!(p));
            } else {
                let p1 = input::prompt_hidden("Contraseña nueva")?;
                let p2 = input::prompt_hidden("Repetir")?;
                if p1 != p2 {
                    return Err("no coinciden".into());
                }
                if !p1.is_empty() {
                    fields.insert("password".into(), serde_json::json!(p1));
                }
            }
        }
    }

    if fields.is_empty() {
        eprintln!("Sin cambios.");
        return Ok(());
    }
    let changed: Vec<&str> = fields.keys().map(String::as_str).collect();
    let resumen = changed.join(", ");
    api::update_entry(&cfg, id, fields)?;
    eprintln!("Actualizado: {resumen}.");
    Ok(())
}

fn cmd_rm(args: &[String]) -> Result<(), String> {
    let id = require_id(args, "rm")?;
    let cfg = config::Config::load()?;
    let yes = args.iter().any(|a| a == "--yes" || a == "-y");

    let title = api::cached_title(id).unwrap_or_else(|| id.to_string());
    if !yes && !input::confirm(&format!("¿Mandar \"{title}\" a la papelera?"))? {
        eprintln!("Cancelado.");
        return Ok(());
    }
    api::delete_entry(&cfg, id)?;
    eprintln!("\"{title}\" en la papelera (recuperable desde la web app; se purga a los 30 días).");
    Ok(())
}

fn cmd_generate(args: &[String]) -> Result<(), String> {
    let len: usize = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(|v| v.parse().map_err(|_| "el largo debe ser un número"))
        .transpose()?
        .unwrap_or(20);
    if !(4..=500).contains(&len) {
        return Err("el largo debe estar entre 4 y 500".into());
    }
    let p = input::generate_password(len)?;
    clip::copy(&p, Some(30))?;
    eprintln!("Contraseña de {len} caracteres copiada — se limpia en 30s.");
    Ok(())
}
