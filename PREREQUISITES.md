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

## Required for per-function CPU sampling

`sample` is Linux-only and needs a **running** game. It uses
`perf_event_open` directly; the `perf` binary is not required.

| Requirement | Detail |
|-------------|--------|
| Linux kernel with `perf_event_open` | Tested on CachyOS x86_64 |
| `perf_event_paranoid <= 2` | `1` or `2` works unprivileged; `3` and `4` deny unprivileged sampling |
| Same user as the game | The sampler reads `/proc/<pid>/maps` and opens per-thread events |
| No `sudo` | The tool never asks for root |
| Ghidra + JDK (optional) | Only for function names; `--no-ghidra` reports raw PE RVAs |

Check it with:

```bash
cat /proc/sys/kernel/perf_event_paranoid
./target/release/game-opt-tool doctor
```

`doctor` prints a `perf_event_open` probe line. If sampling is denied, lower
`perf_event_paranoid` (a system policy change) or run the tool with the
capability your distribution requires. The tool itself never escalates.

## Required for frame-time measurement

`profile` and `analyze --log` read MangoHud CSV logs. To produce those logs for
a Windows game you also need:

| Tool | Needed for | Status on this machine |
|------|------------|------------------------|
| Wine 11.x | launching Windows builds | installed (`/usr/bin/wine`) |
| MangoHud / mangoapp | frame-time capture | installed (`/usr/bin/mangohud`) |

For a native Linux game, Wine is not needed; launch the game through
`mangohud` directly.

## Required for decompilation and function names

`decompile` runs Ghidra's headless analyzer. `list_functions` and `sample`
use the same Ghidra installation to resolve addresses to function names. They
need:

| Tool | Needed for | Status on this machine |
|------|------------|------------------------|
| Ghidra 12.2 | headless decompilation and function names | installed under `~/.local/opt/ghidra_12.2_PUBLIC` |
| Full JDK 25 | running Ghidra 12.2 | installed under `~/.local/opt/jdk-25.0.5+8` |

Ghidra 12.2 requires Java **25**, not 21. A JRE is not enough; the JDK's
`bin/javac` must exist. The tool does not install or download either one.

```bash
export JAVA_HOME="$HOME/.local/opt/jdk-25.0.5+8"
export PATH="$JAVA_HOME/bin:$PATH"
export GHIDRA_INSTALL_DIR="$HOME/.local/opt/ghidra_12.2_PUBLIC"
```

`GHIDRA_INSTALL_DIR` must point at the Ghidra root that contains
`support/analyzeHeadless`, not at the `support` directory itself.

`game-opt-tool doctor` reports all of the above. It exits non-zero only when a
required safety/inspection check fails; missing Java/Ghidra are warnings.
`sample --no-ghidra` works without Ghidra and reports raw PE RVAs.

### Portable setup without root

The verified setup is installed under `~/.local/opt` to avoid system packages
and to survive reboots. A JDK 25 archive can be fetched from Adoptium and
extracted with:

```bash
mkdir -p "$HOME/.local/opt"
curl -L -o /tmp/jdk25.tar.gz \
  'https://api.adoptium.net/v3/binary/latest/25/ga/linux/x64/jdk/hotspot/normal/eclipse'
mkdir -p "$HOME/.local/opt/jdk-25.0.5+8"
tar -xzf /tmp/jdk25.tar.gz \
  -C "$HOME/.local/opt/jdk-25.0.5+8" --strip-components=1
test -x "$HOME/.local/opt/jdk-25.0.5+8/bin/javac"
```

Download Ghidra 12.2 from <https://ghidra-sre.org/> (or its GitHub releases),
unzip it under `~/.local/opt`, and set `GHIDRA_INSTALL_DIR` to the extracted
`ghidra_12.2_PUBLIC` directory. Ghidra is Apache-2.0 licensed.

The archive name may differ; the stable paths above are what this machine uses
and what the OpenCode MCP registration points at.

### Install on CachyOS / Arch

```bash
sudo pacman -S --needed base-devel rustup wine mangohud
rustup default stable
```

Install Ghidra from the AUR or unzip a release. If your distribution's current
`jdk-openjdk` is Java 25, that works; otherwise install a full JDK 25 package
and point `JAVA_HOME` at it. Do not use `jre-openjdk`.

## Build

On this machine `cargo` is not on `PATH`; call it explicitly:

```bash
cargo build --release --bins
```

The binary is `target/release/game-opt-tool`.

## Verify the install

```bash
./target/release/game-opt-tool doctor
./target/release/game-opt-tool doctor --game "/path/to/your/game"

# With the decompilation environment exported:
./target/release/game-opt-tool decompile \
  --game "/path/to/your/game/DeusEx.exe"
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

# 7. Decompile one function (entry point by default).
export JAVA_HOME="$HOME/.local/opt/jdk-25.0.5+8"
export PATH="$JAVA_HOME/bin:$PATH"
export GHIDRA_INSTALL_DIR="$HOME/.local/opt/ghidra_12.2_PUBLIC"
./target/release/game-opt-tool decompile \
  --game "/path/to/your/game/DeusEx.exe"

# 8. With the game running at the scene, sample CPU time per function.
./target/release/game-opt-tool sample \
  --game "/path/to/your/game/DeusEx.exe" \
  --duration 20 --frequency 333 --top 20
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
| `No running process maps <game>` | Launch the game first, or use `--process <name>`/`--pid <pid>` if the path mapping is ambiguous. |
| `perf_event_open` permission denied | Check `/proc/sys/kernel/perf_event_paranoid`; the sampler needs `<= 2` for unprivileged use. `doctor` shows the probe result. |
| Sampling reports mostly external samples | The game is idle, in a menu, or most CPU time is in Wine/driver code. Sample a real gameplay scene; external samples are not game hotspots. |
| `Only ...% of in-game samples fell in gaps` | The Ghidra function map is stale or incomplete. Run with `--refresh-functions`, or use `--no-ghidra` for raw RVAs. |
| `invalid type: map, expected a sequence` | Old build. Fixed: `config/games.yaml` is parsed as a full config, then looked up by game name. |
| `No backups found` | Run `backup --game <file>` first. Backups only exist for files backed up by this tool. |
| `Backup checksum mismatch` | The `.bak` file was modified or corrupted. Restore refuses on purpose; use another backup or the original media. |
| `Refusing to restore ... symbolic link` | Safety guard. Point the command at the real file, not a symlink. |
| `directory is not writable` from `doctor` | Backups live next to the executable. Fix permissions or work on a copy. |
| `Ghidra not found` | Set `GHIDRA_INSTALL_DIR` to the directory containing `support/analyzeHeadless`. |
| `GHIDRA_INSTALL_DIR ... does not contain support/analyzeHeadless` | The variable points one level too deep or at a random directory. Point it at the extracted `ghidra_12.2_PUBLIC` root. |
| `Ghidra rejected the configured Java` | Ghidra 12.2 needs a full JDK 25. Check that `$JAVA_HOME/bin/javac` exists; a JRE or JDK 21 is rejected. |
| `Ghidra produced no decompilation result` | The address has no function, or Ghidra failed. Read the printed tail of Ghidra's log. Try a known function address with `--address`. |
| `Ghidra headless timed out` | Ghidra exceeded the 600-second limit. The binary may be unusually large or the JVM may be stuck; retry and check system memory. |
| `java`/`ghidra` warnings in `doctor` | Decompilation is optional. Export `JAVA_HOME` and `GHIDRA_INSTALL_DIR` as shown above to enable it; all other commands still work. |
