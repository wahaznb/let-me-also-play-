# Reverse-Engineering Analysis (REA)

Static binary analysis for game executables — **no measurement, no running the game**.

## What it does

1. **Scans** executable PE sections with a linear iced-x86 sweep.
2. **Finds** provably safe, length-preserving dead code:
   - True self-moves (for example `mov rax, rax`) → NOPs
   - Dead jumps (`jmp` to the immediately following instruction) → NOPs

   A 32-bit self-move in 64-bit mode (`mov eax, eax`) is **not** treated as
   dead: it zeroes the upper 32 bits of `RAX`, so removing it can change the
   register's value.
3. **Emits** a JSON patch spec you can apply with the verified Patcher.
4. **Tracks** every change in a manifest so you can revert exactly.

REA intentionally does **not** rewrite `test reg,reg` or `and reg,reg` into
`xor reg,reg`. That rewrite zeroes the register and changes its flags, so it is
not behavior-preserving.

## Why no measurement?

This tool doesn't run the game or touch MangoHud. It only finds candidates that are *provably* safe based on instruction semantics. You decide whether to keep them by playing the game or profiling yourself.

## Usage

```bash
# Scan only (no patch written); --json for agents
game-opt-tool rea --game <path_to_game.exe>
game-opt-tool rea --game <path_to_game.exe> --json

# Generate a patch spec without applying it
game-opt-tool rea --game <path.exe> --spec-only

# Apply candidates to a copy and record the reversible manifest
game-opt-tool rea --game <path.exe> \
  --output ./patched.exe --manifest ./patched.patch.json

# Revert a previous REA patch
game-opt-tool rea --game ./patched.exe \
  --revert --manifest ./patched.patch.json
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

Candidates (2):
  0x14001A2B3  dead_move    mov rax, rax is a no-op move; replaced with 3 nop byte(s)
  0x14001C441  dead_jump    jmp to the immediately following instruction (0x14001C444); removed 2 byte(s)
  ...
```

## Safety guarantees

- Every candidate is length-preserving (original bytes == replacement bytes).
- The verified Patcher writes atomically via temp + rename.
- A SHA-256 manifest records every byte change; `unpatch` restores the exact original.
- REA always emits `allow_signed: false`, so the Patcher refuses an Authenticode-signed PE unless you edit the spec yourself.

## Limitations

- Only PE binaries supported today.
- Only x86/x86-64 instruction decoding.
- The sweep is linear, not control-flow-aware. Embedded data can decode as
  instructions and produce a false-positive candidate; the expected-byte guard
  protects against a different build, not against a false positive in the same
  build. Review the addresses before applying to a shipping binary.
- A 32-bit self-move in 64-bit mode is deliberately skipped, even though it
  looks like a no-op, because the write zero-extends into the 64-bit register.
- No performance measurement is performed. A dead-code removal is not a proven
  speedup.
- No GPU or frame-time analysis. `profile`, `sample`, and `compare` are
  optional, separate commands and are not part of the REA decision.
