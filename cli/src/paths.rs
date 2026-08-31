// Rutas XDG. Sin dependencias: HOME + defaults del estándar.

use std::path::PathBuf;

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME no definido"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var(var)
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join(fallback))
        .join("lemonade")
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}

pub fn cache_dir() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache")
}

/// Escribe un archivo con permisos 0600, creando el directorio (0700) si falta.
pub fn write_private(path: &std::path::Path, contents: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creando {}: {e}", dir.display()))?;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("abriendo {}: {e}", path.display()))?;
    // Por si el archivo ya existía con otros permisos.
    let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
    f.write_all(contents.as_bytes())
        .map_err(|e| format!("escribiendo {}: {e}", path.display()))
}
