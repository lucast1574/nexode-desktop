# Linux: paquete AppImage

El formato principal para distribución Linux es un AppImage **nativo**: contiene
la UI Rust, el motor Orbit, GTK 3 y su integración Ayatana de bandeja, DBus y las
dependencias que linuxdeploy puede empaquetar. Incluye también Node.js y el MCP
oficial de Nexode como proceso auxiliar. No añade Electron ni WebView.
Se conserva el archivo tar.gz como alternativa para instalaciones administradas.

## Ejecutar

```sh
chmod +x Nexode-Orbit-0.2.0-x86_64.AppImage
./Nexode-Orbit-0.2.0-x86_64.AppImage
```

Si el sistema no ofrece FUSE, el runtime permite extraer y ejecutar:

```sh
./Nexode-Orbit-0.2.0-x86_64.AppImage --appimage-extract-and-run
```

Guarda el archivo en una ubicación permanente antes de activar el inicio
automático. El inicio usa la ruta del AppImage, no el montaje temporal interno.
No muevas el archivo sin volver a configurar el inicio automático.

## Compatibilidad y límites

La compilación de distribución se realiza en **Ubuntu 22.04 / glibc 2.35** para
ambos ejecutables. El script inspecciona las versiones GLIBC de todos los ELF
empaquetados y falla si alguno exige una versión posterior. Esa base permite
dirigirse a distribuciones glibc modernas como Ubuntu 22.04+, Debian 12+ y sus
derivadas; no significa que cada distribución y escritorio haya sido probado.

El sistema debe proporcionar kernel Linux, glibc compatible, drivers gráficos,
OpenGL, una sesión X11 o Wayland y DBus de sesión. Las credenciales persistentes
necesitan un servicio Secret Service, como GNOME Keyring o una implementación
compatible de KWallet. La biblioteca cliente se puede empaquetar; el llavero y
su desbloqueo pertenecen al escritorio del usuario.

La UI y el motor no necesitan Node.js. El conector MCP oficial usa el Node.js
24.22.2 LTS incluido en `usr/bin/mcp/`, sin requerir una instalación global ni
descargar paquetes al arrancar. Las credenciales no forman parte del paquete:
la aplicación obtiene la conexión personal después de que el usuario ingresa.

GNOME puede necesitar una extensión AppIndicator para mostrar la bandeja. En
escritorios sin bandeja se mantiene la ventana accesible. Alpine/musl, sistemas
anteriores a la base glibc y servidores sin entorno gráfico no son objetivos de
este paquete. AppImage no convierte una compilación de Ubuntu nuevo en un
binario compatible con distribuciones antiguas.

El script admite x86_64 y aarch64 **nativos**; el workflow genera x86_64. Para
ARM64 se necesita un constructor ARM64 con la misma base, y validar ese artefacto
por separado. No se publica un ARM64 por el mero hecho de aceptar su nombre.

## Compilar y empaquetar

Instala en Ubuntu 22.04 las dependencias enumeradas en
`.github/workflows/linux-appimage.yml`, compila ambos proyectos hermanos y ejecuta:

```sh
NEXODE_DESKTOP_BINARY=target/release/nexode-desktop \
NEXODE_ENGINE_BINARY=../nexode-orbit/target/release/nexode \
  bash scripts/package-appimage.sh
```

El resultado aparece en `dist/`, con SHA256 e información de los binarios de
entrada y la versión mínima de glibc observada. El script no borra builds previos
y conserva el AppDir de preparación en `.local/` para inspección.

Herramientas de empaquetado fijadas y comprobadas **antes de ejecutarlas**:

- linuxdeploy `1-alpha-20251107-1`, con SHA256 de los artefactos oficiales x86_64
  y aarch64.
- Plugin GTK, commit `7a3fbc31a9e5075073ff8790f26effbac5f84453`, con SHA256.
- Node.js 24.22.2 LTS desde nodejs.org, con el SHA256 oficial por arquitectura.
- Nexode MCP v0.1.26, commit `237d8d86e9f65906886e8b73d12a2afda2b2e6a4`,
  fuente verificada por SHA256 y dependencias fijadas por package-lock.json.
  Se compila con `npm ci --ignore-scripts`, `npm run build` y se excluyen
  dependencias de desarrollo. Se incluyen licencias de Node y dependencias.

El workflow fija también la imagen de Ubuntu por digest y Rust 1.99.1, y usa
Cargo.lock. Es una receta repetible con herramientas verificadas; no promete
igualdad binaria bit a bit entre ejecuciones: los repositorios apt, metadatos del
compilador y el empaquetado pueden cambiar. Los binarios no se firman aquí.

Referencias: [linuxdeploy](https://github.com/linuxdeploy/linuxdeploy/releases/tag/1-alpha-20251107-1),
[plugin GTK](https://github.com/linuxdeploy/linuxdeploy-plugin-gtk/tree/7a3fbc31a9e5075073ff8790f26effbac5f84453),
[compatibilidad de AppImage](https://docs.appimage.org/introduction/concepts.html).
