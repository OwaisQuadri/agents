# Toolbar capture

The three assets share a 24 × 24 view box, but the screenshot makes the play mark look left-heavy and the search mark look lighter than the menu mark.

```html
<svg viewBox="0 0 24 24" aria-label="Play">
  <path d="M8 5 L19 12 L8 19 Z" fill="currentColor" />
</svg>

<svg viewBox="0 0 24 24" aria-label="Search">
  <circle cx="10.5" cy="10.5" r="5.5" fill="none" stroke="currentColor" stroke-width="1.25" />
  <path d="M15 15 L20 20" fill="none" stroke="currentColor" stroke-width="1.25" />
</svg>

<svg viewBox="0 0 24 24" aria-label="Menu">
  <path d="M4 6 H20 M4 12 H20 M4 18 H20" fill="none" stroke="currentColor" stroke-width="2.4" />
</svg>
```

Asset measurements:

```json
{"asset":"play","view_box_centre_x":12,"paint_centre_x":11.67}
{"asset":"search","line_px":1.25}
{"asset":"menu","line_px":2.4}
```
