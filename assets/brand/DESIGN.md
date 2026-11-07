# Nexode — identidad del agente

Fuente visual: monorepo `ronnaldrangel/nexode`, commit `f913b50`.

## Activos

- `nexode-wordmark.svg`: trazados originales de `frontend/public/logo/logo_white.svg`.
- `nexode-wordmark-dark.svg`: mismos trazados, en el verde oscuro del icono.
- `nexode-mark.svg`: icono orbital original de `frontend/public/favicon.svg`.
- `nexode-mark-inverse.svg`: mismo icono, apto sobre fondos oscuros y claros.
- `../app-icons/Nexode.png`, `.ico`, `.icns`: exportaciones del SVG orbital.
- `nexode-orbit-banner.svg`: composición del logo y el icono, no una captura de la aplicación.

## Colores y tipografía

- Verde orbital: `#10dc79`, del stroke del favicon. Acento de la TUI y del banner.
- Fondo verde profundo: `#071b14`, del círculo del favicon. Fondo del banner y panel de usuario.
- Blanco: `#ffffff`, de los trazados originales del wordmark.
- Panel web: tokens originales en `source-tokens.css`, copiados de `frontend/app/globals.css`.
- Web: Inter, como `frontend/app/layout.tsx`. El wordmark usa trazados, no una fuente sustituta.
- Terminal: fuente monoespaciada del usuario; no se fuerza ni se instala una fuente.
- Conservar colores semánticos diferenciados para errores, avisos, éxito y proveedores.

## Uso

Conservar proporciones y geometría de ambos SVG. No recortar el anillo orbital. Wordmark blanco sobre fondos oscuros y variante oscura sobre fondos claros. La terminal usa texto y gráficos de terminal; no puede incrustar SVG en todos los emuladores.

El motivo de espera es orbital y verde. Conservar adaptación automática a terminales con menos capacidad y preferencias de movimiento; no añadir controles visibles de pausa/reproducción.

No usar emojis decorativos, glassmorphism, focus rings ni sombras que simulen foco. Comunicar foco con color, fondo o contraste conservando el teclado.

El tamaño mínimo y el área de protección no están especificados en el monorepo; se conservan proporciones y espacios del activo original.
