# Game Opt Tool

> A tool that reverse engineers and optimizes game binaries to improve performance while preserving the original look, feel, and behavior.

## What This Tool Does

1. **You point it at a game directory**
2. **It reverse engineers the binary** using Ghidra
3. **It identifies performance bottlenecks** through profiling
4. **It rewrites hot functions** in optimized Rust/C++
5. **It patches the game** with optimized versions
6. **It verifies** the game still works identically

## Core Principles

- **Preservation First** — The game must look and feel exactly the same
- **Quirks Preserved** — Known bugs stay unless they cause crashes
- **No Piracy** — Works only on games you own
- **Single-Player Only** — Avoids anti-cheat complications

## Quick Start

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and build
git clone <repo-url>
cd game-opt-tool
cargo build --release

# Analyze a game
cargo run --release -- analyze --game /path/to/game

# Optimize Deus Ex (prototype)
cargo run --release -- optimize --game /path/to/Deus\ Ex
```

## Supported Games

| Game | Status | Expected Improvement |
|------|--------|---------------------|
| **Deus Ex (2000)** | ✅ Prototype | 2x FPS (60 → 120+) |
| Oblivion Remastered | 🔄 Planned | 30-50% FPS |
| Skyrim SE/AE | 🔄 Planned | 20-40% FPS |
| Star Wars Jedi: Survivor | 🔄 Planned | 25-35% FPS |

## How It Works

```
Game Directory
      │
      ▼
┌─────────────┐
│  Detect     │ ← PE/ELF/Mach-O, architecture, game name
└──────┬──────┘
       ▼
┌─────────────┐
│  Profile    │ ← Find hot functions (RenderDoc/PIX)
└──────┬──────┘
       ▼
┌─────────────┐
│  Decompile  │ ← Ghidra headless mode
└──────┬──────┘
       ▼
┌─────────────┐
│  Optimize   │ ← Rewrite with better algorithms
└──────┬──────┘
       ▼
┌─────────────┐
│  Patch      │ ← Inject optimized code
└──────┬──────┘
       ▼
┌─────────────┐
│  Verify     │ ← Frame-by-frame comparison
└─────────────┘
```

## Technical Details

### Optimization Passes

1. **Loop Unrolling** — Reduces branch overhead
2. **SIMD Vectorization** — Process multiple data points
3. **Cache-Friendly Layout** — Structure of Arrays (SoA)
4. **Constant Folding** — Compile-time constant evaluation
5. **Dead Code Elimination** — Remove unreachable code
6. **Function Inlining** — Reduce function call overhead

### Binary Patching

- Creates trampolines to redirect function calls
- Preserves ABI (calling conventions, stack layout)
- Supports PE (Windows), ELF (Linux), Mach-O (macOS)

## Legal

This tool is for **personal use only**. You must own the game you're optimizing.

- ✅ Personal use on owned games
- ✅ No DRM circumvention
- ✅ Single-player only
- ❌ No distribution of game assets
- ❌ No multiplayer/online games (anti-cheat risk)

See [LEGAL.md](LEGAL.md) for details.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

MIT
