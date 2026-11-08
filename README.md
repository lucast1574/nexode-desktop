# Nexode Orbit — Desktop nativo

Aplicación de escritorio escrita en Rust con **egui/eframe y renderizado por GPU**.
No usa HTML, React, Electron, WebView ni un servidor HTTP local. El motor se
controla directamente mediante el SDK Rust de Orbit.

La organización está inspirada en Codex: proyectos y chats a la izquierda,
conversación central, compositor y panel de cambios de Git. La identidad y los
activos visuales son de Nexode.

## Abrir y compilar

Windows: ejecuta `nexode-desktop.exe`. Para compilar desde este checkout:

```powershell
./start.ps1
# Solo compilar:
./start.ps1 -BuildOnly
```

Linux y macOS:

```sh
bash scripts/start.sh
```

Requisitos para compilar: Rust estable, Git y un checkout hermano
`../nexode-orbit`. Esta versión usa el SDK del commit
`dc71ff1c41068b6eaf4ac3c1f7ca06c091203784`. Compilar la UI no necesita Node ni npm.
Los paquetes incluyen Node y el MCP oficial. Si ejecutas sólo el binario de
desarrollo, el conector MCP necesita Node.js 20+ y npx en PATH.
En Windows también se necesitan las herramientas C++ de Visual Studio.

En Ubuntu/Debian, instala las dependencias nativas:

```sh
sudo apt-get install libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libxcursor-dev libgtk-3-dev libayatana-appindicator3-dev libdbus-1-dev
```

En macOS, instala Xcode Command Line Tools. Para crear el paquete `.app`:

```sh
bash scripts/bundle-macos.sh
```

El paquete generado no está firmado ni notarizado. El flujo de GitHub Actions
compila y prueba en runners Windows, Linux y macOS; su presencia no implica que
ya se haya ejecutado remotamente.

## Motor y credenciales

La UI busca `nexode`/`nexode.exe` junto a su ejecutable, en la instalación local
y en PATH. También puedes seleccionarlo desde **Configuración**, o definir
`NEXODE_BINARY` antes del arranque. El ejecutable del motor debe corresponder al
sistema operativo; un binario Linux no funciona como motor nativo de Windows.

Orbit debe tener configurado un proveedor para responder. Se heredan las
credenciales del usuario del sistema que ejecuta la app. `NEXODE_HOME` permite
seleccionar la configuración de origen. Desde Configuración puedes conectar
proveedores por OAuth o API key. La cuenta Nexode conserva su conector personal
en el almacén seguro del sistema; las claves de IA se entregan al almacén privado
del motor por IPC. No se guardan en preferencias ni en el chat.

Las preferencias y los proyectos se conservan en el directorio de datos local
del usuario, dentro de `nexode-desktop`; la instancia del motor utiliza su
subcarpeta `engine`. `ORBIT_DESKTOP_HOME` permite cambiar esa ubicación.
`ORBIT_PROJECTS` define la carpeta inicial donde buscar repositorios.

La aplicación posee su instancia privada. Salir desde la bandeja termina esa
instancia sin modificar el servidor de la terminal. Cerrar la ventana la oculta
cuando hay bandeja disponible. En Windows un Job Object limita también
la vida de los procesos descendientes al proceso de la aplicación.

## Funciones

- Mascota Nexo animada nativamente, cuenta Nexode y catálogo de servicios.
- Harness sin suscripción Nexode obligatoria; checkout explícito para servicios cloud.
- Conexiones Claude, GPT/Codex, DeepSeek, Gemini y Antigravity.
- Cuotas reales por cuenta en círculos, con detalle y reinicio al pasar el cursor.
- Bandeja e inicio de sesión del sistema en Windows, Linux y macOS.
- Proyectos con selector de carpetas del sistema y preferencias persistentes.
- Chats, historial, Markdown, código, selección y copia de texto.
- Streaming con correcciones por ID de mensaje, actividad de herramientas y cancelación.
- Autorización explícita de las solicitudes de permiso, sin autoaprobarlas.
- Catálogo de modelos del motor y actualización explícita de proveedores.
- Revisión de Git, incluyendo repositorios nuevos sin primer commit.
- `Ctrl+N` / `Cmd+N`: nuevo chat; Enter: enviar; Shift+Enter: nueva línea.
- Interfaz y operaciones del motor en hilos separados. Redibujado por eventos.

Esta entrega no incluye una terminal PTY integrada, actualizaciones automáticas
ni un instalador firmado. El parecido con Codex es de organización y flujo de
trabajo; no reproduce todas sus funciones.

Consulta [HARNESS.md](HARNESS.md) para cuentas, seguridad y límites de las
integraciones; [APPIMAGE.md](APPIMAGE.md) para distribución Linux y requisitos.

## Validación

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
```

La prueba real del SDK está aislada y usa el proveedor sintético de Orbit:

```sh
NEXODE_BINARY=/ruta/al/nexode ORBIT_DEV_BIN=/ruta/al/orbit-dev \
  bash scripts/test-native-engine.sh
```

Comprueba streaming, recuperación del historial, cancelación de una petición
activa y cierre real de la conexión. No necesita saldo ni claves de un proveedor
externo. Los tests normales cubren correcciones del stream y estados de Git.

El modo de captura para pruebas requiere `cargo build --release --features
screenshot` y la variable `EFRAME_SCREENSHOT_TO=/ruta/captura.png`. Usa la
captura del renderer nativo; no abre un navegador.

## Bandeja e inicio de sesión del sistema

La bandeja nativa ofrece **Abrir Nexode Orbit** y **Salir de Nexode Orbit**.
Con la bandeja disponible, cerrar la ventana la oculta y conserva las tareas;
**Salir** termina el motor privado. Abrir de nuevo el ejecutable recupera la
ventana existente. Si la bandeja no se puede inicializar, la ventana permanece
disponible y cerrar termina la aplicación normalmente.

**Iniciar con el sistema** registra el ejecutable actual, por usuario, con
`--background`. Actívalo desde una ubicación estable del paquete. Compilar,
ejecutar tests o abrir la aplicación por primera vez no registra el inicio
automático ni modifica opciones del sistema.

- **Windows:** entrada `Nexode Orbit` en `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
  Aparece en Aplicaciones de inicio / Administrador de tareas. Si Windows la
  deshabilita, Orbit respeta esa decisión y no la reactiva silenciosamente.
- **Linux:** archivo `nexode-orbit.desktop` en `$XDG_CONFIG_HOME/autostart`
  (normalmente `~/.config/autostart`). Requiere un escritorio compatible con
  indicadores de bandeja; GNOME puede necesitar la extensión AppIndicator.
  En AppImage se registra el archivo indicado por `APPIMAGE`, no el ejecutable
  de su montaje temporal. Mover el archivo exige volver a registrar el inicio.
- **macOS:** agente del usuario `~/Library/LaunchAgents/app.nexode.orbit.desktop.plist`.
  Se inicia al próximo login; no es un servicio del sistema ni requiere sudo.
  La validación en un Mac real sigue siendo necesaria.

Dependencias adicionales de compilación en Ubuntu/Debian para bandeja y
credenciales del sistema:

```sh
sudo apt-get install libgtk-3-dev libayatana-appindicator3-dev libdbus-1-dev libxkbcommon-x11-dev
```

En Linux, el entorno de ejecución necesita GTK3, Ayatana AppIndicator3 y una
sesión D-Bus; guardar credenciales requiere además un servicio Secret Service
desbloqueado (por ejemplo GNOME Keyring). No se crea un almacén de claves en texto
plano si ese servicio no está disponible.
