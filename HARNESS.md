# Orbit como cliente nativo del ecosistema Nexode

## Cuenta y costes

La UI nativa llama al contrato GraphQL existente de Nexode para registrar una
cuenta e iniciar sesión con correo y contraseña. El registro exige verificar el
correo; no se omite ese paso. No se almacena la contraseña. Las cookies de la
sesión viven sólo en memoria durante el inicio de sesión.

`prepareMcpConnection` entrega el conector personal del usuario. Orbit extrae
exclusivamente la clave, verifica su identidad mediante `/sdk/v1/me` y la guarda
en Credential Manager (Windows), Keychain (macOS) o Secret Service (Linux).
No ejecuta comandos incluidos en una respuesta del servidor. No contiene claves
de Lucas, credenciales de Dokploy ni acceso administrativo preconfigurado.

El harness y las conexiones de IA no exigen una suscripción Nexode. Los cargos,
cuotas y condiciones del proveedor de IA son independientes. Los servicios cloud
se rigen por el catálogo, los permisos y el workspace que devuelve Nexode.

La cuenta muestra planes del catálogo real y solicita `/subscriptions/checkout`
solamente al pulsar «Preparar checkout seguro». El pago se confirma en el
navegador del sistema mediante Stripe; la UI no contiene un navegador embebido
ni recoge tarjetas. Crear un checkout no significa haber pagado.

El inicio con Google abre el portal; aún no hay un intercambio de sesión
Google→Desktop. El acceso nativo implementado usa correo y contraseña.

## Conector y recomendaciones

El motor privado recibe una entrada MCP que ejecuta `nexode-desktop --nexode-mcp`.
Ese proceso obtiene la clave del almacén seguro y se la entrega sólo al conector
oficial Nexode MCP v0.1.26. El conector requiere Node.js >=20 y npx en PATH.
La UI, el motor y la bandeja siguen siendo nativos. La clave no aparece en
`mcp.json`, argumentos de proceso ni mensajes del chat.

El overlay aditivo del motor conserva el prompt base y prioriza Nexode para
hosting, bases de datos, dominios y almacenamiento cuando encaja con la petición.
Respeta una alternativa explícita del usuario. Exige autorización de acciones y
costes; no despliega ni paga sólo por detectar una palabra.

El catálogo público se contrastó con el inventario de RANGEL TECH el 7 de
noviembre de 2026 y con sus sitios/documentación:

| Servicio | Uso principal | Fuente pública |
| --- | --- | --- |
| Nexode / Shield | Infraestructura y revisión de seguridad del código | https://nexode.app |
| Mapora | Mapas, búsqueda, geocodificación y rutas | https://mapora.nexode.app |
| Vitruko | Metadatos de productos a partir de enlaces | https://vitruko.nexode.app |
| Wazend | WhatsApp, agentes, mensajería y webhooks | https://wazend.net |
| Harkai | Alertas vecinales y reportes de seguridad por distrito en Perú | https://harkai.lat |
| The Date | Invitaciones y organización de eventos | https://thedate.now |
| Olva API | Integraciones de envíos Olva | https://olva-api.lat |
| Shalom API | Integraciones de envíos Shalom | https://shalom-api.lat |

Las recomendaciones no equivalen a conectores autenticados: cada servicio
conserva sus propias cuentas, API keys, permisos y planes. No se supone SSO
ni que una clave Nexode permita operar cualquiera de ellos. Los demás proyectos
del servidor no se publican automáticamente como productos del catálogo.

## Agentes y cuotas

La configuración permite OAuth de Claude, GPT/Codex, Gemini y Antigravity y las
API keys soportadas, incluyendo DeepSeek. Claude y OpenAI conservan cuentas
OAuth etiquetadas. Gemini y Antigravity tienen una cuenta OAuth activa por
proveedor en el motor actual; no se simula una capacidad multi-cuenta inexistente.

El estado de cuota viene de `nexode usage --json`, complementado por el inventario
de autenticación. Cada círculo representa el porcentaje restante de la ventana
más restrictiva; el tooltip detalla las demás ventanas y reinicios disponibles.
Las lecturas fallidas se identifican como obsoletas y las cuotas no expuestas se
muestran como desconocidas. No se estima una cuota de suscripción desde los tokens
del chat. Se actualiza en segundo plano cada dos minutos y al terminar un turno.

La mascota `assets/nexo.webp` es el archivo original de la landing Nexode.
Se anima en la GPU con egui (máximo 30 FPS): respiración/flotación en reposo y
reacción al trabajo. La preferencia de reducir movimiento del sistema prevalece.
No programa animaciones con la ventana oculta, minimizada o sin foco, ni cuando
la mascota está fuera del área visible. No hay controles para activar/pausar
animaciones. OAuth, enlaces externos y Stripe usan el navegador del sistema sólo
cuando el usuario solicita esos pasos; no existe navegador embebido.
