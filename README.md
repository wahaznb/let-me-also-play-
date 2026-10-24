# Game Opt Tool

> A work-in-progress tool for inspecting game binaries and safely trying
> performance changes. The safety layer is real and tested; the optimization
> and measurement layers are still prototypes.

## Current status (verified by `cargo test`)

| Area | Status |
|------|--------|
| PE / ELF / Mach-O detection and architecture detection | **Working** |
| Game config loading (`config/games.yaml`) | **Working** |
| `doctor` prerequisite checks | **Working** |
| Backup with SHA-256 + JSON manifest | **Working, tested** |
| Revert with checksum verification and atomic write | **Working, tested** |
| Verify | **Byte comparison only** — runtime equivalence is not implemented |
| Frame-time profiling | **Placeholder data** — no measurement yet |
| Decompilation | Skeleton; requires Ghidra + JDK |
| Optimizer passes | **Stubs** — placeholder speedup values |
| Binary patching | Writes a byte-identical copy; no code transformations yet |

The tool prints a warning whenever it is about to show placeholder data, and
`verify` never reports a changed binary as safe.

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

Optional future features use Wine, MangoHud, and Ghidra. Run:

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

## Commands

| Command | What it does today |
|---------|--------------------|
| `analyze --game <path>` | Detects format/architecture and prints hot functions (placeholder percentages, clearly labeled) |
| `profile --game <path> --frames N` | Prints placeholder frame metrics (clearly labeled) |
| `optimize --game <path> --output <dir>` | Runs stub passes and writes a copy (placeholder speedups, clearly labeled) |
| `patch --game <path> --output <file>` | Creates a backup if needed and writes a byte-identical copy |
| `verify --original <a> --patched <b>` | Compares bytes; changed files fail until runtime verification exists |
| `backup --game <file> [--list] [--verify]` | Creates, lists, or checks backups |
| `revert --game <file> [--backup <manifest>] [--dry-run]` | Restores the newest or a specific verified backup |
| `doctor [--game <path>]` | Checks config, tools, binary, and backup directory |
| `list-games`, `help <game>` | Lists configured targets / shows target notes |

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
```

Planned pipeline (not implemented):

```
measure frame times → find hot functions → decompile → rewrite → patch → runtime A/B verification
```

## Tests

```bash
~/.cargo/bin/cargo test
```

Current suite: **42 unit tests + 8 end-to-end CLI tests**, all passing. They
cover SHA-256 NIST vectors, backup/restore round-trips, tampered and truncated
backups, symlink refusal, permission preservation, PE/ELF/Mach-O magic
detection, PE `MZ` detection, trampoline encoding and range checks, config
parsing and name lookup, verifier truthfulness, and the real CLI backup →
corrupt → revert flow.

The real-world check on the Deus Ex executable (253,952 bytes) restored
byte-for-byte (`sha256 3e6fdb89…bae00e`).

## Legal

Personal use only, on games you own. No DRM circumvention, no asset
distribution, single-player only. See [LEGAL.md](LEGAL.md).

## License

MIT
