# Deus Ex 2000 Optimization — Prototype

This is the first working prototype of the game optimization tool, targeting Deus Ex (2000).

## Why Deus Ex?

- **Simple binary** — 32-bit x86 PE format, easy to analyze
- **Known issues** — Community has documented performance problems
- **Open engine** — Unreal Engine 1 source code is available
- **No anti-cheat** — Safe to test and modify
- **Active modding community** — Proves there's demand

## Current Status

✅ **Working:**
- Game detection (finds Deus Ex installation)
- Configuration system (TOML-based)
- Profiler skeleton (with known hot functions)
- Optimizer (6 passes implemented)
- Patcher skeleton (trampoline creation)
- Verification framework

🔄 **In Progress:**
- Ghidra integration (requires manual setup)
- Real binary patching (placeholder for now)
- Actual frame comparison

## Getting Started

### 1. Install Requirements

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Ghidra (for decompilation)
# Download from https://ghidra.re/
export GHIDRA_INSTALL_DIR=/path/to/ghidra
```

### 2. Build

```bash
cargo build --release
```

### 3. Find Your Deus Ex Installation

Common locations:
- `C:\Program Files\Deus Ex\`
- `C:\Program Files (x86)\Deus Ex\`
- Steam: `steamapps\common\Deus Ex\`

### 4. Run the Tool

```bash
# Analyze
cargo run --release -- analyze --game "C:/Program Files/Deus Ex"

# Profile
cargo run --release -- profile --game "C:/Program Files/Deus Ex" --frames 100

# Optimize (requires Ghidra)
cargo run --release -- optimize --game "C:/Program Files/Deus Ex"

# Patch
cargo run --release -- patch --game "C:/Program Files/Deus Ex" --output ./DeusEx_optimized.exe

# Verify
cargo run --release -- verify --original "C:/Program Files/Deus Ex/DeusEx.exe" --patched ./DeusEx_optimized.exe
```

## Expected Results

| Metric | Before | After |
|--------|--------|-------|
| FPS | 60 (locked) | 120+ |
| CPU Usage | 100% (1 core) | ~50% (multi-core) |
| Input Lag | ~16ms | ~8ms |

## Files

```
targets/deus-ex/
├── README.md          # Game-specific documentation
├── config.toml        # Game configuration
├── mod.rs             # Module exports
├── profiler.rs        # Performance profiling
├── decompiler.rs      # Ghidra integration
├── optimizer.rs       # Code optimization
├── patcher.rs         # Binary patching
└── verifier.rs        # Correctness verification
```

## Next Steps

1. [ ] Connect real Ghidra integration
2. [ ] Implement actual PE patching
3. [ ] Add frame-by-frame comparison
4. [ ] Test on real Deus Ex installation
5. [ ] Document results

## References

- [Deus Ex Golden Version](https://deusex.goldenversion.com/) — Community patch
- [Unofficial Deus Ex Patch](https://www.deusex.com/) — High FPS unlock
- [Unreal Engine 1 Source](https://github.com/gildor2/UESource) — Reference implementation
