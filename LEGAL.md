# Legal and responsible use

This project is for personal experimentation on single-player games you already
own. It is not legal advice.

## What the tool does

It reads a local executable, reports static candidates, and can apply a guarded
byte patch to a copy. It does not decrypt, unprotect, or redistribute anything.

## Ground rules

1. **Own the game.** Use a legally obtained copy.
2. **Personal use only.** Do not distribute patched executables.
3. **No game assets.** Do not redistribute textures, models, audio, or data.
4. **Single-player only.** Do not use it on multiplayer games, especially ones
   with anti-cheat. Modified binaries are detectable and can get you banned.
5. **No DRM circumvention.** If a binary is encrypted or protected, stop.

## Reverse engineering is jurisdiction-specific

Some jurisdictions recognize reverse engineering for interoperability,
security research, or preservation. Others restrict it, and most game EULAs
prohibit it even where the law is permissive. Whether a particular use is
lawful depends on where you are and what you agreed to. If that matters for
your situation, ask a lawyer rather than relying on this file.

The tool includes no DRM bypass and no code for defeating integrity checks. It
refuses signed PE binaries by default because patching them would invalidate the
signature.

## Anti-cheat

Do not use this tool on games with Easy Anti-Cheat, BattlEye, Vanguard, or any
similar system. Even a harmless byte change is a modified binary, and anti-cheat
software is designed to react to that.

## No warranty

The software is provided under the MIT license, without warranty. A patch can
break a game, corrupt a save, or change behavior in ways static analysis cannot
predict. Keep the original file and the manifest, and test on a copy.
