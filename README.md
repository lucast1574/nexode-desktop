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
`6962272d6f52af63dc90673cbf38a1480c6d293f`. No se necesita Node ni npm.
En Windows también se necesitan las herramientas C++ de Visual Studio.

En Ubuntu/Debian, instala las dependencias nativas:

```sh
sudo apt-get install libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libxcursor-dev
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
seleccionar la configuración de origen. La interfaz no guarda claves.

Las preferencias y los proyectos se conservan en el directorio de datos local
del usuario, dentro de `nexode-desktop`; la instancia del motor utiliza su
subcarpeta `engine`. `ORBIT_DESKTOP_HOME` permite cambiar esa ubicación.
`ORBIT_PROJECTS` define la carpeta inicial donde buscar repositorios.

La ventana posee su instancia privada. Al cerrarla, termina esa instancia sin
modificar el servidor de la terminal. En Windows un Job Object limita también
la vida de los procesos descendientes al proceso de la aplicación.

## Funciones

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
