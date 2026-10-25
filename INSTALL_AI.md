# Connect an AI Agent (MCP)

The tool exposes a [Model Context Protocol](https://modelcontextprotocol.io)
server. Your agent — OpenCode, Claude Code, or any MCP client — calls the
tool's real commands as tools. No API keys are stored by the tool, and no
system packages or `sudo` are required.

## Prerequisites

| Need | Required for |
|------|--------------|
| Rust (stable) | Building the tool |
| Ghidra + JDK | `decompile` tool only (see `INSTALL.md`) |
| MangoHud CSV logs | `profile` / `compare` tools |

You do **not** need Ollama or any local LLM for the MCP workflow. The agent
you already use is the AI.

## Quick setup

```bash
./install_ai.sh
```

The script builds the release binary, smoke-tests the MCP server, and
registers it with OpenCode and/or Claude Code when installed.

## Manual setup

### OpenCode

```bash
opencode mcp add game-opt-tool --global -- \
  /path/to/game-opt-tool/target/release/game-opt-tool mcp-server
```

If you want the `decompile` tool to work, the server needs Ghidra and Java.
Either export them in the shell that launches OpenCode, or pass them at
registration time:

```bash
opencode mcp add game-opt-tool --global \
  --env GHIDRA_INSTALL_DIR=/opt/ghidra \
  --env JAVA_HOME=/usr/lib/jvm/jdk-25 \
  -- /path/to/game-opt-tool/target/release/game-opt-tool mcp-server
```

`game-opt-tool doctor` reports whether Ghidra and Java are visible.

`mcp-server` defaults to stdio, which is what local agents expect.

Equivalent `opencode.json` entry (this is the shape OpenCode 2.x writes):

```json
{
  "mcp": {
    "servers": {
      "game-opt-tool": {
        "type": "local",
        "command": ["/path/to/game-opt-tool/target/release/game-opt-tool", "mcp-server"]
      }
    }
  }
}
```

### Claude Code

```bash
claude mcp add game-opt-tool -- \
  /path/to/game-opt-tool/target/release/game-opt-tool mcp-server
```

### Claude Desktop / other JSON configs

```json
{
  "mcpServers": {
    "game-opt-tool": {
      "command": "/path/to/game-opt-tool/target/release/game-opt-tool",
      "args": ["mcp-server"]
    }
  }
}
```

### Remote / HTTP agents

```bash
game-opt-tool mcp-server --transport http --port 9000
```

Endpoint: `http://127.0.0.1:9000/mcp` (POST, JSON-RPC 2.0). The default bind is
`127.0.0.1`; only use `--host 0.0.0.0` on a network you trust.

## Tools exposed

| Tool | What it does |
|------|--------------|
| `game_info` | Detect executable, format, architecture, entry point |
| `list_functions` | Address/name/size handles only, paginated — pick one to decompile |
| `decompile` | Ghidra headless → C pseudocode, paginated with `next_offset`; assembly opt-in |
| `profile` | Frame-time stats: 1% low, 0.1% low, p99, stutter |
| `compare` | Baseline vs patched frame-time comparison |
| `patch` | Apply an expected-byte-verified JSON patch spec; writes a reversible manifest |
| `unpatch` | Revert exactly the patches in a manifest |
| `backup` | SHA-256 verified backup next to the binary |
| `backup_verify` | Re-check every existing backup |
| `verify` | Compare original and patched binaries |

## Context safety

The server is built for a small agent context:

- Every response is capped. Oversized output becomes a
  `{"truncated": true, "total_bytes": ..., "preview": ...}` envelope and logs a
  warning to stderr.
- `decompile` returns at most 24 KB of pseudocode by default and includes
  `next_offset`; pass it back as `offset` to continue. Assembly is excluded
  unless `include_assembly` is true.
- `list_functions` returns compact handles and paginates (`limit`, `offset`).
- `profile`/`compare` return statistics, not raw rows.
- Tune the hard cap with `GAME_OPT_MCP_MAX_BYTES` (4 KB–1 MB, default 60 KB).

Nothing large is ever streamed through the model: binaries stay local, Ghidra
runs locally, and the agent sees one bounded result at a time.

## Example agent prompts

```text
Use game-opt-tool to inspect /games/DeusEx/System/DeusEx.exe and report the entry point.

List the functions in DeusEx.exe, then decompile only the largest few, one at a time.

Decompile the entry point, explain what it does, and suggest frame-time
optimizations that preserve behavior.

Profile /tmp/mangohud.csv and tell me whether 1% lows improved.

Patch the binary with this spec, then verify the manifest can revert it exactly.
```

## Security model

- The server binds `127.0.0.1` by default.
- `patch` requires an explicit output path and a spec whose `expected` bytes
  must match; it refuses to guess.
- Every patch writes a manifest with before/after bytes and SHA-256 hashes.
- `unpatch` restores only the bytes recorded in that manifest.
- The tool never downloads or distributes game assets.

## Local LLM (optional, not part of MCP)

Ollama is useful only for a future standalone optimization loop; the MCP path
does not call it. If you want that path later, good code models are
`deepseek-coder:33b` (best accuracy), `qwen2.5-coder:32b`, or
`codellama:13b` (smaller). This is tracked in `TODO.md`.
