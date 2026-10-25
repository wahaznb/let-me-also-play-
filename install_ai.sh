#!/usr/bin/env bash
# Game Opt Tool — connect the tool to your AI agent.
#
# This script does NOT install system packages and does NOT need sudo.
# It builds the release binary, smoke-tests the MCP server over stdio,
# and registers it with OpenCode and/or Claude Code when those CLIs exist.

set -euo pipefail

cd "$(dirname "$0")"

echo "=== Game Opt Tool — AI agent setup ==="
echo

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found. Install Rust first: https://rustup.rs" >&2
    exit 1
fi

echo "[1/3] Building release binary..."
cargo build --release

BIN="$PWD/target/release/game-opt-tool"
if [ ! -x "$BIN" ]; then
    echo "error: build completed but $BIN is missing" >&2
    exit 1
fi
echo "      $BIN"
echo

echo "[2/3] Smoke-testing the MCP server over stdio..."
RESPONSE="$(
    printf '%s\n' \
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}' \
        '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
    | "$BIN" mcp-server 2>/dev/null
)"
if ! grep -q '"game-opt-tool"' <<<"$RESPONSE"; then
    echo "error: MCP server did not answer initialize" >&2
    echo "$RESPONSE" >&2
    exit 1
fi
echo "      ok: server initializes and lists tools"
echo

echo "[3/3] Registering with installed agents..."
REGISTERED=0

if command -v opencode >/dev/null 2>&1; then
    if opencode mcp list 2>/dev/null | grep -q "game-opt-tool"; then
        echo "      OpenCode: already registered; leaving configuration unchanged"
        REGISTERED=1
    else
        OPENCODE_ENV=()
        [ -n "${GHIDRA_INSTALL_DIR:-}" ] && OPENCODE_ENV+=(--env "GHIDRA_INSTALL_DIR=$GHIDRA_INSTALL_DIR")
        [ -n "${JAVA_HOME:-}" ] && OPENCODE_ENV+=(--env "JAVA_HOME=$JAVA_HOME")
        if opencode mcp add game-opt-tool --global "${OPENCODE_ENV[@]}" -- "$BIN" mcp-server >/dev/null 2>&1; then
            echo "      OpenCode: registered globally (game-opt-tool)"
            REGISTERED=1
        else
            echo "      OpenCode: automatic registration failed; add this to your opencode.json:"
            echo "      { \"mcp\": { \"servers\": { \"game-opt-tool\": { \"type\": \"local\", \"command\": [\"$BIN\", \"mcp-server\"] } } } }"
        fi
    fi
else
    echo "      OpenCode: not installed (skipped)"
fi

if command -v claude >/dev/null 2>&1; then
    if claude mcp add game-opt-tool -- "$BIN" mcp-server >/dev/null 2>&1; then
        echo "      Claude Code: registered (game-opt-tool)"
        REGISTERED=1
    else
        echo "      Claude Code: automatic registration failed; run:"
        echo "      claude mcp add game-opt-tool -- $BIN mcp-server"
    fi
else
    echo "      Claude Code: not installed (skipped)"
fi

echo
if [ "$REGISTERED" -eq 0 ]; then
    echo "No agent CLI was registered automatically. Manual configuration:"
    echo
    echo "  opencode mcp add game-opt-tool --global -- \"$BIN\" mcp-server"
    echo "  claude mcp add game-opt-tool -- \"$BIN\" mcp-server"
    echo
fi

echo "Done. In your agent, try:"
echo "  \"Use game-opt-tool to inspect /path/to/Game.exe\""
echo
echo "HTTP mode for remote agents (local-only by default):"
echo "  $BIN mcp-server --transport http --port 9000"
echo "  endpoint: http://127.0.0.1:9000/mcp"
