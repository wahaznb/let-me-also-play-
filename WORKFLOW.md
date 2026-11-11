# Workflow

This is the loop I actually use: scan, review, patch a copy, test, and keep the
manifest. No step depends on a measurement, and no step requires an AI agent.

## 1. Pick a file and protect it

Work on a copy. The original should never be the patch target while you are
experimenting.

```bash
cp "/path/to/game/Game.exe" /tmp/Game.exe
./target/release/game-opt-tool backup --game /tmp/Game.exe
```

The backup is stored next to the copy and has a SHA-256 record.

## 2. Scan

```bash
./target/release/game-opt-tool rea --game /tmp/Game.exe
```

The report lists candidates with an address, the original bytes, and the
replacement. Read the addresses. A linear sweep can find a candidate inside
embedded data; those are false positives even though the bytes match.

Use `--json` when you want to script the review.

## 3. Inspect the spec

```bash
./target/release/game-opt-tool rea --game /tmp/Game.exe --spec-only > /tmp/rea.json
```

The spec contains the exact expected bytes and the input SHA-256. If the file is
not the one that was scanned, the patcher will refuse it.

At this point you have three choices:

- **Discard** the spec. Nothing has been written.
- **Edit** the spec by hand to keep only the addresses you trust.
- **Apply** it to a separate copy.

## 4. Apply to a copy

```bash
./target/release/game-opt-tool rea \
  --game /tmp/Game.exe \
  --output /tmp/Game-patched.exe \
  --manifest /tmp/Game.rea.json
```

The original copy is unchanged. The manifest records every byte that changed.

## 5. Test behavior

Run the patched copy yourself. The tool cannot do this for you and does not
claim the patch is behaviorally correct.

If the game crashes, behaves oddly, or saves differently, discard the copy. Do
not chase a result at the cost of the game working.

## 6. Keep or revert

If you keep it, keep the manifest with the patched file. If you want the
original back:

```bash
./target/release/game-opt-tool rea \
  --game /tmp/Game-patched.exe \
  --revert --manifest /tmp/Game.rea.json
```

Revert checks the patched file's SHA-256 and the recorded bytes first. If the
file changed after patching, it refuses rather than guessing.

## Optional: measure before you decide

Measurement is a separate workflow. It never feeds the candidate list.

1. Capture a baseline with MangoHud.
2. Patch a copy and capture the same scene again.
3. Compare the two logs:

```bash
./target/release/game-opt-tool profile --game /tmp/Game.exe --log-dir /tmp/mangohud
./target/release/game-opt-tool compare \
  --baseline /tmp/mangohud/before.csv \
  --patched /tmp/mangohud/after.csv
```

Judge the slow tail (1% low, 0.1% low, p99 frame time, stutter ratio), not
average FPS. One comparison is one sample; short runs, mismatched durations, and
different scenes are warnings, not evidence. Read [METRICS.md](METRICS.md)
before treating a number as a verdict.

## Optional: find a function to read

If you want to understand a code path rather than apply REA candidates:

```bash
# While the game is running on Linux
./target/release/game-opt-tool sample \
  --game "/path/to/game/Game.exe" \
  --duration 20 --top 20

# Read one function
./target/release/game-opt-tool decompile \
  --game "/path/to/game/Game.exe" \
  --address 0x109214b6
```

A hot function is a hypothesis. `sample` sees self CPU time only, so GPU time,
blocked waits, and kernel time are invisible to it.

## Guardrails

- Patch a copy, never the original.
- Keep the manifest; it is the only supported way to reverse exactly.
- Signed PE files are refused unless `allow_signed` is set.
- `verify` compares bytes; it never claims runtime correctness.
- If a candidate looks like data, do not patch it.
