# WoW Chat Translator

Traduce el chat de World of Warcraft en tiempo real en una ventana aparte, y te ayuda a
responder en el idioma del otro jugador. Es una app nativa de Windows más un addon pequeño
para WoW.

![Captura de WoW Chat Translator](docs/screenshot.png)
<!-- TODO: añadir docs/screenshot.png -->

## Requisitos

- Windows 10 u 11 (64 bits).
- WoW en modo de pantalla **"Ventana (pantalla completa)"** (pantalla completa en ventana).
  En "Pantalla completa" exclusiva la app no puede leer la tira de píxeles.
- El addon **no** se instala a mano: la app lo instala y actualiza desde
  *Ajustes → General → Addon*, en las versiones de WoW que marques (Retail, Classic, etc.).

## Instalación

1. Descarga `WoW-Chat-Translator-Setup-0.2.0.exe` de la página de *Releases*.
2. Ejecútalo. No pide permisos de administrador: se instala solo para tu usuario en
   `%LOCALAPPDATA%\Programs\WoW Chat Translator`.
3. Al terminar, deja marcado **"Abrir WoW Chat Translator"**. La primera vez la app detecta
   tus instalaciones de WoW y te ofrece instalar el addon.
4. Entra al juego (o escribe `/reload` si ya estabas dentro).

**Aviso de SmartScreen:** el instalador no está firmado, así que Windows puede mostrar
"Windows protegió su PC". Pulsa **"Más información" → "Ejecutar de todos modos"**.

## Cómo funciona la tira

1. El addon dibuja una tira fina de píxeles de colores en una esquina de la pantalla de WoW.
2. La app lee esos píxeles desde la imagen de la ventana y los convierte de nuevo en mensajes de chat.
3. La app traduce cada mensaje y lo muestra en su propia ventana, sobre el juego.

## Traductores

- **Google** (por defecto): gratis, sin configuración.
- **OpenRouter**, **LM Studio** (LLM local) y **Anthropic**: necesitas tu propia clave
  (o tu propio servidor de LM Studio). La clave se guarda cifrada con tu usuario de Windows.
- **Claude Code**: necesita Claude Code instalado y con sesión iniciada; usa los límites de tu plan.

## Seguridad

- No lee la memoria del juego, no automatiza teclado ni ratón y no inyecta nada en WoW.
- Solo mira píxeles de la ventana, igual que una captura de pantalla.
- Las respuestas se copian al portapapeles: tú las pegas y las envías a mano
  (Enter, Ctrl+V, Enter).
- Blizzard no se ha pronunciado sobre este tipo de herramientas. **Úsala bajo tu propio riesgo.**

## Desinstalar

*Configuración de Windows → Aplicaciones → WoW Chat Translator → Desinstalar*. El desinstalador
cierra la app si está abierta, quita "Iniciar con Windows" si apunta a esta instalación y te
pregunta si quieres borrar tu configuración (`%APPDATA%\wow-chat-translator-native`); por
defecto se conserva. El addon de las carpetas de WoW **no** se borra: elimínalo de
`Interface\AddOns\WoWChatTranslator` si ya no lo quieres.

## Compilar desde el código

Requisitos: Rust (toolchain MSVC) e [Inno Setup 6](https://jrsoftware.org/isdl.php).

```powershell
cd native
cargo test
cargo build --release          # native\target\release\wow-chat-translator.exe
cd ..
powershell -ExecutionPolicy Bypass -File installer\build.ps1   # dist\WoW-Chat-Translator-Setup-0.2.0.exe
```

`installer\build.ps1` compila en release y luego ejecuta `ISCC`. Usa `-SkipCargo` para empaquetar
un exe ya compilado.

## Créditos

- Autor: Jorge Luis Bustamante Jara. Licencia MIT (ver `LICENSE`).
- Emojis: Twemoji graphics by Twitter, Inc and other contributors, licensed under CC-BY 4.0.
- Diseño inspirado en [Parley](https://github.com/Duhlude/parley) (MIT) y
  [LibSerpix](https://github.com/alex-berliner/LibSerpix) (MIT).
- Resto de licencias de terceros: `THIRD_PARTY_NOTICES.md`.

---

## English

**WoW Chat Translator** translates World of Warcraft chat live in a separate window and helps
you reply in the other player's language. It is a native Windows app plus a small addon.

- **Requirements:** Windows 10/11 (64-bit), WoW in *Windowed (Fullscreen)* mode. The app installs
  and updates the addon for you (*Ajustes → General → Addon*).
- **Install:** run `WoW-Chat-Translator-Setup-0.2.0.exe` (per-user, no admin). If SmartScreen
  warns, click *More info → Run anyway* (the installer is unsigned).
- **How it works:** the addon draws a thin pixel strip; the app reads those pixels from the game
  window, decodes the chat messages and shows the translations.
- **Translators:** Google (default, free); OpenRouter, LM Studio and Anthropic need your own key;
  Claude Code needs Claude Code installed and uses your plan's limits.
- **Safety:** no memory reading, no input automation, no injection. Replies go to the clipboard
  and you paste them yourself. Blizzard has not stated a position on this kind of tool; use it at
  your own risk.
- **Uninstall:** Windows Settings → Apps. Your settings are kept unless you choose to delete
  them; the addon in your WoW folders is not removed.
- **License:** MIT. Third-party notices in `THIRD_PARTY_NOTICES.md`.
