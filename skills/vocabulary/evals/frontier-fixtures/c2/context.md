# Checkout action capture

The screenshot shows two adjacent actions. The intended main action does not separate clearly from the panel or the other action.

```html
<div class="checkout-actions">
  <button class="buy">Buy now</button>
  <button class="later">Save for later</button>
</div>
```

```css
.checkout-actions {
  background: #5c5e66;
  padding: 20px;
}

.buy {
  color: #73757c;
  background: #62646c;
  border: 0;
  font: 500 16px/1.2 system-ui;
}

.later {
  color: #777981;
  background: #5f6169;
  border: 1px solid #696b73;
  font: 500 16px/1.2 system-ui;
}
```

Computed color samples from the capture:

```text
panel       #5c5e66
main fill   #62646c
main label  #73757c
other fill  #5f6169
```
