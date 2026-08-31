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

/// Contraseña aleatoria **compatible con el generador de la web app de
/// Lemonade**: mismo charset (`!@#$%^&*()_+-=?.` + alfanumérico), garantiza
/// al menos una minúscula, una mayúscula, un número y un símbolo, y mezcla.
/// A diferencia de la web usa rejection sampling (sin sesgo de módulo) y
/// entropía fresca para la mezcla.
pub fn generate_password(len: usize) -> Result<String, String> {
    const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const DIGIT: &[u8] = b"0123456789";
    const SYMBOL: &[u8] = b"!@#$%^&*()_+-=?.";
    const ALL: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()_+-=?.";

    fn pick(set: &[u8]) -> Result<u8, String> {
        let n = set.len();
        let limit = (256 / n) * n;
        let mut b = [0u8; 1];
        loop {
            getrandom::getrandom(&mut b).map_err(|e| format!("sin entropía: {e}"))?;
            if (b[0] as usize) < limit {
                return Ok(set[b[0] as usize % n]);
            }
        }
    }

    let mut out: Vec<u8> = Vec::with_capacity(len);
    // Una de cada clase, como la web app (largo mínimo 4 ya validado).
    for set in [LOWER, UPPER, DIGIT, SYMBOL] {
        if out.len() < len {
            out.push(pick(set)?);
        }
    }
    while out.len() < len {
        out.push(pick(ALL)?);
    }
    // Fisher-Yates para que las clases garantizadas no queden al inicio.
    for i in (1..out.len()).rev() {
        let j = {
            let n = i + 1;
            let limit = (256 / n) * n;
            let mut b = [0u8; 1];
            loop {
                getrandom::getrandom(&mut b).map_err(|e| format!("sin entropía: {e}"))?;
                if (b[0] as usize) < limit {
                    break b[0] as usize % n;
                }
            }
        };
        out.swap(i, j);
    }
    Ok(String::from_utf8(out).unwrap())
}

/// ¿Hay terminal interactiva disponible?
pub fn has_tty() -> bool {
    std::fs::File::open("/dev/tty").is_ok()
}

/// Lee líneas de /dev/tty hasta una línea que sea solo ".".
pub fn read_multiline() -> Result<String, String> {
    use std::io::BufRead;
    let f = std::fs::File::open("/dev/tty")
        .map_err(|_| "no hay terminal interactiva".to_string())?;
    let mut out = String::new();
    for line in std::io::BufReader::new(f).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim() == "." {
            break;
        }
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}
