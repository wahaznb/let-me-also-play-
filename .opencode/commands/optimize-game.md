---
description: Guided optimization loop — inspect, decompile one function, patch a copy, measure, compare
---

Run a guided, measurement-first optimization session for one owned single-player
game binary. Personal use only; no DRM circumvention and no asset distribution.

Game path: $ARGUMENTS
If the argument above is empty, use:
`/path/to/your/game/DeusEx.exe`

Read `WORKFLOW.md` in this repository first. It defines the user/AI/tool split,
the measurement contract, and the safety rules.

## Non-negotiable rules

- Use the `game-opt-tool` MCP tools for inspection, decompilation, profiling,
  patching, and verification. Do not edit the Rust source unless I explicitly
  ask for tool changes.
- Never patch the original binary. Work on a copy under `/tmp`.
- One function per `decompile` call. One patch at a time.
- Never claim a speedup without `compare` evidence. Average FPS alone is not a
  win; 1% low, 0.1% low, p99, and stutter ratio decide.
- Do not run the `optimize` CLI subcommand; it is a stub that writes an
  identical copy.
- Stop at every gate and wait for my explicit approval before continuing.

## Phase 0 — readiness (no writes)

1. Confirm the `game-opt-tool` tools are available. If `list_functions` is
   missing, stop and tell me the MCP server is stale and OpenCode must be
   restarted.
2. Run the read-only `./target/release/game-opt-tool doctor` command and report
   whether Ghidra and Java are visible. Decompilation cannot work without them.
3. Ask me for a repeatable scene/save and the path to the baseline MangoHud CSV
   if one exists.
4. If anything required is missing, stop and list exactly what is missing.

## Phase 1 — identify and protect (gate)

1. Call `game_info` on the game path. Report format, architecture, and entry
   point.
2. Call `backup` on the game path. Report the backup manifest path and hash.
3. Do not decompile or patch yet.

Show a compact fact sheet, then stop and wait for `continue`.

## Phase 2 — baseline (gate)

1. If I gave you a baseline CSV, call `profile` and report: frame count, average
   FPS, 1% low, 0.1% low, p99 frame time, and stutter ratio.
2. If there is no baseline, tell me exactly how to capture one with MangoHud and
   stop. Do not propose a patch without a baseline.

## Phase 3 — choose candidates (gate)

1. Call `list_functions` with a small limit (20–50). If I named a subsystem, use
   its `filter`.
2. Rank 3–5 candidates using name, size, and my description of the slowdown.
   State clearly that this ranking is a heuristic, not profile data.
3. Ask me which function to inspect. Do not decompile all of them.

Stop and wait.

## Phase 4 — decompile one function (gate)

1. Call `decompile` for the chosen function only.
2. If the result has `truncated: true`, follow `next_offset` until you can
   explain the function. Summarize each page; do not paste more than one page
   into the conversation.
3. Request `include_assembly: true` only if the instruction-level change matters.
4. Explain: what the function does, where time could plausibly go, one concrete
   behavior-preserving change candidate, and the uncertainty in that hypothesis.

Stop and wait for approval to draft a patch.

## Phase 5 — draft exactly one patch (gate)

1. Write a JSON patch spec to `/tmp/deusex-<function>-patch.json`. Use
   `schema_version: 1`, a descriptive `name`, `target_sha256` when useful, and
   exact `expected` bytes taken from the real binary.
2. Explain the expected effect and the behavioral risk in plain language.
3. Call `patch` with `dry_run: true`, input = the original game path, output =
   `/tmp/DeusEx.<function>.patched.exe`. Report the validation result.
4. Do not apply the patch. Wait for my explicit `apply`.

## Phase 6 — apply to a copy only

1. Call `patch` without `dry_run`, same input/output/spec.
2. Report the output path and the manifest path. Keep the manifest.
3. Tell me to launch that copy under Wine with MangoHud for the same scene, and
   give me the exact command.
4. Never touch the original file.

## Phase 7 — compare and verdict (gate)

When I provide the patched CSV:

1. Call `profile` on the patched log.
2. Call `compare` with the baseline and patched logs.
3. Report deltas for 1% low, 0.1% low, p99, stutter ratio, and average FPS.
4. Recommend KEEP or DISCARD based on consistency, not average FPS. If the run
   is short, mismatched, or ambiguous, say so instead of declaring a win.
5. KEEP: keep the patched copy and manifest, and summarize what changed.
   DISCARD: leave the original untouched; optionally call `unpatch` on the copy
   and confirm the original hash is unchanged.
6. Stop.

## Output format

After each phase, give a compact fact sheet:

```text
PHASE:
RAN:
FOUND:
UNCERTAIN:
NEXT DECISION:
```

Never dump raw decompilation, full disassembly, or CSV rows into the chat.
Follow `next_offset` for more pseudocode. If a response is replaced by a
`{"truncated": true, ...}` envelope, narrow the request instead of retrying it
unchanged.
