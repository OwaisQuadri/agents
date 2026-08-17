<!-- TODO(AGNT-0008.T08): replace this draft with the supported tool authoring path. -->
# Contributing to the Agents Repo

This document explains how to add tools, skills, agents, and workflows to the agents framework, making them available across all supported harnesses: Claude, Pi, and Codex.

## Overview

This repo uses a **symlink-based installation** model where:
- Tools are compiled Rust binaries
- Skills are single-purpose capabilities with dedicated tools
- Agents are specialized workers with their own tool sets and models
- Workflows are multi-agent graph specifications

All artifacts are version-controlled in this single repository and symlinked to each harness's respective root.

## Adding a New Tool

### Prerequisites
- Rust toolchain (needed to build the tool)
- Git
- Basic understanding of the agents framework

### Step 1: Create the Tool
1. **Create the tool in `tools/` directory**
   ```bash
   mkdir -p tools/your-tool
   cd tools/your-tool
   cargo new --bin your-tool
   ```

2. **Implement your tool's functionality**
   - Your tool should be a Rust binary
   - Use standard MCP protocol if it needs to be exposed as a server
   - Place compiled binary at `target/release/your-tool`

### Step 2: Register the Tool in MCP Manifest
Add your tool to `config/mcp-servers.toml`:

```toml
[servers.your-tool-name]
command = "/opt/homebrew/bin/node"
args = ["/opt/homebrew/lib/node_modules/your-tool/dist/server.js"]
platforms = ["macos", "linux", "windows"]

# Or if it's a Rust binary:

[servers.your-tool-name]
command = "your-tool"
args = []
# Note: The binary will be available via PATH through install.sh
```

**What this does:** The `install.sh` script uses `mcp-sync` to render this manifest into `~/.claude.json` and `~/.codex/config.toml`, making your tool available to Claude and Codex agents.

### Step 3: Make the Tool Available Across All Harnesses

#### For Claude
1. Run `./install.sh` to register the tool for Claude
2. The tool will be symlinked to `~/.claude/tools/` via the skills root
3. Claude will discover it through the MCP registration in `~/.claude.json`

#### For Codex
1. Same `./install.sh` run registers it for Codex
2. Tool appears in `~/.codex/config.toml`
3. Codex discovers it through its own MCP client

#### For Pi
Pi uses the same tool registration mechanism but requires an additional step:

1. Add to `~/.pi/plugins/` if your tool has Pi-specific capabilities:
   ```bash
   echo '{"your-tool": "~/.local/bin/your-tool"}' >> ~/.pi/plugins/tools.json
   ```

2. Pi's harness picks up tools registered in its plugin directory automatically

### Step 4: Update Handlebars/MCP Configuration
If your tool needs specific permissions or requires additional configuration:

#### For Claude
Edit `config/settings.json`:
```json
{
  "permissions": {
    "allow": [
      "Bash(source:your-tool-name)",
      "FileWrite(~/your-tool-workspace)"
    ]
  }
}
```

#### For Pi
Update `~/.pi/settings.json`:
```json
{
  "enabledPlugins": [
    "your-tool-name"
  ],
  "toolPaths": {
    "your-tool-name": "~/.local/bin/your-tool"
  }
}
```

#### For Codex
Codex typically inherits configuration from Claude's settings but you may need to update `config/settings.local.json` for any Codex-specific overrides.

## Adding a New Skill

Skills are single-purpose capabilities that map to tools:

### Step 1: Create Skill Directory
```bash
mkdir -p skills/your-skill-name
```

### Step 2: Create Skill Definition
Create `skills/your-skill-name/SKILL.md` with:
- `description`: What the skill does
- `trigger`: When the skill should be invoked
- `recipe`: The tool command(s) to execute

Example:
```markdown
---
description: "Generate a git changelog"
trigger: "git log --oneline"
recipe: "git-changelog --format markdown"
---

## Skill Implementation

This skill generates formatted git changelogs from commit history.
```

### Step 3: Update Skill Root
The `install.sh` script automatically symlinks all skills directories from the repo to:
- `~/.claude/skills/`
- `~/.codex/skills/`
- `~/.pi/skills/` (if applicable)

## Adding a New Agent

Agents are specialized workers with their own tools and models:

### Step 1: Create Agent Definition
Create in `agents/your-agent-name/your-agent-name.md`:
- `model`: The AI model to use
- `tools`: Which skills/agents this agent can use
- `instructions`: Specific role-based instructions

Example:
```markdown
---
model: "claude-3-5-sonnet"
tools: ["git-tools", "web-search", "file-read-write"]
instructions: |
  You are a senior software engineer specializing in code reviews.
  Focus on code quality, performance, and best practices.
---

## Agent Implementation

This agent specializes in comprehensive code review analysis.
```

### Step 2: Update Harness Configuration
Agents are loaded based on their definitions in the repo. No additional configuration needed for basic functionality.

## Adding a New Workflow

Workflows are multi-agent graph specifications:

### Step 1: Create Workflow Definition
Create in `workflows/your-workflow-name/SKILL.md` with:
- `description`: What the workflow accomplishes
- `steps`: The agent sequence and dependencies
- `graph`: The directed acyclic graph (DAG) specification

Example:
```markdown
---
description: "Research and analysis workflow"
steps:
  - agent: "web-research-summarizer"
    input: "research question"
    output: "cited findings"
  - agent: "anchor-verifier"
    input: "research findings"
    output: "verified anchors"
---

## Workflow Implementation

This workflow performs comprehensive web research and verification.
```

### Step 2: Update Harness Configuration
Workflows are discovered by their skill definitions in the workflows directory.

## Testing Your Changes

### Step 1: Dry Run
Always test your changes with:
```bash
./install.sh --dry-run
```

This shows what will be installed without making changes.

### Step 2: Actual Installation
```bash
./install.sh
```

### Step 3: Verify Tool Availability
Check that your tool is available in each harness:

#### Claude
```bash
claude mcp list | grep your-tool-name
```

#### Codex
```bash
codx mcp list | grep your-tool-name
```

#### Pi
```bash
pi tools list | grep your-tool-name
```

### Step 4: Run the Tool
Test that your tool actually works:

#### Claude
```bash
claude "use your-tool-name with input: test"
```

#### Codex
```bash
codex "use your-tool-name with input: test"
```

#### Pi
```bash
pi "use your-tool-name with input: test"
```

## Common Issues and Solutions

### Tool Not Available in Harness
**Check:**
1. Did you run `./install.sh`?
2. Is the tool compiled? (`tools/your-tool/target/release/your-tool`)
3. Is the MCP entry correctly formatted in `config/mcp-servers.toml`?

### Permission Denied
**Fix:**
1. Update the appropriate settings file (`~/.claude/settings.json` or `~/.pi/settings.json`)
2. Ensure the tool is allowed in the harness-specific configuration

### Tool Binary Not Found
**Check:**
1. Tool was compiled correctly?
2. Is the binary in `~/.local/bin/`?
3. Does the MCP `command` match the actual binary path?

### Tool Not Discovered by MCP
**Verify:**
1. The `mcp-sync` binary is up-to-date
2. The harness configuration files (`~/.claude.json`, `~/.codex/config.toml`) have been regenerated
3. The tool is registered in `config/mcp-servers.toml` without syntax errors

## Continuous Integration

All tools are tested through the `hooks/test.sh` script. The tool build process includes:
- Cargo tests for Rust tools
- Syntax validation for configuration files
- Dry-run installation verification

Run the full test suite:
```bash
./hooks/test.sh
```

## Getting Help

If you encounter issues:
1. Check the existing logs in `tools/<tool-name>/logs/`
2. Refer to the repo's `README.md` for high-level overview
3. Check the `docs/` directory for detailed guides
4. For persistent issues, create a GitHub issue with the error logs

## Committing Your Changes

When submitting a PR:
1. Create a new branch: `git checkout -b feature/your-tool-name`
2. Commit your changes: `git add tools/config/SKILL.md && git commit -m "Add your-tool-name"`
3. Push to your branch: `git push origin feature/your-tool-name`
4. Create a pull request
5. Wait for CI tests to pass
6. Merge once all tests pass and reviews are complete

## License

By contributing, you agree that your contributions will be licensed under the project's license.