# 🍋 Lemonade para Omarchy

Plugin de barra de [Omarchy](https://omarchy.org/) (Quattro+) para
[Lemonade Password Manager](https://github.com/oktubr3/lemonade): buscá una
credencial desde la barra y copiá la contraseña, el usuario o el código TOTP
al clipboard — o tipeala directo en la ventana enfocada.

Dos piezas:

| Pieza | Qué es |
|---|---|
| `cli/` | **`lemonade`**, un CLI nativo en Rust que habla con tu backend de Lemonade (Firebase). Binario de ~2 MB, arranca en milisegundos. |
| `BarWidget.qml` + `Panel.qml` | El plugin Quickshell de la barra. Solo dibuja: toda la lógica vive en el CLI. |

## Uso

Clic en el 󰌾 de la barra (o IPC: `omarchy-shell ipc call io.github.oktubr3.lemonade toggle`):

```
escribir   filtra
↑/↓        navega
Enter      copia la contraseña (el clipboard se limpia solo a los 30s)
Ctrl+U     copia el usuario (también Ctrl+↵)
Ctrl+L     copia la URL
Alt+↵      copia el código TOTP vigente
Shift+↵    cierra el panel y tipea la contraseña en la ventana enfocada
Ctrl+E     edita la entrada en terminal flotante (también clic derecho)
Ctrl+⌫     borra — pide confirmación inline (Enter confirma, Esc cancela)
Esc        cierra (o cancela la confirmación pendiente)
```

El CLI también se usa solo:

```bash
lemonade login          # OAuth de Google en el browser, una sola vez
lemonade list           # listado (metadata, cacheado)
lemonade copy <id>      # contraseña → clipboard, auto-clear 30s
                        #   --field username|url copia metadata (sin auto-clear)
lemonade totp <id>      # código TOTP → clipboard
lemonade type <id>      # tipea en la ventana enfocada (--full: usuario⇥contraseña)
lemonade add            # crear (interactivo; --generate crea y copia la contraseña)
lemonade edit <id>      # editar (interactivo, Enter conserva; o por flags)
lemonade rm <id>        # a la papelera de Lemonade (recuperable; se purga a los 30 días)
lemonade generate 24    # generar contraseña y copiarla, sin crear entrada
```

El **+** del panel abre el alta interactiva en una terminal flotante — la
contraseña se ingresa ahí con el eco apagado; el panel jamás la ve.

## Instalación

Requisitos: Rust (`cargo`), `wl-clipboard`, `wtype`, y una cuenta en un
deployment de Lemonade (hosted o self-hosted).

```bash
git clone https://github.com/oktubr3/lemonade-omarchy.git
cd lemonade-omarchy
./install.sh
omarchy restart shell
```

### Config

El CLI lee `~/.config/lemonade/config.json` (nunca va a ningún repo):

```json
{
  "api_key": "<Firebase Web API key de tu deployment>",
  "project_id": "<Firebase project id>",
  "functions_url": "https://us-central1-<project_id>.cloudfunctions.net",
  "oauth_client_id": "<id>.apps.googleusercontent.com",
  "oauth_client_secret": "<secret del cliente OAuth Desktop>"
}
```

Los primeros tres valores son los mismos que usa la web app (son públicos por
diseño). Los últimos dos salen de crear un **OAuth Client ID tipo "Desktop
app"** en Google Cloud Console → APIs & Services → Credentials, en el mismo
proyecto GCP de tu Firebase. El "secret" de un cliente Desktop no es
confidencial (RFC 8252), pero igual vive en tu config local.

Después: `lemonade login` abre el browser, elegís tu cuenta de Google y listo.

## Seguridad

- **El panel QML nunca ve una contraseña.** El CLI la recibe del backend por
  TLS y la entrega a `wl-copy`/`wtype` **por stdin** — nunca por argv, que es
  legible en `/proc/*/cmdline`.
- **Auto-clear inteligente**: a los 30s se limpia el clipboard solo si todavía
  contiene la contraseña (si copiaste otra cosa en el medio, no se pisa).
- **Tokens** con permisos `0600` en `~/.local/state/lemonade/` — igual
  categoría de riesgo que la sesión persistida de la web app en el browser.
- **Cache** de listado metadata-only (título/usuario/url, `0600`): jamás
  material cifrado ni descifrado en disco.
- El vault principal de Lemonade se descifra **en el server** (mismo modelo
  que la web app); este cliente no maneja claves de cifrado. El Env Vault
  (zero-knowledge) no está soportado todavía.
- Login con OAuth **loopback + PKCE** (RFC 8252), con `state` verificado.

## Licencia

AGPL-3.0, igual que Lemonade.
