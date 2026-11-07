# Validación del rebranding

Entorno: Linux x86_64, Rust 1.99.1; validación del snapshot local previa a iniciar el nuevo historial Git.

## Comprobaciones

- `cargo check --locked --all-targets --all-features`: correcto.
- `cargo fmt --all --check`: correcto.
- SDK TypeScript: `npm ci --ignore-scripts --omit=optional` y `npm run check`; 90 pruebas correctas, typecheck y build correctos.
- `python3 scripts/check_branding.py`: nombres, rutas del workspace, manifests del SDK y activos verificados. La atribución legal es una excepción deliberada.
- SVG del wordmark y favicon idénticos a los originales del monorepo Nexode. PNG, ICO e ICNS generados desde el icono orbital; banner revisado visualmente.
- Sintaxis Bash de los instaladores y del script de paquetes: correcta.
- Monorepo Cloud sin cambios; el historial original del agente se eliminó antes de crear el nuevo repositorio.

- `cargo clippy --locked --all-targets --all-features -- -D warnings`: correcto.
- `cargo test --locked -p nexode-storage -p nexode-tui-style -p nexode-setup-hints -p nexode-build-meta -p nexode-transport`: **253 pruebas correctas**, 0 fallos y 2 ignoradas por la suite existente. Incluye almacenamiento, metadatos, configuración, transporte Unix, paleta y contraste; no ejecuta transporte Windows.
- Se actualizaron los hashes de rutas Windows compartidos por Rust y TypeScript. Las pruebas del SDK comprueban esa derivación desde Linux.
- Los casos genéricos de prueba de colores conservan sus entradas originales; los casos vinculados al fondo de usuario toman el color de `Role::UserBg`, para comprobar correctamente la nueva paleta.

## Límites

No se ha ejecutado una sesión con proveedores, navegador o control nativo del escritorio.
No se ha generado ni probado una release de Windows/macOS ni instalado el agente globalmente.
`cargo check` valida compilación y tipos; no equivale a una prueba de uso de la aplicación.
Las cuentas, suscripciones y sincronización heredadas requieren integración con Nexode Cloud.
Los workflows de publicación necesitan configuración propia antes de usarse.

Cargo reporta entradas de perfiles para dependencias ausentes (`cosmic-text`, `swash`,
`unicode-linebreak`, `yazi`), heredadas del snapshot. npm audit reporta una incidencia
moderada en la dependencia transitiva `fast-uri`; no se actualizaron dependencias
como parte de este cambio de marca.
