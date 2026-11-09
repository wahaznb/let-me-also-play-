# Reverse-Engineering Analysis (REA)

Static binary analysis for game executables — **no measurement, no running the game**.

## What it does

1. **Scans** executable PE sections with a linear iced-x86 sweep.
2. **Finds** provably safe, length-preserving candidates:
   - Dead moves (`mov reg, reg`) → NOPs
   - Dead jumps (`jmp` to next instruction) → NOPs
   - `test reg,reg` / `and reg,reg` → canonical `xor reg,reg`
3. **Emits** a JSON patch spec you can apply with the verified Patcher.
4. **Tracks** every change in a manifest so you can revert exactly.

## Why no measurement?

This tool doesn't run the game or touch MangoHud. It only finds candidates that are *provably* safe based on instruction semantics. You decide whether to keep them by playing the game or profiling yourself.

## Usage

```bash
# Scan only (no patch written)
game-opt-tool rea --game <path_to_game.exe>

# Generate a patch spec without applying it
game-opt-tool rea --game <path.exe> --spec-only

# Apply candidates to an output file
game-opt-tool rea --game <path.exe> --output ./patched.exe

# Revert a previous REA patch
game-opt-tool rea --game <path.exe> --revert --manifest <manifest.json>
```

## Output example

```
=== REA: static analysis ===
Game:       Mortal Shell
Executable: /games/Mortal Shell/Dungeonhaven-Win64-Shipping.exe
Format:     PE
Arch:       x64

  .text      VA 0x140000000  51200 bytes  3210 insns  4 data  7 candidates
  .code      VA 0x140020000  10240 bytes  540 insns  0 data  1 candidates

Candidates (8):
  0x14001A2B3  dead_move    mov rax, rax is a no-op move; replaced with 2 nop byte(s)
  0x14001C441  test_self    test eax,eax normalizes to xor eax,eax ...
  ...
```

## Safety guarantees

- Every candidate is length-preserving (original bytes == replacement bytes).
- The PatchReport writes atomically via temp + rename.
- A SHA-256 manifest records every byte change; `unpatch` restores the exact original.
- Signed PE binaries are refused unless `allow_signed: true` is set in the spec.

## Limitations

- Only PE binaries supported today.
- Only x86/x86-64 instruction decoding.
- The sweep is linear, not control-flow-aware (data between instructions may be misinterpreted as code on subsequent bytes if alignment is off; this is noted in the report).
- No GPU or frame-time analysis; those remain separate commands (`profile`, `sample`, `compare`).
