# Migración de herramientas a Rust

La migración del código Python propio se completó el 7 de noviembre de 2026. No quedan archivos `.py` versionados ni bloques Python ejecutables en los lanzadores o workflows de Orbit.

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

## Cierre y validación

Las herramientas de releases, diagnóstico, memoria, SSH, TUI, Discovery, swarm,
Harbor, Lambda y TestFlight tienen implementaciones Rust en `tools/orbit-dev`.
Los lanzadores shell usan `scripts/orbit_dev.sh`. La integración heredada
`nexode cloud sessions`, dependiente de `jade_sessions.py`, se retiró por indicación
del mantenedor; se conservan `cloud move`, `return`, `env`, `replace` y `ephemeral`.

El registro y los límites de validación están en [MIGRACION_PENDIENTE.md](../MIGRACION_PENDIENTE.md).
La suite completa de herramientas pasó 332 pruebas; Clippy de todos sus targets
pasó sin warnings. La aceptación adicional usó runtime, OpenSSH y Docker reales
con servicios locales y credenciales sintéticas. No se hicieron despliegues ni
llamadas pagadas a modelos. Los diagnósticos de arranque/input lag conservan y
reportan sus incumplimientos de presupuesto: no son pruebas declaradas aprobadas.

La capacidad del agente para trabajar con proyectos Python se conserva. Algunos
datasets externos evalúan programas Python y requieren su propio intérprete;
esto es distinto del orquestador nativo. El runner Terminal-Bench implementa el
contrato de tareas Linux de un contenedor y documenta los contratos no soportados.

El ensayo conserva el requisito del updater de compilar con `NEXODE_RELEASE_REPO`
configurado para probar una actualización real. Las pruebas locales del servidor,
archivo, sandbox y lanzamiento no publican releases. Se mantiene el SDK TypeScript
y el JavaScript necesario de la extensión del navegador.
