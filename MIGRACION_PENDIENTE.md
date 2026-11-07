# Registro de migración de Python a Rust

Migración completada el 7 de noviembre de 2026 sobre la base `485f1b1`. Se conserva este nombre de archivo para no romper enlaces al inventario histórico.

## Alcance y estado

- Quedan **0 archivos Python versionados**. Los bloques Python de shell y sus consumidores de CI se sustituyeron por comandos Rust. Las rutas marcadas abajo son el registro de los originales retirados.
- Las herramientas ya migradas están principalmente en `tools/orbit-dev`; sus comandos y opciones están documentados en [su README](tools/orbit-dev/README.md). Revisar esos comandos antes de duplicar implementaciones.
- El SDK TypeScript y el JavaScript necesario para la extensión de Chrome se conservan por decisión del mantenedor. No forman parte de este pendiente de Python.
- El worker de telemetría se retiró anteriormente. El catálogo/carrito ficticio de las pruebas de Discovery se conserva: es una simulación local, sin pagos reales.
- Los avisos y atribuciones de terceros de `NOTICE` deben conservarse.

## Inventario histórico de archivos migrados

Cada casilla implica portar la funcionalidad y sus pruebas, actualizar consumidores y eliminar el archivo antiguo después de verificar equivalencia. La clasificación agrupa rutas; no equivale a una auditoría completa de su comportamiento.

### Cloud, SSH y actualización del runtime

- [x] `scripts/cloud_acceptance_tui.py` — gate nativo `test-cloud-acceptance` y helpers Rust.
- [x] `scripts/runtime_upgrade_acceptance.py` — `runtime-upgrade-acceptance`; probado contra el runtime real, con red y credenciales aisladas.
- [x] `scripts/test_cloud_acceptance.py` — gate nativo `test-cloud-acceptance` y helpers Rust.
- [x] `scripts/test_cloud_session_cli.py` — `test-cloud-session-cli`; probado contra la CLI real y transporte nativo hermético.
- [x] `scripts/test_update_cloud_runtime.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/update_cloud_runtime.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `tests/test_cloud_acceptance_harness.py` — gate nativo `test-cloud-acceptance` y helpers Rust.
- [x] `tests/test_cloud_credentials.py` — `test-cloud-credentials`; 14 escenarios con binario real y credenciales sintéticas.
- [x] `tests/test_native_ssh_cli.py`
- [x] `tests/test_native_ssh_import.py`
- [x] `tests/test_native_ssh_login.py`

### Benchmarks, Discovery, Harbor y swarm

- [x] `scripts/bench_headless_memory.py`
- [x] `scripts/bench_memory_cli.py`
- [x] `scripts/benchmark_browser_handoff.py`
- [x] `scripts/benchmark_discovery.py`
- [x] `scripts/benchmark_discovery_rate.py`
- [x] `scripts/benchmark_swarm.py`
- [x] `scripts/benchmark_takehome.py`
- [x] `scripts/nexode_harbor_agent.py`
- [x] `scripts/nexode_harbor_claude_agent.py`
- [x] `scripts/run_terminal_bench_campaign.py`
- [x] `scripts/test_benchmark_discovery.py`
- [x] `scripts/test_benchmark_discovery_rate.py`
- [x] `scripts/test_swarm.py`
- [x] `scripts/test_swarm_debug.py`

### Memoria, perfiles y estrés

- [x] `scripts/analyze_runtime_memory_log.py` — analizador Rust; 16 escenarios de equivalencia y CLI sin intérprete.
- [x] `scripts/heap_diff_idle_sessions.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/profile_remote_resume_burst.py` → `orbit-dev profile-remote-resume-burst` (PTY real, CPU/RSS y timeout explícito).
- [x] `scripts/profile_spawn.py` → `orbit-dev profile-spawn` (dos ejecuciones reales cliente/daemon, métricas completas de /proc).
- [x] `scripts/stress_test.py` → `orbit-dev stress-test` (40 sesiones reales; recursos y comprobaciones de limpieza).
- [x] `scripts/test_analyze_runtime_memory_log.py` — analizador Rust; 16 escenarios de equivalencia y CLI sin intérprete.
- [x] `scripts/test_memory.py` → `orbit-dev test-memory` (pruebas locales reales y contrato live con fixture; Jev/extracción live opt-in).

### TUI, recarga, escritorio y aceptación

- [x] `scripts/reload_recovery_audit.py` — auditor Rust; equivalencia JSON de once registros con logs/trazas/durables y filtros.
- [x] `scripts/repro_expand_edit_shortcut.py` — migrado a `orbit-dev`, verificado con runtime real.
- [x] `scripts/repro_input_lag.py` — PTY Rust: cinco escenarios, dos clientes de fondo, limpieza real; diagnóstico de idle conserva umbral y exit 1.
- [x] `scripts/repro_live_duplicate.py` — seis snapshots reales por cliente y control positivo de duplicación, con recarga/reconexión.
- [x] `scripts/test_desktop_selfdev.py` — migrado a `cargo xtask test-desktop-selfdev`.
- [x] `scripts/test_reload.py` — `test-reload`: 12 grupos contra runtime real y 34 tests Rust de señales, contexto y marcadores.
- [x] `scripts/test_soft_interrupt.py` — suite nativa de ocho escenarios, validada con runtime y herramientas reales.
- [x] `scripts/tests/mcp_cli_acceptance.py` — `mcp-cli-acceptance`; diez casos contra CLI real, OAuth/PKCE/callback/SSE locales.
- [x] `tests/e2e/expand_badge_headed_wtype.py` — Rust; protocolo/foco/limpieza probados con dobles. Compositor Wayland real no disponible en este entorno.
- [x] `tests/e2e/expand_badge_headless.py` — `expand-badge-headless`; probado con PTY y runtime reales, en un daemon aislado.
- [x] `tests/test_injection_fix.py` — suite nativa de ocho escenarios, validada con runtime y herramientas reales.
- [x] `tests/test_injection_thorough.py` — suite nativa de ocho escenarios, validada con runtime y herramientas reales.
- [x] `tests/test_selfdev_reload.py` — migrado a `orbit-dev`, verificado con runtime real.

### Servicios del teléfono y TestFlight

- [x] `scripts/phone-server/breaker-lambda.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/phone-server/test_wake_lambda.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/phone-server/testflight-setup.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/phone-server/wake-lambda.py` — migrado a Rust en `tools/orbit-dev`.

### Otros scripts y pruebas

- [x] `docs/reddit_dashboard.py` — `reddit-dashboard`: ocho paneles PNG/SVG, datos históricos y métricas JSON, sin Python.
- [x] `scripts/test_caching_detailed.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/test_oauth_usage.py` — migrado a Rust en `tools/orbit-dev`.

## Python incrustado y lanzadores migrados

Se revisaron también invocaciones de Python, bloques incrustados y configuración de PYTHONPATH. Los lanzadores siguientes delegan a Rust; webfetch conserva una URL legítima de documentación externa.

- [x] `scripts/antigravity_multiturn_coverage.sh` — parser JSON Rust; cuatro turnos y fallo JSON probados con CLI sintética.
- [x] `scripts/bench_compile.sh` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/bench_selfdev_build.sh` — Python sustituido por comandos nativos de orbit-dev.
- [x] `scripts/bench_selfdev_checkpoints.sh` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/benchmark_tools.sh` — benchmark Rust; 54 ejecuciones reales de herramientas, CSV y contrato con fixture.
- [x] `scripts/compare_discovery_rate.sh`
- [x] `scripts/compile_time_probe.sh` — Python sustituido por comandos nativos de orbit-dev.
- [x] `scripts/generate_release_notes.sh` — lanzador de `generate-release-notes` nativo.
- [x] `scripts/record_demo.sh` — Python sustituido por comandos nativos de orbit-dev.
- [x] `scripts/replay_recording.sh` — Python sustituido por comandos nativos de orbit-dev.
- [x] `scripts/run_terminal_bench_claude.sh`
- [x] `scripts/run_terminal_bench_harbor.sh`
- [x] `scripts/startup_acceptance_matrix.sh` — matriz nativa aislada, once escenarios reales; R7 pasó, conserva fallos visuales diagnosticados.
- [x] `scripts/stress_test_40.sh` — helpers Rust de socket/JSON; sintaxis shell y protocolo probados.
- [x] `scripts/test_cloud_environment_e2e.sh` — assertions Rust; aceptación con runtime real.
- [x] `scripts/test_cloud_group_e2e.sh` — gate nativo `test-cloud-acceptance` y helpers Rust.
- [x] `scripts/test_cloud_move_agent_e2e.sh` — gate nativo `test-cloud-acceptance` y helpers Rust.
- [x] `scripts/test_cloud_move_e2e.sh` — gate nativo `test-cloud-acceptance` y helpers Rust.
- [x] `scripts/test_cloud_move_tui_e2e.sh` — gate nativo `test-cloud-acceptance` y helpers Rust.
- [x] `scripts/test_cloud_replace_e2e.sh` — assertions Rust; aceptación con runtime real.
- [x] `scripts/test_ssh_slash_workflows.sh` — migrado a `orbit-dev`, verificado con runtime real.
- [x] `scripts/verify_alt_shift_e_reaches_terminal.sh` — capturador de terminal Rust y selección de ventanas nativa; captura verificada con PTY.
- [x] `scripts/webfetch_corpus.sh` — auditado: python.org es una URL de prueba, no una invocación de Python.

## Integraciones que requieren atención especial

- [x] `.github/workflows/cloud-acceptance.yml`: usa harness y pruebas Rust; ya no instala Python.
- [x] Integración heredada `cloud sessions` y helper privado `jade_sessions.py` retirados por indicación del mantenedor; 424 pruebas CLI y rechazo del comando eliminado verificados.
- [x] Adaptadores Harbor sustituidos por runner Docker y campañas Rust, con contratos soportados y rechazos explícitos documentados en `docs/TERMINAL_BENCH.md`.
- [x] Lambdas de encendido/apagado y emparejamiento: protocolos y cliente AWS firmados en Rust, configuración externa y pruebas locales; sin boto3 ni AWS CLI.
- [x] TestFlight: JWT y API en Rust, probados localmente sin enviar builds ni modificar cuentas.
- [x] Documentación y consumidores actualizados; se conservan ejemplos legítimos del lenguaje y contratos de datasets externos.

### Referencia externa que no debe borrarse a ciegas

`scripts/install_justrust_hosted_ci.sh` conserva el archivo externo `scripts/sign-release.py` de justrust porque su fuente fijada lo incluye como datos durante la compilación. La firma de Orbit ya usa Rust y ese flujo no invoca un intérprete Python. Eliminar el archivo sin adaptar primero esa dependencia rompe el bootstrap; verificar el uso real antes de cambiarlo.

## Validación reproducible

Los comandos y opciones están en `tools/orbit-dev/README.md`, `docs/NATIVE_SSH.md`, `docs/NATIVE_DISCOVERY_SWARM_BENCHMARKS.md`, `docs/TERMINAL_BENCH.md` y `docs/MEMORY_BENCHMARKS.md`.

## Criterios aplicados

- Conservar comportamiento útil y cobertura; no borrar pruebas solamente para reducir el conteo de Python.
- Probar errores, timeouts y limpieza de procesos cuando corresponda. Usar hogares temporales, credenciales sintéticas, sockets propios y servidores locales.
- No usar sesiones personales, llamadas reales a modelos, servidores remotos, cambios cloud ni publicaciones como validación automática.
- Ejecutar las pruebas relevantes y Clippy del workspace de herramientas; hacer commits separados por funcionalidad y subirlos tras validar.
- Revisar compatibilidad de plataforma: varias herramientas son Unix/Linux; una prueba local en Linux no acredita Windows ni macOS.
- Confirmar que no quedan consumidores del script antiguo antes de eliminarlo.

## Validación y límites

- Suite completa de `tools/orbit-dev`: **332 pruebas aprobadas**, 0 fallos, 1 aceptación SSH ignorada por defecto. Esa aceptación se ejecutó explícitamente y pasó junto con CLI/login/importación contra OpenSSH real aislado. Clippy de todos los targets pasó con `-D warnings`.
- Runtime real: recarga, duplicación de transcript, herramientas, memoria, sesiones concurrentes, interrupciones y SSH. Docker real: intentos concurrentes, verificador, reward, tokens, timeout/limpieza y reanudación de campaña. Los proveedores fueron locales y las credenciales sintéticas; no se hizo inferencia pagada ni despliegues.
- Los benchmarks conservan la capacidad de devolver fallos: la matriz de arranque pasó R7 (6 ms de catálogo, límite 40 ms), pero reprodujo fallos visuales de primer frame. La prueba inmediata de input lag mantuvo baja latencia de escritura y detectó actividad idle superior al presupuesto. No se presentan estos diagnósticos como aprobados.
- Los benchmarks live de proveedores y navegador requieren sus servicios configurados. La ejecución de un desafío externo Python (por ejemplo `original_performance_takehome`) puede requerir el intérprete del propio desafío; el orquestador de Orbit es Rust. El runner Terminal-Bench soporta tareas Linux de un contenedor y rechaza contratos avanzados no implementados; consultar sus límites en la documentación.
- Los checks de módulos y límites de dependencias pasaron. El guardrail global de tamaño detecta 20 excesos respecto a su baseline en archivos no modificados por el bloque final; no se elevaron los límites para ocultarlos.
- La validación descrita fue en Linux/WSL. No certifica Windows/macOS, ausencia de fugas a largo plazo ni el Clippy completo del workspace raíz, que tiene errores previos fuera de este trabajo.

## Cómo actualizar el inventario

Desde la raíz del repositorio:

```sh
git ls-files '*.py'
rg -n -i 'python3?|\.py\b|PYTHONPATH' scripts tests .github tools src docs
```

Clasificar los resultados: ejecución real, prueba, ejemplo de lenguaje, comentario o datos externos. El conteo de archivos no sustituye esa revisión.
