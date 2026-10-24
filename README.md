# Game Opt Tool

> A work-in-progress tool for inspecting game binaries, measuring frame-time
> consistency, and safely trying performance changes. The measurement and
> backup/revert layers are real and tested; decompilation, optimization, and
> binary rewriting are still prototypes.

## Current status (verified by `cargo test`)

| Area | Status |
|------|--------|
| PE / ELF / Mach-O detection and architecture detection | **Working** |
| Game config loading (`config/games.yaml`) | **Working** |
| `doctor` prerequisite checks | **Working** |
| Backup with SHA-256 + JSON manifest | **Working, tested** |
| Revert with checksum verification and atomic write | **Working, tested** |
| Frame-time profiling from MangoHud CSV | **Working, tested against a real MangoHud log** |
| `analyze` | Detection plus real frame-time summary when a log is available; function hotspots are not implemented |
| Verify | **Byte comparison only** — runtime equivalence is not implemented |
| Decompilation | Skeleton; requires Ghidra + JDK |
| Optimizer passes | **Stubs** — placeholder speedup values |
| Binary patching | Writes a byte-identical copy; no code transformations yet |

The tool does not fabricate frame-time measurements: when no log is available,
`profile` fails with instructions and `analyze` explains how to generate one.
Optimizer speedups are still labelled as placeholders, and `verify` never
reports a changed binary as safe.

## Safety model

- A backup is created automatically before `patch` overwrites an existing file.
- Every backup records the original SHA-256 in a JSON manifest.
- Restores verify the backup *before* touching the target, then read the target
  back and verify it again.
- Writes use a temp file + fsync + rename, so a crash cannot leave a truncated
  executable.
- Symlinks are refused as backup/restore targets.
- `--dry-run` previews a restore without writing.
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
```

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
| `optimize --game <path> --output <dir>` | Runs stub passes and writes a copy (placeholder speedups, clearly labeled) |
| `patch --game <path> --output <file>` | Creates a backup if needed and writes a byte-identical copy |
| `verify --original <a> --patched <b>` | Compares bytes; changed files fail until runtime verification exists |
| `backup --game <file> [--list] [--verify]` | Creates, lists, or checks backups |
| `revert --game <file> [--backup <manifest>] [--dry-run]` | Restores the newest or a specific verified backup |
| `doctor [--game <path>]` | Checks config, tools, binary, and backup directory |
| `list-games`, `help <game>` | Lists configured targets / shows target notes |

If neither `--log` nor `--log-dir` is given, `profile` looks for the newest
MangoHud log in the current directory.

Global flags: `--help`, `--version`.

## Supported games

| Game | Status |
|------|--------|
| **Deus Ex (2000)** | Test target: PE32/x86 detection and backup/revert verified against the real executable |
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
```

Planned pipeline (not implemented):

```
measure frame times → A/B compare runs → find hot functions → decompile →
rewrite → patch → runtime A/B verification
```

## Tests

```bash
~/.cargo/bin/cargo test
```

Current suite: **59 unit tests + 17 end-to-end CLI tests**, all passing. They
cover SHA-256 NIST vectors, backup/restore round-trips, tampered and truncated
backups, symlink refusal, permission preservation, PE/ELF/Mach-O magic
detection, PE `MZ` detection, trampoline encoding and range checks, config
parsing and name lookup, verifier truthfulness, MangoHud header/preamble and
metric math, log discovery, and the real CLI backup → corrupt → revert flow.

The real-world checks include:

- Deus Ex executable (253,952 bytes) restored byte-for-byte
  (`sha256 3e6fdb89…bae00e`).
- A genuine 143-frame MangoHud log produced by `glxgears`: the tool reported
  average FPS `48.01` versus MangoHud's own summary `48.0`, average frame time
  `20.83 ms` versus `20.8 ms`, and the same `0.1%` low of `42.45 FPS`.

## Legal

Personal use only, on games you own. No DRM circumvention, no asset
distribution, single-player only. See [LEGAL.md](LEGAL.md).

## License

MIT
