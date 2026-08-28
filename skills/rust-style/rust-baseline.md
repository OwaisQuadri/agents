---
paths:
  - "**/*.rs"
---

# Rust baseline

These rules apply to Rust code. The manual rules in `docs/code-style.md` override them.

- Follow the Rust Application Programming Interface Guidelines for public interfaces.
- Format all Rust code with `rustfmt`.
- Run `clippy` for each changed target. Fix each finding unless the exception has a recorded reason.
- Build with warnings promoted to errors (`RUSTFLAGS="-D warnings" cargo build`, or the crate-local equivalent). Fix every rustc warning, not only clippy findings; a kept lint gets `#[expect(..., reason = "...")]`, never a silent pass.
- Use unsafe code only when no safe design meets the requirement. Record its safety requirements and how the code satisfies them.
- Use `#[expect]` with a reason for each intentional lint exception.

## Sources

- Use `https://microsoft.github.io/rust-guidelines/guidelines/checklist/` as the source for the full Microsoft checklist that informs this baseline and its future revisions.
- Use `https://rust-lang.github.io/api-guidelines/checklist.html` for the upstream checklist.
