# Telemetría en Nexode Orbit

Nexode Orbit no distribuye un servicio de recolección de telemetría.
El worker heredado, sus dashboards, migraciones D1, scripts de despliegue y dependencias JavaScript se han retirado.

El cliente Rust mantiene sus interfaces internas para no romper el runtime, pero el envío de telemetría está desactivado en las compilaciones de esta distribución. No se requiere Cloudflare ni una base D1 para usar Orbit.

La observabilidad local (logs y diagnóstico) y el monitor SSH de una máquina elegida por el usuario son funciones diferentes. El monitor SSH consulta esa máquina para representar su estado en la terminal; no es un servicio de analítica centralizado.

No se ha eliminado ningún servicio remoto ni datos externos: esta retirada afecta al código distribuido en este repositorio.
