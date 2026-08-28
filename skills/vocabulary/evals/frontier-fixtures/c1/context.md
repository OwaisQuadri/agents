# Hero build snapshot

The browser capture at 1440 × 900 shows a 420 px-wide content column. The headline wraps to three lines. Its last line almost touches the paragraph, and the paragraph almost touches the action.

```html
<section class="hero">
  <h1>Move work forward without the busywork</h1>
  <p>Plan, review, and ship from one calm workspace.</p>
  <button>Start free</button>
</section>
```

```css
.hero {
  inline-size: 420px;
  padding: 12px 16px;
}

.hero h1 {
  font-size: 64px;
  line-height: 0.92;
  letter-spacing: -0.055em;
  margin: 0;
}

.hero p {
  line-height: 1.05;
  margin: 4px 0 0;
}

.hero button {
  margin-block-start: 6px;
}
```
