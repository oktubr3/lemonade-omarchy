// Clipboard (wl-copy) y tipeo (wtype), siempre por stdin/env — nunca argv,
// porque /proc/*/cmdline es legible por cualquier proceso del usuario.

use std::io::Write;
use std::process::{Command, Stdio};

fn pipe_to(cmd: &str, args: &[&str], input: &str) -> Result<(), String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("¿está instalado {cmd}? — {e}"))?;
    child
        .stdin
        .take()
        .ok_or("sin stdin")?
        .write_all(input.as_bytes())
        .map_err(|e| format!("{cmd}: {e}"))?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("{cmd} salió con {status}"));
    }
    Ok(())
}

/// Copia al clipboard. Con `clear_secs`, agenda un job desatendido que lo
/// limpia solo si el clipboard todavía contiene este valor (si el usuario
/// copió otra cosa en el medio, no se la pisamos).
pub fn copy(text: &str, clear_secs: Option<u64>) -> Result<(), String> {
    pipe_to("wl-copy", &[], text)?;

    if let Some(secs) = clear_secs {
        // El valor viaja por env (0400 del dueño), no por argv.
        Command::new("sh")
            .args([
                "-c",
                r#"sleep "$LEMONADE_CLEAR_AFTER"; [ "$(wl-paste --no-newline 2>/dev/null)" = "$LEMONADE_CLIP" ] && wl-copy --clear"#,
            ])
            .env("LEMONADE_CLIP", text)
            .env("LEMONADE_CLEAR_AFTER", secs.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("agendando auto-clear: {e}"))?;
    }
    Ok(())
}

/// Tipea texto en la ventana enfocada (wtype lee de stdin con "-").
pub fn type_text(text: &str) -> Result<(), String> {
    pipe_to("wtype", &["-"], text)
}

/// Presiona una tecla especial (ej. Tab).
pub fn type_key(key: &str) -> Result<(), String> {
    let status = Command::new("wtype")
        .args(["-k", key])
        .status()
        .map_err(|e| format!("¿está instalado wtype? — {e}"))?;
    if !status.success() {
        return Err(format!("wtype -k {key} falló"));
    }
    Ok(())
}
