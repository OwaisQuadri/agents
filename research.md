# Research: Programmatic testing of `pi-subagents` tools and terminal UI

## Summary
Pi currently supports programmatic extension testing through its Software Development Kit (SDK), extension loader, `AgentSession`, Registered Tool definitions, and Remote Procedure Call (RPC) commands. This gives a real supported route for testing `Agent`, `steer_subagent`, and `/agents` command behavior, but it does **not** provide a supported black-box API for injecting terminal keypresses into the interactive Text User Interface (TUI). Therefore, FleetView navigation and `Escape` require either testing the extension's internal UI components directly or an external pseudo-terminal (PTY) harness; neither is a Pi-supported terminal-test mechanism.

## Findings

1. **Pi explicitly supports programmatic agent testing through the SDK.** `createAgentSession()` loads extensions through `ResourceLoader`; `AgentSession.prompt()` sends prompts; `AgentSession.steer()` queues steering; and session events expose tool execution. The documentation lists “Test agent behavior programmatically” as an SDK use case. This is the supported integration-level route for exercising a real extension-loaded session. [Pi SDK](https://pi.dev/docs/latest/sdk)

2. **A real extension-registered model tool can be tested through a real session.** Load `@tintinweb/pi-subagents` (or a fixture) with `DefaultResourceLoader`/`additionalExtensionPaths`, create a session with a deterministic fake model, and inspect `session.agent.state.tools` (or the documented active-tool accessor in the candidate's tests). Then drive a model response containing a tool call and assert the resulting `tool_execution_start`/`tool_execution_end` events and result. Pi documents extension tools via `pi.registerTool()` and the tool execution lifecycle. [Pi extensions](https://pi.dev/docs/latest/extensions), [Pi SDK](https://pi.dev/docs/latest/sdk)

   The candidate project demonstrates this with a real Pi runtime, not a mocked Pi loader: `test/agent-runner-e2e.test.ts` loads `test/fixtures/e2e-probe-ext.mjs`, calls `runAgent`, captures `session.getActiveToolNames()`, and verifies extension-tool allowlisting. That test is a strong template for `Agent` tool reachability. [candidate `test/agent-runner-e2e.test.ts`](https://github.com/tintinweb/pi-subagents/blob/master/test/agent-runner-e2e.test.ts)

   **Severity: none** for tool registration/reachability. **Residual risk: moderate** if only tool presence is checked; presence does not prove `Agent`'s spawn lifecycle, result retrieval, or error paths.

3. **`Agent` and `steer_subagent` can also be tested at the registered-tool boundary.** The extension factory receives an `ExtensionAPI`; a test double can capture each `registerTool()` definition and invoke its documented `execute` function with a controlled extension context. This is the practical unit-test route for exact arguments, return text, event emission, queued steering, and failures. It is not a special Pi test runner; it tests the public registration/execute contract.

   The candidate already uses this pattern in `test/steer-subagent-wiring.test.ts`: it boots the real `src/index.ts` extension with a small Pi double, captures `Agent` and `steer_subagent`, invokes them, and verifies pending steering, ordered delivery, failure reporting, and `subagents:steered`. [candidate `test/steer-subagent-wiring.test.ts`](https://github.com/tintinweb/pi-subagents/blob/master/test/steer-subagent-wiring.test.ts)

   **Severity: none** for unit-level tool behavior. **Residual risk: moderate**: direct `execute()` tests do not prove that a real model can discover and call the tools; combine them with the real-session test in finding 2.

4. **The `/agents` extension command is programmatically invokable, with limits.** Pi's SDK says `session.prompt("/mycommand")` handles extension commands immediately, including while streaming. RPC mode likewise accepts a `prompt` command containing `/command`; `get_commands` lists extension commands. Thus `/agents` command registration and handler behavior can be tested through SDK or RPC using a fake UI context, and the candidate's real extension can be booted with a captured `registerCommand` handler. [Pi SDK](https://pi.dev/docs/latest/sdk), [Pi RPC mode](https://pi.dev/docs/latest/rpc)

   This does not make built-in TUI commands testable: Pi explicitly says built-in commands such as `/settings` and `/hotkeys` are interactive-mode-only and are not included in RPC `get_commands`. [Pi RPC mode](https://pi.dev/docs/latest/rpc)

5. **FleetView's list logic and `Escape` behavior are testable only at the component/internal boundary, not through a supported Pi terminal API.** Pi's public TUI component contract has `render(width)` and optional `handleInput(data)`, so an extension can unit-test a component by constructing it and feeding escape sequences. [Pi TUI components](https://pi.dev/docs/latest/tui)

   The candidate's `test/fleet-list.test.ts` does exactly this: it supplies a fake `FleetUICtx`, captures `onTerminalInput`, feeds `DOWN`, `ENTER`, and `ESC` (`"\\x1b"`), and asserts FleetView deactivation and overlay behavior. It covers `Esc deactivates`, overlay lifecycle, and steering composer wiring. [candidate `test/fleet-list.test.ts`](https://github.com/tintinweb/pi-subagents/blob/master/test/fleet-list.test.ts)

   However, `FleetList`, `ConversationViewer`, and their wiring are candidate implementation modules, not Pi's supported public test harness. Pi documents how to implement components, but does not document a public API to start the full interactive TUI and inject terminal input. **Severity: blocker** for a supported end-to-end test of “real terminal FleetView + Escape”; use a PTY/browser-like terminal harness only if an external test dependency is acceptable, and classify that as out-of-band.

6. **The candidate's extension-level FleetView wiring is covered without a real terminal.** `test/fleet-wiring.test.ts` boots the real extension, captures UI callbacks, verifies `tool_execution_start` installs the FleetView input hook, verifies a spawned background agent registers the `belowEditor` widget, and verifies `session_shutdown` clears it. This is the best supported-by-the-candidate integration test for wiring, but it does not validate Pi's actual terminal renderer or terminal input dispatch. [candidate `test/fleet-wiring.test.ts`](https://github.com/tintinweb/pi-subagents/blob/master/test/fleet-wiring.test.ts)

7. **`steer_subagent` has a second supported Pi-level equivalent, but it is not the same feature.** `AgentSession.steer()` and RPC `steer` test Pi's generic steering queue. They do not test `pi-subagents`' agent-ID routing, pending-steer queue, ownership checks, or `AgentManager` behavior. Those require the candidate's captured registered-tool test or a real `Agent` spawn plus `steer_subagent` call.

## Sources

- Kept: [Pi SDK](https://pi.dev/docs/latest/sdk) — official session, extension loading, prompt, steering, and programmatic testing API.
- Kept: [Pi extensions](https://pi.dev/docs/latest/extensions) — official tool, command, event, and extension-context contract.
- Kept: [Pi TUI components](https://pi.dev/docs/latest/tui) — official component `render`/`handleInput` contract, but no full-TUI test runner.
- Kept: [Pi RPC mode](https://pi.dev/docs/latest/rpc) — official JSONL commands, extension-command invocation, and explicit interactive-only limitation.
- Kept: [candidate `test/agent-runner-e2e.test.ts](https://github.com/tintinweb/pi-subagents/blob/master/test/agent-runner-e2e.test.ts) — real Pi runtime extension-tool reachability pattern.
- Kept: [candidate `test/steer-subagent-wiring.test.ts](https://github.com/tintinweb/pi-subagents/blob/master/test/steer-subagent-wiring.test.ts) — real extension plus captured registered-tool execution pattern.
- Kept: [candidate `test/fleet-list.test.ts](https://github.com/tintinweb/pi-subagents/blob/master/test/fleet-list.test.ts) — direct component input tests, including `Escape`.
- Kept: [candidate `test/fleet-wiring.test.ts](https://github.com/tintinweb/pi-subagents/blob/master/test/fleet-wiring.test.ts) — extension lifecycle/widget wiring test.
- Dropped: npm and search-result mirrors — retained GitHub candidate sources and official Pi documentation instead.

## Gaps

- Pi does not publish a supported end-to-end TUI/terminal-input test harness or a documented API for injecting `Escape` into an interactive process.
- A real-model test of `Agent` requires either a deterministic fake model/provider or network credentials; the candidate's existing real-runtime test intentionally stops at active-tool gating.
- The candidate tests' exact import paths and public accessor availability depend on the installed Pi version; pin compatible Pi peer versions when reproducing them.

## Acceptance report

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "Concrete supported mechanisms and limitations are documented with file paths: test/agent-runner-e2e.test.ts, test/steer-subagent-wiring.test.ts, test/fleet-list.test.ts, and test/fleet-wiring.test.ts; the FleetView/Escape end-to-end gap is marked blocker."
    }
  ],
  "changedFiles": [],
  "testsAddedOrUpdated": [],
  "commandsRun": [],
  "validationOutput": [
    "Reviewed official Pi SDK, extensions, TUI, and RPC documentation plus current candidate GitHub test sources; repository was not inspected or edited."
  ],
  "residualRisks": [
    "No supported Pi black-box terminal API exists for full FleetView and Escape testing.",
    "Real-session tool presence tests do not alone prove every Agent lifecycle path."
  ],
  "noStagedFiles": true,
  "diffSummary": "Research artifact only; no source changes.",
  "reviewFindings": [
    "blocker: full interactive FleetView + Escape cannot be driven through a documented Pi-supported programmatic mechanism.",
    "none: Agent and steer_subagent have viable registered-tool and real-session test routes."
  ],
  "manualNotes": "Use candidate unit/integration patterns for tools and wiring; use a PTY harness only as an external, unsupported terminal integration layer."
}
```