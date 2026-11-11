# Architecture

The tool is a single Rust binary plus a small MCP server that calls the same
handlers. There is no daemon, no database, and no runtime network access.

## Modules

```text
src/
├── main.rs          CLI parsing, dispatch, and output formatting
├── config/          config/games.yaml parsing
├── game/            format and architecture detection, game lookup
├── hash/            SHA-256 helpers
├── backup/          verified backups and restores
├── patcher/         PE patching, patch specs, manifests
├── verifier/        byte-level comparison only
├── decompiler/      Ghidra headless runner
├── profiler/        MangoHud CSV parsing and frame-time math
├── sampler/         Linux perf_event_open self-CPU sampling
├── rea/             static dead-code analysis (the primary workflow)
├── targets/         per-game detection
└── mcp/             stdio + HTTP MCP transports
```

## The primary path: REA

```text
game.exe
   │  read + SHA-256
   ▼
PE headers ──► executable sections
   │
   ▼
iced-x86 linear sweep ──► DecodedInstruction stream
   │
   ▼
candidate rules ──► Candidate { address, original, replacement, kind }
   │
   ▼
PatchSpec (expected bytes + target hash)
   │
   ▼
Patcher ──► bounds check ──► expected-byte check ──► section check
   │
   ▼
atomic write ──► patched copy + manifest
   │
   ▼
revert ──► verify patched hash + bytes ──► restore original bytes
```

`rea` only emits two rules today: true self-moves and jumps to the immediately
following instruction. Both are replaced with same-length NOPs, so no
instruction moves and no relocation table is needed.

The linear sweep is the main limitation. It decodes executable sections without
building a control-flow graph, so a byte pattern inside embedded data can look
like an instruction. That is why every candidate is reported with its address
and why the tool never applies a patch without an explicit `--output`.

## Patch spec and manifest

A `PatchSpec` is the input to the patcher:

```json
{
  "schema_version": 1,
  "name": "rea-your-game",
  "target_sha256": "...",
  "allow_signed": false,
  "patches": [
    { "address": "0x140001000", "expected": "4889c0", "replace": "0f1f00", "note": "..." }
  ],
  "sections": []
}
```

The patcher refuses to write when:

- the input hash does not match `target_sha256`;
- a patch address is outside a PE section;
- the bytes at the address do not match `expected`;
- the file is Authenticode-signed and `allow_signed` is false;
- the output path aliases the spec, the manifest, or the input.

The manifest is the inverse operation: address, file offset, original bytes,
replacement bytes, and both hashes. `unpatch` and `rea --revert` use it.

## Backups

Backups are stored next to the file: `<name>.<sha256-prefix>.<timestamp>.bak`
with a matching `.bak.json` record. Restore verifies the backup checksum, writes
through a temporary file, re-reads the result, and refuses if the target is a
symlink or the backup no longer matches.

## Optional measurement

These paths are independent of REA and never feed automatic decisions:

- `profiler/` parses MangoHud CSV files and computes averages, lows,
  percentiles, and stutter ratio.
- `sampler/` reads per-thread CPU time with `perf_event_open`, maps instruction
  pointers through `/proc/<pid>/maps` to PE RVAs, and resolves names from a
  cached Ghidra function list.
- `decompiler/` runs `analyzeHeadless` and parses its output for one function at
  a time.

A CPU hotspot is not a frame-time verdict. `METRICS.md` explains why.

## MCP server

```text
MCP client
    │  JSON-RPC 2.0
    ├── stdio (default)
    └── HTTP POST http://127.0.0.1:<port>/mcp
    ▼
mcp-server ──► same validated command handlers as the CLI
```

Responses are capped at 60 KB by default (`GAME_OPT_MCP_MAX_BYTES`, 4 KB–1 MB).
Oversized output becomes a truncation envelope with a preview. The server never
exposes raw shell access; stdout is protocol-only and logs go to stderr.

## Security boundaries

1. All user-provided addresses, bytes, and paths are validated before use.
2. File offsets are checked against the actual file length.
3. SHA-256 guards every backup, patch, and restore transition.
4. Symlinks are refused as patch, backup, and restore targets.
5. Writes are atomic (temp file, `fsync`, rename).
6. The tool never downloads, phones home, or escalates privileges.
