# Pi model capability evidence

`pi-model-capabilities.json` is the tracked model inventory used to design and audit routing evaluations.

The snapshot records every model visible to Pi through both the normal extension-enabled list and the core Remote Procedure Call (RPC) registry. Each row keeps the exact provider and model identity, supported Pi thinking levels, context and output limits, input modes, prices, and alias status.

Generate a new snapshot from the repository root:

```console
cargo run --quiet --manifest-path tools/skill-eval/Cargo.toml --bin skill-eval -- \
  model-capabilities --output research/model-routing/pi-model-capabilities.json
```

The command refuses to replace an existing file. Generate to a new path, review the differences, verify the destination, and then replace the tracked snapshot.

The snapshot is evidence, not routing authority. Moving aliases stay visible but cannot qualify as exact routes. `config/model-tiers.json` changes only after model evaluations and owner approval.
