# Game Opt Tool

> A work-in-progress tool for inspecting game binaries, measuring frame-time
> consistency, and safely trying performance changes. The measurement,
> backup/revert, and single-function Ghidra decompilation layers are real and
> tested; optimization and binary rewriting are still prototypes.

## Current status (verified by `cargo test`)

| Area | Status |
|------|--------|
| PE / ELF / Mach-O detection and architecture detection | **Working** |
| Game config loading (`config/games.yaml`) | **Working** |
| `doctor` prerequisite checks | **Working** |
| Backup with SHA-256 + JSON manifest | **Working, tested** |
| Revert with checksum verification and atomic write | **Working, tested** |
| Frame-time profiling from MangoHud CSV | **Working, tested against a real MangoHud log** |
| A/B log comparison | **Working, tested** — deltas for average FPS, 1%/0.1% lows, p99, and stutter |
| `analyze` | Detection plus real frame-time summary when a log is available |
| Per-function CPU sampling (`sample`) | **Working on Linux** — real `perf_event_open` self-CPU samples, per-thread, no `sudo`; Linux-only and blind to GPU/blocked time |
| Verify | **Byte comparison only** — runtime equivalence is not implemented |
| Decompilation | **Working for one function at a time** — real Ghidra headless output; needs Ghidra 12.2 + JDK 25 |
| Optimizer passes | **Stubs** — placeholder speedup values |
| PE byte patches / code caves | **Working for PE** — `--spec` applies bounds-checked byte patches and optional appended executable sections; JSON manifest enables exact rollback via `unpatch`. Copy mode still writes a byte-identical copy. |
| MCP server for AI agents | **Working, tested** — stdio and HTTP transports; eleven tools wired to the real sampler, decompiler, profiler, patcher, backup, and verifier; responses are size-capped and decompilation is paginated |

The tool does not fabricate frame-time measurements: when no log is available,
`profile` fails with instructions and `analyze` explains how to generate one.
Decompilation is real Ghidra output, not generated pseudocode. Optimizer
speedups are still labelled as placeholders, and `verify` never reports a
changed binary as safe.

## Safety model

- A backup is created automatically before `patch` overwrites an existing file.
- Every backup records the original SHA-256 in a JSON manifest.
- Restores verify the backup *before* touching the target, then read the target
  back and verify it again.
- Patches refuse to write their manifest over the input or output binary, and
  refuse spec/output/manifest path aliases. They also refuse symlinks and
  non-regular files.
- Writes use a temp file + fsync + rename, so a crash cannot leave a truncated
  executable.
- Symlinks are refused as backup/restore and patch targets.
- `--dry-run` previews a restore or patch without writing.
- Backups live next to the original file; there is no hidden database.
- No network access, no telemetry, no elevated privileges.

## Prerequisites

See [PREREQUISITES.md](PREREQUISITES.md). Short version: Rust 1.80+ to build,
write access to the game executable's directory, and no root.

Frame-time measurement additionally needs **MangoHud** and, for Windows games,
**Wine**:

```bash
./target/release/game-opt-tool doctor --game "/path/to/DeusEx.exe"
```

Per-function CPU sampling needs **Linux** with `perf_event_open` allowed for
the current user. It is tested at `perf_event_paranoid=2` and needs no `sudo`
and no `perf` binary. Sampling is not implemented on Windows or macOS.

Decompilation additionally needs **Ghidra 12.2** and a full **JDK 25**. Ghidra
12.2 rejects JDK 21 and JRE-only installations. The verified local install is:

```bash
export GHIDRA_INSTALL_DIR="$HOME/.local/opt/ghidra_12.2_PUBLIC"
export JAVA_HOME="$HOME/.local/opt/jdk-25.0.5+8"
```

See [PREREQUISITES.md](PREREQUISITES.md) for the current portable setup.

## Quick start

```bash
# Build
~/.cargo/bin/cargo build --release --bins

# Check the environment
./target/release/game-opt-tool doctor

# Work on a copy while testing
cp "/path/to/your/game/DeusEx.exe" /tmp/DeusEx.exe

# Back up, inspect, and restore
./target/release/game-opt-tool backup --game /tmp/DeusEx.exe
./target/release/game-opt-tool backup --game /tmp/DeusEx.exe --list
./target/release/game-opt-tool backup --game /tmp/DeusEx.exe --verify
./target/release/game-opt-tool revert --game /tmp/DeusEx.exe --dry-run
./target/release/game-opt-tool revert --game /tmp/DeusEx.exe

# Inspect a real game executable
./target/release/game-opt-tool analyze --game "/path/to/your/game/DeusEx.exe"

# Compare an original and a copy
./target/release/game-opt-tool verify --original /tmp/DeusEx.exe --patched /tmp/DeusEx-patched.exe

# Decompile the executable's entry point with Ghidra
./target/release/game-opt-tool decompile \
  --game "/path/to/your/game/DeusEx.exe"

# Sample CPU time per function while the game is running
./target/release/game-opt-tool sample \
  --game "/path/to/your/game/DeusEx.exe" \
  --duration 20 --top 20
```

## Decompile a function

`decompile` runs Ghidra 12.2 in headless mode, imports the binary into a
temporary project, and prints the pseudocode and function signature. With no
`--address`, it uses the PE/ELF/Mach-O entry point:

```bash
export JAVA_HOME="$HOME/.local/opt/jdk-25.0.5+8"
export GHIDRA_INSTALL_DIR="$HOME/.local/opt/ghidra_12.2_PUBLIC"

./target/release/game-opt-tool decompile \
  --game "/path/to/your/game/DeusEx.exe"

# A specific function, with its x86 disassembly:
./target/release/game-opt-tool decompile \
  --game "/path/to/your/game/DeusEx.exe" \
  --address 0x109214b6 --assembly

# Machine-readable output for scripts:
./target/release/game-opt-tool decompile \
  --game "/path/to/your/game/DeusEx.exe" \
  --address 0x109214b6 --json
```

The command fails if Ghidra cannot start or returns no result. It never falls
back to placeholder pseudocode. Each invocation re-analyzes the binary, which
takes roughly 20–30 seconds for Deus Ex; project caching is not implemented yet.
Use `sample` (below) to find a real hot address, or `list_functions` to browse
handles. A decompiled function is not yet fed into the optimizer
automatically.

## Sample CPU time per function

`sample` attaches to every thread of a running game with Linux
`perf_event_open`, records instruction pointers on a user-space CPU clock,
maps them through `/proc/<pid>/maps` to the PE image, and aggregates self CPU
time by function. It does not use the `perf` binary and does not need root at
`perf_event_paranoid=2`.

```bash
# Launch the game first, reach the scene, then sample it.
./target/release/game-opt-tool sample \
  --game "/path/to/your/game/DeusEx.exe" \
  --duration 20 --frequency 333 --top 20

# Raw PE RVAs when Ghidra is not configured:
./target/release/game-opt-tool sample \
  --game "/path/to/your/game/DeusEx.exe" \
  --duration 10 --no-ghidra --json

# Explicit process when the game path mapping is ambiguous:
./target/release/game-opt-tool sample \
  --game "/path/to/your/game/DeusEx.exe" \
  --process DeusEx.exe --duration 10
```

The report separates **game**, **external** (Wine, ntdll, graphics driver,
audio), and **unknown in-game** samples, and reports lost samples and
ring-buffer overwrites. External samples are real CPU time, but they are not
ranked as game functions. The first Ghidra-backed run builds a function map
and can take ~30 seconds; later runs reuse the cache. `--refresh-functions`
forces a rebuild.

Read the result as one hypothesis, not a verdict. Sampler blind spots:

- self CPU time only; callees are not attributed;
- blocked time (locks, I/O, vsync) and kernel time are not sampled;
- GPU busy time is invisible;
- a hot function may not be on the frame-time critical path.

Pair `sample` with `profile`/`compare` and read
[METRICS.md](METRICS.md) before deciding that a CPU hotspot explains a hitch.

## Measure real frame times

MangoHud writes one CSV row per frame. Run the game through it once:

```bash
mkdir -p /tmp/mangohud
cd "/path/to/your/game"
MANGOHUD_CONFIG="output_folder=/tmp/mangohud,autostart_log=1,log_duration=0" \
  mangohud --dlsym wine DeusEx.exe
```

`log_duration=0` keeps logging until the game exits. Then analyze the newest
log in that directory:

```bash
./target/release/game-opt-tool profile \
  --game "/path/to/your/game/DeusEx.exe" \
  --log-dir /tmp/mangohud

# Or a specific log, only the last 3600 frames, as JSON:
./target/release/game-opt-tool profile \
  --game "/path/to/your/game/DeusEx.exe" \
  --log /tmp/mangohud/DeusEx_2026-10-24_18-00-00.csv \
  --frames 3600 --json
```

The parser understands the metadata rows MangoHud writes before its
`fps,frametime,...` header, reads the `frametime` column in milliseconds, and
falls back to the `fps` column if `frametime` is absent. Invalid rows are
skipped; long frames are kept so loading stalls are not hidden.

### Compare two runs

Capture a baseline and a patched run of the same scene, then compare them:

```bash
./target/release/game-opt-tool compare \
  --baseline /tmp/mangohud/before.csv \
  --patched /tmp/mangohud/after.csv

# Machine-readable, with the last 3600 frames of each log:
./target/release/game-opt-tool compare \
  --baseline /tmp/mangohud/before.csv \
  --patched /tmp/mangohud/after.csv \
  --frames 3600 --json
```

The comparison reports the observed change in average FPS, 1% low, 0.1% low,
p99 frame time, and stutter ratio, and marks whether each change is better or
worse in the metric's own direction. It warns about short runs, mismatched
durations, tail trimming, and does not claim statistical significance. A real
capture pair showed why this matters: uncapping the frame rate raised average
FPS from `48.0` to `1382.8`, but stutter ratio worsened from `1.06x` to
`58.71x`. Raw FPS alone would have hidden that regression.

### Metric definitions

| Metric | Definition |
|--------|------------|
| Average FPS | `1000 / mean frame time` |
| 1% low FPS | `1000 / mean of the slowest 1% of frame times` |
| 0.1% low FPS | `1000 / mean of the slowest 0.1% of frame times` |
| p95 / p99 frame time | Linear-interpolation percentiles |
| Stutter ratio | `p99 frame time / median frame time` |
| Min / max FPS | `1000 / slowest frame` and `1000 / fastest frame` |

The 1% low is an *average of the slowest frames*, not the single 1st-percentile
frame. That is why it can differ from the "1% Min FPS" column in MangoHud's own
summary CSV. Both are reported by their own definitions; this tool's value is
the one used for optimization comparisons.

## Commands

| Command | What it does today |
|---------|--------------------|
| `analyze --game <path> [--log <csv>] [--log-dir <dir>]` | Detects format/architecture and prints measured frame-time statistics when a log is available |
| `profile --game <path> [--log <csv>] [--log-dir <dir>] [--frames N] [--json]` | Computes real metrics from a MangoHud CSV; `--frames` analyzes only the last N frames |
| `sample --game <path> [--pid N\|--process NAME] [--duration N] [--frequency N] [--top N] [--no-ghidra] [--refresh-functions] [--json]` | Samples self CPU time per thread and resolves hot addresses through a cached Ghidra function map (Linux only) |
| `compare --baseline <csv> --patched <csv> [--frames N] [--json]` | Compares two measured runs and reports per-metric deltas with interpretation notes |
| `decompile --game <path> [--address <hex>] [--assembly] [--json]` | Decompiles one function with Ghidra; defaults to the executable entry point |
| `optimize --game <path> --output <dir>` | Stub: writes a copy with placeholder speedups; not a working optimizer |
| `patch --game <path> --output <file> [--spec <json>] [--manifest <json>] [--dry-run] [--json]` | Writes a byte-identical copy unless `--spec` is supplied; `--spec` applies bounds-checked byte patches and optional code caves, writes a manifest, and `unpatch` rolls back exactly |
| `unpatch --manifest <json>` | Restores exactly the bytes recorded in a patch manifest |
| `verify --original <a> --patched <b>` | Compares bytes; changed files fail until runtime verification exists |
| `backup --game <file> [--list] [--verify]` | Creates, lists, or checks backups |
| `revert --game <file> [--backup <manifest>] [--dry-run]` | Restores the newest or a specific verified backup |
| `doctor [--game <path>]` | Checks config, tools, binary, backup directory, and the sampling probe |
| `mcp-server [--transport stdio\|http] [--host H] [--port P]` | Exposes the tools to AI agents over MCP (JSON-RPC 2.0); stdio by default |
| `list-games`, `help <game>` | Lists configured targets / shows target notes |

If neither `--log` nor `--log-dir` is given, `profile` looks for the newest
MangoHud log in the current directory.

Global flags: `--help`, `--version`.

## Use with an AI agent (MCP)

The tool exposes its real commands as MCP tools, so an agent such as OpenCode
or Claude Code can inspect, decompile, profile, patch, and revert a binary
without shell access.

```bash
# Build and register with OpenCode (no sudo, no LLM download)
./install_ai.sh

# Or register manually
opencode mcp add game-opt-tool --global -- \
  "$PWD/target/release/game-opt-tool" mcp-server

# HTTP mode for remote/self-hosted agents
./target/release/game-opt-tool mcp-server --transport http --port 9000
# endpoint: http://127.0.0.1:9000/mcp
```

Exposed tools: `game_info`, `decompile`, `list_functions`, `sample`, `profile`,
`compare`, `patch`, `unpatch`, `backup`, `backup_verify`, `verify`.

The `sample` tool needs a running game and Linux `perf_event_open` permission.
`decompile`/`list_functions`/name resolution need Ghidra and Java in the MCP
server's environment. `profile`/`compare` need a MangoHud CSV captured by you;
the tool never launches the game.

### Guided optimization (`/optimize-game`)

With OpenCode running in this repository, start the whole safe loop with:

```text
/optimize-game
# or
/optimize-game /path/to/your/game/DeusEx.exe
```

The command inspects and backs up the binary, profiles the baseline, samples
the running game for real CPU hotspots, lists/decompiles one function at a
time, drafts one patch spec, runs it with `dry_run`, applies only to a copy,
then compares the runs. It stops for approval after each gate. See
[WORKFLOW.md](WORKFLOW.md) for the user/AI/tool split, measurement rules, and
limits, and [METRICS.md](METRICS.md) for the metric policy.

### Context safety

MCP output is designed for an agent context window, not for dumping a binary:

- Every tool response is capped; oversized responses are replaced with a
  `{"truncated": true, ...}` envelope and a warning on stderr.
- `decompile` returns pseudocode only (assembly is opt-in via
  `include_assembly`) and is paginated: read `next_offset`, then call again
  with `offset`. Default page: 24 KB (~6k tokens).
- `list_functions` returns only address/name/size handles and is paginated, so
  the agent can pick one function to decompile instead of reading everything.
- `profile` and `compare` return statistics, never raw CSV rows.
- Override the hard cap with `GAME_OPT_MCP_MAX_BYTES` (4 KB–1 MB, default
  60 KB).

The MCP server is local-only by default (`127.0.0.1`), stores no API keys, and
calls exactly the same validated code paths as the CLI. The `decompile` tool
needs `GHIDRA_INSTALL_DIR` and `JAVA_HOME` in the server's environment; pass
them with `opencode mcp add --env KEY=VALUE ...` or export them before
launching the agent. See [INSTALL_AI.md](INSTALL_AI.md) and
[AI_PROVIDERS.md](AI_PROVIDERS.md).

## Supported games

| Game | Status |
|------|--------|
| **Deus Ex (2000)** | Test target: PE32/x86 detection, backup/revert, and Ghidra decompilation of entry point `0x109214B6` verified against the real executable |
| Oblivion Remastered, Skyrim SE, Jedi: Survivor | Config entries only; no target-specific work |

There are no measured speedups for any game yet. The table in older docs was
aspirational.

## How it works today

```
Path → format/arch detection → config lookup → command
                                      │
                    ┌─────────────────┼──────────────────┐
                    ▼                 ▼                  ▼
                backup/revert     doctor            verify (bytes)
              SHA-256 + atomic

MangoHud CSV → header discovery → frame times → avg / lows / percentiles / stutter
                                              │
                                              ▼
                                     A/B compare (same scene)

Running game → perf_event_open (per thread) → IP → /proc maps → PE RVA
                              │
                              ▼
                    Ghidra function map (cached) → self CPU by function

PE/ELF/Mach-O → entry point or --address → Ghidra headless → pseudocode + disassembly

AI agent (OpenCode / Claude Code / MCP client)
        │  JSON-RPC 2.0 (stdio or HTTP)
        ▼
mcp-server → game_info / decompile / list_functions / sample / profile / compare / patch / unpatch / backup / verify
        │
        ▼
bounded, paginated JSON (decompile pages, function handles, stats only)
        │
        ▼
same validated code paths as the CLI
```

Planned pipeline (not fully implemented):

```
measure frame times → A/B compare runs → sample CPU per function (working) →
form one hypothesis → decompile one function (working) → hand-written patch →
patch a copy (working) → matched runtime A/B compare (manual) →
keep only if the tail improves and behavior is unchanged
```

## Tests

```bash
~/.cargo/bin/cargo test
```

Current suite: **193 tests passing, 1 ignored** (142 unit + 51 end-to-end
CLI/MCP tests, plus one ignored real-Ghidra test). They cover SHA-256 NIST
vectors, backup/restore round-trips, tampered and truncated backups, symlink
refusal, permission preservation, PE/ELF/Mach-O magic detection, PE `MZ`
detection, trampoline encoding and range checks, config parsing and name
lookup, verifier truthfulness, MangoHud header/preamble and metric math, log
discovery, A/B delta math and warnings, decompiler output parsing,
function-list parsing, address validation, MCP initialize/tools-list/stdio/HTTP
behavior, response capping and truncation envelopes, decompile pagination
slicing, MCP patch → unpatch exact-byte round-trips, the real CLI backup →
corrupt → revert flow, the real `perf_event_open` probe, sampling a busy child,
ring-buffer wrap handling, `/proc` map parsing, Ghidra function-map matching and
cache behavior, and CLI/MCP `sample` validation and failure paths.

Run the real Ghidra test explicitly (it takes about 30 seconds):

```bash
export JAVA_HOME="$HOME/.local/opt/jdk-25.0.5+8"
export GHIDRA_INSTALL_DIR="$HOME/.local/opt/ghidra_12.2_PUBLIC"
export GAME_OPT_TEST_BINARY="/path/to/your/game/DeusEx.exe"
~/.cargo/bin/cargo test --test cli_decompile \
  decompiles_a_real_binary_entry_point -- --ignored
```

The real-world checks include:

- Deus Ex executable (253,952 bytes) restored byte-for-byte
  (`sha256 3e6fdb89…bae00e`).
- Ghidra 12.2 headless decompiled the real Deus Ex entry point
  `0x109214B6` and returned the CRT `entry` pseudocode plus its x86
  disassembly. It is startup code, not a performance hotspot; the point is that
  the pipeline is real.
- A genuine 143-frame MangoHud log produced by `glxgears`: the tool reported
  average FPS `48.01` versus MangoHud's own summary `48.0`, average frame time
  `20.83 ms` versus `20.8 ms`, and the same `0.1%` low of `42.45 FPS`.
- Two genuine MangoHud captures of the same workload: `compare` reported
  average FPS `48.01 → 1382.81` but stutter ratio `1.06x → 58.71x`, correctly
  flagging the raw-FPS gain as a consistency regression.
- Real per-function sampling on the Deus Ex menu: 20 seconds at 333 Hz produced
  5,352 samples, 3 in the game module and 5,349 in external Wine/driver code,
  with 0 samples lost. This is a real idle-workload measurement, not a
  gameplay hotspot; it also shows why external samples are reported separately
  instead of being blamed on the game. Sampler correctness is covered by the
  busy-child and ring-wrap tests.
- Real Ghidra symbol resolution: `list_functions` returned 945 Deus Ex handles
  and the three menu sample addresses resolved to `FUN_10902020`,
  `FUN_10902a20`, and `FUN_109146fb`. The binary is stripped, so these are
  analysis names, not source symbols. Three samples do not rank hot functions.

## Legal

Personal use only, on games you own. No DRM circumvention, no asset
distribution, single-player only. See [LEGAL.md](LEGAL.md).

## License

MIT
