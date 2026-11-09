---
description: Static REA loop — scan a binary, review safe candidates, apply to a copy, record a reversible manifest. No measurement.
---

Run a static reverse-engineering analysis (REA) session for one owned
single-player game binary. Personal use only; no DRM circumvention and no asset
distribution.

Game path: $ARGUMENTS
If the argument above is empty, ask me for the path.

Read `docs/rea.md` in this repository first. It defines what REA can and cannot
prove.

## Non-negotiable rules

- Use the `game-opt-tool rea` CLI for scanning and patching. Do not edit the
  Rust source unless I explicitly ask for tool changes.
- Never patch the original binary. Always write to a separate `--output` copy.
- REA does not run the game and does not measure FPS, frame times, or CPU.
  Never claim a speedup, a latency win, or a frame-time improvement.
- Dead-code candidates remove work. REA does not rewrite flag-sensitive idioms
  such as `test reg,reg`; only provably dead code is emitted.
- Every applied change must have a manifest. No manifest, no patch.
- Stop at every gate and wait for my explicit approval before continuing.

## Phase 0 — readiness (no writes)

1. Run `./target/release/game-opt-tool doctor` and report whether the binary is
   built and whether the analysis prerequisites are available.
2. Confirm `./target/release/game-opt-tool rea --help` works.
3. If the binary is missing, stop and tell me to run `cargo build --release`.

## Phase 1 — scan (gate)

1. Ask me which executable to analyze if the path was not supplied.
2. Run:
   `./target/release/game-opt-tool rea --game <path> --json`
3. Report: game, format, architecture, each executable section scanned (bytes,
   instructions, data skips), total candidates, and counts by kind/class.
4. State the limits: PE only, linear sweep (not control-flow aware), and no
   runtime or GPU behavior is observed.

Stop and wait.

## Phase 2 — review candidates (gate)

1. Group candidates by class:
   - `dead_code` — `dead_move`, `dead_jump`
2. Explain in plain language what each candidate changes. Do not paste the full
   candidate list if it is long; show the first 10 and the counts.
3. State explicitly: "No speedup is claimed. This is a static change list."
   Also state that the sweep is linear, so a candidate inside embedded data is
   a false positive and addresses should be reviewed before applying.
4. Ask me whether to generate a spec, apply to a copy, or stop.

Stop and wait.

## Phase 3 — spec or apply (gate)

1. To show the patch spec without writing:
   `./target/release/game-opt-tool rea --game <path> --spec-only`
2. To apply, write to a copy and a named manifest:
   `./target/release/game-opt-tool rea --game <path> --output /tmp/<name>.patched.exe --manifest /tmp/<name>.rea.json`
3. Never use the original path as `--output`.
4. Report the output path, manifest path, and both SHA-256 hashes.

## Phase 4 — record (gate)

1. Show the manifest summary: every patched address, original bytes, replacement
   bytes, and the expected-byte guard.
2. Tell me the exact revert command:
   `./target/release/game-opt-tool rea --game <patched-copy> --revert --manifest <manifest>`
3. Do not delete the manifest.
4. Stop.

## Phase 5 — revert (on request)

1. Run the revert command from Phase 4, passing the patched copy as `--game`.
2. Report the restored SHA-256 and confirm it matches the original binary hash.
3. If the patched file was modified after patching, stop and report the
   mismatch instead of forcing a restore.

## Output format

After each phase, give a compact fact sheet:

```text
PHASE:
RAN:
FOUND:
UNCERTAIN:
NEXT DECISION:
```

Never dump raw disassembly or a full candidate list into the chat. Summarize
counts and show a short sample.
