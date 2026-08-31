// Entrada interactiva por /dev/tty (funciona aunque stdin venga de un pipe)
// y generador de contraseñas sin sesgo de módulo.

use std::io::{BufRead, BufReader, Write};
use std::process::Command;

fn tty_reader() -> Result<BufReader<std::fs::File>, String> {
    std::fs::File::open("/dev/tty")
        .map(BufReader::new)
        .map_err(|_| "no hay terminal interactiva — pasá los datos por flags".to_string())
}

/// Pregunta visible. `default` se muestra y se usa si el usuario da Enter.
pub fn prompt(label: &str, default: Option<&str>) -> Result<String, String> {
    let mut reader = tty_reader()?;
    match default {
        Some(d) if !d.is_empty() => eprint!("{label} [{d}]: "),
        _ => eprint!("{label}: "),
    }
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let line = line.trim_end_matches('\n').trim().to_string();
    if line.is_empty() {
        return Ok(default.unwrap_or("").to_string());
    }
    Ok(line)
}

/// Pregunta con eco apagado (stty -echo sobre /dev/tty).
pub fn prompt_hidden(label: &str) -> Result<String, String> {
    let tty_in = || std::fs::File::open("/dev/tty").map_err(|e| e.to_string());
    let mut reader = tty_reader()?;

    eprint!("{label}: ");
    let _ = std::io::stderr().flush();

    let off = Command::new("stty")
        .arg("-echo")
        .stdin(tty_in()?)
        .status()
        .map_err(|e| format!("stty: {e}"))?;
    if !off.success() {
        return Err("no pude apagar el eco de la terminal".into());
    }

    let mut line = String::new();
    let read = reader.read_line(&mut line);

    // Reencender el eco SIEMPRE, incluso si la lectura falló.
    let _ = Command::new("stty").arg("echo").stdin(tty_in()?).status();
    eprintln!();

    read.map_err(|e| e.to_string())?;
    Ok(line.trim_end_matches('\n').to_string())
}

/// Confirmación y/N.
pub fn confirm(label: &str) -> Result<bool, String> {
    let answer = prompt(&format!("{label} (y/N)"), Some("n"))?;
    Ok(matches!(answer.to_lowercase().as_str(), "y" | "yes" | "s" | "si" | "sí"))
}

/// Contraseña aleatoria: letras, números y símbolos, sin sesgo
/// (rejection sampling sobre getrandom).
pub fn generate_password(len: usize) -> Result<String, String> {
    const CHARSET: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*()-_=+[]{}:,.?";
    let n = CHARSET.len(); // 85
    let limit = (256 / n) * n; // 255: rechazo por encima para no sesgar
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    while out.len() < len {
        getrandom::getrandom(&mut buf).map_err(|e| format!("sin entropía: {e}"))?;
        for &b in buf.iter() {
            if (b as usize) < limit && out.len() < len {
                out.push(CHARSET[b as usize % n] as char);
            }
        }
    }
    Ok(out)
}
