# AI Agent Providers

The tool does not embed or call an LLM. It exposes its work as MCP tools, and
you choose which agent drives them. This keeps binaries, decompiled code, and
prompts under your control.

## Supported agent surfaces

| Agent | Transport | Registration |
|-------|-----------|--------------|
| **OpenCode** | stdio (local) | `opencode mcp add game-opt-tool --global -- <bin> mcp-server` |
| **Claude Code** | stdio (local) | `claude mcp add game-opt-tool -- <bin> mcp-server` |
| **Claude Desktop** | stdio (local) | JSON config with `command` + `args` |
| **Cursor / Windsurf / other MCP clients** | stdio or HTTP | client-specific MCP config |
| **Remote/self-hosted agents** | HTTP POST | `game-opt-tool mcp-server --transport http --port 9000` |

See `INSTALL_AI.md` for copy-paste configurations.

## Choosing an agent

| If you want... | Use |
|----------------|-----|
| Local, open-source agent | OpenCode |
| Tight integration with Anthropic models | Claude Code / Claude Desktop |
| No code leaves the machine at all | A local MCP client plus an offline/local model |
| A scripted pipeline | Any MCP client that can call JSON-RPC tools |

## What the agent sees

The server advertises eleven tools (`game_info`, `list_functions`, `decompile`,
`sample`, `profile`, `compare`, `patch`, `unpatch`, `backup`, `backup_verify`,
`verify`) with JSON schemas. It never exposes raw shell access. Results are
bounded and paginated: decompilation arrives one function page at a time,
function listing returns address/name/size handles only, sampling returns a
bounded ranked summary, and oversized payloads are replaced by a truncation
envelope.

## Data flow

1. The agent asks the tool to inspect, decompile, or sample a running game.
2. The tool runs locally (Ghidra headless for decompilation/name resolution;
   `perf_event_open` for CPU samples).
3. Tool output (pseudocode, sample counts, stats, hashes) is returned to the
   agent.
4. The agent reasons about it and requests a patch.
5. The tool validates expected bytes, writes the output and a manifest.
6. The agent can ask for exact rollback at any time.

Sampling is Linux-only and requires `perf_event_open` access for the user
running the MCP server; it does not need root at `perf_event_paranoid=2`. The
agent must not treat a CPU hotspot as proof of a frame-time win; see
`METRICS.md`.

## Local LLMs (Ollama)

A local LLM is not required for the MCP workflow, because the agent already
provides the model. If you later want a standalone optimization loop without
an external agent, these are reasonable local choices:

| Model | Size | Notes |
|-------|------|-------|
| `deepseek-coder:33b` | ~18 GB | Strongest local code understanding |
| `qwen2.5-coder:32b` | ~17 GB | Comparable, good C reasoning |
| `codellama:13b` | ~7.5 GB | Good balance |
| `phi3:medium` | ~2.7 GB | Low-resource fallback |

That standalone path is tracked in `TODO.md`; it is not wired into the CLI yet.
