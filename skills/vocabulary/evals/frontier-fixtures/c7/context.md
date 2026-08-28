# Live stats capture

The value column is right-aligned. Repeated captures show the left edge moving while the right edge stays fixed.

```html
<table class="stats">
  <tr><th>Requests</th><td data-live-value>111</td></tr>
  <tr><th>Errors</th><td data-live-value>808</td></tr>
</table>
```

```css
.stats td {
  width: 8ch;
  text-align: right;
  font-family: Inter, sans-serif;
  font-variant-numeric: normal;
}
```

```json
{"at_ms":0,"value":"111","left":684.2,"right":720.0,"width":35.8}
{"at_ms":500,"value":"808","left":671.6,"right":720.0,"width":48.4}
{"at_ms":1000,"value":"101","left":676.9,"right":720.0,"width":43.1}
```
