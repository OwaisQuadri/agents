# Direct edit rule contract

## Scope and status

This contract defines pre-write checks for direct Pi `edit` and `write` calls. It covers privacy, comment length, comment shape, and Boolean naming. One compiled Rust registry evaluates the selected rules. One Pi extension owns enforcement.

Issue #319 covers shell commands. The user deferred that issue. Shell commands remain unchanged and outside this contract's enforcement coverage. Existing commit-time checks remain active. The `no-ai-attribution` rules and invocation remain unchanged.

This document specifies required behavior. It does not certify an implementation or a performance result. The user approved the direct-only release scope for #317 and #318 in pull request #349. Human-only emergency approval remains unavailable because the inspected host transport does not establish human origin. The unfinished authority requirement moves to [#351](https://github.com/OwaisQuadri/agents/issues/351). Missing measurements move to [#352](https://github.com/OwaisQuadri/agents/issues/352); they do not count as passed.

## Parser boundary

Issues #310 and #328 selected Tree-sitter for accuracy, with whole-file lexical fallback when parsing fails. The registry reuses the existing comment parser. Swift, Objective-C++, and Perl currently use lexical extraction.

A rule receives the complete proposed text, its language, and the actual changed text ranges. Parser structural changes do not replace text differences. Syntax-aware rules must distinguish unsupported evidence from a failed required check. Unknown type evidence does not establish a Boolean violation. A parser failure must not silently skip comment checks.

The existing comment extractor emits `start_line`, `end_line`, `kind`, and `text`. The integration must validate and convert these fields. It must not cast them to a different field convention.

## Direct file boundary

The extension reuses installed Pi edit matching, normalization, candidate reconstruction, and result rendering. Validation runs inside the supplied file-write operation against the exact proposed bytes. The existing per-path queue orders Pi mutations. It does not lock out external programs.

The extension replaces both supplied write operations. Its directory operation creates nothing. The extension blocks a request whose parent directory does not exist. The extension rejects symbolic-link targets and regular files with multiple hard links. Unsupported operations return a reason before writing.

The extension checks original content and file identity again before writing. A detected conflict blocks the edit. This recheck does not provide an atomic comparison against external writers. The contract does not promise crash-safe direct writes.

Rule rejection must leave the target bytes and metadata unchanged. A filesystem failure after validation is an execution error, not proof that the file stayed unchanged. The caller must distinguish these outcomes.

## Cancellation

The wrapper owns the original cancellation signal. It checks cancellation before and after pre-write asynchronous operations and immediately before writing. It does not pass that signal to the installed tool's post-write abort check.

Cancellation before writing blocks the request. A completed write returns the installed success result, even if cancellation follows. The wrapper must not report a completed write as a rejected proposal.

## Wire protocol

Version 1 uses one JSON (JavaScript Object Notation) request and response on standard streams. Text uses valid UTF-8 (Unicode Transformation Format, 8-bit). Invalid encoding blocks instead of replacing bytes.

```text
Request {
  version: 1,
  request_id: string,
  operation: "edit" | "write",
  path: absolute string,
  repository_root: absolute string | null,
  original_text: string | null,
  proposed_text: string,
  changed_ranges: [{start_byte: integer, end_byte: integer}],
  budget_ms: integer
}

Diagnostic {
  path: string,
  line: positive integer,
  rule: "privacy" | "comment-length" | "comment-shape" | "boolean-name" | "checker",
  reason: string
}

JudgmentInput {
  comment: string,
  code_context: string,
  language: string,
  rule_document: string,
  prompt_version: integer,
  schema_version: integer,
  judgment_configuration: string
}

JudgmentRequest {
  line: positive integer,
  input: JudgmentInput
}

Response {
  version: 1,
  request_id: string,
  decision: "pass" | "block" | "needs_judgment" | "error",
  diagnostics: Diagnostic[],
  judgments: JudgmentRequest[],
  elapsed_ms: integer,
  budget_ms: positive integer
}

CacheEntry {
  version: 1,
  key: digest of every JudgmentInput field,
  decision: "pass" | "block",
  reason: string
}
```

The `checker` category identifies infrastructure errors only, with decision `error`. It is diagnostic-only, not a configurable rule. Protocol version 1 and the diagnostic path, line, and reason fields remain unchanged.

Source line metadata wraps a judgment input. It does not enter the judgment cache key. The wrapper preserves source positions for failed judgments without changing their decision inputs.

The caller validates fields, versions, request identity, integer ranges, and decision combinations. It rejects extra terminal responses and malformed output. A response from another request cannot authorize a write. Only a valid final pass permits writing. `needs_judgment` never permits writing by itself.

## Changed content

Changed ranges are half-open byte ranges in the proposed text. The registry derives affected content from the original and proposed text. Caller-provided ranges cannot hide a difference. A deletion retains a zero-width boundary. Comment selection includes blocks that contain or touch that boundary.

A full write checks the whole candidate. An edit checks actual changed text for privacy while parsing the complete candidate. An unchanged comment needs a fresh decision when its supplied following-code context changes. Source line numbers start at one.

## Rules

### Privacy

The registry reuses `privacy-lint` matching behavior and its existing allowances. It loads named private identifiers when the optional machine-local file exists. A missing optional file means shape-only scanning, without named identifiers.

An unreadable present file blocks the request. An explicit identifier-file argument or environment setting requires a readable file, even when that file is missing. Required configuration and input errors block the new direct-edit caller. Existing command interfaces retain their behavior.

A diagnostic must not repeat the matched private value. The path, line, rule identifier, and safe reason provide the repair location. Tests must distinguish newly introduced text from unchanged violations and removed violations.

### Comment length and shape

The registry preserves existing comment extraction and the current three-line limit. Documentation blocks retain their current length exemption. Adjacent full-line comments retain their existing grouping behavior.

For comment shape, `is_empty_rust_comment` in `registry.rs` blocks empty full-line Rust comments, including empty documentation comments. Every other selected comment requires the existing judgment worker or a verified cache decision. The registry supplies no local shape approvals. An uncertain local filter must not invent approval. Existing pass and block fixtures remain required parity tests.

A judgment receives the comment, following-code context, language, rule document, and versioned judgment configuration. Missing required rule documents block the request. A malformed worker decision is not a pass.

### Boolean naming

Names for proven Boolean variables, fields, properties, parameters, functions, and methods must use the `is` prefix. Evidence includes explicit Boolean types and language-defined Boolean expressions. Truthy values do not establish Boolean types.

The rule preserves external names only within the resolved Rust trait-method coverage declared below. The release provides no general protocol, framework, serialization, or foreign-interface exemption. An annotation or syntax marker alone does not prove an external naming requirement. Local aliases remain separately subject to the rule.

Generated-file selection provides the generated-code exemption. Unknown language or type evidence must remain explicit in coverage reports. The implementation must not claim proof for unsupported forms.

The Boolean checker accepts these case-sensitive extensions:

```text
rs ts tsx js jsx mjs cjs py go java cs c h cc cpp hpp scala kt kts zig
```

It checks simple named declarations, not destructuring or general type inference. Tested forms include variables, fields, properties, parameters, functions, and methods across the supported grammars, not every form in every language. Python assignments and Go short declarations also have fixtures. Names that start with `is` and the name `_` produce no finding.

Explicit evidence uses `bool` for Rust, C, C++, C#, and Zig; C also accepts `_Bool`. TypeScript and Java use `boolean`. Python and Go use unshadowed `bool`. Kotlin and Scala use unshadowed `Boolean` or the qualified types `kotlin.Boolean` and `scala.Boolean`. Shadow detection covers the whole parsed file, not just the declaration's scope.

Expression evidence includes Boolean literals and parenthesized Boolean expressions. Python adds `not`, but not comparisons or `and`/`or`. JavaScript, TypeScript, Rust, Go, and Java add comparisons. Rust, Go, and Java add `&&` and `||`; JavaScript and TypeScript do not. JavaScript, TypeScript, Go, and Java add unary `!`; Rust does not. The checker does not infer Rust function return types from bodies or C++ Boolean types from overloaded comparisons.

Resolved external ownership covers only Rust implementation methods: `eq` and `ne` for `PartialEq`; `lt`, `le`, `gt`, and `ge` for `PartialOrd`. The trait path must resolve to `std::cmp` or `core::cmp`. Supported forms include prelude names, qualified paths, absolute paths, explicit imports, grouped imports, and import aliases. Local bindings, ambiguous imports, wildcard imports, attributed imports, and disabled preludes can prevent resolution. The resolver also checks `no_std` and `no_core`; it does not resolve generic trait arguments or arbitrary external traits.

Framework, serialization, and foreign-interface markers supply no exemption by themselves. Local Boolean declarations inside an exempt method still require the prefix.

Unsupported extensions and trees with syntax errors return no Boolean findings. They do not trigger Boolean lexical fallback or establish naming compliance. Grammar setup failures and missing parse trees return errors. Comment checks retain their separate whole-file lexical fallback.

## Configuration and selection

Tracked global defaults live in `config/edit-time.toml`. An optional tracked `.edit-time.toml` supplies repository overrides. The target path determines the repository, not the session directory. Files outside repositories use global defaults.

Configuration selects compiled rule identifiers, file patterns, and lower total or per-rule time limits. It cannot supply executable commands or approval switches. Defaults exclude dependencies, build output, caches, and generated files. Repository settings can only preserve or strengthen the installed checks and file coverage.

The registry accepts missing optional overrides. Invalid or unreadable present configuration blocks applicable requests. The registry reads rules and effective configuration consistently for each request. Judgment-affecting configuration forms part of the cache identity.

Configuration uses TOML (Tom's Obvious Minimal Language). Each present file requires `version = 1`. The selection fields use this shape from the global configuration:

```toml
version = 1
rules = ["privacy", "comment-length", "comment-shape", "boolean-name"]
include = ["**"]
exclude = ["**/.git/**", "**/node_modules/**", "**/vendor/**", "**/target/**", "**/build/**", "**/dist/**", "**/.cache/**", "**/__pycache__/**", "**/.venv/**"]
generated = ["**/*.generated.*", "**/*.min.js", "**/*.g.cs"]
```

The time-limit fields follow these selection fields in the same file:

```toml
total_ms = 20000
process_budget_ms = 3000

[rule_ms]
privacy = 500
comment-length = 500
comment-shape = 500
boolean-name = 500
```

Every limit must be a positive integer. The global limits cannot exceed these approved ceilings. Repository limits can only lower the global limits. Unknown rule identifiers and invalid limits block the request.

Each supplied `rules`, `include`, `exclude`, or `generated` list replaces its inherited list before validation. Omitted fields retain inherited values: built-in defaults for installed global configuration, then installed values for repository settings. Entries in `rule_ms` lower individual inherited ceilings; omitted entries retain their limits.

Installed global configuration retains unrestricted selection replacement. An empty global `rules` list selects no checks. An empty global `include` list selects no files. Repository configuration has stricter limits:

- `rules` must retain every inherited rule. It can add compiled rules.
- `include` must retain every inherited pattern. It can add patterns.
- `exclude` and `generated` can retain or remove inherited patterns. They cannot add patterns.

Pattern comparison uses exact strings, not glob-language containment. The registry rejects replacement patterns whose coverage it cannot prove under these limits. This includes replacements that would select the same files. An empty repository `rules` or `include` list fails when its inherited list is not empty. Empty `exclude` and `generated` lists remove those exemptions.

This repository override strengthens the global example above. Omitted rule and include lists retain all inherited checks and coverage:

```toml
version = 1
exclude = []
generated = []
total_ms = 10000
```

With those global defaults, `rules = []`, `include = ["src/**"]`, and `exclude = ["**/private/**"]` each block the request. A repository cannot disable inherited checks through selection changes.

Patterns use `globset` with literal path separators. A single `*` does not cross `/`; `**` can match directory levels. Matching uses repository-relative paths, or absolute paths without the leading `/` outside repositories. Selection requires an include match and no exclude or generated match. Directory patterns match components, not substrings: defaults exclude `node_modules/x.ts`, but select `src/node_modules_notes.ts`.

Each pattern list allows at most 256 entries. Each pattern must contain 1 to 1,024 bytes and no null byte. Invalid globs block configuration. Unknown fields, unknown or duplicate rule identifiers, unsupported versions, and invalid field types block configuration. No unknown key acts as a command or approval switch.

## Deadlines and failure results

The approved total validation deadline is 20,000 milliseconds per request. It includes process startup, checks, cache work, and fresh judgments. Proposed inner ceilings are 500 milliseconds per deterministic rule and 3,000 milliseconds for the Rust process. These ceilings do not establish measured latency.

The response supplies the effective total `budget_ms` after configuration. This ceiling cannot exceed the requested budget or the approved total. The extension measures it from the original request start, not response receipt.

Validation remains globally serialized. Queue waiting uses the original request deadline and cancellation signal. An expired or cancelled waiter returns without starting checks and cannot let later requests overtake an active worker. Cancellation and deadline expiry return their own guidance, not build advice.

Every worker shares the remaining request budget. Expiry blocks writing. The parent stops expired workers and drains their output. Partial and late results cannot authorize the request. No error or timeout disables a required rule.

A failed rule reports its identifier and elapsed time. Missing tools, rule crashes, invalid configuration, malformed output, and required-check timeouts block the request. A result must preserve every failure decision even when display space runs out.

The combined blocking diagnostic has a 2,000-character limit. It includes the omitted-diagnostic count. Each displayed failure names the validated target path, line, category, and controlled repair guidance. The display escapes the path and never copies arbitrary checker reason text or matched private values. The display omits and counts any diagnostic whose full location and guidance do not fit. Display truncation never converts a block to a pass.

## Judgment cache

A key includes every `JudgmentInput` field. Changed text, context, language, rules, versions, or judgment configuration invalidate the prior decision. Old text-only keys are misses, not approvals.

Only validated completed worker decisions enter the process-local memory cache, after cleanup and deadline checks. The cache holds at most 256 immutable records and evicts the oldest stored record when full. It is not persisted or restored from session records. A fresh process starts empty. Agent-writable disk records, including legacy `decisions-v1` records with complete keys, provide no approval authority and are never read by this decision path.

A cache miss triggers a fresh judgment within the same request deadline. The request remains blocked until that judgment passes. Failure, malformed output, cancellation, or timeout cannot cache a pass and leaves the target unchanged.

Cached block decisions remain blocks. Historical records retain their original decisions as diagnostic evidence, without authority from incomplete old identities. The approved release replaces exact historical agreement with correctness against approved rules and fixed expected examples. Historical judgment parity remains unverified. Synthetic validation and cache tests establish mechanics, not historical decision parity.

Documentation attached above a public declaration must describe that declaration, even if its content could otherwise qualify as architecture. Architecture comments must yield to clearer code names when those names can express the purpose. The user resolved historical case 7 as a block on that basis. The recorded sample matched 11 of 12 historical decisions; that disagreement remains visible. Four independent cases produced eight recorded decisions across two instruction versions, all with the expected shapes. Those calls show observed behavior, not repeatable answers or universal historical agreement.

## Emergency approval follow-up

The approved scope change moves the following original requirement to [#351](https://github.com/OwaisQuadri/agents/issues/351). No emergency bypass ships in this release.

An emergency approval must originate from a verified human action. The host must record approval. Approval must expire and authorize one exact proposed change. Its identity includes the proposal and original target. A changed proposal or target invalidates approval. Use consumes the approval.

Approval can waive rule decisions only. It cannot waive unsupported filesystem operations. A tool argument, environment switch, command, or agent-writable file cannot provide authority. Repository configuration is not an approval channel. Its exact-pattern checks preserve every inherited rule and prevent reduced file coverage.

The inspected Pi client response transport proves receipt of a response, not a human action. Therefore this implementation exposes no emergency approval path. The scope change removes this criterion from release acceptance without marking it complete. Missing, invalid, failed, or timed-out required checks still block writes.

## Required fixtures

Selection fixtures cover ordinary code and text, excluded directories, generated files, repository overrides, and files outside a repository. A filename that contains an excluded directory's name must not count as that directory. Missing optional configuration must differ from invalid present configuration.

Privacy fixtures preserve current address and configured-name decisions. Comment fixtures preserve current pass, block, grouping, documentation, and lexical-fallback behavior. Cache fixtures change each decision input separately. Boolean fixtures cover each supported language and declaration form, external ownership, local aliases, truthiness, and unknown evidence.

Integrity fixtures cover disjoint, ambiguous, overlapping, and normalized replacements. They cover line endings, byte-order marks, Unicode positions, deletions, and unchanged comments with changed context. Tests also cover stale targets, links, missing parents, cancellation, failed tools, invalid output, and late results.

Every rejected direct-edit fixture checks original bytes and metadata. Successful fixtures compare the validated candidate with the written bytes. Concurrent fixtures use unique paths.

## Measurement and release checks

The approved release accepts existing bounded process and token comparisons only for their recorded scope. It does not certify complete memory, historical retry attribution, or additional cold/warm measurement coverage.

The original measurement requirements below remain in [#352](https://github.com/OwaisQuadri/agents/issues/352). They are follow-up work, not passed release criteria.

Compare one process per request with a persistent checker on the same repeated workload. Report process startup, blocking time, memory, cache state, judgment count, and diagnostic size. Choose the simpler process model if it meets the measured budget. A persistent process must earn its added complexity.

Compare edit-time and commit-time feedback on identical starting files and intended changes. Use the same model tier, task instructions, and test outcomes. Run repeated trials with separate cold and warm cache conditions. Record actual retry counts and main-model tokens from session records.

Edit-time feedback must use fewer median retry tokens without a 95th-percentile regression. Report the workload size and failed trials. Do not replace token measurements with character counts or estimates. Historical session checks establish prior behavior, not a controlled causal comparison.

The release still requires two warm import and registration measurements with `PI_TIMING=1`. The combined limit is 50 milliseconds unless the user approves an explicit justification. Defer other work until first use.

Release checks include existing privacy, rule-based comment, and attribution suites, direct-edit integration tests, strict builds, independent tests, and independent review. Final candidate checks, independent review, visible behavior evidence, and manual signoff remain pending. Unsupported criteria and unmeasured results remain unfinished. Manual signoff precedes landing.

## Complexity review

Issue [#290](https://github.com/OwaisQuadri/agents/issues/290) remains open and outside this release. Its preserved work extends `simplify`, `engineer`, and `code-reviewer` with time and space analysis. The execution-evidence runner also remains outside the direct-edit release. This scope change does not cancel or complete that work.
