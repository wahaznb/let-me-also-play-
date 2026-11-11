# Roadmap

What exists is listed in the README status table. This file is what does not
exist yet, in rough order of usefulness.

## Next

- **More candidate rules for `rea`.** The current set is two rules. Any addition
  has to be provably length-preserving and behavior-preserving; "probably dead"
  is not enough.
- **Control-flow awareness.** The linear sweep cannot tell code from data. A
  proper disassembly pass would cut false positives, at the cost of a much
  larger analyzer.
- **Richer manifests.** Record the candidate rule and the reason for each patch
  so a revert can explain what it is undoing.
- **Windows verification.** CI builds the Windows binaries; I have not run the
  patch/revert loop on a real Windows host yet.

## Later

- **Runtime equivalence checks.** `verify` compares bytes today. Real behavioral
  verification needs a way to run the game deterministically, which this tool
  deliberately does not do.
- **Multi-run comparison.** `compare` treats one pair of logs as one sample.
  Run-to-run variance should be reported explicitly.
- **GPU and off-CPU attribution.** The sampler sees self CPU time only.
  Frame-duration data from MangoHud stays a separate input.
- **Chunked backups.** Backups are full copies. Very large games would benefit
  from a content-addressed store.
- **Relocation support.** Patches are same-length by design, so nothing moves.
  Supporting code caves that move instructions is a much larger project.

## Explicitly out of scope

- Automatic "optimization" without evidence.
- Fabricated speedup numbers or placeholder percentages.
- DRM circumvention, anti-cheat bypass, or multiplayer targets.
- Distributing game assets or patched executables.
