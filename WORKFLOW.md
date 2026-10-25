# Guided optimization: who does what

This is the honest workflow. **You** own the game and the decision. **The AI**
reasons about decompiled code and proposes one patch at a time. **The tool**
does the deterministic, reversible work: hashing, backups, Ghidra, statistics,
and byte patches.

> There is no automatic optimizer yet. The `optimize` CLI subcommand is a stub:
> it prints a warning and writes a byte-identical copy. Real changes go through
> a JSON patch spec, a dry run, a copy, and an A/B measurement.

## Division of labor

| | You (user) | AI (OpenCode) | Tool (game-opt-tool) |
|---|---|---|---|
| Owns | Game files, saves, play-testing, final decision | Analysis and the patch proposal | Deterministic local operations |
| Reads | Tool reports, game behavior | Decompiled functions, profiles, manifests | Binary bytes, MangoHud CSV, Ghidra output |
| Produces | Baseline/after runs, approvals | Candidate list, explanation, patch spec, verdict | Backup, patched copy, manifest, statistics |
| Never | Apply a patch blindly or trust average FPS alone | Patch the original or claim an unmeasured speedup | Invent "optimized code" or launch the game |

### You do

1. Provide the absolute binary path and a repeatable scene/save.
2. Capture the baseline and after runs with MangoHud. The tool never launches
   the game or plays it for you.
3. Approve each gate. The command is designed to stop and wait.
4. Play-test the patched copy for behavior and quirks.
5. Decide **keep** or **discard** from the stats plus your own experience.
6. Keep the original untouched and the patch manifest safe.

### The AI does

1. Inspects the binary, architecture, entry point, and hash.
2. Lists function handles and ranks a few candidates. This is a heuristic based
   on names and sizes, **not** real per-function profiling data.
3. Decompiles one function per call, summarizes it, and follows `next_offset`
   when a page is truncated.
4. Proposes exactly one behavior-preserving patch as a JSON spec, with the
   expected effect and risk.
5. Runs the patch with `dry_run: true` before applying anything.
6. Applies only to a copy, and only after your explicit approval.
7. Reads `profile` / `compare` and gives an evidence-based keep/discard
   recommendation, including when the runs are too short to conclude.

### The tool does

| Tool | Deterministic job |
|---|---|
| `game_info` | PE/ELF/Mach-O detection, architecture, entry point, SHA-256 |
| `list_functions` | Address/name/size handles only; paginated, no pseudocode |
| `decompile` | Real Ghidra headless output; one function, paginated, bounded |
| `profile` | Avg FPS, 1% low, 0.1% low, p99 frame time, stutter ratio |
| `compare` | Baseline vs patched deltas, with direction and warnings |
| `patch` | Expected-byte-verified byte patches and one optional code cave |
| `unpatch` | Exact reversal from a manifest |
| `backup` / `backup_verify` | SHA-256 checked backup and verification |
| `revert` (CLI) | Restore from a verified backup |
| `verify` | Byte/section diff only; never claims runtime correctness |

It also enforces the safety and context rules: bounds checks, expected bytes,
SHA-256, atomic writes, symlink refusal, response caps, and truncation
metadata.

### The tool does not do

- Find hot functions by sampling. There is no sampling profiler yet.
- Rewrite C pseudocode into machine code automatically.
- Launch the game or capture frames.
- Prove a patch is behaviorally correct at runtime.
- Guarantee a speedup. Only `compare` can show one.

## The loop

| # | Step | Who | What happens | Gate |
|---|---|---|---|---|
| 0 | Setup | You + AI | `doctor`, Ghidra/Java check, path and scene confirmed | AI reports readiness |
| 1 | Identify + protect | AI | `game_info`, `backup`, hash | **Stop.** You approve continuing |
| 2 | Baseline | You, then AI | You capture a MangoHud run; AI runs `profile` | **Stop.** No baseline, no patch |
| 3 | Candidates | AI | `list_functions` with `filter`/`limit`; rank 3–5 | **Stop.** You pick a function |
| 4 | Decompile | AI | `decompile` one function, one page at a time | **Stop.** You approve drafting a patch |
| 5 | Patch proposal | AI | JSON spec + `patch` with `dry_run: true` | **Stop.** You approve applying |
| 6 | Apply to a copy | AI | Patch writes a copy + manifest | You launch and test that copy |
| 7 | Compare | You, then AI | You capture the after run; AI runs `profile` + `compare` | **Stop.** Keep or discard |
| 8 | Close | You | Keep manifest/copy, or discard the copy | Original hash unchanged |

### CLI equivalents

```bash
cd game-opt-tool
BIN=./target/release/game-opt-tool
GAME="/path/to/your/game/DeusEx.exe"
```

| Step | Command |
|---|---|
| Readiness | `$BIN doctor` |
| Protect | `$BIN backup --game "$GAME"` |
| Verify backups | `$BIN backup --game "$GAME" --verify` |
| Baseline/after stats | `$BIN profile --game "$GAME" --log-dir /tmp/mangohud` |
| A/B comparison | `$BIN compare --baseline before.csv --patched after.csv` |
| Decompile | `$BIN decompile --game "$GAME" --address 0x1090A050 --json` |
| Patch dry run | `$BIN patch --game "$GAME" --output /tmp/DeusEx.patched.exe --spec /tmp/patch.json --dry-run` |
| Patch a copy | `$BIN patch --game "$GAME" --output /tmp/DeusEx.patched.exe --spec /tmp/patch.json` |
| Reverse a copy | `$BIN unpatch --manifest "<manifest path from the report>"` |
| Restore the original | `$BIN revert --game "$GAME"` |

## Measurement contract

A patch is only a candidate until it survives measurement:

- Use the **same scene, settings, and roughly the same duration** for both runs.
  Aim for at least 60 seconds; longer is better.
- Judge **1% low, 0.1% low, p99 frame time, and stutter ratio** first.
  Average FPS is context, not the verdict.
- One variable at a time: one patch, then one comparison.
- Treat short runs, mismatched durations, and tail trimming as warnings.
- Never say "faster" without a `compare` result. If the result is ambiguous,
  say it is ambiguous.

## Context contract

The tool is built for a ~16k-token working budget:

- One function per `decompile` call (~24 KB of pseudocode by default).
- Follow `next_offset` when `truncated` is true; do not re-request the same page.
- Use `list_functions` with `filter` and `limit` instead of dumping all 945
  handles.
- Request `include_assembly` only when instruction-level detail matters.
- If a response is replaced by a `{"truncated": true, ...}` envelope, narrow the
  request: one address, a smaller page, fewer frames.
- The hard cap is 60 KB (~15k tokens) by default. Override with
  `GAME_OPT_MCP_MAX_BYTES` (4 KB–1 MB).

## Safety contract

- Patch a **copy**, never the original.
- `dry_run` first; a patch is refused unless every expected byte matches.
- Keep the manifest. It is the only supported way to reverse exactly.
- Signed PE files are refused unless `allow_signed` is set; at most one
  appended executable section is allowed.
- `verify` is truthful: a changed binary is not "verified safe" just because
  the bytes differ as expected.
- If a copy behaves oddly, discard it. Do not chase a speedup at the cost of
  game behavior.

## Starting it

In OpenCode, with this repository as the project:

```text
/optimize-game
```

or with an explicit path:

```text
/optimize-game /path/to/your/game/DeusEx.exe
```

The command stops after inspection/backup, after the candidate list, after the
decompile, and before applying any patch. Reply `continue` or pick the function
at each gate.

To make the command available outside this repository:

```bash
mkdir -p ~/.config/opencode/commands
cp .opencode/commands/optimize-game.md ~/.config/opencode/commands/
```

## Ghidra requirement

`decompile` and `list_functions` need Ghidra and Java in the MCP server's
environment:

```bash
export GHIDRA_INSTALL_DIR=/path/to/ghidra
export JAVA_HOME=/path/to/jdk
```

Everything else — backup, profile, compare, patch, unpatch, revert, verify —
works without them. `doctor` reports what is missing.

## Known limits

- Hotspot discovery is manual. The profiler reports frame-time statistics, not
  per-function samples.
- Every Ghidra call re-analyzes the binary; expect ~20–30 seconds per call.
- No automatic patch generation, runtime verification, or measured speedup has
  shipped yet.
- The `optimize` subcommand remains a stub and is not part of this loop.
