# Sistema visual nativo de Nexode Orbit

Dominio: proyectos, conversaciones, ramas, cambios de código y herramientas.
Firma: marca orbital original, verde #10dc79 contenido, estado local persistente,
proyecto visible en el compositor y revisión de Git junto al chat.

Implementación: egui/eframe nativo, sin HTML ni CSS. Dibujo de SVG por GPU.
- Lienzo #191b19; sidebar #141614; compositor #232723.
- Sidebar 244px, conversación hasta 820px, panel de revisión redimensionable.
- Texto base 14px, metadatos 11px, título de bienvenida 36px.
- Espaciado de 4px; superficies planas y bordes suaves, sin tarjetas de métricas.
- Foco mediante fondo/contraste. Sin focus outlines ni anillos.
- Menús de egui tematizados y selector de archivos/carpetas del sistema operativo.
- Iconos SVG coherentes; nunca emojis como decoración.
- Sin animaciones ornamentales ni controles de pausa; repintado por eventos.
- Ventana mínima 1024 × 640 para conservar la lectura con ambos paneles abiertos.

Codex es una referencia de distribución y jerarquía. Se mantienen el nombre,
la marca y la identidad de Nexode, sin copiar activos de OpenAI.
