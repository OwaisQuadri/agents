Add `try_parse_port(&str) -> Result<u16, ParseIntError>` and keep `parse_port` compatible. Add tests for valid and invalid input. Run `cargo fmt --check`, `cargo check`, `cargo test`, and `cargo clippy -- -D warnings`.

Write `REPORT.md` with these exact field labels and truthful command results:

Rust baseline: applied.
Checks: <commands and results>.
Exceptions: <none or each exception and reason>.
