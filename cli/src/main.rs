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
mod env;
mod input;
mod paths;

use std::process::ExitCode;

const USAGE: &str = "\
lemonade — Lemonade Password Manager CLI

SESIÓN
  login | logout | status

CONTRASEÑAS
  list [--json] [--refresh]      Listar (★ favoritas primero)
  copy <id> [--field password|username|url] [--custom <label>] [--clear <segs>]
  totp <id>                      Código TOTP → clipboard
  type <id> [--full] [--delay <ms>]   Tipear en la ventana enfocada
  show <id>                      Ficha completa (campos custom, notas, TOTP)
  open <id> [--copy]             Abrir la URL en el browser (--copy: antes
                                 copia la contraseña al clipboard)
  add [--title T] [--username U] [--url URL] [--generate [largo]]
  edit <id> [flags]              Sin flags es interactivo
  rm <id> [--yes]                A la papelera
  fav <id>                       Marcar/desmarcar favorita
  generate [largo]               Generar y copiar (formato de la web, default 16)
  history <id> [--copy <n>]      Historial de contraseñas de la entrada

COMPARTIR
  share <id>                     Copiar formateada para chat (auto-clear 45s)
  send <id> <email>              Compartir a otro usuario Lemonade
  shares                         Pendientes recibidos
  shares accept|reject <shareId>

NOTAS SEGURAS
  note list                      Listar
  note show <id>                 Ver contenido
  note copy <id>                 Contenido → clipboard (auto-clear 45s)
  note add [--title T]           Crear (contenido por editor/stdin)
  note edit <id>                 Editar título/contenido
  note rm <id> [--yes]           A la papelera de notas

PAPELERA
  trash                          Listar borradas
  trash restore <id>             Restaurar
  trash purge <id> [--yes]       Borrado PERMANENTE (irreversible)

ENV VAULT (zero-knowledge: pide la master password, nada sale de acá)
  env projects                   Listar proyectos
  env vars <proyecto>            Variables de un proyecto (nombres)
  env copy <proyecto> <VAR>      Valor de una variable → clipboard
  env export <proyecto>          Todas las variables formato .env → clipboard";

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
        "fav" | "star" => cmd_fav(&args[1..]),
        "share" => cmd_share(&args[1..]),
        "generate" | "gen" => cmd_generate(&args[1..]),
        "show" => cmd_show(&args[1..]),
        "open" => cmd_open(&args[1..]),
        "history" => cmd_history(&args[1..]),
        "send" => cmd_send(&args[1..]),
        "shares" => cmd_shares(&args[1..]),
        "note" | "notes" => cmd_note(&args[1..]),
        "trash" => cmd_trash(&args[1..]),
        "env" => cmd_env(&args[1..]),
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
                    "highlighted": e.highlighted,
                })
            })
            .collect();
        out = serde_json::Value::Array(items).to_string();
        out.push('\n');
    } else {
        for e in &entries {
            let totp = if e.has_totp { " [TOTP]" } else { "" };
            let star = if e.highlighted { "★ " } else { "" };
            out.push_str(&format!("{}\t{}{}\t{}{}\n", e.id, star, e.title, e.username, totp));
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

    // Custom field: por label (insensible a mayúsculas).
    if let Some(label) = flag_value(args, "--custom") {
        let entry = api::get_entry(&cfg, id)?;
        let (l, v, t) = entry
            .custom_fields
            .iter()
            .find(|(l, _, _)| l.eq_ignore_ascii_case(label))
            .ok_or_else(|| {
                let known: Vec<&str> = entry.custom_fields.iter().map(|(l, _, _)| l.as_str()).collect();
                format!("no hay campo \"{label}\" (hay: {})", known.join(", "))
            })?;
        let secret = t == "password" || t == "pin";
        clip::copy(v, if secret { Some(clear) } else { None })?;
        if secret {
            eprintln!("\"{l}\" copiado. El clipboard se limpia en {clear}s.");
        } else {
            eprintln!("\"{l}\" copiado.");
        }
        return Ok(());
    }

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
        .unwrap_or(16);
    if !(4..=500).contains(&len) {
        return Err("el largo debe estar entre 4 y 500".into());
    }
    let p = input::generate_password(len)?;
    clip::copy(&p, Some(30))?;
    eprintln!("Contraseña de {len} caracteres copiada — se limpia en 30s.");
    Ok(())
}

fn cmd_fav(args: &[String]) -> Result<(), String> {
    let id = require_id(args, "fav")?;
    let cfg = config::Config::load()?;
    let title = api::cached_title(id).unwrap_or_else(|| id.to_string());
    if api::toggle_highlight(&cfg, id)? {
        eprintln!("★ \"{title}\" marcada como favorita.");
    } else {
        eprintln!("\"{title}\" ya no es favorita.");
    }
    Ok(())
}

fn cmd_share(args: &[String]) -> Result<(), String> {
    let id = require_id(args, "share")?;
    let clear: u64 = flag_value(args, "--clear")
        .map(|v| v.parse().map_err(|_| "--clear debe ser segundos"))
        .transpose()?
        .unwrap_or(45);

    let cfg = config::Config::load()?;
    let e = api::get_entry(&cfg, id)?;

    // Formato pensado para pegar en un chat. Contiene la contraseña →
    // auto-clear obligatorio.
    let mut text = format!("🍋 {}\n", e.title);
    if !e.username.is_empty() {
        text.push_str(&format!("Usuario: {}\n", e.username));
    }
    text.push_str(&format!("Contraseña: {}\n", e.password));
    if !e.url.is_empty() {
        text.push_str(&format!("URL: {}\n", e.url));
    }
    clip::copy(&text, Some(clear))?;
    eprintln!(
        "Entrada \"{}\" copiada lista para compartir. El clipboard se limpia en {clear}s — pegala ya.",
        e.title
    );
    Ok(())
}

fn cmd_show(args: &[String]) -> Result<(), String> {
    let id = require_id(args, "show")?;
    let cfg = config::Config::load()?;
    let e = api::get_entry(&cfg, id)?;
    let meta = api::list_entries(&cfg, false)?;
    let has_totp = meta.iter().find(|m| m.id == id).map(|m| m.has_totp).unwrap_or(false);

    println!("🍋 {}", e.title);
    if !e.username.is_empty() {
        println!("Usuario:    {}", e.username);
    }
    println!("Contraseña: ●●●●●●●●  (lemonade copy {id})");
    if !e.url.is_empty() {
        println!("URL:        {}", e.url);
    }
    if has_totp {
        println!("TOTP:       sí  (lemonade totp {id})");
    }
    if !e.notes.is_empty() {
        println!("Notas:\n{}", e.notes);
    }
    if !e.custom_fields.is_empty() {
        println!("Campos custom:");
        for (label, value, ftype) in &e.custom_fields {
            if ftype == "password" || ftype == "pin" {
                println!("  {label} [{ftype}]: ●●●●  (lemonade copy {id} --custom \"{label}\")");
            } else {
                println!("  {label}: {value}");
            }
        }
    }
    Ok(())
}

fn cmd_history(args: &[String]) -> Result<(), String> {
    let id = require_id(args, "history")?;
    let cfg = config::Config::load()?;
    let history = api::password_history(&cfg, id)?;
    if history.is_empty() {
        eprintln!("Sin historial: la contraseña nunca cambió.");
        return Ok(());
    }

    if let Some(n) = flag_value(args, "--copy") {
        let n: usize = n.parse().map_err(|_| "--copy espera el número de la lista")?;
        let item = history
            .get(n.saturating_sub(1))
            .ok_or(format!("no hay entrada #{n} (hay {})", history.len()))?;
        clip::copy(&item.password, Some(30))?;
        eprintln!("Contraseña #{n} (de {}) copiada — se limpia en 30s.", item.changed_at);
        return Ok(());
    }

    for (i, h) in history.iter().enumerate() {
        println!("{}. {} — ●●●●●●●●  (--copy {})", i + 1, h.changed_at, i + 1);
    }
    Ok(())
}

fn cmd_send(args: &[String]) -> Result<(), String> {
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let (id, email) = match positional.as_slice() {
        [id, email] => (id.as_str(), email.as_str()),
        _ => return Err("uso: lemonade send <id> <email>".into()),
    };
    let cfg = config::Config::load()?;

    let user = api::find_user(&cfg, email)?
        .ok_or(format!("no hay ningún usuario Lemonade con el email {email} (búsqueda exacta)"))?;
    let title = api::cached_title(id).unwrap_or_else(|| id.to_string());

    let who = if user.display_name.is_empty() { &user.email } else { &user.display_name };
    if !args.iter().any(|a| a == "--yes")
        && !input::confirm(&format!("¿Compartir \"{title}\" con {who} <{}>?", user.email))?
    {
        eprintln!("Cancelado.");
        return Ok(());
    }
    api::share_to_user(&cfg, id, &user.user_id)?;
    eprintln!("\"{title}\" compartida con {who}. Le va a aparecer como pendiente para aceptar.");
    Ok(())
}

fn cmd_shares(args: &[String]) -> Result<(), String> {
    let cfg = config::Config::load()?;
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let pending = api::pending_shares(&cfg)?;
            if pending.is_empty() {
                eprintln!("No tenés compartidas pendientes.");
                return Ok(());
            }
            for s in &pending {
                println!("{}\t{}\tde {} <{}>", s.id, s.title, s.from_name, s.from_email);
            }
            eprintln!("\nAceptar: lemonade shares accept <id> · Rechazar: lemonade shares reject <id>");
            Ok(())
        }
        Some("accept") => {
            let id = require_id(&args[1..], "shares accept")?;
            api::accept_share(&cfg, id)?;
            eprintln!("Aceptada: ya está entre tus contraseñas.");
            Ok(())
        }
        Some("reject") => {
            let id = require_id(&args[1..], "shares reject")?;
            api::reject_share(&cfg, id)?;
            eprintln!("Rechazada.");
            Ok(())
        }
        Some(other) => Err(format!("subcomando desconocido: shares {other}")),
    }
}

fn cmd_note(args: &[String]) -> Result<(), String> {
    let cfg = config::Config::load()?;
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let notes = api::list_notes(&cfg)?;
            if args.iter().any(|a| a == "--json") {
                let items: Vec<serde_json::Value> = notes
                    .iter()
                    .map(|n| serde_json::json!({ "id": n.id, "title": n.title }))
                    .collect();
                println!("{}", serde_json::Value::Array(items));
            } else {
                for n in &notes {
                    println!("{}\t{}", n.id, n.title);
                }
            }
            Ok(())
        }
        Some("show") => {
            let id = require_id(&args[1..], "note show")?;
            let (title, content) = api::get_note(&cfg, id)?;
            println!("📝 {title}\n\n{content}");
            Ok(())
        }
        Some("copy") => {
            let id = require_id(&args[1..], "note copy")?;
            let (title, content) = api::get_note(&cfg, id)?;
            clip::copy(&content, Some(45))?;
            eprintln!("Nota \"{title}\" copiada — el clipboard se limpia en 45s.");
            Ok(())
        }
        Some("add") => {
            let rest = &args[1..];
            let title = match flag_value(rest, "--title") {
                Some(t) => t.to_string(),
                None => input::prompt("Título", None)?,
            };
            if title.is_empty() {
                return Err("el título es obligatorio".into());
            }
            eprintln!("Contenido (terminá con una línea que diga solo \".\"):");
            let content = input::read_multiline()?;
            if content.trim().is_empty() {
                return Err("la nota está vacía".into());
            }
            let id = api::create_note(&cfg, &title, &content)?;
            eprintln!("Nota \"{title}\" creada ({id}).");
            Ok(())
        }
        Some("edit") => {
            let id = require_id(&args[1..], "note edit")?;
            let (cur_title, cur_content) = api::get_note(&cfg, id)?;
            eprintln!("Editando \"{cur_title}\" — Enter conserva el título.");
            let title = input::prompt("Título", Some(&cur_title))?;
            eprintln!("Contenido actual:\n{cur_content}\n");
            let content = if input::confirm("¿Reemplazar el contenido?")? {
                eprintln!("Contenido nuevo (terminá con una línea \".\"):");
                Some(input::read_multiline()?)
            } else {
                None
            };
            let title_opt = if title != cur_title { Some(title.as_str()) } else { None };
            if title_opt.is_none() && content.is_none() {
                eprintln!("Sin cambios.");
                return Ok(());
            }
            api::update_note(&cfg, id, title_opt, content.as_deref())?;
            eprintln!("Nota actualizada.");
            Ok(())
        }
        Some("rm") => {
            let rest = &args[1..];
            let id = require_id(rest, "note rm")?;
            if !rest.iter().any(|a| a == "--yes") && !input::confirm("¿Mandar la nota a la papelera?")? {
                eprintln!("Cancelado.");
                return Ok(());
            }
            api::delete_note(&cfg, id)?;
            eprintln!("Nota en la papelera.");
            Ok(())
        }
        Some(other) => Err(format!("subcomando desconocido: note {other}")),
    }
}

fn cmd_trash(args: &[String]) -> Result<(), String> {
    let cfg = config::Config::load()?;
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let entries = api::list_trash(&cfg)?;
            if entries.is_empty() {
                eprintln!("Papelera vacía.");
                return Ok(());
            }
            for e in &entries {
                println!("{}\t{}\t{}", e.id, e.title, e.username);
            }
            eprintln!("\nRestaurar: lemonade trash restore <id> · Borrar definitivo: lemonade trash purge <id>");
            Ok(())
        }
        Some("restore") => {
            let id = require_id(&args[1..], "trash restore")?;
            api::restore_entry(&cfg, id)?;
            eprintln!("Restaurada: volvió a tus contraseñas.");
            Ok(())
        }
        Some("purge") => {
            let rest = &args[1..];
            let id = require_id(rest, "trash purge")?;
            if !rest.iter().any(|a| a == "--yes")
                && !input::confirm("⚠️  Borrado PERMANENTE e irreversible. ¿Seguro?")?
            {
                eprintln!("Cancelado.");
                return Ok(());
            }
            api::purge_entry(&cfg, id)?;
            eprintln!("Borrada permanentemente.");
            Ok(())
        }
        Some(other) => Err(format!("subcomando desconocido: trash {other}")),
    }
}

fn env_project_id(cfg: &config::Config, name_or_id: &str) -> Result<(String, String), String> {
    let uid = auth::TokenStore::load().ok_or("sin sesión")?.uid;
    let projects = env::firestore_query(cfg, "env_projects", &[("userId", &uid)])?;
    projects
        .iter()
        .find(|(id, f)| {
            id == name_or_id
                || f["name"]["stringValue"]
                    .as_str()
                    .map(|n| n.eq_ignore_ascii_case(name_or_id))
                    .unwrap_or(false)
        })
        .map(|(id, f)| {
            (
                id.clone(),
                f["name"]["stringValue"].as_str().unwrap_or(id).to_string(),
            )
        })
        .ok_or_else(|| format!("no hay proyecto \"{name_or_id}\" (mirá: lemonade env projects)"))
}

fn cmd_env(args: &[String]) -> Result<(), String> {
    let cfg = config::Config::load()?;
    let uid = auth::TokenStore::load().ok_or("sin sesión")?.uid;

    match args.first().map(String::as_str) {
        Some("projects") => {
            let projects = env::firestore_query(&cfg, "env_projects", &[("userId", &uid)])?;
            if projects.is_empty() {
                eprintln!("Sin proyectos en el Env Vault.");
                return Ok(());
            }
            if args.iter().any(|a| a == "--json") {
                let items: Vec<serde_json::Value> = projects
                    .iter()
                    .map(|(id, f)| {
                        serde_json::json!({
                            "id": id,
                            "name": f["name"]["stringValue"].as_str().unwrap_or("?"),
                        })
                    })
                    .collect();
                println!("{}", serde_json::Value::Array(items));
            } else {
                for (id, f) in &projects {
                    println!("{}\t{}", id, f["name"]["stringValue"].as_str().unwrap_or("?"));
                }
            }
            Ok(())
        }
        Some("vars") => {
            let project = require_id(&args[1..], "env vars")?;
            let (pid, pname) = env_project_id(&cfg, project)?;
            let vars = env::firestore_query(
                &cfg,
                "env_variables",
                &[("userId", &uid), ("projectId", &pid)],
            )?;
            if args.iter().any(|a| a == "--json") {
                let items: Vec<serde_json::Value> = vars
                    .iter()
                    .map(|(_, f)| {
                        serde_json::json!({ "name": f["name"]["stringValue"].as_str().unwrap_or("?") })
                    })
                    .collect();
                println!("{}", serde_json::Value::Array(items));
            } else {
                eprintln!("Proyecto {pname} — {} variables (solo nombres; el valor con env copy):", vars.len());
                for (_, f) in &vars {
                    println!("{}", f["name"]["stringValue"].as_str().unwrap_or("?"));
                }
            }
            Ok(())
        }
        Some("copy") => {
            let positional: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with("--")).collect();
            let (project, var) = match positional.as_slice() {
                [p, v] => (p.as_str(), v.as_str()),
                _ => return Err("uso: lemonade env copy <proyecto> <VAR>".into()),
            };
            let (pid, _) = env_project_id(&cfg, project)?;
            let vars = env::firestore_query(
                &cfg,
                "env_variables",
                &[("userId", &uid), ("projectId", &pid)],
            )?;
            let (_, fields) = vars
                .iter()
                .find(|(_, f)| f["name"]["stringValue"].as_str() == Some(var))
                .ok_or_else(|| format!("no hay variable {var} en ese proyecto"))?;

            // Recién acá se pide la master password: sin variable no hay unlock.
            let key = env::unlock(&cfg)?;
            let value = key.decrypt_blob(&env::map_blob(&fields["encryptedValue"]))?;
            clip::copy(&value, Some(30))?;
            eprintln!("{var} copiada — el clipboard se limpia en 30s.");
            Ok(())
        }
        Some("export") => {
            let project = require_id(&args[1..], "env export")?;
            let (pid, pname) = env_project_id(&cfg, project)?;
            let vars = env::firestore_query(
                &cfg,
                "env_variables",
                &[("userId", &uid), ("projectId", &pid)],
            )?;
            if vars.is_empty() {
                return Err("el proyecto no tiene variables".into());
            }
            let key = env::unlock(&cfg)?;
            let mut out = String::new();
            for (_, f) in &vars {
                let name = f["name"]["stringValue"].as_str().unwrap_or("?");
                let value = key.decrypt_blob(&env::map_blob(&f["encryptedValue"]))?;
                out.push_str(&format!("{name}={value}\n"));
            }
            clip::copy(&out, Some(60))?;
            eprintln!(
                "{} variables de {pname} copiadas formato .env — el clipboard se limpia en 60s.",
                vars.len()
            );
            Ok(())
        }
        _ => Err("uso: lemonade env projects | vars <p> | copy <p> <VAR> | export <p>".into()),
    }
}

fn cmd_open(args: &[String]) -> Result<(), String> {
    let id = require_id(args, "open")?;
    let cfg = config::Config::load()?;

    let mut url = api::cached_field(&cfg, id, "url")?;
    if url.is_empty() {
        return Err("la entrada no tiene URL".into());
    }
    if !url.contains("://") {
        url = format!("https://{url}");
    }

    if args.iter().any(|a| a == "--copy") {
        let entry = api::get_entry(&cfg, id)?;
        if !entry.password.is_empty() {
            clip::copy(&entry.password, Some(30))?;
            eprintln!("Contraseña copiada (se limpia en 30s).");
        }
    }

    // xdg-open respeta $BROWSER y el default del sistema. Desatendido.
    std::process::Command::new("xdg-open")
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("xdg-open: {e}"))?;
    eprintln!("Abriendo {url}");
    Ok(())
}
