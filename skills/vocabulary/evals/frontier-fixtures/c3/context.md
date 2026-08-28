# Load recording

The recording was sampled before and after the delayed type resource completed.

```json
{"at_ms":0,"resource":"system-ui","headline":{"top":104,"height":92},"card":{"top":228,"height":184}}
{"at_ms":731,"resource":"/assets/BrandSans.woff2","event":"decoded"}
{"at_ms":756,"resource":"BrandSans","headline":{"top":104,"height":116},"card":{"top":252,"height":184}}
```

The card moves down by 24 px without user input. A later image request also changes the media box from 0 × 0 to 640 × 360 before the final capture.

```text
0 ms    headline and card painted
731 ms  BrandSans.woff2 decoded
756 ms  headline wraps differently; card top changes
911 ms  photo dimensions become available; footer top changes
```
