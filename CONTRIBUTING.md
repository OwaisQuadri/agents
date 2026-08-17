# Contributing executable tools

`config/tools.toml` is the manifest for executable tools. `tool-sync` installs its entries on macOS and Linux.

## Add a manifest entry

Add one `[[tools]]` table for each tool:

```toml
[[tools]]
name = "example"
platforms = ["macos", "linux"]
commands = ["bin/example"]
mcp_server = "example"
pi_extension = "pi/extensions/example.ts"
source = { url = "https://github.com/example/example.git", revision = "FULL_COMMIT_ID" }
installer = { command = "./install.sh", args = [], preview_args = ["--dry-run"] }
```

The fields have these meanings:

- `name` is the unique tool name. It also names the managed Git checkout.
- `platforms` contains one or both supported values: `macos` and `linux`.
- `commands` lists repository-relative executable paths that the installer creates.
- `source` selects one embedded source or one Git source.
- `installer.command` names the installer that runs from the source directory.
- `installer.args` contains the arguments for an apply run. The default is an empty list.
- `installer.preview_args` contains the arguments for a dry run. The default is an empty list.
- `mcp_server` is optional, nonempty metadata. `tool-sync` validates it but does not render MCP configuration from it.
- `pi_extension` is an optional path inside this repository. Set it when the tool needs a Pi tool interface.

Names and command basenames must not conflict with another entry. Command, embedded-source, and Pi-extension paths must stay inside their repositories.

### Choose a source

Use an embedded source when this repository contains the tool:

```toml
source = { path = "tools/example-runtime" }
```

The installer runs in that directory. `tool-sync` links each declared command from the same directory.

Use a Git source when another repository contains the tool:

```toml
source = { url = "https://github.com/example/example.git", revision = "FULL_COMMIT_ID" }
```

Set `revision` to the tested commit ID. An apply run checks out that exact revision with a detached `HEAD`.

Git sources use `$HOME/.cache/tool-sync/<name>`. A later run fetches the existing checkout instead of cloning another copy.

`tool-sync` checks the cached checkout before it fetches or changes revisions. It refuses a dirty checkout, including untracked files.

It also refuses a checkout whose `origin` differs from the manifest URL. Resolve the cache state yourself before another run.

## Add a Pi extension

Pi does not use the command declaration as a tool interface. Add an extension when Pi must call the executable as a tool.

Place the TypeScript extension in this repository. Set `pi_extension` to its repository-relative path.

An apply run links the extension into `$HOME/.pi/agent/extensions/`.

The extension must register its Pi tool and call the installed command. Keep process errors and output parsing inside the extension.

## Preview and apply

Run the complete installer from the repository root:

```sh
REPO_TARGET="$PWD" ./install.sh --dry-run
```

This command prints the top-level plan. When the `tool-sync` binary exists, it also prints the executable-tool plan.

The dry run passes `preview_args` to installers for existing embedded or cached sources.

A dry run does not clone a missing Git source. It prints the clone and later actions without changing the isolated home.

Apply the plan after you review it:

```sh
REPO_TARGET="$PWD" ./install.sh
```

The apply run clones or fetches Git sources. It runs each installer with `args` and links commands into `$HOME/.local/bin/`.

The apply run replaces an existing managed symlink when its target changes. It refuses a command or Pi-extension destination that is not a symlink.

For a focused manifest check, build and run `tool-sync` directly:

```sh
cargo run --quiet --manifest-path tools/tool-sync/Cargo.toml -- \
  --repository-root "$PWD" \
  --manifest config/tools.toml \
  --home "$HOME" \
  --dry-run
```

Use `--check` when you only want plan validation and rendering. That mode does not run installer previews.

## The `rag` entry

The tracked manifest installs `rag` on macOS and Linux. Its source uses this pinned revision:

```text
91fe6c77eb5c075d6a12dd581f62eedd4eafc926
```

The entry links the `rag` command into `$HOME/.local/bin/rag`. It also links `pi/extensions/rag.ts` into Pi's extension directory.

The Pi extension registers `search_memory`. It runs `rag search` with JSON output and returns the parsed search hits.

`search_memory` requires `query`. It accepts `k`, which defaults to `8`, and an optional `source_filter`.
