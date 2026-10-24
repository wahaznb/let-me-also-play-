# Next Steps

See [STATUS.md](STATUS.md) for verified current state.
See [PREREQUISITES.md](PREREQUISITES.md) for setup instructions.

## Immediate

1. **Test on Windows VM** — Cross-compile and verify the binary works
2. **Fix Windows issues** — File locking, paths, permissions
3. **Add pre-built binaries** — GitHub Actions already configured

## Short Term

4. **Hot function discovery** — CPU sampling to find hot functions automatically
5. **Runtime A/B verification** — Launch game, measure, compare, gate patches
6. **Real optimizer** — Replace stub with actual optimization passes

## Long Term

7. **Chunk-based backup vault** — For large games (50GB+)
8. **Relocation handling** — For patches that move code
9. **Multi-run confidence** — Aggregate run-to-run variance
