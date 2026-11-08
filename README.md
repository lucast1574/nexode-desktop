# Nexode Desktop

Interfaz local de Nexode Orbit, inspirada en la organización de Codex: proyectos
y conversaciones a la izquierda, chat en el centro y revisión de Git al lado.
Conserva los activos y el verde orbital de Nexode.

Esta primera versión funciona en el navegador. No es todavía un instalador
Windows ni incluye una terminal PTY integrada. El agente y el servidor son
Rust; React/TypeScript se utiliza únicamente para la interfaz.

## Abrir en este equipo

Desde PowerShell, en esta carpeta:

```powershell
./start.ps1
```

Después abre <http://localhost:4318>. El script compila la interfaz y el servidor
en Ubuntu/WSL. Necesita Node, Rust dentro de WSL y el checkout hermano
`../nexode-orbit`. Busca `nexode` en PATH o el ejecutable compilado en
`~/orbit-root-target/debug/nexode`. Mantén el proceso abierto mientras uses la UI.

No se ha publicado esta aplicación en Internet. El servidor escucha solamente
en loopback y rechaza peticiones API sin la cabecera de cliente, de otros
orígenes o con nombres Host inesperados.

## Uso

- Selecciona un proyecto y escribe en el chat. Enter envía; Shift+Enter añade línea.
- Las sugerencias rellenan el mensaje; no ejecutan nada hasta que lo envías.
- Los chats se guardan en la instancia privada `~/.nexode-desktop`.
- La barra lateral abre el historial de las conversaciones.
- El selector de modelos consulta el catálogo real de Orbit después de crear
  o abrir una conversación. Incluye actualización manual.
- El botón cuadrado cancela el turno; las solicitudes de permiso permiten
  autorizar o denegar sin aprobarlas automáticamente.
- Cambios muestra el estado de Git y el diff contra HEAD. Los archivos nuevos
  se enumeran en el estado; sus contenidos no aparecen en el diff.
- Añadir carpeta acepta una ruta en el sistema de Orbit, por ejemplo
  `/mnt/d/vscode/mi-proyecto` en WSL.

El motor necesita un proveedor configurado para responder. El SDK hereda las
credenciales y la configuración del usuario de **WSL**, no automáticamente las
de Windows. Si usas otro directorio de configuración, define `NEXODE_HOME`
antes de arrancar. Nunca pongas claves en el frontend ni en este repositorio.

Variables del servidor:

| Variable             | Uso                                                         |
| -------------------- | ----------------------------------------------------------- |
| `NEXODE_BINARY`      | Ejecutable nativo de Orbit                                  |
| `NEXODE_HOME`        | Configuración de origen que hereda el SDK                   |
| `ORBIT_DESKTOP_HOME` | Estado privado de esta interfaz                             |
| `ORBIT_PROJECTS`     | Carpeta cuyos repositorios Git aparecen en la barra lateral |

## Desarrollo y pruebas

```sh
npm ci
npm run dev
npm run build
npx playwright install chromium
npm test
cargo fmt --check --manifest-path server/Cargo.toml
cargo clippy --locked --manifest-path server/Cargo.toml -- -D warnings
```

Vite utiliza el puerto 1420 y redirige `/api` al servidor Rust en 4318. El
servidor sirve `dist/` en producción. Ejecutarlo desde la raíz de este proyecto.

`tests/interface.spec.ts` prueba chat, historial, modelos, diff, teclado,
adaptación móvil y errores de conexión mediante fixtures explícitas.
`tests/live.spec.ts` es opt-in (`ORBIT_LIVE_TEST=1`): necesita el proveedor
sintético nativo `orbit-dev fake-openai-server` del repositorio Orbit, etiquetado
`desktop`, y un servidor iniciado con esa configuración aislada. Nunca ejecutar
esa prueba contra un proveedor de pago.

Validación inicial: build de frontend, build y Clippy Rust, tres pruebas de
interfaz y una prueba real navegador → Rust SDK → Orbit → proveedor sintético.
Se verificaron el streaming, el historial, Git y el rechazo de peticiones
locales sin autorización de origen. No se realizó inferencia de pago.

La UI permanece en un proyecto separado porque `nexode-orbit/AGENTS.md`
reserva Desktop para otro repositorio. El checkout de Orbit usado para esta
versión es `d8dbb7637e9e6389676b7695a6fe47730a85d25a`.
