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

El panel tiene las **tres pestañas de la web app** — 🔑 Passwords · 🔒 Env
Vault · 📝 Notas — con `Ctrl+1/2/3` (o `Tab`) para cambiar. Cada pestaña
tiene su búsqueda, sus atajos (los chips de abajo cambian solos) y su
contador. En Env Vault: `Enter` abre el proyecto, `Enter` en una variable
abre la terminal que pide la master password y copia el valor; `Ctrl+S`
exporta el `.env` del proyecto. En Notas: `Enter` copia el contenido,
`Ctrl+D` la muestra, `+` crea.

Clic en el 󰌾 de la barra (o IPC: `omarchy-shell ipc call io.github.oktubr3.lemonade toggle`):

```
escribir   filtra
↑/↓        navega
Enter      copia la contraseña (el clipboard se limpia solo a los 30s)
Ctrl+U     copia el usuario (también Ctrl+↵)
Ctrl+L     copia la URL
Ctrl+O     copia la contraseña Y abre la URL en el browser (el ↗ de la fila solo abre)
Alt+↵      copia el código TOTP vigente
Shift+↵    cierra el panel y tipea la contraseña en la ventana enfocada
Ctrl+S     copia la entrada formateada para compartir (WhatsApp, etc.)
Ctrl+F     marca/desmarca favorita (la estrella verde de la web app)
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
lemonade show <id>      # ficha completa: campos custom, notas, TOTP
lemonade history <id>   # historial de contraseñas (--copy N copia una vieja)
lemonade send <id> <email>       # compartir a otro usuario Lemonade
lemonade shares [accept|reject]  # compartidas pendientes
lemonade note list|show|copy|add|edit|rm   # notas seguras
lemonade trash [restore|purge]             # papelera
lemonade env projects|vars|copy|export     # Env Vault (zero-knowledge)
```

### Env Vault

El compartimento zero-knowledge funciona igual que en la web: la master
password se pide oculta en cada uso, la clave se deriva **localmente**
(PBKDF2-SHA256 600k → HKDF, réplica exacta del `cryptoWorker` de la web,
verificada bit a bit contra WebCrypto) y ni la password ni la clave tocan
disco jamás. Requiere vault con verifier v3 (los viejos migran solos al
desbloquearse una vez en la web).

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

## Estética

El panel usa la paleta **Lemon Noir** del modo oscuro de la web app
(`#FFD700` dorado, `#161B22` cards, `#28A745` verde de favoritas,
`#0D1117` fondo), con las favoritas primero y su borde verde, contador
de entradas, y ayuda como chips de teclas.

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
