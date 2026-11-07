# Migración pendiente de Python a Rust

Estado al 7 de noviembre de 2026, tomando como base el commit `485f1b1`. Migración reanudada por petición del mantenedor. Este documento registra pendientes; no afirma que el repositorio esté libre de Python.

## Alcance y estado

- Quedan **22 archivos Python versionados**, enumerados abajo, además de Python incrustado en scripts de shell y referencias de CI.
- Las herramientas ya migradas están principalmente en `tools/orbit-dev`; sus comandos y opciones están documentados en [su README](tools/orbit-dev/README.md). Revisar esos comandos antes de duplicar implementaciones.
- El SDK TypeScript y el JavaScript necesario para la extensión de Chrome se conservan por decisión del mantenedor. No forman parte de este pendiente de Python.
- El worker de telemetría se retiró anteriormente. El catálogo/carrito ficticio de las pruebas de Discovery se conserva: es una simulación local, sin pagos reales.
- Los avisos y atribuciones de terceros de `NOTICE` deben conservarse.

## Inventario de archivos Python pendientes

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
- [ ] `scripts/test_reload.py`
- [ ] `scripts/test_soft_interrupt.py`
- [x] `scripts/tests/mcp_cli_acceptance.py` — `mcp-cli-acceptance`; diez casos contra CLI real, OAuth/PKCE/callback/SSE locales.
- [x] `tests/e2e/expand_badge_headed_wtype.py` — Rust; protocolo/foco/limpieza probados con dobles. Compositor Wayland real no disponible en este entorno.
- [x] `tests/e2e/expand_badge_headless.py` — `expand-badge-headless`; probado con PTY y runtime reales, en un daemon aislado.
- [ ] `tests/test_injection_fix.py`
- [ ] `tests/test_injection_thorough.py`
- [x] `tests/test_selfdev_reload.py` — migrado a `orbit-dev`, verificado con runtime real.

### Servicios del teléfono y TestFlight

- [x] `scripts/phone-server/breaker-lambda.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/phone-server/test_wake_lambda.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/phone-server/testflight-setup.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/phone-server/wake-lambda.py` — migrado a Rust en `tools/orbit-dev`.

### Otros scripts y pruebas

- [ ] `docs/reddit_dashboard.py`
- [x] `scripts/test_caching_detailed.py` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/test_oauth_usage.py` — migrado a Rust en `tools/orbit-dev`.

## Python incrustado y lanzadores pendientes

Estas rutas de shell contienen invocaciones de Python, bloques incrustados o configuración de PYTHONPATH. Migrar también estos puntos: borrar únicamente archivos `.py` no elimina la dependencia.

- [x] `scripts/antigravity_multiturn_coverage.sh` — parser JSON Rust; cuatro turnos y fallo JSON probados con CLI sintética.
- [x] `scripts/bench_compile.sh` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/bench_selfdev_build.sh` — Python sustituido por comandos nativos de orbit-dev.
- [x] `scripts/bench_selfdev_checkpoints.sh` — migrado a Rust en `tools/orbit-dev`.
- [x] `scripts/benchmark_tools.sh` — benchmark Rust; 54 ejecuciones reales de herramientas, CSV y contrato con fixture.
- [ ] `scripts/compare_discovery_rate.sh`
- [x] `scripts/compile_time_probe.sh` — Python sustituido por comandos nativos de orbit-dev.
- [x] `scripts/generate_release_notes.sh` — lanzador de `generate-release-notes` nativo.
- [x] `scripts/record_demo.sh` — Python sustituido por comandos nativos de orbit-dev.
- [x] `scripts/replay_recording.sh` — Python sustituido por comandos nativos de orbit-dev.
- [ ] `scripts/run_terminal_bench_claude.sh`
- [ ] `scripts/run_terminal_bench_harbor.sh`
- [ ] `scripts/startup_acceptance_matrix.sh`
- [x] `scripts/stress_test_40.sh` — helpers Rust de socket/JSON; sintaxis shell y protocolo probados.
- [x] `scripts/test_cloud_environment_e2e.sh` — assertions Rust; aceptación con runtime real.
- [x] `scripts/test_cloud_group_e2e.sh` — gate nativo `test-cloud-acceptance` y helpers Rust.
- [x] `scripts/test_cloud_move_agent_e2e.sh` — gate nativo `test-cloud-acceptance` y helpers Rust.
- [x] `scripts/test_cloud_move_e2e.sh` — gate nativo `test-cloud-acceptance` y helpers Rust.
- [x] `scripts/test_cloud_move_tui_e2e.sh` — gate nativo `test-cloud-acceptance` y helpers Rust.
- [x] `scripts/test_cloud_replace_e2e.sh` — assertions Rust; aceptación con runtime real.
- [x] `scripts/test_ssh_slash_workflows.sh` — migrado a `orbit-dev`, verificado con runtime real.
- [x] `scripts/verify_alt_shift_e_reaches_terminal.sh` — capturador de terminal Rust y selección de ventanas nativa; captura verificada con PTY.
- [ ] `scripts/webfetch_corpus.sh`

## Integraciones que requieren atención especial

- [ ] `.github/workflows/cloud-acceptance.yml`: instala Python y ejecuta el harness y las pruebas de aceptación cloud. Actualizar junto con sus reemplazos nativos; no habilitar ni ejecutar infraestructura real como parte de una comprobación local.
- [x] Integración heredada `cloud sessions` y helper privado `jade_sessions.py` retirados por indicación del mantenedor; 424 pruebas CLI y rechazo del comando eliminado verificados.
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

La suite completa de `tools/orbit-dev` pasó en Linux/WSL: **223 pruebas, 0 fallos**; Clippy de todos los targets pasó con `-D warnings`. Se compiló también el cliente real de aceptación del SDK de Desktop y se compararon las notas de versión antiguas/nativas en tres escenarios con salida idéntica. Los flujos cloud, heap y consumo se verificaron con fixtures locales; no se ejecutaron despliegues cloud, inferencia real ni aceptación completa contra daemons de producción. Esto no certifica los 46 scripts Python pendientes ni Windows/macOS. El Clippy del workspace raíz mantiene errores previos fuera de esta migración; no debe presentarse como aprobado. Las pruebas locales de RSS verificaron funcionamiento, no demostraron ausencia de fugas a largo plazo.

## Cómo actualizar el inventario

Desde la raíz del repositorio:

```sh
git ls-files '*.py'
rg -n -i 'python3?|\.py\b|PYTHONPATH' scripts tests .github tools src docs
```

Clasificar los resultados: ejecución real, prueba, ejemplo de lenguaje, comentario o datos externos. El conteo de archivos no sustituye esa revisión.
