# Game Opt Tool

A local command-line tool for one narrow job: take a game executable you own,
find a few instruction-level changes that can be made safely **without running
or measuring the game**, apply them to a copy, and keep a manifest that can undo
every byte.

It is a static candidate finder, not an FPS booster. The analyzer only removes
demonstrably dead instructions from PE x86/x86-64 binaries. It never launches
the game, reads a frame-time log, or claims a speedup.

```text
scan → review candidates → emit guarded patch spec → patch a copy → manifest → exact revert
```

If you read only two sections, read [The loop](#the-loop-in-six-steps) and
[What "safe" means](#what-safe-means-and-what-it-does-not-mean).

## Why this exists

Most "game optimizer" tools either make unverifiable claims or quietly change
behavior. I wanted the opposite: a small tool where every byte change is
recorded, tied to the exact build that was scanned, and reversible from a
manifest. If a candidate turns out to be wrong, I want to put the file back
exactly, not reinstall the game.

I test it on single-player games I own, primarily Deus Ex (2000). It is not for
multiplayer, anti-cheat, DRM circumvention, or redistributing game assets.

## The loop in six steps

### 1. Identify the binary

You point the tool at an executable. It detects the container format
(PE/ELF/Mach-O) and the architecture. REA also hashes the exact file so the
patch spec is tied to that build. REA currently accepts **PE, x86 or x86-64**
only.

```text
your game .exe
      │
      ▼
format + architecture + exact-file SHA-256
```

### 2. Scan the executable sections

`rea` decodes executable sections with `iced-x86`, one instruction at a time. It
is a linear sweep: it does not build a control-flow graph, and it does not know
which bytes are code versus embedded data.

```text
PE executable sections
      │  iced-x86 linear sweep
      ▼
instruction stream + data skips
```

### 3. Propose only provably dead instructions

The catalog is deliberately small:

| Candidate | What was found | Replacement |
|-----------|----------------|-------------|
| `dead_move` | A true self-move, e.g. `mov rax, rax` | Same-length NOP |
| `dead_jump` | A near `jmp` whose target is the next instruction | Same-length NOP |

Every replacement is **exactly the same length** as the original bytes, so no
following instruction moves and no relocation is needed.

Some things are deliberately *not* rewritten:

- `test reg, reg` and `and reg, reg` are **not** turned into `xor`. That would
  zero the register and change flags.
- `mov eax, eax` in 64-bit mode is **not** treated as dead. A 32-bit write
  zeroes the upper 32 bits of `RAX`, so removing it can change the register.

### 4. Turn the candidates into a guarded patch spec

Each candidate becomes one entry in a JSON `PatchSpec`:

```json
{
  "schema_version": 1,
  "name": "rea-your-game",
  "target_sha256": "<hash of the exact input file>",
  "allow_signed": false,
  "patches": [
    {
      "address": "0x140001000",
      "expected": "4889c0",
      "replace": "0f1f00",
      "note": "[rea:dead_move ...]"
    }
  ],
  "sections": []
}
```

The `expected` bytes are a version guard: if the binary is not the one that was
scanned, the patch refuses to apply. The patcher independently re-checks the
address, the PE section, the expected bytes, and the target SHA-256.

### 5. Apply to a copy and back up anything you overwrite

The recommended command writes to a **separate output path**, so the original
game executable is untouched.

```text
original .exe ──────────────► unchanged
      │
      └── scan ──► spec ──► patcher ──► patched copy
                                      + manifest
```

If the output path already exists, it is backed up first with a SHA-256 record.
Writes use a temporary file, `fsync`, and rename, so a crash cannot leave a
half-written executable. Signed PE binaries are refused by default because
patching would invalidate the signature.

### 6. Record and revert

The manifest records, for every change:

- the virtual address and file offset;
- the original and replacement bytes;
- the input and output SHA-256;
- the PE metadata needed to recognize the patched file.

Revert verifies the patched file's SHA-256 and the recorded bytes before it
changes anything. If the file was modified after patching, revert refuses
instead of forcing a restore.

```text
manifest ──► verify patched file ──► restore original bytes ──► verify result
```

## Quick start

```bash
# Build
cargo build --release --bins

# Check the environment (no game required)
./target/release/game-opt-tool doctor

# Work on a copy while testing
cp "/path/to/your/game/Game.exe" /tmp/Game.exe

# 1. Scan only. Nothing is written.
./target/release/game-opt-tool rea --game /tmp/Game.exe

# 2. Machine-readable report
./target/release/game-opt-tool rea --game /tmp/Game.exe --json

# 3. Inspect the generated spec without applying it
./target/release/game-opt-tool rea --game /tmp/Game.exe --spec-only

# 4. Apply to a separate copy with a named manifest
./target/release/game-opt-tool rea \
  --game /tmp/Game.exe \
  --output /tmp/Game-patched.exe \
  --manifest /tmp/Game.rea.json

# 5. Revert that copy exactly
./target/release/game-opt-tool rea \
  --game /tmp/Game-patched.exe \
  --revert --manifest /tmp/Game.rea.json
```

Optional backup commands for a file you intend to overwrite in place:

```bash
./target/release/game-opt-tool backup --game /tmp/Game.exe
./target/release/game-opt-tool backup --game /tmp/Game.exe --list
./target/release/game-opt-tool backup --game /tmp/Game.exe --verify
./target/release/game-opt-tool revert --game /tmp/Game.exe --dry-run
./target/release/game-opt-tool revert --game /tmp/Game.exe
```

## Commands

| Command | What it does today |
|---------|--------------------|
| `rea --game <path> [--json] [--spec-only] [--output <file>] [--manifest <json>] [--revert]` | **Primary workflow.** Static scan for safe dead-code candidates; optional guarded patch to a copy and exact revert. No measurement. |
| `unpatch --manifest <json> [--game <file>] [--dry-run]` | Reverts the exact byte changes recorded in a patch manifest. |
| `backup --game <file> [--list] [--verify]` | Creates, lists, or verifies SHA-256 backups. |
| `revert --game <file> [--backup <manifest>] [--dry-run]` | Restores the newest or a specific verified backup. |
| `doctor [--game <path>]` | Checks config, tools, binary, backup directory, and the sampling probe. |
| `analyze --game <path> [--log <csv>] [--log-dir <dir>]` | Detects format/architecture; adds measured statistics only if you give it a log. |
| `decompile --game <path> [--address <hex>] [--assembly] [--json]` | Decompiles one function with Ghidra; defaults to the entry point. |
| `patch --game <path> --output <file> [--spec <json>] [--manifest <json>] [--dry-run] [--json]` | Applies a hand-written spec, or writes a byte-identical copy when no spec is supplied. |
| `verify --original <a> --patched <b>` | Compares bytes only; runtime equivalence is not implemented. |
| `profile --game <path> [--log <csv>] [--log-dir <dir>] [--frames N] [--json]` | Optional MangoHud frame-time statistics. Not part of REA. |
| `compare --baseline <csv> --patched <csv> [--frames N] [--json]` | Optional A/B comparison of two measured runs. |
| `sample --game <path> [--pid N\|--process NAME] [--duration N] [--frequency N] [--top N] [--no-ghidra] [--refresh-functions] [--json]` | Optional Linux per-function self-CPU sampling of a running game. |
| `mcp-server [--transport stdio\|http] [--port P]` | Exposes the same commands over MCP (JSON-RPC). |
| `list-games`, `help <game>` | Lists configured targets / shows target notes. |

## Safety model

- **Original stays intact** when you use a separate `--output` copy.
- **Existing output is backed up** before it is overwritten.
- **Expected-byte guard** — every patch must match the bytes that were scanned.
- **Target SHA-256** — the spec records the exact input file hash.
- **Signed PE refusal** — Authenticode-signed binaries are rejected by default.
- **No symlinks or special files** as patch, backup, or restore targets.
- **Atomic writes** — temp file + `fsync` + rename.
- **Manifest** — every byte change, offset, and hash is recorded.
- **Verified revert** — the patched file's hash and bytes are checked before
  anything is restored.
- **No network, no telemetry, no elevated privileges.**

## What "safe" means, and what it does not mean

**Safe here means:** the replacement is provably equivalent at the instruction
level for the emitted candidate classes, is the same byte length, and is
guarded by the exact bytes that were scanned.

**Safe does not mean:**

- that the game behaves identically at runtime — REA does not run it;
- that a candidate is definitely code and not embedded data — the sweep is
  linear, so a candidate inside data is a false positive;
- that the change improves FPS, frame time, stutter, or latency — nothing is
  measured;
- that the binary is compatible with a different game build or patch version.

The expected-byte guard protects against a **different build**. It cannot
protect against a **false positive in the same build**. Review the reported
addresses before applying a patch to a game you care about.

## Optional measurement

These commands exist and are tested, but they are **not used by REA**:

```bash
# Frame-time statistics from a MangoHud CSV you captured
./target/release/game-opt-tool profile \
  --game "/path/to/your/game/Game.exe" \
  --log-dir /tmp/mangohud

# Compare two measured runs
./target/release/game-opt-tool compare \
  --baseline /tmp/mangohud/before.csv \
  --patched /tmp/mangohud/after.csv

# Sample self CPU time per function while the game runs (Linux only)
./target/release/game-opt-tool sample \
  --game "/path/to/your/game/Game.exe" \
  --duration 20 --top 20
```

Read [METRICS.md](METRICS.md) before treating a metric as a verdict. The
sampler sees self CPU time only; it is blind to GPU time, blocked time, and
kernel time. A hot function is a hypothesis, not a proven frame-time win.

## Decompile a function

`decompile` runs Ghidra in headless mode, imports the binary into a temporary
project, and prints the pseudocode and signature. With no `--address`, it uses
the executable entry point:

```bash
export JAVA_HOME="$HOME/.local/opt/jdk-25.0.5+8"
export GHIDRA_INSTALL_DIR="$HOME/.local/opt/ghidra_12.2_PUBLIC"

./target/release/game-opt-tool decompile \
  --game "/path/to/your/game/Game.exe"

./target/release/game-opt-tool decompile \
  --game "/path/to/your/game/Game.exe" \
  --address 0x109214b6 --assembly
```

If Ghidra cannot start or returns nothing, the command fails. It never falls
back to placeholder pseudocode. Each invocation re-analyzes the binary, which
takes roughly 20–30 seconds for Deus Ex. See [INSTALL.md](INSTALL.md) for the
portable Ghidra/JDK setup.

## MCP server

The tool can expose the same validated commands over MCP, so an MCP client can
inspect, decompile, patch, and revert without shell access.

```bash
# stdio (default)
./target/release/game-opt-tool mcp-server

# HTTP for a local client
./target/release/game-opt-tool mcp-server --transport http --port 9000
# endpoint: http://127.0.0.1:9000/mcp
```

Exposed tools: `game_info`, `decompile`, `list_functions`, `sample`, `profile`,
`compare`, `patch`, `unpatch`, `backup`, `backup_verify`, `verify`.

Responses are capped (60 KB by default, `GAME_OPT_MCP_MAX_BYTES` overrides),
decompilation is paginated, and `profile`/`compare` return statistics rather
than raw CSV rows. The server binds to `127.0.0.1` by default and stores no API
keys or credentials.

## Status

| Area | Status |
|------|--------|
| PE / ELF / Mach-O detection and architecture detection | **Working** |
| Game config loading (`config/games.yaml`) | **Working** |
| `doctor` prerequisite checks | **Working** |
| Backup with SHA-256 + JSON manifest | **Working, tested** |
| Revert with checksum verification and atomic write | **Working, tested** |
| Static REA analysis (`rea`) | **Working, tested** — PE + x86/x64, length-preserving candidates, JSON reports, reversible apply |
| Frame-time profiling from MangoHud CSV | **Working, tested** — optional |
| A/B log comparison | **Working, tested** — optional |
| Per-function CPU sampling (`sample`) | **Working on Linux** — real `perf_event_open` self-CPU samples, no `sudo` |
| Decompilation | **Working for one function at a time** — real Ghidra headless output |
| Verify | **Byte comparison only** — runtime equivalence is not implemented |
| MCP server | **Working, tested** — stdio and HTTP, bounded responses |

There are **no measured speedups** for any game in this repository. The tool
does not fabricate frame-time numbers: `profile` explains what is missing when
no log is available, and `verify` never reports a changed binary as runtime-safe.

## Supported games

| Game | Status |
|------|--------|
| **Deus Ex (2000)** | Reference target: PE32/x86 detection, backup/revert, and Ghidra decompilation of the entry point verified against a real executable |
| Anything else | REA is not limited to a list. It scans any PE x86/x86-64 executable you are allowed to modify |

## Tests

```bash
cargo test
```

Current suite: **220 tests passing, 1 ignored**. The ignored test is the real
Ghidra decompilation check; it needs Ghidra, JDK 25, and a game binary.

The suite covers SHA-256 vectors, backup/restore round-trips, tampered and
truncated backups, symlink refusal, permission preservation, PE/ELF/Mach-O
detection, trampoline encoding, config parsing, verifier truthfulness, MangoHud
header and metric math, log discovery, A/B delta math, decompiler output
parsing, MCP transports and response caps, the real `perf_event_open` probe,
`/proc` map parsing, Ghidra function-map caching, and the REA scan → spec →
apply → revert flow, including refusal after a guarded byte changes.

Run the Ghidra test explicitly:

```bash
export JAVA_HOME="$HOME/.local/opt/jdk-25.0.5+8"
export GHIDRA_INSTALL_DIR="$HOME/.local/opt/ghidra_12.2_PUBLIC"
export GAME_OPT_TEST_BINARY="/path/to/your/game/DeusEx.exe"
cargo test --test cli_decompile \
  decompiles_a_real_binary_entry_point -- --ignored
```

## Documentation

- [docs/rea.md](docs/rea.md) — the static analysis and patching contract
- [ARCHITECTURE.md](ARCHITECTURE.md) — modules and data flow
- [INSTALL.md](INSTALL.md) — build, Ghidra, sampling, platform notes
- [WORKFLOW.md](WORKFLOW.md) — how a scan becomes a reviewed patch
- [METRICS.md](METRICS.md) — optional measurement policy
- [ROADMAP.md](ROADMAP.md) — what is not built yet

## Legal

Personal use only, on games you own. No DRM circumvention, no asset
distribution, single-player only. See [LEGAL.md](LEGAL.md).

## License

MIT — see [LICENSE](LICENSE).
