# Next Steps

See [STATUS.md](STATUS.md) for verified current state.
See [PREREQUISITES.md](PREREQUISITES.md) for setup instructions.

## Done

- **MCP server** — stdio + HTTP transports, eleven tools wired to the real
  sampler, decompiler, profiler, patcher, backup, and verifier. Tested
  end-to-end and connected to OpenCode. Use `./install_ai.sh` to register.
- **Per-function CPU sampling** — Linux `perf_event_open`, per-thread
  user-space CPU clock, runtime IP → PE RVA mapping, cached Ghidra function
  map, game/external/unknown split, and lost-sample reporting. CLI + MCP.
- **Context safety** — per-call response cap (`GAME_OPT_MCP_MAX_BYTES`),
  truncation envelopes with `truncated`/`next_offset`/`preview`, paginated
  `decompile`, and handle-only paginated `list_functions`.
- **Metrics policy** — `METRICS.md` defines 1% lows, p99, stutter, frame-count
  requirements, matched-run comparison, and CPU-sampling blind spots.

## Immediate

1. **Commit and push the sampler** — include `METRICS.md`, tests, and stable
   Ghidra/JDK paths; verify CI including Windows jobs.
2. **Restart OpenCode** — the running MCP server predates the `sample` tool, so
   the live session cannot see it until the release binary is rebuilt and the
   agent restarts.
3. **Test on a Windows VM** — Cross-compile and verify the binary works, and
   confirm `sample` reports an unsupported-platform error cleanly.

## Short Term

4. **GPU/off-CPU attribution** — CPU sampling cannot see GPU stalls or blocked
   waits. Decide whether to integrate PresentMon/MangoHud frame-duration data
   or add off-CPU sampling.
5. **Runtime A/B verification** — Give `verify` real behavioral evidence; today
   it only compares bytes.
6. **Real optimizer** — Replace the stub with actual optimization passes, but
   only after a measured candidate exists.
7. **Multi-run confidence** — Aggregate run-to-run variance instead of treating
   one `compare` as significance.
8. **Standalone local-LLM path (optional)** — Wire the Ollama/OpenAI-compatible
   client into `optimize` for agents that are not driving MCP. The MCP path does
   not need this.

## Long Term

9. **Chunk-based backup vault** — For large games (50GB+)
10. **Relocation handling** — For patches that move code
11. **Cross-platform sampler** — Windows ETW/PresentMon, macOS Instruments
12. **Pre-built binaries** — Publish the CI artifacts for users without a Rust
    toolchain
