# Game Opt Tool — Verified Status

Last verified: 2026-10-24. Baseline commit `b27ad1e`; this document includes
the real Ghidra decompilation work built on top of the measurement and A/B
comparison passes.

## One-line summary

Binary detection, config loading, backup/revert, `doctor`, byte-level
verification, real frame-time measurement from MangoHud CSV logs, A/B
comparison of measured runs, and single-function Ghidra decompilation are
implemented and tested. Function-level profiling, optimization, and binary
rewriting are still placeholders and are labelled as such.

## Test evidence

```bash
~/.cargo/bin/cargo test --all-targets
# 79 unit tests + 25 end-to-end CLI tests — all passing, 1 real-Ghidra test ignored

~/.cargo/bin/cargo build --release --bins
# 0 errors, 6 dead-code/style warnings
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

Real decompilation check against `DeusEx.exe`:

- The tool computed the PE entry point `0x109214B6` (image base + entry RVA)
  and Ghidra decompiled the CRT `entry` function at that address.
- The output file contained a signature (`undefined entry(void)`), return type,
  parameters, real pseudocode, and an x86 instruction listing.
- `decompile --json` reported the same address and pseudocode; `--assembly`
  printed instructions beginning with `PUSH EBP` / `MOV EBP,ESP`.
- The decoder rejects missing output, an unterminated pseudocode section, empty
  pseudocode, and malformed addresses. There is no placeholder fallback.

Real A/B measurement check:

- Captured two genuine MangoHud logs of `glxgears` (~3 seconds each), one
  vsync-limited and one uncapped.
- `compare` reported average FPS `48.01 → 1382.81` (+2780%), 1% low
  `43.90 → 71.88`, and stutter ratio `1.06x → 58.71x`.
- The tool therefore exposed the raw-FPS gain as a frame-time consistency
  regression, which is the tradeoff this project exists to catch.
- Both logs had real metadata preambles; discovery skipped the `_summary.csv`
  files and selected the detailed logs.
- The baseline's average FPS `48.01` and frame time `20.83 ms` match MangoHud's
  own summary (`48.0`, `20.8`); the 0.1% low matches at `42.45 FPS`.

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
| Measurement output | Human-readable text or `--json`; `--frames N` analyzes the last N frames |

## What is still a stub

| Feature | Current behavior |
|---------|------------------|
| `analyze` hot functions | Not implemented; analyze reports measured frame times only and says so |
| Automatic hot-function discovery | Not implemented; `decompile` needs an address or defaults to the entry point |
| `optimize` | Stub passes; speedup values are placeholders |
| `patch` | Writes a byte-identical copy; refuses in-place writes without a backup |
| `verify` runtime behavior | Not implemented; only bytes are compared |
| Launch automation | The user launches the game under MangoHud manually |

## Fixed in the measurement pass

- Removed the synthetic frame-time generator; `profile` now requires a real
  MangoHud log and prints instructions when none exists.
- Removed hardcoded hot-function percentages from `analyze`; it no longer
  presents invented function data as measurement.
- Added streaming parsing so large logs do not need to fit in memory.
- Handles MangoHud metadata preamble rows before the CSV header, discovered by
  testing against a genuine log.
- Retains long frames instead of silently dropping stalls; rejects
  non-finite/overflowing statistics.
- Added `--frames` tail analysis and `--json` output.
- Added `compare` for A/B log comparison with per-metric improvement direction
  and explicit short-run, duration, scene, and significance caveats.

## Decompilation pass

- Replaced the previous skeleton with a real `support/analyzeHeadless`
  invocation. The old code checked for `support/analyze` and used an
  unsupported Python script, so it could never work.
- Added a Java `GhidraScript` that resolves the function containing the
  requested address, runs `DecompInterface`, and writes a deterministic result
  file.
- Added entry-point detection from the binary (PE image base + RVA, ELF entry,
  Mach-O entry), a `decompile` CLI command, `--address`, `--assembly`, and
  `--json`.
- Output parsing requires a name, entry address, and non-empty pseudocode;
  malformed or missing output fails the command instead of inventing a result.
- Ghidra runs in a unique temporary project, has a 600-second timeout, and the
  project is deleted afterward.
- `doctor` now verifies `support/analyzeHeadless`, checks for a full JDK rather
  than a JRE, and reports the Java major version required by the installed
  Ghidra.

## Known limitations

- No function-level profiling. Frame times alone cannot identify which
  function is hot; that needs CPU sampling or instrumentation.
- A/B comparison is single-run: no confidence intervals, multi-run aggregation,
  or automatic scene alignment.
- No automatic game launch or save-state handling.
- 1% low is defined as the mean of the slowest 1% of frames, which differs
  from tools that report a single percentile.
- MangoHud must be able to overlay the game; anti-cheat or unsupported
  renderers can block capture. Deus Ex under Wine is the intended path.
- Each `decompile` invocation re-imports and re-analyzes the binary (about
  20–30 seconds for Deus Ex). Ghidra project/function caching is not implemented.
- Automatic hot-function discovery is not implemented, and a decompiled
  function is not yet wired into the optimizer.
- Ghidra is a large third-party Java application parsing the target binary. Only
  analyze game files you trust.
- No real patch code generation, no code caves, no relocation handling.
- Restore-over-existing on Windows is remove-then-rename rather than atomic.
- Backup creation reads the whole file into memory (fine for game executables,
  not for multi-GB assets).
- The backup directory is the executable's directory; read-only installs need
  a writable copy.

## Next steps, in priority order

1. **Real patching.** PE section parsing, bounds-checked byte patches, code
   caves, relocations, and a patch manifest that records every modified byte
   range for precise rollback.
2. **Runtime A/B verification.** Launch original and patched builds with the
   same save file, compare frame-time distributions with `compare`, and gate
   patches on measured 1% lows before raw FPS.
3. **Hot-function discovery and decompiler caching.** Add CPU sampling or
   instrumentation to identify candidates, cache Ghidra analysis per binary
   hash, and feed a selected function into the optimizer.
4. **Multi-run confidence and launch automation.** Repeat captures, aggregate
   run-to-run variance, and add a one-command capture helper that launches the
   game under MangoHud.
5. **Crash-safe experiment loop.** If the patched build fails to start, offer
   one-command revert automatically.
