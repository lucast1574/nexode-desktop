# Nexode Harness: alcance de la distribución

Esta copia local conserva el código del snapshot de `NOTICE`, sin el historial Git original. Se inicia un historial nuevo por componentes en `lucast1574/nexode-harness`. El rebranding cambia identificadores de producto, crates `nexode-*`, imports `nexode_*`, binarios, SDKs `@nexode/agent-*`, variables `NEXODE_*`, rutas `.nexode`, documentación y activos de marca.

## Distribución e integración pendientes

`NEXODE_RELEASE_REPO=owner/repository` es una variable de compilación del actualizador y de ejecución de los instaladores remotos. Sin ella, no se buscan actualizaciones automáticamente y la instalación remota devuelve un error explicativo. Los instaladores locales siguen disponibles. No apuntar al monorepo Cloud salvo que publique los binarios correctos del agente.

`NEXODE_SOURCE_REPO_URL` configura, al compilar, el repositorio de fuentes usado por self-dev si no encuentra un checkout local. Sin URL no clona un repositorio ajeno.

La telemetría queda desactivada en builds normales. Los tests unitarios conservan sus capturas locales para comprobar contratos. El worker conserva su código como referencia, sin dominios ni IDs de bases de datos originales.

Las cuentas, gateways, sincronización y cloud gestionado heredados no están conectados al backend NestJS del monorepo. Sus URLs por defecto usan el dominio reservado `.invalid`; requieren una integración explícita. Los proveedores configurados directamente mantienen sus endpoints originales.

Los workflows y scripts de publicación heredados son plantillas pendientes de configurar: repositorio, firmas, registries y credenciales propios. Los paquetes npm están marcados como privados. Su workflow exige `NEXODE_PUBLISH_ENABLED=true`; los workflows de releases exigen `vars.NEXODE_RELEASE_REPO`. Los taps de Homebrew deben configurarse con `NEXODE_HOMEBREW_REPO`. No se ha publicado, desplegado ni instalado globalmente esta copia.

## Marca

Los SVG se copiaron del monorepo Cloud. PNG, ICO e ICNS se renderizaron desde el SVG orbital. Los medios promocionales originales se retiraron de esta copia; no se han grabado demos Nexode nuevas.

La documentación técnica heredada describe la arquitectura original. Las menciones de despliegues, disponibilidad, rendimiento y servicios remotos en documentos históricos no son verificaciones de esta distribución. Este documento y el README definen su estado real.

## Compatibilidad

El namespace nuevo evita reutilizar automáticamente sesiones y credenciales de otra distribución. Los clientes de los SDK deben regenerarse o usar los nombres nuevos. El puente externo de navegador conserva sus IDs y su protocolo externo; necesita validación en vivo antes de distribuirlo como integración soportada.

El núcleo está en Rust, pero se conservan SDK TypeScript, scripts auxiliares y dependencia de extensión JavaScript. El rebranding no convierte esas piezas a Rust ni añade computer use para Windows/Linux.

## Repositorio GitHub

Repositorio nuevo: `lucast1574/nexode-harness`, privado, rama `main`. El código se importa por componentes con un historial nuevo; no representa la cronología del desarrollo original. GitHub Actions permanece desactivado hasta configurar y revisar los workflows heredados.
