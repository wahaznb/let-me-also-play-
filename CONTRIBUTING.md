# Contributing

Thanks for taking a look. This is a small tool with a narrow scope, and that is
intentional.

## Before you start

- Read [README.md](README.md) and [docs/rea.md](docs/rea.md).
- Only work with binaries you own and are allowed to modify.
- No DRM circumvention, no multiplayer/anti-cheat targets, no game assets.

## Setup

```bash
git clone https://github.com/wahaznb/let-me-also-play-.git
cd let-me-also-play-
cargo build --release --bins
cargo test
```

Ghidra and JDK 25 are optional; only the decompiler and function naming need
them. See [INSTALL.md](INSTALL.md).

## Layout

```text
src/
├── rea/          static analysis: decode, candidates, patch specs
├── patcher/      PE patching, expected-byte guards, manifests
├── backup/       SHA-256 backups and restores
├── decompiler/   Ghidra headless runner
├── sampler/      Linux perf_event_open sampling
├── profiler/     MangoHud CSV frame-time math
├── mcp/          MCP server (stdio + HTTP)
└── config/       config/games.yaml parsing
config/           game metadata
docs/             protocol and workflow notes
tests/            CLI-level integration tests
```

## What fits

- A new provably safe, length-preserving candidate rule for `rea`.
- A bug fix in patching, backup, revert, or manifest handling.
- A test that demonstrates a real failure.
- Documentation corrections where the code and the text disagree.

## What does not fit

- Optimizer passes that estimate speedups without measurements.
- Features that claim a performance win the tool cannot verify.
- Anything that patches a running game's memory, bypasses DRM, or targets
  multiplayer.
- Large rewrites of working code without a concrete failing case.

## Tests

```bash
cargo test
cargo clippy --all-targets
```

The suite runs without a game. The one ignored test needs a real binary,
Ghidra, and JDK 25:

```bash
export GAME_OPT_TEST_BINARY="/path/to/your/game/DeusEx.exe"
cargo test --test cli_decompile -- --ignored
```

If you change the patch path, add a test that proves the change is reversible
and that a guard mismatch is refused.

## Adding a game

REA does not need a per-game entry. If you only want metadata, add a block to
`config/games.yaml`:

```yaml
games:
  my-game:
    name: "My Game"
    version: "1.0"
    engine: "Unknown"
    binary_format: PE
    architecture: X64
    hot_functions: []
    quirks: []
    optimization_hints: {}
```

Do not add guessed addresses or function names. Addresses come from `rea` or
`decompile`, and they belong in a patch spec, not in a static table.

## Commits

Use conventional commits:

- `feat:` new behavior
- `fix:` bug fix
- `docs:` documentation
- `refactor:` restructuring without behavior change
- `test:` tests
- `chore:` maintenance

Keep the subject under about 72 characters and explain *why* in the body when
the change is not obvious.

## License

By contributing, you agree that your contribution is licensed under the MIT
license in [LICENSE](LICENSE).
