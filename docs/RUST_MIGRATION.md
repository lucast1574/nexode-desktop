# Migración de herramientas a Rust

La migración está en curso. Este documento no declara al repositorio libre de Python.

## Implementado

- Worker de telemetría eliminado: servicio, D1, SQL, dashboards, dependencias y scripts JavaScript.
- Inventario remoto implementado en `nexode-remote-inspect`, con un comando interno del binario Orbit. Se retiraron el colector Python y su suite, portando sus escenarios a Rust.
- Comprobación de marca y manifests: `cargo xtask check-branding`.
- Guardrails de módulos, dependencias, tamaños, panics, errores descartados y reexports: Rust; se conservan los límites registrados.
- Fixtures locales de navegador y proveedor OpenAI-compatible: Rust; tests de protocolo y servidor sin Python. El runner live del navegador es Rust y sigue siendo opt-in.
- Informes de arquitectura, caché WebSocket, badges y gráfico de estrellas: Rust.
- Ensayo de actualizaciones: `cargo xtask update-rehearsal`, servidor HTTP local, tar/gzip y SHA-256 en Rust; TUI conectada al comando Cargo.
- Diagnóstico de locks de entorno y comparación Terminal-Bench: `find-unlocked-env-tests` y `tb-compare`.
- Evaluador de planes exportados: `cargo xtask score-plans`, sin rutas privadas del autor original.
- Versionado de paquetes SDK: `cargo xtask version-sdk`, con pruebas Rust. Se conservan el SDK TypeScript y sus pruebas de consumidor.
- Timeout de comandos de CI: `cargo xtask timeout`, sin intérprete Python.
- Registro JSONL de compilaciones: Rust, invocado desde el wrapper Bash.
- Empaquetado del puente externo: `cargo xtask package-browser`. El host nativo y CLI de ese puente ya son Rust; el código JS de la extensión continúa usando APIs del navegador y DOM.

## Compatibilidad

Para inventario remoto, el servidor SSH debe usar un binario que incluya
`__remote-inventory`. Con una versión anterior el panel muestra inventario no
disponible/obsoleto; no instala Python ni lo usa como fallback. El comando no
inicializa proveedores, sesiones de agente ni el daemon. No se conecta a un
backend de Nexode Cloud.

## Trabajo pendiente

Quedan scripts Python de guardrails, CI, releases, benchmarks, diagnóstico,
pruebas de integración y self-dev. También quedan fragmentos Python incrustados
en wrappers. Hay integraciones con helpers de otros repositorios (Jade y Desktop)
cuyos fuentes no están incluidos aquí. No se han eliminado para simular una
migración terminada ni se han sustituido por wrappers que ejecuten Python.

Continúa la migración de releases, diagnóstico y suites SSH/TUI/benchmarks. Cada reemplazo debe mantener sus contratos y comprobarse
antes de retirar el original.

La aceptación final requiere cero scripts/intérpretes Python necesarios en
los flujos soportados de build, pruebas, instalación y ejecución, verificados
en entornos sin Python. La capacidad del agente para trabajar con proyectos
Python de los usuarios se conserva.

El ensayo conserva el requisito del updater de compilar con `NEXODE_RELEASE_REPO` configurado para probar una actualización real. Las pruebas locales del servidor, archivo, sandbox y lanzamiento no publican releases.
