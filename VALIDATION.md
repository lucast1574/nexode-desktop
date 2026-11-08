# Validación de Nexode Orbit Desktop

## Arquitectura y alcance

Motor y aplicación Rust; egui/eframe 0.33.3 para conversaciones, cuenta,
configuración y bandeja. El catálogo usa HTML/CSS local. La bienvenida y Servicios utilizan Wry 0.57.1 con
HTML/CSS y activos locales. No hay Electron ni servidor HTTP local. El WebView
no recibe credenciales ni contenido de chats; CSP bloquea la red y el IPC se
limita a acciones conocidas. Se destruye al abrir páginas nativas, entrar al
editor desde la bienvenida, ocultar la ventana o si no carga en diez segundos.

Motor/SDK fijado: `dc71ff1c41068b6eaf4ac3c1f7ca06c091203784`.

## Comprobaciones locales

| Plataforma | Evidencia |
| --- | --- |
| Windows x64 | Clippy sin advertencias, 34 pruebas unitarias aprobadas y una integración real ejecutada explícitamente. Build release de UI y motor; el empaquetado ZIP reproducible incluye Node/MCP verificados. |
| Linux x64 | Clippy sin advertencias, 35 pruebas unitarias aprobadas y la misma integración real. Builds release sobre Ubuntu/WSL y Ubuntu 22.04. Capturas X11 reales a 1380×880 y 1024×640. |
| macOS Apple Silicon | Formato, pruebas, Clippy, release y paquete aprobados en GitHub Actions. ZIP descargado y comprobado: ejecutables ARM64 de UI, motor y Node, permisos ejecutables, MCP y licencias. No se afirma revisión visual en un Mac físico. |

La integración utiliza exclusivamente un proveedor sintético local. Comprueba:
creación y persistencia de chats, modelos antes del primer mensaje, streaming,
historial, cancelación y cierre del daemon. Crear un segundo chat después de
abrir el anterior devuelve otro ID, mantiene vacío su historial y conserva el
primero. Cada nuevo chat abre una conexión SDK al mismo daemon privado.

También se comprobaron instalación sin proyectos, recuperación explícita de
carpetas antiguas, equivalencia de rutas Windows, cuenta inicial integrada,
catálogo de servicios, movimiento reducido del sistema y cierre/restauración
desde bandeja. No se importan automáticamente credenciales o proyectos del CLI.

La captura de egui no incluye el WebView hijo: la revisión híbrida utiliza el
compositor X11 real. En Windows la captura automatizada de la ventana no fue
fiable; la compilación e integración del motor sí se ejecutaron allí.

## Cuenta Nexode y Google

La plataforma desplegada contiene la autorización de dispositivo con PKCE,
expiración de cinco minutos, código explícito y consumo atómico de un solo uso.
Backend: 61 pruebas ampliadas y seis pruebas finales de autorización; lint/build
aprobados. Frontend: ocho pruebas, lint y build aprobados; se repitieron pruebas
y build después de integrar los cambios recientes de main.

En producción se verificaron inicio de autorización, estado pendiente sin
conector, rechazo del verificador incorrecto y página HTTP 200 con CSP anti-frame,
X-Frame-Options DENY y no-store. La página de consentimiento y su botón Google
se inspeccionaron en navegador. No se inició sesión con una cuenta humana de
Google ni se realizó una compra; esas acciones no se simulan como aprobadas.

## Distribución y límites

Los paquetes incluyen motor release, Node 24.22.2 y Nexode MCP v0.1.26. El MCP
empaquetado superó initialize y tools/list (140 herramientas) sin usar una API
key real ni efectuar llamadas a servicios. La base Linux de empaquetado es
Ubuntu 22.04/glibc 2.35; AppImage y tar.gz comparten el mismo AppDir. La receta
incluye WebKitGTK y sus auxiliares, manteniendo su sandbox. El sistema necesita
un escritorio compatible, Secret Service y un portal de archivos. X11/XWayland
activa la bienvenida web; Wayland sin X11 mantiene una bienvenida nativa.

No son instaladores firmados ni aplicaciones notarizadas. macOS requiere 13.5+
por Node 24. No se promete compatibilidad con todas las distribuciones Linux,
ni paridad completa con Codex: no incluye terminal PTY ni actualización automática.

Las cuentas con cuotas no expuestas muestran estado desconocido; no se inventan
porcentajes. Las integraciones pagadas conservan sus permisos y condiciones.

## Evidencia final de distribución

La ejecución [42449653478](https://github.com/lucast1574/nexode-desktop/actions/runs/42449653478)
completó con éxito los jobs Linux y macOS Apple Silicon. Por indicación del
mantenedor se retiró macOS Intel de la matriz y se cancelaron los empaquetados
remotos todavía activos; Windows y AppImage ya tenían paquetes locales completos
verificados. La cancelación no se presenta como un workflow íntegramente verde.

Windows: ZIP verificado, 34 pruebas y Clippy aprobados, integración real del SDK
aprobada. Linux: 35 pruebas, Clippy e integración real aprobados; AppImage y tar.gz
arrancaron sobre Debian 12. El renderer WebKit se verificó con NoNewPrivs=1,
Seccomp=2 y namespace PID propio. El launcher portable pasó concurrencia,
permisos 0700, rutas con espacios y limpieza tras éxito, error y SIGTERM.

Distribución macOS: sólo Apple Silicon, macOS 13.5+. No se entrega un paquete
para Mac Intel. La corrección de prueba del commit 2d9688f sólo normaliza la ruta
temporal de macOS; no cambia los ejecutables Windows/Linux empaquetados antes.

## Layout, conversaciones y agentes

Cuenta y Configuración comparten la barra lateral con los chats. Configuración
es una página con pestañas, sin ventana flotante. El inicio guiado ofrece varios
proveedores y permite omitir la conexión. DeepSeek utiliza API key con costes
propios del proveedor. Las pruebas usan cuentas sintéticas, sin autenticar
suscripciones reales ni consumir inferencia pagada.

Chats generales sin proyecto: directorio privado de conversaciones; seleccionar
carpeta es opcional. Fijados y progreso de inicio guiado persistidos. El logo
Nexode reemplaza la mascota. Fondo común #0F0F0F, incluyendo las vistas HTML.
Servicios comparte el host local con navegación por documentos y allowlist de
acciones; Rust resuelve los enlaces públicos del catálogo.

Comprobación funcional final de layout a 1024×640 y 1380×880 con configuración
privada de pruebas: chat general enviado a proveedor sintético, pin conservado
tras reinicio sin duplicación, selected=null y projects=[] conservados. Cuenta,
General y Motor mantienen sidebar; cerrar sesión sólo aparece en Cuenta.
Búsqueda WhatsApp del catálogo filtra los servicios esperados; Explorar entrega
https://wazend.net/ al navegador de pruebas. Preparar vuelve al compositor con un
plan y no envía el mensaje. No se autenticaron cuentas humanas ni se compró nada.
