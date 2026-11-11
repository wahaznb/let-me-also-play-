# Deus Ex (2000)

Deus Ex is the reference target for this tool. It is a 32-bit x86 Windows
executable (PE32), which makes it a good sanity check for format detection,
backup/restore, and Ghidra decompilation.

## What is verified

- Format detection: PE32 / x86.
- Entry point reported by the tool matches the real executable.
- Ghidra headless decompilation of the entry point (`0x109214B6` in the GOG
  build used for testing).
- Backup, revert, and byte-level patching on a copy.

## What is not claimed

There is no verified list of hot functions for this game in this repository.
Names like "AI update" or "physics step" would be guesses until they are
measured or read in the decompiler. Use `sample` while the game runs, or
`decompile --address <hex>` on an address you found with `rea`.

## Notes

- Start the game from its `System` directory so it can find its data files.
- The engine is Unreal Engine 1; the original source is publicly available and
  useful when reading decompiled output.
- Single-player only. Do not point this tool at a multiplayer install.
