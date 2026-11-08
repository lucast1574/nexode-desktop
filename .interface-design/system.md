# Sistema visual nativo de Nexode Orbit

Dominio: proyectos, conversaciones, ramas, cambios de código y herramientas.
Firma: marca orbital original, verde #10dc79 contenido, estado local persistente,
proyecto visible en el compositor y revisión de Git junto al chat.

Implementación: egui/eframe nativo; sólo la bienvenida utiliza HTML/CSS local en Wry. Dibujo nativo de SVG por GPU.
- Lienzo #191b19; sidebar #141614; compositor #232723.
- Sidebar 244px, conversación hasta 820px, panel de revisión redimensionable.
- Texto base 14px, metadatos 11px, título de bienvenida 36px.
- Espaciado de 4px; superficies planas y bordes suaves, sin tarjetas de métricas.
- Foco mediante fondo/contraste. Sin focus outlines ni anillos.
- Menús de egui tematizados y selector de archivos/carpetas del sistema operativo.
- Iconos SVG coherentes; nunca emojis como decoración.
- Mascota Nexo animada con movimiento reducido automático; sin controles de pausa. Repintado por eventos fuera de la bienvenida.
- Ventana mínima 1024 × 640 para conservar la lectura con ambos paneles abiertos.

Codex es una referencia de distribución y jerarquía. Se mantienen el nombre,
la marca y la identidad de Nexode, sin copiar activos de OpenAI.

Acceso de cuenta: columna de 392px centrada, cabecera con Nexo y título 27/30px.
Geist local en la página de cuenta. Formulario sobre superficie #1e211e con radio
16px e interior 20/24px. Google: marca SVG multicolor, botón claro de 44px,
texto #202124. Campos de 40px; acción primaria de 42px, verde sólo cuando válida.
Estado desactivado sobre superficie neutra. Nota de privacidad secundaria al pie.
