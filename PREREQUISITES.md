# Prerequisites

## Required for the features that actually work today

`doctor`, `analyze`, `backup`, `revert`, and `verify` need:

- **Linux, macOS, or Windows.** Tested on CachyOS (Arch-based) x86_64.
- **Rust 1.80 or newer** to build (`std::sync::LazyLock` requires 1.80).
  Built and tested with Rust 1.99.1.
- **Write permission to the directory containing the game executable.**
  Backups and their manifests are stored next to the original file, so
  reverting never depends on a database or the current directory.
- **No root, no network, and no Wine** for these commands.

The tool never downloads anything at runtime and does not contain telemetry.

## Optional, for features that are not implemented yet

| Tool | Needed for | Status on this machine |
|------|------------|------------------------|
| Wine 11.x | launching Windows builds | installed (`/usr/bin/wine`) |
| MangoHud / mangoapp | frame-time capture | installed (`/usr/bin/mangohud`) |
| Ghidra 11.x+ | headless decompilation | **not installed** |
| JDK 21 | running Ghidra | **not installed** |

`game-opt-tool doctor` reports all of the above.

### Install on CachyOS / Arch

```bash
sudo pacman -S --needed base-devel rustup wine mangohud
rustup default stable

# For future decompilation support:
sudo pacman -S --needed jdk21-openjdk
# Ghidra: install from the AUR (e.g. `yay -S ghidra`) or unzip a release
# from https://ghidra-sre.org, then point the tool at it:
export GHIDRA_INSTALL_DIR=/opt/ghidra
```

## Build

On this machine `cargo` is not on `PATH`; call it explicitly:

```bash
~/.cargo/bin/cargo build --release --bins
```

The binary is `target/release/game-opt-tool`.

## Verify the install

```bash
./target/release/game-opt-tool doctor
./target/release/game-opt-tool doctor --game "/path/to/your/game"
```

`doctor` exits non-zero only when a required check fails. Missing optional
tools (Java, Ghidra) are warnings.

## Low-friction workflow

```bash
# 1. Work on a copy while testing; backups are created next to the file.
cp "/path/to/your/game/DeusEx.exe" /tmp/DeusEx.exe

# 2. Back it up (records SHA-256 + a JSON manifest).
./target/release/game-opt-tool backup --game /tmp/DeusEx.exe

# 3. Inspect backups / re-check checksums at any time.
./target/release/game-opt-tool backup --game /tmp/DeusEx.exe --list
./target/release/game-opt-tool backup --game /tmp/DeusEx.exe --verify

# 4. Preview and then perform a restore.
./target/release/game-opt-tool revert --game /tmp/DeusEx.exe --dry-run
./target/release/game-opt-tool revert --game /tmp/DeusEx.exe
```

Backups are named `<file>.<sha256-prefix>.<timestamp>.bak` with a matching
`.bak.json` manifest. A restore refuses to run if the backup no longer matches
its recorded checksum, and writes through a temp file plus rename so a crash
cannot leave a half-written executable.

## Troubleshooting

| Symptom | Meaning / fix |
|---------|----------------|
| `invalid type: map, expected a sequence` | Old build. Fixed: `config/games.yaml` is parsed as a full config, then looked up by game name. |
| `No backups found` | Run `backup --game <file>` first. Backups only exist for files backed up by this tool. |
| `Backup checksum mismatch` | The `.bak` file was modified or corrupted. Restore refuses on purpose; use another backup or the original media. |
| `Refusing to restore ... symbolic link` | Safety guard. Point the command at the real file, not a symlink. |
| `directory is not writable` from `doctor` | Backups live next to the executable. Fix permissions or work on a copy. |
| `java`/`ghidra` warnings in `doctor` | Expected: decompilation and real optimization are not implemented yet. |
