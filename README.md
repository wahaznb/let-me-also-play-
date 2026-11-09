# Game Opt Tool

Game Opt Tool is a local command-line tool for one narrow job:

> Take a game executable you own, find a few instruction-level changes that can
> be made safely **without running or measuring the game**, apply them to a
> copy, and keep a manifest that can undo every byte.

It is a reverse-engineering **candidate finder**, not an FPS booster. The static
analyzer (REA) only removes demonstrably dead instructions from PE x86/x86-64
binaries. It never launches the game, reads a MangoHud log, or claims a speedup.

If you only read one section, read [The loop](#the-loop-in-six-steps) and
[What "safe" means](#what-safe-means-and-what-it-does-not-mean).

---

## The loop in six steps

### 1. Identify the binary

You point the tool at a game executable. It detects the container format
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

`rea` decodes the executable sections with `iced-x86`, one instruction at a
time. It is a linear sweep: it does not build a control-flow graph, and it does
not know which bytes are code versus embedded data.

```text
PE executable sections
      │  iced-x86 linear sweep
      ▼
instruction stream + data-skips
```

### 3. Propose only provably dead instructions

The current catalog is deliberately small:

| Candidate | What was found | Replacement |
|-----------|----------------|-------------|
| `dead_move` | A self-move that cannot change the register, e.g. `mov rax, rax` | Same-length NOP |
| `dead_jump` | A near `jmp` whose target is the immediately following instruction | Same-length NOP |

Every replacement is **exactly the same length** as the original bytes, so no
following instruction moves and no relocation is needed.

Some things are deliberately *not* rewritten:

- `test reg, reg` and `and reg, reg` are **not** turned into `xor`. That would
  zero the register and change flags, so it is not behavior-preserving.
- `mov eax, eax` in 64-bit mode is **not** treated as dead. A 32-bit write
  zeroes the upper 32 bits of `RAX`, so removing it can change the register's
  value.

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
scanned, the patch refuses to apply. The Patcher independently re-checks the
address, the PE section, the expected bytes, and the target SHA-256.

### 5. Apply to a copy and back up anything you overwrite

The recommended command writes to a **separate output path**. The original game
executable is then untouched.

```text
original .exe ──────────────► unchanged
      │
      └── scan ──► spec ──► Patcher ──► patched copy
                                      + manifest
```

If the output path already exists, it is backed up first with a SHA-256 record.
Writes use a temporary file, `fsync`, and rename, so a crash cannot leave a
half-written executable. Signed PE binaries are refused by default because
patching would invalidate the signature.

### 6. Record and revert

The manifest records, for every change:

- the virtual address and file offset;
- the original bytes and the replacement bytes;
- the input and output SHA-256;
- the PE metadata needed to recognise the patched file.

Revert verifies the patched file's SHA-256 and the recorded bytes before it
changes anything. If the file was modified after patching, revert refuses
instead of forcing a restore.

```text
manifest ──► verify patched file ──► restore original bytes ──► verify result
```

---

## How your intent maps to the implementation

| What you asked for | What the tool actually does | Where it lives |
|---|---|---|
| "I don't want it to measure anything." | REA never launches the game, never reads MangoHud, and never samples CPU. `profile`, `sample`, and `compare` are separate, optional commands. | `src/rea/` |
| "Analyze the game and find a safe improvement." | Linear `iced-x86` sweep of executable PE sections, restricted to dead code. | `src/rea/decode.rs`, `src/rea/candidates.rs` |
| "Produce a patch." | Emits a JSON `PatchSpec` with expected bytes and replacement bytes. | `src/rea/patch.rs` |
| "Back up before applying." | Applying to a copy preserves the original. If the output copy already exists, it is backed up first. The manifest is the change record. | `src/backup/`, `src/patcher/` |
| "Apply it." | The verified Patcher re-checks everything and writes atomically. | `src/patcher/mod.rs` |
| "Track every change." | A JSON manifest records bytes, offsets, hashes, and PE metadata. | `src/patcher/mod.rs` |
| "Let me undo it." | `rea --revert --manifest ...` verifies and restores exactly. | `src/main.rs`, `src/patcher/mod.rs` |
| "Don't claim a speedup you didn't measure." | The REA path makes no performance claim. Removing dead code is a cleanup, not a proven FPS win. | `docs/rea.md` |

---

## What "safe" means, and what it does not mean

**Safe here means:** the replacement is provably equivalent at the instruction
level for the emitted candidate classes, is the same byte length, and is
guarded by the exact bytes that were scanned.

**Safe does not mean:**

- that the game will behave identically at runtime — REA does not run it;
- that the candidate is definitely code and not embedded data — the sweep is
  linear, so a candidate inside data is a false positive;
- that the change improves FPS, frame time, stutter, or latency — nothing is
  measured;
- that the binary is compatible with a different game build or patch version.

The expected-byte guard protects against a **different build**. It cannot
protect against a **false positive in the same build**. Review the reported
addresses before applying a patch to a game you care about.

---

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

# 2. Machine-readable report for scripts or agents
./target/release/game-opt-tool rea --game /tmp/Game.exe --json

# 3. Inspect the generated patch spec without applying it
./target/release/game-opt-tool rea --game /tmp/Game.exe --spec-only

# 4. Apply to a separate copy and write a named manifest
./target/release/game-opt-tool rea \
  --game /tmp/Game.exe \
  --output /tmp/Game-patched.exe \
  --manifest /tmp/Game.rea.json

# 5. Revert that copy exactly
./target/release/game-opt-tool rea \
  --game /tmp/Game-patched.exe \
  --revert --manifest /tmp/Game.rea.json
```

Optional backup commands for a file you intend to overwrite directly:

```bash
./target/release/game-opt-tool backup --game /tmp/Game.exe
./target/release/game-opt-tool backup --game /tmp/Game.exe --list
./target/release/game-opt-tool backup --game /tmp/Game.exe --verify
./target/release/game-opt-tool revert --game /tmp/Game.exe --dry-run
./target/release/game-opt-tool revert --game /tmp/Game.exe
```

---

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
| `optimize --game <path> --output <dir>` | **Stub.** Writes a copy with placeholder speedups; not a working optimizer. |
| `mcp-server [--transport stdio\|http] [--port P]` | Exposes the tools to AI agents over MCP. |
| `list-games`, `help <game>` | Lists configured targets / shows target notes. |

---

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
  restoring anything.
- **No network, no telemetry, no elevated privileges.**

---

## Why do I still see `.gitignore` and `.opencode`?

A leading dot is only a Unix naming convention. `ls` hides dotfiles by default;
`ls -a` shows them. Git does not hide files based on the dot.

- **`.gitignore` is tracked on purpose.** It contains the ignore rules that the
  repository needs. Git cannot ignore itself, and adding `.gitignore` to
  `.gitignore` would not remove it from the repository.
- **`.opencode/commands/optimize-game.md` is tracked on purpose.** It is the
  `/optimize-game` workflow that ships with the repository.
- **`.opencode/state/` is ignored.** That is local session state, not source.
- **`.vibe-wise/`, `.court/`, `target/`, and game files are ignored.** They are
  local or machine-specific and are not published.

If you want the OpenCode command to stay private instead of shipping it:

```bash
# Remove it from git, keep it on disk, and ignore it locally.
git rm -r --cached .opencode
printf '\n/.opencode/\n' >> .gitignore
git commit -m "chore: stop tracking local OpenCode commands"
```

If you only want a local ignore rule without changing the shared `.gitignore`,
put the pattern in `.git/info/exclude` instead.

---

## Optional: measurement and profiling

These commands exist and are tested, but they are **not used by REA** and are
not required for the safe-patch loop:

```bash
# Frame-time statistics from a MangoHud CSV you captured yourself
./target/release/game-opt-tool profile \
  --game "/path/to/your/game/Game.exe" \
  --log-dir /tmp/mangohud

# Compare two measured runs
./target/release/game-opt-tool compare \
  --baseline /tmp/mangohud/before.csv \
  --patched /tmp/mangohud/after.csv

# Sample self CPU time per function while the game is running (Linux only)
./target/release/game-opt-tool sample \
  --game "/path/to/your/game/Game.exe" \
  --duration 20 --top 20
```

Read [METRICS.md](METRICS.md) before treating a metric as a verdict, and
[WORKFLOW.md](WORKFLOW.md) for the optional measurement workflow. The sampler
sees self CPU time only; it is blind to GPU time, blocked time, and kernel time.
A hot function is still only a hypothesis, not a proven frame-time win.

---

## Status

| Area | Status |
|------|--------|
| PE / ELF / Mach-O detection and architecture detection | **Working** |
| Game config loading (`config/games.yaml`) | **Working** |
| `doctor` prerequisite checks | **Working** |
| Backup with SHA-256 + JSON manifest | **Working, tested** |
| Revert with checksum verification and atomic write | **Working, tested** |
| Static REA analysis (`rea`) | **Working, tested** — PE + x86/x64 linear sweep, guarded length-preserving candidates, JSON reports, reversible apply |
| Frame-time profiling from MangoHud CSV | **Working, tested** — optional, not used by REA |
| A/B log comparison | **Working, tested** — optional, not used by REA |
| Per-function CPU sampling (`sample`) | **Working on Linux** — real `perf_event_open` self-CPU samples, no `sudo`; blind to GPU/blocked time |
| Decompilation | **Working for one function at a time** — real Ghidra headless output; needs Ghidra 12.2 + JDK 25 |
| Verify | **Byte comparison only** — runtime equivalence is not implemented |
| Optimizer passes | **Stubs** — placeholder speedup values |
| PE byte patches / code caves | **Working for PE** — bounds-checked patches, optional appended executable sections, exact rollback |
| MCP server for AI agents | **Working, tested** — stdio and HTTP; bounded and paginated responses |

There are **no measured speedups** for any game yet. The tool does not fabricate
frame-time numbers: `profile` fails with instructions when no log is available,
and `verify` never reports a changed binary as runtime-safe.

---

## Supported games

| Game | Status |
|------|--------|
| **Deus Ex (2000)** | Test target: PE32/x86 detection, backup/revert, and Ghidra decompilation of entry point `0x109214B6` verified against the real executable |
| Oblivion Remastered, Skyrim SE, Jedi: Survivor | Config entries only; no target-specific work |

REA is not limited to these games. It scans any PE x86/x86-64 executable whose
license and ownership allow you to modify a local copy.

---

## How it works today

```text
Path → format/arch detection → config lookup → command
                                      │
                    ┌─────────────────┼──────────────────┐
                    ▼                 ▼                  ▼
                backup/revert     doctor            verify (bytes)
              SHA-256 + atomic

PE executable → linear iced-x86 sweep → safe candidates (length-preserving)
                                      │
                                      ▼
                       guarded patch spec → verified Patcher → manifest
                                                                  │
                                                                  ▼
                                                               revert

MangoHud CSV → frame times → avg / lows / percentiles / stutter
                                      │
                                      ▼
                             optional A/B compare

Running game → perf_event_open (per thread) → IP → /proc maps → PE RVA
                          │
                          ▼
                Ghidra function map (cached) → self CPU by function

PE/ELF/Mach-O → entry point or --address → Ghidra headless → pseudocode
```

The `optimize` command is still a stub. It is not part of the REA loop.

---

## Decompile a function (optional)

`decompile` runs Ghidra 12.2 in headless mode, imports the binary into a
temporary project, and prints the pseudocode and function signature. With no
`--address`, it uses the executable entry point:

```bash
export JAVA_HOME="$HOME/.local/opt/jdk-25.0.5+8"
export GHIDRA_INSTALL_DIR="$HOME/.local/opt/ghidra_12.2_PUBLIC"

./target/release/game-opt-tool decompile \
  --game "/path/to/your/game/Game.exe"

./target/release/game-opt-tool decompile \
  --game "/path/to/your/game/Game.exe" \
  --address 0x109214b6 --assembly
```

The command fails if Ghidra cannot start or returns no result. It never falls
back to placeholder pseudocode. Each invocation re-analyzes the binary, which
takes roughly 20–30 seconds for Deus Ex.

See [PREREQUISITES.md](PREREQUISITES.md) for the portable Ghidra/JDK setup.

---

## Use with an AI agent (MCP)

The tool exposes its real commands as MCP tools, so an agent such as OpenCode
or Claude Code can inspect, decompile, patch, and revert a binary without shell
access.

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

MCP output is designed for an agent context window, not for dumping a binary:
responses are capped, decompilation is paginated, and `profile`/`compare`
return statistics rather than raw CSV rows. The server is local-only by default
(`127.0.0.1`) and stores no API keys.

The `/optimize-game` command in `.opencode/commands/` walks through the static
REA loop and stops for approval at each gate. See [WORKFLOW.md](WORKFLOW.md) for
the user/AI/tool split.

---

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
apply → revert flow, including refusal after a guarded byte is changed.

Run the real Ghidra test explicitly:

```bash
export JAVA_HOME="$HOME/.local/opt/jdk-25.0.5+8"
export GHIDRA_INSTALL_DIR="$HOME/.local/opt/ghidra_12.2_PUBLIC"
export GAME_OPT_TEST_BINARY="/path/to/your/game/DeusEx.exe"
cargo test --test cli_decompile \
  decompiles_a_real_binary_entry_point -- --ignored
```

---

## Legal

Personal use only, on games you own. No DRM circumvention, no asset
distribution, single-player only. See [LEGAL.md](LEGAL.md).

## License

MIT — see [LICENSE](LICENSE).
