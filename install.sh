#!/bin/bash
# Instala el CLI `lemonade` y el plugin de barra de Omarchy.
# Idempotente: correrlo de nuevo actualiza los dos.
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_ID="io.github.oktubr3.lemonade"
PLUGIN_DIR="$HOME/.config/omarchy/plugins/$PLUGIN_ID"
BIN_DIR="$HOME/.local/bin"

say() { printf '\033[1;33m🍋 %s\033[0m\n' "$*"; }

# --- 1. CLI en Rust ---
say "Compilando el CLI (cargo build --release)…"
command -v cargo >/dev/null || { echo "Falta cargo (Rust). Instalalo con: mise use -g rust@latest" >&2; exit 1; }
(cd "$REPO_DIR/cli" && cargo build --release)

mkdir -p "$BIN_DIR"
install -m 755 "$REPO_DIR/cli/target/release/lemonade" "$BIN_DIR/lemonade"
say "CLI instalado en $BIN_DIR/lemonade"

# --- 2. Config ---
CONFIG="$HOME/.config/lemonade/config.json"
if [[ ! -f "$CONFIG" ]]; then
  say "Falta $CONFIG — el CLI te muestra el template con: lemonade status"
fi

# --- 3. Plugin de barra ---
if [[ -d "$PLUGIN_DIR/.git" ]]; then
  say "Actualizando plugin instalado…"
  git -C "$PLUGIN_DIR" pull --ff-only
else
  say "Instalando plugin en $PLUGIN_DIR…"
  git clone "$REPO_DIR" "$PLUGIN_DIR"
fi

omarchy plugin validate "$PLUGIN_DIR"
omarchy plugin enable "$PLUGIN_ID" 2>/dev/null || true

say "Listo. Reiniciá el shell para ver el ícono: omarchy restart shell"
say "Primer uso: lemonade login"
