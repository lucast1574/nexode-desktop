# Validación de Nexode Orbit Desktop

## Arquitectura y alcance

Motor y aplicación Rust; egui/eframe 0.33.3 para conversaciones, cuenta,
servicios, configuración y bandeja. Sólo la bienvenida utiliza Wry 0.57.1 con
HTML/CSS y activos locales. No hay Electron ni servidor HTTP local. El WebView
no recibe credenciales ni contenido de chats; CSP bloquea la red y el IPC se
limita a acciones conocidas. Se destruye al abrir páginas nativas, entrar al
editor desde la bienvenida, ocultar la ventana o si no carga en diez segundos.

Motor/SDK fijado: `dc71ff1c41068b6eaf4ac3c1f7ca06c091203784`.

## Comprobaciones locales

| Plataforma | Evidencia |
| --- | --- |
| Windows x64 | Clippy sin advertencias, 31 pruebas unitarias aprobadas y una integración real ejecutada explícitamente. Build release de UI y motor; el empaquetado ZIP reproducible incluye Node/MCP verificados. |
| Linux x64 | Clippy sin advertencias, 32 pruebas unitarias aprobadas y la misma integración real. Builds release sobre Ubuntu/WSL y Ubuntu 22.04. Capturas X11 reales a 1380×880 y 1024×640. |
| macOS Apple Silicon e Intel | Matriz CI nativa configurada para ambas arquitecturas. La versión anterior completó build/tests y paquetes en ambas arquitecturas; la validación de esta revisión híbrida se confirma mediante el run asociado al commit. No se afirma revisión visual en un Mac físico. |

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
