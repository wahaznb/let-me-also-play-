# Game Opt Tool — Verified Status

Last verified: 2026-10-26. Branch `feat/verified-safety-layer`.

## What this tool does

This tool reverse-engineers and optimizes owned single-player game binaries.
It focuses on **frame-time consistency** (1% lows) over raw FPS, preserves
original behavior and quirks, and keeps every change **reversible** via
cryptographic verification. Real per-function CPU sampling is available on
Linux; it is evidence for a hypothesis, not proof of a frame-time win. See
[METRICS.md](METRICS.md) for the measurement policy.

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         GAME OPT TOOL                                       │
│                                                                             │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐   │
│  │  PE Parser   │  │   Patcher    │  │  Decompiler  │  │   Optimizer  │   │
│  │   (pe.rs)    │  │  (mod.rs)    │  │   (Ghidra)   │  │    (stub)    │   │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘   │
│         │                 │                 │                 │             │
│         └─────────────────┴─────────────────┴─────────────────┘             │
│                           │                                                 │
│              ┌────────────▼────────────┐                                    │
│              │    CLI Interface        │                                    │
│              │   (main.rs / clap)      │                                    │
│              └────────────┬────────────┘                                    │
│                           │                                                 │
│         ┌─────────────────┼─────────────────┐                               │
│         │                 │                 │                               │
│  ┌──────▼──────┐  ┌──────▼──────┐  ┌──────▼──────┐                          │
│  │   Backup    │  │   Verify    │  │  MangoHud   │                          │
│  │   System    │  │   System    │  │  Profiler   │                          │
│  └─────────────┘  └─────────────┘  └─────────────┘                          │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

The sampler (`src/sampler/`) is a separate Linux-only path: a running game's
threads are sampled with `perf_event_open`, runtime IPs are mapped to PE RVAs,
and the cached Ghidra function map turns them into per-function self CPU time.

## Core Components

### 1. PE Parser (`src/patcher/pe.rs`)
- Parses PE32/PE32+ headers with strict validation
- Maps virtual addresses to file offsets
- Allocates new executable sections
- Computes correct PE checksums

### 2. Patcher (`src/patcher/mod.rs`)
- Applies bounds-checked byte patches
- Appends code caves (new executable sections)
- Generates JSON manifests for exact rollback
- Validates expected bytes before writing

### 3. Backup System (`src/backup/mod.rs`)
- Content-addressed backups with SHA-256
- Atomic writes (temp file + fsync + rename)
- Verification against recorded hashes
- Permission preservation

### 4. Decompiler (`src/decompiler/mod.rs`)
- Headless Ghidra 12.2 integration
- Entry-point auto-detection
- Pseudocode + assembly output

### 5. Frame-Time Profiler (`src/profiler/mod.rs`)
- CSV log ingestion
- Frame-time metrics (avg, 1% low, p99, stutter)
- A/B comparison between baseline and patched runs

### 6. CPU Sampler (`src/sampler/`)
- Linux `perf_event_open` per-thread user-space CPU-clock sampling
- Runtime IP → PE RVA mapping through `/proc/<pid>/maps`
- Cached Ghidra function map, raw-RVA fallback
- Separates game / external / unknown samples; reports lost and overwritten data
- Linux-only; returns a clear unsupported error on other platforms

### 7. MCP Server (`src/mcp/mod.rs`)
- JSON-RPC 2.0 over stdio (default) and HTTP (`--transport http`)
- Eleven tools wired to the real sampler, decompiler, profiler, patcher,
  backup, and verifier
- Context-safe by construction: per-call response cap, truncation envelopes,
  paginated `decompile`, handle-only `list_functions`
- stdout is protocol-only; logs go to stderr

## Commands Reference

| Command | Purpose | Example |
|---------|---------|---------|
| `backup` | Create backup with SHA-256 record | `backup --game DeusEx.exe` |
| `backup --list` | Show existing backups | `backup --list --game DeusEx.exe` |
| `backup --verify` | Check backup integrity | `backup --verify --game DeusEx.exe` |
| `revert` | Restore from backup | `revert --game DeusEx.exe` |
| `patch --spec` | Apply byte patches with manifest | `patch --game game.exe --output patched.exe --spec patch.json` |
| `unpatch` | Rollback to original | `unpatch --manifest patched.exe.sha256.patch.json` |
| `decompile` | Decompile function with Ghidra | `decompile --game game.exe --address 0x109214b6` |
| `sample` | Per-function CPU sampling of a running game (Linux) | `sample --game game.exe --duration 20 --top 20` |
| `profile` | Analyze MangoHud logs | `profile --game game.exe --log-frame.csv` |
| `compare` | Compare two runs | `compare --baseline before.csv --patched after.csv` |
| `doctor` | Check prerequisites | `doctor --game DeusEx.exe` |
| `mcp-server` | Expose tools to AI agents | `mcp-server` or `mcp-server --transport http --port 9000` |

## Test Evidence

```bash
cargo test --all-targets
# 194 passed, 0 failed, 1 ignored (real-Ghidra test)

cargo build --release --bins
# 0 errors, 5 warnings (pre-existing optimizer/verifier/backup dead code)
```

The ignored real-Ghidra test was run explicitly with JDK 25 and Ghidra 12.2:

```bash
export JAVA_HOME="$HOME/.local/opt/jdk-25.0.5+8"
export GHIDRA_INSTALL_DIR="$HOME/.local/opt/ghidra_12.2_PUBLIC"
export GAME_OPT_TEST_BINARY="/path/to/your/game/DeusEx.exe"
cargo test --test cli_decompile \
  decompiles_a_real_binary_entry_point -- --ignored
# 1 passed; finished in 30.02s
```

Sampler evidence:

- Real `perf_event_open` probe succeeds at `perf_event_paranoid=2` without root.
- A busy child process produced 198 samples in ~2 s in the standalone proof;
  the in-tree busy-child and ring-wrap tests pass.
- Deus Ex menu sampling: 20 s at 333 Hz, 5,352 total samples, 3 game, 5,349
  external, 0 lost. This is an idle/menu workload, not a gameplay hotspot.
- Real Ghidra `list_functions` returned 945 handles. The three menu sample IPs
  resolved into real function ranges: `0x10902283 → FUN_10902020`,
  `0x10902A6F → FUN_10902a20`, and `0x1091489A → FUN_109146fb`. The binary is
  stripped, so these are analysis names, not source symbols. This validates the
  sampler→Ghidra address mapping even though three samples are not a hotspot.
- MCP `sample` was exercised over stdio against a process mapping the real PE
  (a synthetic mapping test, not a game): 666 samples in 2 s at 333 Hz, all
  external as expected, 0 lost, with the correct "no in-game samples" warning.
  This validates the MCP transport and report shape; the live OpenCode process
  still needs a restart to expose the new tool.
- CLI/MCP `sample` tests cover help, argument conflicts, missing game, no
  running process, PID zero, JSON-on-failure behavior, tool schema, and a
  missing-game MCP tool error. They do not require `perf` permission.

## Real Binary Validation

Backup check against `/path/to/your/game/DeusEx.exe`
(253,952 bytes, PE32/x86):

1. `doctor --game` detected `DeusEx (PE, x86)` on a working copy under `/tmp`.
2. `backup` created a manifest and a backup with SHA-256
   `3e6fdb89b880a60ee984b885a48f262070019572f1531ed70d96108781bae00e`.
3. A copy was overwritten with junk, then `revert` restored it.
4. `cmp` and `sha256sum` confirmed byte-for-byte equality with the original.

## MCP Validation (2026-10-26)

1. `mcp-server` over stdio answered `initialize`, `tools/list` (11 tools), and
   `ping`; notifications produced no responses.
2. `tools/call game_info` against the real
   `/path/to/your/game/DeusEx.exe` returned
   `DeusEx`, PE, x86, entry `0x109214B6`.
3. `tools/call patch` with an expected-byte spec produced
   `sha256 4c62ef92…6d677`; `tools/call unpatch` restored
   `sha256 3e6fdb89…bae00e` exactly.
4. HTTP transport answered `initialize` and tool calls at
   `POST http://127.0.0.1:<port>/mcp`.
5. OpenCode registered the server globally and exposed the tools as
   `game-opt-tool.*` in a live session.
6. `list_functions` returned 945 real Deus Ex handles; a 1000-row request was
   budget-limited to 929 rows with `next_offset`, staying under 60 KB.
7. `decompile` of the 22,221-byte largest function returned 2,048-byte pages
   with `truncated: true` and a working `next_offset` (2,048 → 4,096).
8. An oversized `patch` report was replaced by a bounded truncation envelope
   that retained `manifest_path` in its preview.
9. `sample` is registered as the 11th tool with a bounded schema
   (`duration_secs` 1–120, `frequency` 1–999) and fails cleanly for a missing
   game. The CLI menu run produced 5,352 samples with 0 lost. MCP `sample` was
   exercised against a synthetic mapped-PE process (666 samples, all external,
   0 lost), which validates the transport and report shape. No MCP call has yet
   sampled live gameplay, and the running OpenCode process predates the tool and
   must be restarted after the release rebuild.

## What is real

| Feature | Implementation |
|---------|----------------|
| PE detection | `MZ` header + `PE\0\0` signature at `e_lfanew` |
| ELF / Mach-O detection | Goblin parsing plus fat-slice validation; magic fallback for byte-swapped fat headers; both Mach-O endiannesses |
| Architecture detection | goblin PE/ELF machine types and Mach-O CPU types; universal binaries prefer the host slice on macOS |
| Config | `config/games.yaml` parsed as a full `Config`; fuzzy game-name lookup |
| Backup | Content-addressed `.bak` + `.bak.json` manifest, `create_new`, SHA-256 |
| Revert | Checksum verification, symlink refusal, atomic temp+rename, read-back verification, permission preservation, dry-run |
| Verify | SHA-256/byte comparison; changed binaries are reported as NOT VERIFIED |
| Doctor | Config parse, Wine/MangoHud/Java/Ghidra presence, Ghidra's required Java version, JDK-vs-JRE check, binary detection, writable backup dir, existing backups |
| Decompilation | Ghidra 12.2 headless, embedded Java post-script, PE/ELF/Mach-O entry-point detection, address-based lookup, pseudocode + x86 disassembly, strict result parsing, temporary-project cleanup and timeout |
| MangoHud ingestion | Streaming CSV parser; metadata preamble, BOM, CRLF, header variants, `fps` fallback |
| Frame metrics | Average/min/max FPS, 1% and 0.1% lows, median/p95/p99 frame time, duration, stutter ratio |
| Log discovery | Newest `.csv`/`.log` in a directory whose header contains `frametime` or `fps` |
| A/B comparison | Deltas for average FPS, 1%/0.1% lows, p99 frame time, and stutter, with short-run/duration/scene notes |
| PE byte patches | Strict PE parser, bounds-checked patches, expected-byte verification, code cave allocation, manifest-backed rollback |
| CPU sampling | Linux `perf_event_open`, per-thread user-space CPU clock, runtime IP → PE RVA, cached Ghidra function map, raw-RVA fallback, game/external/unknown split, lost-sample reporting; 21 sampler tests |
| MCP server | JSON-RPC 2.0 stdio + HTTP, initialize/tools-list/tools-call, eleven real tools, tool-level error reporting, response-size cap with truncation envelopes, paginated decompile and function listing; verified end-to-end against the real Deus Ex executable and a PE round-trip |

## What is still a stub or a limit

| Feature | Current behavior |
|---------|------------------|
| `analyze` hot functions | Analyze reports measured frame times and detection only; use `sample` for per-function CPU data |
| `optimize` | Stub passes; speedup values are placeholders |
| `verify` runtime behavior | Not implemented; only bytes are compared |
| Sampler platforms | Linux only. Windows/macOS builds compile but return an unsupported error |
| Sampler blind spots | Self CPU only; no GPU time, blocked time, kernel time, or call-graph attribution |
| Launch automation | The user launches the game and captures MangoHud logs manually |
| GPU/off-CPU attribution | Not measured; use PresentMon/MangoHud and treat CPU hotspots as hypotheses |

## Safety model

1. **Backups are content-addressed**: The backup name includes the first 16 hex chars of the SHA-256 hash. You can always identify which file a backup came from.

2. **Every backup has a manifest**: The `.bak.json` file records the original path, SHA-256, size, and creation time.

3. **Restore verifies before and after**: The backup is read and its SHA-256 checked before writing. After writing, the target is re-read and verified.

4. **Patches write manifests**: Every `patch --spec` creates a JSON manifest recording exactly which bytes were changed and what they were before.

5. **Unpatch is exact**: `unpatch --manifest` restores the original bytes byte-for-byte and verifies the result matches the input SHA-256.

6. **Atomic writes everywhere**: All file writes use temp file + fsync + rename. A crash cannot leave a truncated executable.

7. **Symlinks are refused**: The tool refuses to create backups or write patches over symlinks.

8. **No network, no telemetry**: Everything runs locally.

## Usage example: full patch lifecycle

```bash
# 1. Copy game to writable location
cp -a "/path/to/your/game" /tmp/mygame/

# 2. Create backup (ONE TIME)
./target/release/game-opt-tool backup --game /tmp/mygame/DeusEx.exe
# → Backup created: DeusEx.exe.3e6fdb89b880a60e.123456.bak

# 3. Write patch spec
cat > /tmp/mygame/patch.json << 'EOF'
{
  "schema_version": 1,
  "name": "entry-nop",
  "patches": [{
    "address": "0x109214b6",
    "expected": "55 8B EC 6A FF",
    "replace": "90 90 90 90 90"
  }]
}
