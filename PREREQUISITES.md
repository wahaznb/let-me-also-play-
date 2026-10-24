# Prerequisites

## Required for the safety and inspection features

`doctor`, `analyze`, `backup`, `revert`, `verify`, `profile`, and `compare`
need:

- **Linux, macOS, or Windows.** Tested on CachyOS (Arch-based) x86_64.
- **Rust 1.80 or newer** to build (`std::sync::LazyLock` requires 1.80).
  Built and tested with Rust 1.99.1.
- **Write permission to the directory containing the game executable for
  `backup`/`revert`.** Backups and their manifests are stored next to the
  original file, so reverting never depends on a database or the current
  directory. `profile` and `compare` only read logs.
- **No root, no network, and no Wine** for these commands.

The tool never downloads anything at runtime and does not contain telemetry.

## Required for frame-time measurement

`profile` and `analyze --log` read MangoHud CSV logs. To produce those logs for
a Windows game you also need:

| Tool | Needed for | Status on this machine |
|------|------------|------------------------|
| Wine 11.x | launching Windows builds | installed (`/usr/bin/wine`) |
| MangoHud / mangoapp | frame-time capture | installed (`/usr/bin/mangohud`) |

For a native Linux game, Wine is not needed; launch the game through
`mangohud` directly.

## Optional, for features that are not implemented yet

| Tool | Needed for | Status on this machine |
|------|------------|------------------------|
| Ghidra 11.x+ | headless decompilation | **not installed** |
| JDK 21 | running Ghidra | **not installed** |

`game-opt-tool doctor` reports all of the above. It exits non-zero only when a
required safety/inspection check fails; missing Java/Ghidra are warnings.

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

# 5. Measure a run: capture once, then analyze the newest log.
mkdir -p /tmp/mangohud
cd "/path/to/your/game"
MANGOHUD_CONFIG="output_folder=/tmp/mangohud,autostart_log=1,log_duration=0" \
  mangohud --dlsym wine DeusEx.exe

cd /path/to/game-opt-tool
./target/release/game-opt-tool profile \
  --game "/path/to/your/game/DeusEx.exe" \
  --log-dir /tmp/mangohud

# 6. Compare a baseline and a patched run of the same scene.
./target/release/game-opt-tool compare \
  --baseline /tmp/mangohud/before.csv \
  --patched /tmp/mangohud/after.csv
```

Use the same save, settings, and route for both captures. The comparison
warns about short runs, different durations, and tail trimming, but it cannot
detect a scene mismatch for you.

`log_duration=0` means "keep logging until the game exits". `autostart_log=1`
starts capture without opening the MangoHud menu. Use the `mangohud` wrapper
rather than setting `MANGOHUD=1` by hand so the correct layer is loaded.

For Deus Ex, start the game from its `System` directory so it can find its
data files. MangoHud writes a detailed CSV plus a `_summary.csv`; the tool
selects the detailed log by checking for a `frametime` or `fps` header and
ignores the summary file.

Backups are named `<file>.<sha256-prefix>.<timestamp>.bak` with a matching
`.bak.json` manifest. A restore refuses to run if the backup no longer matches
its recorded checksum, and writes through a temp file plus rename so a crash
cannot leave a half-written executable.

## Troubleshooting

| Symptom | Meaning / fix |
|---------|----------------|
| `No MangoHud CSV logs found` | No log exists in the directory yet, or MangoHud could not inject. Check that the log directory contains a CSV whose header includes `fps,frametime`. |
| MangoHud overlay does not appear | The renderer may not be supported, or the game runs in a mode MangoHud cannot hook. Try `mangohud --dlsym <command>`; for Wine use `mangohud --dlsym wine <game.exe>`. |
| `No MangoHud header found` | The file is not a MangoHud frame log (or is truncated). The tool skips MangoHud metadata rows but needs a `fps` or `frametime` header. |
| 1% low differs from MangoHud's summary | By design: this tool averages the slowest 1% of frame times; MangoHud's summary reports a percentile. See the README metric definitions. |
| `compare` warns that run durations differ | Capture both runs for the same length and scene, or use `--frames` to trim both tails to the same frame count. |
| `compare` warns about fewer than 1000 frames | The lows are not statistically stable yet. Capture a longer, representative run. |
| `invalid type: map, expected a sequence` | Old build. Fixed: `config/games.yaml` is parsed as a full config, then looked up by game name. |
| `No backups found` | Run `backup --game <file>` first. Backups only exist for files backed up by this tool. |
| `Backup checksum mismatch` | The `.bak` file was modified or corrupted. Restore refuses on purpose; use another backup or the original media. |
| `Refusing to restore ... symbolic link` | Safety guard. Point the command at the real file, not a symlink. |
| `directory is not writable` from `doctor` | Backups live next to the executable. Fix permissions or work on a copy. |
| `java`/`ghidra` warnings in `doctor` | Expected: decompilation and real optimization are not implemented yet. |
