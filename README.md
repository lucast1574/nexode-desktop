# Nexode Desktop · Orbit

Aplicación de escritorio escrita en Rust con **egui/eframe y renderizado por GPU**.
La bienvenida y Servicios usan HTML/CSS local en el WebView del sistema mediante Wry;
los chats, la cuenta, la configuración, la bandeja y el motor siguen en Rust. No usa
Electron, React ni un servidor HTTP local. El motor se controla mediante el SDK
Rust de Orbit. El WebView se destruye al abrir una conversación, una pantalla
nativa o al ocultar/minimizar la ventana.

La organización está inspirada en Codex: proyectos y chats a la izquierda,
conversación central, compositor y panel de cambios de Git. La identidad y los
activos visuales son de Nexode.

## Qué puedes hacer

### Cuenta Nexode y primer inicio

- Iniciar sesión con correo y contraseña o con Google en el navegador del sistema.
- Crear una cuenta y verificar el correo antes de acceder. El formulario enlaza a la recuperación de contraseña en Nexode.
- Autorizar el acceso con Google mediante un código temporal y aprobación de dispositivo con PKCE; puedes copiar el código o cancelar el proceso.
- Conectar tus agentes en el inicio guiado o saltar ese paso y configurarlos después.
- Consultar tu correo, workspace, saldo y créditos disponibles desde **Mi cuenta**, y actualizar sus datos.
- Consultar los planes que devuelve el catálogo de Nexode y preparar un checkout de Stripe explícitamente. El pago se completa en el navegador.
- Cerrar sesión en este dispositivo desde **Mi cuenta**, eliminando su conector Nexode local.

Una cuenta gratuita de Nexode es suficiente para usar el harness. Los proveedores
de IA y los servicios cloud conservan sus propios costes, límites y permisos.

### Conversaciones y proyectos

- Crear chats generales sin seleccionar una carpeta. Se guardan en un directorio privado de conversaciones, separado del directorio de trabajo con el que se abrió la aplicación.
- Añadir carpetas mediante el selector nativo del sistema y crear varios chats por proyecto.
- Volver a abrir conversaciones y recuperar su historial persistente.
- Buscar chats por título, sin distinguir mayúsculas, en conversaciones generales, proyectos y fijados.
- Fijar y desfijar chats para tenerlos a mano en una sección propia.
- Expandir o contraer los chats de cada proyecto y consultar su número de conversaciones.
- Actualizar la lista de chats y recuperar explícitamente proyectos guardados por versiones anteriores.
- Elegir **Chat general** o un proyecto desde el compositor al preparar un chat nuevo.
- Ocultar y volver a mostrar la barra lateral y ajustar su anchura.

Las instalaciones nuevas empiezan sin proyectos. Añadir una carpeta es una
acción explícita; Orbit no importa automáticamente carpetas del equipo.

### Trabajo con el agente

- Enviar solicitudes al motor Orbit para explorar un proyecto, trabajar con código o revisar cambios mediante las herramientas que tenga habilitadas.
- Ver respuestas en streaming, con correcciones de texto por identificador de mensaje.
- Leer mensajes con Markdown y bloques de código, seleccionar texto y copiarlo.
- Consultar **Actividad**, un bloque desplegable con las herramientas ejecutadas y sus resultados o errores.
- Permitir o denegar las solicitudes de autorización que envía el motor.
- Detener una respuesta en curso sin cerrar la aplicación.
- Consultar errores de conexión o ejecución y acceder a la configuración del motor cuando no esté conectado.
- Partir de sugerencias para entender la estructura del proyecto, revisar Git o proponer mejoras. Las sugerencias preparan el mensaje para revisarlo antes de enviarlo.

La interfaz y las operaciones del motor se ejecutan en hilos separados. El
redibujado responde a eventos y la conversación acompaña el avance de la respuesta.

### Proveedores de IA y modelos

En **Configuración → Agentes de IA** puedes mantener varios proveedores conectados:

| Proveedor | Conexión desde la app | Cuentas OAuth |
| --- | --- | --- |
| Claude | Cuenta Claude o API key de Anthropic | Varias cuentas con etiquetas |
| GPT / Codex | Cuenta de Codex o API key de OpenAI | Varias cuentas con etiquetas |
| Gemini | Cuenta de Google o API key de Gemini | Una cuenta activa |
| DeepSeek | API key propia | No aplica |
| Antigravity | Autorización con cuenta de Google | Una cuenta activa |

- Conectar un proveedor conserva las conexiones de los demás.
- En Claude y GPT/Codex, una etiqueta nueva añade una cuenta; reutilizar una etiqueta actualiza esa cuenta.
- En Gemini y Antigravity, una nueva autorización actualiza la cuenta OAuth activa.
- Abrir la autorización en el navegador y completar el retorno solicitado por cada proveedor, o cancelar el flujo pendiente.
- Guardar API keys mediante el IPC privado del motor, sin incluirlas en mensajes del chat ni preferencias de la interfaz.
- Elegir el modelo de una sesión desde el compositor y solicitar **Actualizar modelos** al motor. Las opciones dependen de los proveedores conectados y del catálogo que devuelve Orbit.

Las conexiones por API tienen facturación independiente de las suscripciones de
chat de cada proveedor. Conectar una suscripción no garantiza acceso a su API.

### Cuotas y estado de las cuentas de IA

- Indicadores circulares por cuenta con el porcentaje restante que informa el proveedor.
- Detalle al pasar el cursor: ventanas de uso, reinicios, etiqueta de cuenta y plan, cuando esos datos están disponibles.
- El círculo representa la ventana con menos porcentaje restante.
- Actualización en segundo plano cada dos minutos y al finalizar un turno; los cambios de credenciales también solicitan una actualización.
- Las cuotas no expuestas se muestran como desconocidas y las lecturas que no se pueden renovar se identifican como obsoletas. No se estiman porcentajes a partir de los tokens del chat.

### Revisión de Git

- Abrir o cerrar el panel **Cambios** y ajustar su anchura.
- Consultar la rama actual y el estado corto del árbol de trabajo, incluidos archivos sin seguimiento.
- Ver el diff respecto a `HEAD`, con líneas añadidas y eliminadas diferenciadas por color.
- Trabajar también con repositorios sin primer commit: en ese caso, el diff muestra los cambios preparados en el índice.
- Actualizar la revisión manualmente y consultar los errores de Git en el panel.

Los archivos sin seguimiento aparecen en el estado; su contenido no forma parte
del diff hasta incorporarlos al índice. El panel es de consulta y no ofrece
controles propios para preparar archivos o crear commits.

### Servicios y conector Nexode MCP

La pantalla **Servicios** permite buscar por nombre, propósito o palabras clave,
filtrar por categoría y explorar este catálogo incluido en la app:

| Servicio | Uso |
| --- | --- |
| Nexode | Aplicaciones, bases de datos, dominios, almacenamiento y automatizaciones |
| Nexode Shield | Revisión de seguridad de repositorios |
| Mapora | Mapas, geocodificación, lugares y rutas |
| Vitruko | Metadatos públicos de productos desde enlaces |
| Wazend | WhatsApp, mensajería, agentes y webhooks |
| Harkai | Alertas vecinales y reportes por distrito en Perú |
| The Date | Invitaciones y experiencias para eventos |
| Olva API / Shalom API | Integraciones de envíos y logística |
| API Sunat | Comprobantes y guías de remisión electrónicas |
| HeyBunny | Agentes de IA conectados a herramientas |
| Wazend Calls | Llamadas de WhatsApp e historial |
| Zend CRM | Ventas y conversaciones de WhatsApp |
| Cerebrads | Campañas, creativos y estrategias publicitarias |

**Explorar** abre el sitio del servicio en el navegador. **Preparar con Orbit**
crea un chat con una solicitud para revisar la documentación, el acceso y los
costes antes de contratar o desplegar. No envía el mensaje automáticamente.

La cuenta Nexode prepara el conector oficial **Nexode MCP** para su workspace.
El agente puede utilizar las operaciones que ese conector expone y que la clave
permite. El catálogo de otros servicios ofrece recomendaciones y enlaces; no
implica que sus cuentas estén conectadas ni que compartan credenciales con Nexode.
Consulta [SERVICE-CATALOG.md](SERVICE-CATALOG.md) y [HARNESS.md](HARNESS.md).

### Configuración e integración con el sistema

- **Agentes de IA:** conexiones OAuth y API keys de los proveedores disponibles.
- **General:** comportamiento de bandeja, inicio automático por usuario, estado del registro de inicio y ubicación de los datos locales.
- **Motor:** estado de conexión, selección de un ejecutable compatible y reconexión manual.
- Bandeja nativa con **Abrir Nexode Orbit** y **Salir de Nexode Orbit**.
- Cerrar la ventana conserva la aplicación en la bandeja cuando está disponible; salir desde su menú termina la instancia privada del motor.
- Abrir de nuevo el ejecutable recupera la ventana existente en lugar de iniciar otra instancia con los mismos datos locales.
- Inicio en segundo plano con la sesión del sistema, sólo cuando se activa explícitamente.
- Logo animado que respeta el movimiento reducido del sistema y la visibilidad de la ventana.
- Bienvenida y Servicios con activos y fuentes locales; si el WebView no está disponible en Linux, la bienvenida conserva una alternativa nativa.

| Atajo | Acción |
| --- | --- |
| `Ctrl+N` / `Cmd+N` | Crear un chat nuevo |
| `Enter` en el compositor | Enviar el mensaje |
| `Shift+Enter` | Insertar una nueva línea |
| `Escape` en Servicios | Volver a los chats |

### Motor, CLI/TUI y SDK incluidos

`nexode-orbit/` contiene el motor Rust, su interfaz de terminal, las herramientas,
los contratos de sesiones y los SDK Rust/TypeScript. Desktop controla una
instancia privada mediante el SDK Rust: creación y apertura de sesiones,
historial, modelos, eventos, ejecución, cancelación y decisiones de permisos.
Las capacidades adicionales del motor y sus SDK se documentan en el
[README de Orbit](nexode-orbit/README.md) y en [sus documentos](nexode-orbit/docs/).

El escritorio actual no incluye una terminal PTY integrada, un editor de archivos,
actualizaciones automáticas ni un instalador firmado. Las capacidades del motor
y SDK no equivalen a controles disponibles en la interfaz gráfica.

## Un solo repositorio

El repositorio público [lucast1574/nexode-desktop](https://github.com/lucast1574/nexode-desktop)
contiene la aplicación de escritorio en la raíz y el motor, CLI/TUI y SDK en
`nexode-orbit/`. Son archivos normales de Git, sin submódulos ni descargas de otro
repositorio durante la compilación. El historial original de ambos se conserva.
Los cambios futuros de Desktop y Orbit se realizan aquí, en la misma revisión.
El repositorio público original se eliminó tras verificar la importación completa
y la conservación de su historial. Este es el repositorio único de desarrollo.
La CI activa está en `.github/workflows/` de la raíz: prueba Desktop, la paridad del SDK
y la detección del motor en el monorepo, y construye los paquetes desde este único checkout. Los
workflows importados bajo `nexode-orbit/.github/` se conservan como referencia
histórica y GitHub no los ejecuta desde esa subcarpeta.

```sh
git clone https://github.com/lucast1574/nexode-desktop.git
cd nexode-desktop
```

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

Los comandos de arranque compilan Desktop y el motor incluido, y colocan ambos
binarios en el mismo directorio para que la aplicación encuentre su motor sin una
instalación previa. Para compilar ambos sin abrir la UI en Linux/macOS, usa
`bash scripts/build.sh`.

Requisitos para compilar: Rust estable y Git. No hace falta otro checkout.
El SDK se enlaza directamente desde `nexode-orbit/crates/nexode-sdk`.
Compilar la UI y el motor no necesita Node ni npm.
Los paquetes incluyen Node y el MCP oficial. Si ejecutas sólo el binario de
desarrollo, el conector MCP necesita Node.js 20+ y npx en PATH.
En Windows también se necesitan las herramientas C++ de Visual Studio.

En Ubuntu/Debian, instala las dependencias nativas:

```sh
sudo apt-get install libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libxcursor-dev libgtk-3-dev libayatana-appindicator3-dev libdbus-1-dev libwebkit2gtk-4.1-dev libxdo-dev
```

La bienvenida utiliza WebView2 en Windows y WKWebView en macOS. El paquete Mac se distribuye sólo para Apple Silicon (M1 o posterior) y
requiere macOS 13.5 o posterior por el runtime Node 24 incluido para MCP
([requisitos oficiales](https://github.com/nodejs/node/blob/v24.22.2/BUILDING.md#platform-list)). En Linux usa
WebKitGTK 4.1 sobre X11/XWayland; si sólo hay Wayland nativo o no se puede crear
el WebView, conserva una bienvenida nativa funcional. No carga sitios remotos,
no recibe credenciales y sólo comunica acciones limitadas al host Rust.

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

Orbit debe tener configurado un proveedor para responder. La aplicación empieza
con su propia configuración y no copia automáticamente las credenciales ni la
configuración de otras instalaciones de Orbit. Las variables de entorno que
asignes explícitamente al proceso siguen disponibles para el motor. Desde Configuración puedes conectar
proveedores por OAuth o API key. La cuenta Nexode conserva su conector personal
en el almacén seguro del sistema; las claves de IA se entregan al almacén privado
del motor por IPC. No se guardan en preferencias ni en el chat.

Las preferencias y los proyectos se conservan en el directorio de datos local
del usuario, dentro de `nexode-desktop`; la instancia del motor utiliza su
subcarpeta `engine`. `ORBIT_DESKTOP_HOME` permite cambiar esa ubicación.
Las instalaciones nuevas empiezan sin proyectos. Añade cada carpeta desde la
interfaz; no se explora el directorio de trabajo ni se utiliza `ORBIT_PROJECTS`.
Las carpetas detectadas por versiones anteriores se conservan como una lista
de recuperación opcional, sin aparecer como proyectos activos hasta confirmarlas.
Los chats se agrupan bajo su proyecto y se pueden expandir y volver a abrir.

La aplicación posee su instancia privada. Salir desde la bandeja termina esa
instancia sin modificar el servidor de la terminal. Cerrar la ventana la oculta
cuando hay bandeja disponible. En Windows un Job Object limita también
la vida de los procesos descendientes al proceso de la aplicación.

## Validación

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo test --manifest-path nexode-orbit/Cargo.toml --locked -p nexode-sdk parity
cargo test --manifest-path nexode-orbit/Cargo.toml --locked -p nexode-build-support paths::tests::find
```

Para compilar la aplicación y el motor juntos, usa `./start.ps1 -BuildOnly` en
Windows o `bash scripts/build.sh` en Linux/macOS. Consulta [VALIDATION.md](VALIDATION.md)
para la evidencia de pruebas, distribución y limitaciones conocidas.

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
sudo apt-get install libgtk-3-dev libayatana-appindicator3-dev libdbus-1-dev libwebkit2gtk-4.1-dev libxdo-dev libxkbcommon-x11-dev
```

En Linux, el entorno de ejecución necesita GTK3, Ayatana AppIndicator3 y una
sesión D-Bus; guardar credenciales requiere además un servicio Secret Service
desbloqueado (por ejemplo GNOME Keyring). No se crea un almacén de claves en texto
plano si ese servicio no está disponible.

### Paquete completo para Windows

Ejecuta `./scripts/bundle-windows.ps1` en PowerShell desde Windows x64. Compila la interfaz y el motor Rust, verifica por SHA256 Node.js 24.22.2 y el código fijado de Nexode MCP 0.1.26, instala sus dependencias mediante el lockfile y crea `dist/Nexode-Orbit-Windows-x64.zip` con un archivo SHA256. Extrae la carpeta completa antes de abrir `nexode-desktop.exe`; Node y MCP están incluidos y no requieren instalación global. El script conserva un staging independiente y no modifica la aplicación instalada ni registra el inicio automático.

Para reutilizar compilaciones existentes, pasa `-DesktopBinary <ruta.exe> -EngineBinary <ruta.exe>`. `-NodeArchive <archivo.zip>` y `-McpArchive <archivo.tar.gz>` reutilizan descargas locales, siempre comprobando sus hashes. `-OutputDirectory <carpeta>` permite preparar una entrega separada. CI publica este ZIP completo en el artefacto Windows.
