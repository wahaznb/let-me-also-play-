# Game Opt Tool — Verified Status

Last verified: 2026-10-24, commit `4b19b77` + local changes.

## One-line summary

Binary detection, config loading, backup/revert, `doctor`, and byte-level
verification are implemented and tested. Profiling, decompilation, optimization,
and real binary patching are still placeholders and are labelled as such in the
CLI.

## Test evidence

```bash
~/.cargo/bin/cargo test
# 42 unit tests + 8 end-to-end CLI tests — all passing

~/.cargo/bin/cargo build --release --bins
# 0 errors
```

Real-world check against `/path/to/your/game/DeusEx.exe`
(253,952 bytes, PE32/x86):

1. `doctor --game` detected `DeusEx (PE, x86)` and a writable backup directory.
2. `backup` created a manifest and a backup with SHA-256
   `3e6fdb89b880a60ee984b885a48f262070019572f1531ed70d96108781bae00e`.
3. The working copy was overwritten with junk, then `revert` restored it.
4. `cmp` and `sha256sum` confirmed byte-for-byte equality with the original.

## What is real

| Feature | Implementation |
|---------|----------------|
| PE detection | `MZ` header + `PE\0\0` signature at `e_lfanew` (previously checked `ME`, a bug) |
| ELF / Mach-O detection | Magic bytes, both Mach-O endiannesses |
| Architecture detection | goblin PE/ELF machine types |
| Config | `config/games.yaml` parsed as a full `Config`; fuzzy game-name lookup |
| Backup | Content-addressed `.bak` + `.bak.json` manifest, `create_new`, SHA-256 |
| Revert | Checksum verification, symlink refusal, atomic temp+rename, read-back verification, permission preservation, dry-run |
| Verify | SHA-256/byte comparison; changed binaries are reported as NOT VERIFIED |
| Doctor | Config parse, Wine/MangoHud/Java/Ghidra presence, binary detection, writable backup dir, existing backups |

## What is still a stub

| Feature | Current behavior |
|---------|------------------|
| `profile` | Returns hardcoded numbers and prints a warning |
| `analyze` hot functions | Hardcoded names/percentages and prints a warning |
| `optimize` | Prepends marker comments in memory; no code generation |
| `patch` | Writes a byte-identical copy; refuses in-place writes without a backup |
| `verify` runtime behavior | Not implemented; only bytes are compared |
| Decompilation | Requires Ghidra + Java, neither installed here |

## Fixed in this pass

- `GameInfo::load_config()` no longer parses `games.yaml` as a single game;
  it parses `Config` and looks the game up. `analyze` therefore completes.
- `Config::load()` now picks YAML/TOML by extension instead of parsing YAML
  with the TOML parser.
- PE detection used `M`/`E` instead of `M`/`Z`; fixed and unit-tested.
- Clap's built-in `help` subcommand collided with the custom `Help` variant in
  debug builds; `disable_help_subcommand` fixes it.
- `optimize --output <dir>` previously passed a directory to the patcher as if
  it were a file; it now resolves to `<dir>/<game exe>`.
- The verifier no longer fabricates a +25% gain or an unconditional PASS.
- Replaced stale docs that claimed a working pipeline and measured 2× gains.

## Known limitations

- No frame-time measurement yet (MangoHud CSV ingestion is not implemented).
- No Ghidra/JDK installed, so decompilation is untested end-to-end.
- No real patch code generation, no code caves, no relocation handling.
- Restore-over-existing on Windows is remove-then-rename rather than atomic.
- Backup creation reads the whole file into memory (fine for game executables,
  not for multi-GB assets).
- The backup directory is the executable's directory; read-only installs need
  a writable copy.

## Next steps, in priority order

1. **Real measurement first.** Ingest MangoHud CSV logs produced under Wine and
   compute avg / 1% low / p99 frame times, stutter ratio, and run-to-run
   confidence. Replace all placeholder profiler values.
2. **Decompilation.** Install JDK 21 + Ghidra, validate headless mode under
   `xvfb-run`, and cache function lists per binary hash.
3. **Real patching.** PE section parsing, code caves, relocations, and a patch
   manifest that records every modified byte range for precise rollback.
4. **Runtime A/B verification.** Launch original and patched builds with the
   same save file, compare screenshots/state at checkpoints, and gate patches
   on measured frame-time consistency (1% lows before raw FPS).
5. **Crash-safe experiment loop.** If the patched build fails to start, offer
   one-command revert automatically.
