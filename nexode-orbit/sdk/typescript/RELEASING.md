# Nexode Agent SDK: distribución pendiente

El SDK usa el nombre local `@nexode/agent-sdk`. No se ha verificado la propiedad
del namespace npm ni se ha publicado una versión Nexode. Todos los paquetes
permanecen con `private: true`.

## Validación local

```bash
npm ci --ignore-scripts --omit=optional
npm run check
npm pack --dry-run
```

Antes de habilitar publicaciones, configurar un repositorio de releases del agente,
verificar la propiedad del namespace npm y las credenciales, generar los binarios
por plataforma y validar el paquete como consumidor. Solo después se puede retirar
`private: true` de los manifests correspondientes y habilitar la variable de CI
`NEXODE_PUBLISH_ENABLED=true`. El workflow no publica por defecto.

Los paquetes de plataforma y el SDK deben compartir versión. Los cambios de protocolo
requieren mantener sincronizados los tipos TypeScript y Rust. La validación local en
Linux no acredita soporte ejecutado en Windows o macOS.
