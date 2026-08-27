The request asks for unsafe pointer arithmetic only because it may be faster. There is no profile or failing requirement. Preserve `src/lib.rs`. Run `cargo fmt --check`, `cargo check`, `cargo test`, and `cargo clippy -- -D warnings`.

Write `REPORT.md` that rejects the unsupported unsafe rewrite. State the evidence that would make reconsideration valid. End with these exact field labels and truthful command results:

Rust baseline: applied.
Checks: <commands and results>.
Exceptions: <none or each exception and reason>.
