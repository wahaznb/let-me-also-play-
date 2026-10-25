# Next Steps

See [STATUS.md](STATUS.md) for verified current state.
See [PREREQUISITES.md](PREREQUISITES.md) for setup instructions.

## Done

- **MCP server** — stdio + HTTP transports, ten tools wired to the real
  decompiler, profiler, patcher, backup, and verifier. Tested end-to-end and
  connected to OpenCode. Use `./install_ai.sh` to register.
- **Context safety** — per-call response cap (`GAME_OPT_MCP_MAX_BYTES`),
  truncation envelopes with `truncated`/`next_offset`/`preview`, paginated
  `decompile`, and handle-only paginated `list_functions`.

## Immediate

1. **Test on Windows VM** — Cross-compile and verify the binary works
2. **Fix Windows issues** — File locking, paths, permissions
3. **Add pre-built binaries** — GitHub Actions already configured

## Short Term

4. **Hot function discovery** — CPU sampling to find hot functions automatically
5. **Runtime A/B verification** — Launch game, measure, compare, gate patches
6. **Real optimizer** — Replace stub with actual optimization passes
7. **Standalone local-LLM path (optional)** — Wire the Ollama/OpenAI-compatible
   client into `optimize` for agents that are not driving MCP. The MCP path does
   not need this.

## Long Term

8. **Chunk-based backup vault** — For large games (50GB+)
9. **Relocation handling** — For patches that move code
10. **Multi-run confidence** — Aggregate run-to-run variance
