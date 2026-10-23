# Contributing to Game Opt Tool

Thanks for wanting to help! This document outlines how to contribute.

## Code of Conduct

- Be respectful and inclusive
- Help others learn
- Focus on preservation and education
- No piracy encouragement

## Getting Started

1. Fork the repo
2. Clone your fork
3. Run `./setup.sh`
4. Read [ARCHITECTURE.md](docs/architecture.md)
5. Pick an issue from the tracker

## Project Structure

```
game-opt-tool/
├── src/
│   ├── profiler/       # Benchmarking & profiling
│   ├── decompiler/     # Ghidra integration
│   ├── optimizer/      # Code optimization engine
│   ├── patcher/        # Binary patching
│   ├── verifier/       # Verification & testing
│   └── cli/            # Command-line interface
├── targets/            # Game-specific configs
├── config/             # Configuration files
└── docs/               # Documentation
```

## Adding a New Game Target

1. Create a directory in `targets/<game-name>/`
2. Add `config.yaml` with:
   - Binary format (PE, ELF, Mach-O)
   - Architecture (x86, x86-64, ARM)
   - Known hot functions
   - Quirks to preserve
3. Add `patches/` with any pre-built patches
4. Update `config/games.yaml` to include the game
5. Submit a PR

## Testing

Run tests before submitting:
```bash
cargo test
cargo clippy
```

## Commit Style

Use conventional commits:
- `feat:` — New feature
- `fix:` — Bug fix
- `docs:` — Documentation
- `refactor:` — Code restructuring
- `test:` — Tests
- `chore:` — Maintenance

## Legal Guidelines

Remember:
- Only work on games users own
- No DRM circumvention
- Single-player only
- No distribution of game assets

## Questions?

Open an issue or join our Discord.
