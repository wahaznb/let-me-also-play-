# Game Opt Tool — Verified Status

Last verified: 2026-10-24. Baseline commit `b27ad1e`.

## What this tool does

This tool reverse-engineers and optimizes owned single-player game binaries.
It focuses on **frame-time consistency** (1% lows) over raw FPS, preserves
original behavior and quirks, and keeps every change **reversible** via
cryptographic verification.

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

### 5. MangoHud Profiler (`src/profiler/mod.rs`)
- CSV log ingestion
- Frame-time metrics (avg, 1% low, p99, stutter)
- A/B comparison between baseline and patched runs

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
| `profile` | Analyze MangoHud logs | `profile --game game.exe --log-frame.csv` |
| `compare` | Compare two runs | `compare --baseline before.csv --patched after.csv` |
| `doctor` | Check prerequisites | `doctor --game DeusEx.exe` |

## Test Evidence

```bash
~/.cargo/bin/cargo test --all-targets
# 102 unit tests + 34 CLI end-to-end tests — all passing, 1 real-Ghidra test ignored

~/.cargo/bin/cargo build --release --bins
# 0 errors, 5 warnings (pre-existing optimizer/verifier/backup dead code)
```

The ignored real-Ghidra test was run explicitly with JDK 25 and Ghidra 12.2:

```bash
export JAVA_HOME=/tmp/opencode/jdk25-full
export GHIDRA_INSTALL_DIR=/tmp/opencode/ghidra_extracted/ghidra_12.2_PUBLIC
export GAME_OPT_TEST_BINARY="/path/to/your/game/DeusEx.exe"
~/.cargo/bin/cargo test --test cli_decompile \
  decompiles_a_real_binary_entry_point -- --ignored
# 1 passed; finished in 30.02s
```

## Real Binary Validation

Backup check against `/path/to/your/game/DeusEx.exe`
(253,952 bytes, PE32/x86):

1. `doctor --game` detected `DeusEx (PE, x86)` on a working copy under `/tmp`.
2. `backup` created a manifest and a backup with SHA-256
   `3e6fdb89b880a60ee984b885a48f262070019572f1531ed70d96108781bae00e`.
3. A copy was overwritten with junk, then `revert` restored it.
4. `cmp` and `sha256sum` confirmed byte-for-byte equality with the original.

## What is real

| Feature | Implementation |
|---------|----------------|
| PE detection | `MZ` header + `PE\0\0` signature at `e_lfanew` |
| ELF / Mach-O detection | Magic bytes, both Mach-O endiannesses |
| Architecture detection | goblin PE/ELF machine types |
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

## What is still a stub

| Feature | Current behavior |
|---------|------------------|
| `analyze` hot functions | Not implemented; analyze reports measured frame times only and says so |
| Automatic hot-function discovery | Not implemented; `decompile` needs an address or defaults to the entry point |
| `optimize` | Stub passes; speedup values are placeholders |
| `verify` runtime behavior | Not implemented; only bytes are compared |
| Launch automation | The user launches the game under MangoHud manually |

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
