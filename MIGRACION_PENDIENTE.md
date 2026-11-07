# Migración pendiente de Python a Rust

Estado al 7 de noviembre de 2026, tomando como base el commit `485f1b1`. Migración reanudada por petición del mantenedor. Este documento registra pendientes; no afirma que el repositorio esté libre de Python.

## Alcance y estado

- Quedan **49 archivos Python versionados**, enumerados abajo, además de Python incrustado en scripts de shell y referencias de CI.
- Las herramientas ya migradas están principalmente en `tools/orbit-dev`; sus comandos y opciones están documentados en [su README](tools/orbit-dev/README.md). Revisar esos comandos antes de duplicar implementaciones.
- El SDK TypeScript y el JavaScript necesario para la extensión de Chrome se conservan por decisión del mantenedor. No forman parte de este pendiente de Python.
- El worker de telemetría se retiró anteriormente. El catálogo/carrito ficticio de las pruebas de Discovery se conserva: es una simulación local, sin pagos reales.
- Los avisos y atribuciones de terceros de `NOTICE` deben conservarse.

## Inventario de archivos Python pendientes

Cada casilla implica portar la funcionalidad y sus pruebas, actualizar consumidores y eliminar el archivo antiguo después de verificar equivalencia. La clasificación agrupa rutas; no equivale a una auditoría completa de su comportamiento.

### Cloud, SSH y actualización del runtime

- [ ] `scripts/cloud_acceptance_tui.py`
- [ ] `scripts/runtime_upgrade_acceptance.py`
- [ ] `scripts/test_cloud_acceptance.py`
- [ ] `scripts/test_cloud_session_cli.py`
- [x] `scripts/test_update_cloud_runtime.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/update_cloud_runtime.py` — migrado a Rust en `tools/orbit-dev`.
- [ ] `tests/test_cloud_acceptance_harness.py`
- [ ] `tests/test_cloud_credentials.py`
- [ ] `tests/test_native_ssh_cli.py`
- [ ] `tests/test_native_ssh_import.py`
- [ ] `tests/test_native_ssh_login.py`

### Benchmarks, Discovery, Harbor y swarm

- [ ] `scripts/bench_headless_memory.py`
- [ ] `scripts/bench_memory_cli.py`
- [ ] `scripts/benchmark_browser_handoff.py`
- [ ] `scripts/benchmark_discovery.py`
- [ ] `scripts/benchmark_discovery_rate.py`
- [ ] `scripts/benchmark_swarm.py`
- [ ] `scripts/benchmark_takehome.py`
- [ ] `scripts/nexode_harbor_agent.py`
- [ ] `scripts/nexode_harbor_claude_agent.py`
- [ ] `scripts/run_terminal_bench_campaign.py`
- [ ] `scripts/test_benchmark_discovery.py`
- [ ] `scripts/test_benchmark_discovery_rate.py`
- [ ] `scripts/test_swarm.py`
- [ ] `scripts/test_swarm_debug.py`

### Memoria, perfiles y estrés

- [ ] `scripts/analyze_runtime_memory_log.py`
- [x] `scripts/heap_diff_idle_sessions.py` — migrado a Rust en `tools/orbit-dev`.
- [ ] `scripts/profile_remote_resume_burst.py`
- [ ] `scripts/profile_spawn.py`
- [ ] `scripts/stress_test.py`
- [ ] `scripts/test_analyze_runtime_memory_log.py`
- [ ] `scripts/test_memory.py`

### TUI, recarga, escritorio y aceptación

- [ ] `scripts/reload_recovery_audit.py`
- [ ] `scripts/repro_expand_edit_shortcut.py`
- [ ] `scripts/repro_input_lag.py`
- [ ] `scripts/repro_live_duplicate.py`
- [ ] `scripts/test_desktop_selfdev.py`
- [ ] `scripts/test_reload.py`
- [ ] `scripts/test_soft_interrupt.py`
- [ ] `scripts/tests/mcp_cli_acceptance.py`
- [ ] `tests/e2e/expand_badge_headed_wtype.py`
- [ ] `tests/e2e/expand_badge_headless.py`
- [ ] `tests/test_injection_fix.py`
- [ ] `tests/test_injection_thorough.py`
- [ ] `tests/test_selfdev_reload.py`

### Servicios del teléfono y TestFlight

- [ ] `scripts/phone-server/breaker-lambda.py`
- [ ] `scripts/phone-server/test_wake_lambda.py`
- [ ] `scripts/phone-server/testflight-setup.py`
- [ ] `scripts/phone-server/wake-lambda.py`

### Otros scripts y pruebas

- [ ] `docs/reddit_dashboard.py`
- [ ] `scripts/test_caching_detailed.py`
- [ ] `scripts/test_oauth_usage.py`

## Python incrustado y lanzadores pendientes

Estas rutas de shell contienen invocaciones de Python, bloques incrustados o configuración de PYTHONPATH. Migrar también estos puntos: borrar únicamente archivos `.py` no elimina la dependencia.

- [ ] `scripts/antigravity_multiturn_coverage.sh`
- [ ] `scripts/bench_compile.sh`
- [ ] `scripts/bench_selfdev_build.sh`
- [ ] `scripts/bench_selfdev_checkpoints.sh`
- [ ] `scripts/benchmark_tools.sh`
- [ ] `scripts/compare_discovery_rate.sh`
- [ ] `scripts/compile_time_probe.sh`
- [ ] `scripts/generate_release_notes.sh`
- [ ] `scripts/record_demo.sh`
- [ ] `scripts/replay_recording.sh`
- [ ] `scripts/run_terminal_bench_claude.sh`
- [ ] `scripts/run_terminal_bench_harbor.sh`
- [ ] `scripts/startup_acceptance_matrix.sh`
- [ ] `scripts/stress_test_40.sh`
- [ ] `scripts/test_cloud_environment_e2e.sh`
- [ ] `scripts/test_cloud_group_e2e.sh`
- [ ] `scripts/test_cloud_move_agent_e2e.sh`
- [ ] `scripts/test_cloud_move_e2e.sh`
- [ ] `scripts/test_cloud_move_tui_e2e.sh`
- [ ] `scripts/test_cloud_replace_e2e.sh`
- [ ] `scripts/test_ssh_slash_workflows.sh`
- [ ] `scripts/verify_alt_shift_e_reaches_terminal.sh`
- [ ] `scripts/webfetch_corpus.sh`

## Integraciones que requieren atención especial

- [ ] `.github/workflows/cloud-acceptance.yml`: instala Python y ejecuta el harness y las pruebas de aceptación cloud. Actualizar junto con sus reemplazos nativos; no habilitar ni ejecutar infraestructura real como parte de una comprobación local.
- [ ] `src/cli/commands.rs` y `src/cli/args/cloud_sessions.rs`: integración opcional con el helper externo privado `jade_sessions.py`. Definir un contrato nativo o retirar explícitamente esa integración; revisar también sus pruebas y documentación. Ese helper no está incluido en este repositorio.
- [ ] Adaptadores Harbor: dependen de interfaces del framework Python externo. Evaluar un runner/contrato compatible en Rust; envolver o invocar Python no completa la migración. Actualizar también los lanzadores de Terminal Bench.
- [ ] Lambdas de encendido/apagado y emparejamiento: portar el protocolo y clientes AWS a Rust, externalizar configuración heredada y probar con fixtures. No sustituir boto3 por AWS CLI si el objetivo es eliminar Python.
- [ ] TestFlight: portar autenticación JWT y llamadas de API, con pruebas locales; no enviar builds ni modificar cuentas durante la validación.
- [ ] Revisar documentación, comandos de ejemplo y consumidores indirectos para corregir referencias a scripts migrados. Distinguir ejemplos legítimos de lenguaje Python de dependencias ejecutadas por Orbit.

### Referencia externa que no debe borrarse a ciegas

`scripts/install_justrust_hosted_ci.sh` conserva el archivo externo `scripts/sign-release.py` de justrust porque su fuente fijada lo incluye como datos durante la compilación. La firma de Orbit ya usa Rust y ese flujo no invoca un intérprete Python. Eliminar el archivo sin adaptar primero esa dependencia rompe el bootstrap; verificar el uso real antes de cambiarlo.

## Orden sugerido para retomar

1. Terminar herramientas de diagnóstico, memoria, TUI y recarga, reutilizando PTY, protocolo de debug y métricas nativas existentes.
2. Migrar suites de aceptación y sus fixtures (escritorio, MCP, SSH y cloud), junto con lanzadores y CI.
3. Portar benchmarks y análisis de resultados; conservar esquemas JSON, argumentos, códigos de salida y límites de tiempo. Resolver Harbor de forma explícita.
4. Migrar servicios externos y empaquetado de runtime con configuraciones explícitas y pruebas de APIs simuladas.
5. Auditar shell, workflows y documentación para cerrar dependencias residuales.

## Criterios para cerrar cada bloque

- Conservar comportamiento útil y cobertura; no borrar pruebas solamente para reducir el conteo de Python.
- Probar errores, timeouts y limpieza de procesos cuando corresponda. Usar hogares temporales, credenciales sintéticas, sockets propios y servidores locales.
- No usar sesiones personales, llamadas reales a modelos, servidores remotos, cambios cloud ni publicaciones como validación automática.
- Ejecutar las pruebas relevantes y Clippy del workspace de herramientas; hacer commits separados por funcionalidad y subirlos tras validar.
- Revisar compatibilidad de plataforma: varias herramientas son Unix/Linux; una prueba local en Linux no acredita Windows ni macOS.
- Confirmar que no quedan consumidores del script antiguo antes de eliminarlo.

## Límites de la validación actual

La última suite completa de `tools/orbit-dev` y su Clippy de todos los targets pasaron para el bloque de diagnóstico de RSS de sesiones ociosas. Esto no certifica todos los scripts pendientes ni todas las plataformas. El Clippy del workspace raíz mantiene errores previos fuera de esta migración; no debe presentarse como aprobado. Las pruebas locales de RSS verificaron funcionamiento, no demostraron ausencia de fugas a largo plazo.

## Cómo actualizar el inventario

Desde la raíz del repositorio:

```sh
git ls-files '*.py'
rg -n -i 'python3?|\.py\b|PYTHONPATH' scripts tests .github tools src docs
```

Clasificar los resultados: ejecución real, prueba, ejemplo de lenguaje, comentario o datos externos. El conteo de archivos no sustituye esa revisión.
