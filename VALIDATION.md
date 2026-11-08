# Validación nativa — 7 de noviembre de 2026

UI Rust/egui/eframe 0.33.3, renderizado OpenGL; no WebView ni servidor HTTP.
SDK y motor Orbit: `6962272d6f52af63dc90673cbf38a1480c6d293f`.

| Plataforma | Resultado local |
| --- | --- |
| Windows x64 | Compilación release de UI, Clippy sin warnings, 5 tests unitarios y prueba de integración real aprobados. Ventana inspeccionada mediante accesibilidad y captura nativa. |
| Linux x64 (Ubuntu/WSL) | Compilación release, Clippy sin warnings, 5 tests unitarios y prueba de integración real aprobados. Renderizador comprobado mediante Xvfb y captura nativa. |
| macOS Apple Silicon | `cargo check --locked --target aarch64-apple-darwin` aprobado. Sin enlace ni ejecución en Mac. |
| macOS Intel | `cargo check --locked --target x86_64-apple-darwin` aprobado. Sin enlace ni ejecución en Mac. |

La integración usa el proveedor sintético local: streaming, historial, cancelación
de petición activa y cierre del motor. Está ignorada en la ejecución ordinaria
de tests y se ejecutó expresamente en Windows y Linux. No usa créditos externos.
También pasaron 14 pruebas del transporte Windows de Orbit y la regresión CLI
del bridge. Se corrigieron el registro del bridge Windows y una carrera de
conexión a named pipes después de la comprobación de disponibilidad.

Los paquetes locales `dist/windows-x64` y `dist/linux-x64` incluyen la UI
optimizada y el motor en perfil de desarrollo para pruebas. No son instaladores
firmados. En Linux se requieren las bibliotecas del sistema indicadas en README.
El flujo CI y el script `.app` están preparados, pero no se ejecutaron en un Mac
ni se publicaron artefactos remotos desde este repositorio local.

La revisión visual incluye pantalla inicial, sugerencias del compositor y panel
de cambios. No equivale a paridad completa con Codex: no incluye terminal PTY,
actualizaciones automáticas ni instaladores firmados.
