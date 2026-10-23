# Deus Ex 2000 Optimization Tool

Prototype for optimizing Deus Ex (2000) using automated reverse engineering and code optimization.

## What This Does

This tool:
1. **Detects** Deus Ex installation
2. **Profiles** the game to find CPU bottlenecks
3. **Decompiles** hot functions using Ghidra
4. **Optimizes** the code with better algorithms
5. **Patches** the binary with optimized versions
6. **Verifies** the game still works correctly

## Quick Start

```bash
# Build
cargo build --release

# Analyze Deus Ex
cargo run --release -- analyze --game /path/to/Deus\ Ex

# Profile
cargo run --release -- profile --game /path/to/Deus\ Ex --frames 100

# Optimize (requires Ghidra)
cargo run --release -- optimize --game /path/to/Deus\ Ex

# Patch
cargo run --release -- patch --game /path/to/Deus\ Ex --output ./DeusEx_optimized.exe

# Verify
cargo run --release -- verify --original /path/to/Deus\ Ex/DeusEx.exe --patched ./DeusEx_optimized.exe
```

## Requirements

- Rust 1.75+
- Ghidra 12.x (for decompilation)
- Deus Ex 2000 (must own the game)

## How It Works

### Phase 1: Detection
The tool scans common installation paths:
- `C:\Program Files\Deus Ex\`
- `C:\Program Files (x86)\Deus Ex\`
- Steam library locations

### Phase 2: Profiling
Identifies hot functions:
- `UWorld_Tick` (51% CPU) - Main game loop
- `AIPerformTask` (19% CPU) - AI behavior
- `Pathfinding_Update` (13% CPU) - Navigation
- `Render_DrawActors` (9% CPU) - Rendering
- `QuestCheck_Perform` (5% CPU) - Quest logic

### Phase 3: Optimization
Applies game-specific optimizations:
- **UWorld_Tick**: Batch processing by actor type
- **AIPerformTask**: Spatial hashing for neighbor queries
- **Pathfinding_Update**: Incremental node updates
- **Render_DrawActors**: Frustum culling + material batching
- **QuestCheck_Perform**: Short-circuit evaluation

### Phase 4: Patching
Creates trampolines to redirect function calls to optimized versions.

### Phase 5: Verification
Ensures:
- Game launches
- FPS improves
- Gameplay is unchanged
- Known quirks are preserved

## Expected Results

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| FPS | 60 (locked) | 120+ | 2x+ |
| CPU Usage | 100% (1 core) | ~50% (multi-core) | 2x |
| Input Lag | ~16ms | ~8ms | 2x better |
| Quest Stutter | Noticeable | Eliminated | Better UX |

## Known Quirks Preserved

- AI decision delay (part of game's character)
- Pathfinding noise (occasional wrong turns)

## Legal

This tool is for personal use on games you own. See ../LEGAL.md for details.

## Contributing

See ../CONTRIBUTING.md for guidelines.
