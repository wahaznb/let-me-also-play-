# Installation

## Requirements

- **Rust 1.80 or newer** (`std::sync::LazyLock` is used).
- **Linux, macOS, or Windows.** Development and testing happen on Linux
  (x86_64); the Windows and macOS jobs in CI compile the same code.
- **Write permission to the directory that holds the executable** if you use
  `backup`, `revert`, or `--output` next to the game. Backups live beside the
  file they protect, so restoring never depends on a database.

No root, no network access at runtime, and no telemetry. The tool never
downloads anything on its own.

## Build

```bash
git clone https://github.com/wahaznb/let-me-also-play-.git
cd let-me-also-play-
cargo build --release --bins
```

The binary is `target/release/game-opt-tool`.

## First check

`doctor` inspects the config, the optional external tools, and the directory you
intend to work in. It does not need a game.

```bash
./target/release/game-opt-tool doctor
./target/release/game-opt-tool doctor --game "/path/to/your/game/Game.exe"
```

## Optional: decompilation

`decompile`, `list_functions`, and function naming in `sample` need Ghidra and
a full JDK. Ghidra 12.2 is built against **Java 25**; a JRE or JDK 21 is
rejected, and `$JAVA_HOME/bin/javac` must exist.

```bash
export JAVA_HOME="$HOME/.local/opt/jdk-25.0.5+8"
export PATH="$JAVA_HOME/bin:$PATH"
export GHIDRA_INSTALL_DIR="$HOME/.local/opt/ghidra_12.2_PUBLIC"
```

`GHIDRA_INSTALL_DIR` must point at the Ghidra root that contains
`support/analyzeHeadless`, not at `support` itself.

Download links:

- JDK 25: <https://adoptium.net/>
- Ghidra 12.2: <https://github.com/NationalSecurityAgency/ghidra/releases>

Extract both somewhere permanent (not `/tmp`). `doctor` reports whether the
paths are usable; missing Ghidra is a warning, not a failure.

## Optional: CPU sampling

`sample` is Linux-only. It calls `perf_event_open` directly and does not need
the `perf` binary or `sudo`.

| Requirement | Detail |
|---|---|
| Linux with `perf_event_open` | x86_64 tested |
| `perf_event_paranoid <= 2` | 1 or 2 works unprivileged; 3 and 4 deny it |
| Same user as the game | The sampler reads `/proc/<pid>/maps` |
| Running game | It samples an existing process, it does not launch one |
| Ghidra + JDK (optional) | Only for function names; `--no-ghidra` prints raw RVAs |

```bash
cat /proc/sys/kernel/perf_event_paranoid
./target/release/game-opt-tool doctor
```

## Optional: frame-time logs

`profile` and `compare` read MangoHud CSV files that you capture yourself. On
Linux this works for native games and for Windows games under Wine:

```bash
MANGOHUD_CONFIG="output_folder=/tmp/mangohud,autostart_log=1,log_duration=0" \
  mangohud --dlsym wine Game.exe
```

`log_duration=0` logs until the game exits. Use the `mangohud` wrapper rather
than setting `MANGOHUD=1` by hand.

## Platform notes

### Linux

Everything works after `cargo build`. Wine and MangoHud are only needed for
measurement, not for the static REA loop.

### Windows

- Build with `cargo build --release` in a normal terminal; no admin required.
- Windows locks executables that are running. Close the game before backing up
  or patching.
- Work in a normal folder such as `C:\Users\you\games\`, not `Program Files`.
- Antivirus software may flag a patched executable. Exclude your work folder if
  you trust the changes you made.
- `sample` returns an unsupported-platform error on Windows.

### macOS

The code compiles for x86_64 and Apple Silicon. `sample` is gated to Linux and
returns a clear error instead of failing to build. Ghidra and the backup/patch
commands work the same way as on Linux.

### Cross-compiling

```bash
# Windows GNU target from Linux (needs mingw-w64 to link)
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu

# macOS
rustup target add x86_64-apple-darwin
cargo build --release --target x86_64-apple-darwin
```

CI builds Windows (MSVC and GNU), Linux, and macOS artifacts on every push.

## Common problems

| Symptom | Fix |
|---|---|
| `cargo: command not found` | Install Rust from <https://rustup.rs/> and open a new shell. |
| `doctor` says the backup directory is not writable | Work on a copy in a writable folder, or fix permissions. |
| `Backup checksum mismatch` | The `.bak` file was modified. Restore refuses on purpose; use another backup. |
| `Refusing to restore ... symbolic link` | Point the command at the real file, not a symlink. |
| `Ghidra not found` | Set `GHIDRA_INSTALL_DIR` to the directory containing `support/analyzeHeadless`. |
| `Ghidra rejected the configured Java` | Use a full JDK 25; check that `$JAVA_HOME/bin/javac` exists. |
| `Ghidra produced no decompilation result` | The address may not be a function. Try a known address with `--address`. |
| `perf_event_open` permission denied | Lower `perf_event_paranoid` to 2 or less, or use your distribution's `perf` capability setup. |
| `No MangoHud CSV logs found` | Check that the log folder contains a CSV with an `fps` or `frametime` header. |

## Storage

| Item | Size |
|---|---|
| Tool binary | ~3 MB |
| Backup (`.bak`) | Same size as the executable |
| Patch manifest | 1–2 KB |
| Ghidra (optional) | ~1.5 GB |
| JDK 25 (optional) | ~300 MB |
