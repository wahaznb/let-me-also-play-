# Game Opt Tool Architecture

## System Overview

```
                    ┌─────────────────────────┐
                    │   User (CLI Interface)   │
                    └────────────┬────────────┘
                                 │
                    ┌────────────▼────────────┐
                    │    Main Coordinator      │
                    │   (src/main.rs)          │
                    └────────────┬────────────┘
                                 │
         ┌───────────┬───────────┼───────────┬───────────┐
         │           │           │           │           │
┌────────▼──┐ ┌──────▼────┐ ┌───▼────┐ ┌───▼────┐ ┌───▼────┐
│   Game    │ │ Profiler  │ │Decompiler│ │Optimizer│ │ Patcher │
│ Detector  │ │           │ │        │ │        │ │        │
└───────────┘ └───────────┘ └────────┘ └────────┘ └────────┘
         │           │           │           │           │
         └───────────┴───────────┴───────────┴───────────┘
                                 │
                    ┌────────────▼────────────┐
                    │    Verifier (Output)     │
                    └─────────────────────────┘
```

## Module Breakdown

### 1. CLI (`src/cli/mod.rs`)
Command-line interface using clap. Provides commands:
- `analyze` — Detect game and show basic info
- `profile` — Run profiler to find hot functions
- `optimize` — Decompile and optimize specific functions
- `patch` — Apply optimization patches to binary
- `verify` — Verify patched game works correctly
- `list-games` — Show supported games
- `help <game>` — Show game-specific help

### 2. Game Detection (`src/game/mod.rs`)
Identifies the target game by:
- Scanning the game directory for executables
- Detecting binary format (PE/ELF/Mach-O)
- Detecting architecture (x86/x64/ARM)
- Loading game-specific configuration
- Extracting basic metadata (name, version, engine)

### 3. Profiler (`src/profiler/mod.rs`)
Identifies performance bottlenecks:
- **CPU profiling** — Using RenderDoc/PIX integration
- **Hot function detection** — Functions consuming most CPU time
- **Frame time analysis** — FPS, p99, stutter detection
- **Call stack sampling** — Understanding what's slow

### 4. Decompiler (`src/decompiler/mod.rs`)
Reverse engineers the binary:
- **Ghidra integration** — Uses Ghidra's headless mode
- **Function disassembly** — Extracts assembly from binary
- **Decompilation** — Converts to C-like pseudocode
- **Symbol resolution** — Attempts to find function names
- **Cross-reference analysis** — Finds callers/callees

### 5. Optimizer (`src/optimizer/mod.rs`)
Applies optimizations:
- **Static analysis** — Understands the decompiled code
- **Pattern matching** — Identifies optimization opportunities
- **Code generation** — Produces optimized Rust/C++ code
- **AI assistance** — Uses LLM to suggest improvements (optional)

Optimization passes:
1. Loop unrolling
2. SIMD vectorization
3. Cache-friendly data layout (SoA)
4. Constant folding
5. Dead code elimination
6. Function inlining

### 6. Patcher (`src/patcher/mod.rs`)
Injects optimized code:
- **Binary parsing** — Reads PE/ELF/Mach-O formats
- **Function finding** — Locates target functions in binary
- **Trampoline creation** — Creates jump stubs
- **Code injection** — Patches the binary or creates DLL
- **ABI preservation** — Maintains calling conventions

### 7. Verifier (`src/verifier/mod.rs`)
Ensures correctness:
- **Launch testing** — Verifies game starts
- **Frame comparison** — Pixel-by-pixel diff
- **Memory state comparison** — Heap/stack/globals
- **Quirk verification** — Confirms bugs still exist
- **Performance benchmarking** — Measures FPS gain

## Data Flow

```
1. User provides game path
         │
         ▼
2. GameInfo::from_path() detects the game
         │
         ▼
3. Profiler finds hot functions
         │
         ▼
4. Decompiler extracts pseudocode
         │
         ▼
5. Optimizer rewrites functions
         │
         ▼
6. Patcher injects optimized code
         │
         ▼
7. Verifier checks correctness
         │
         ▼
8. User gets patched game + report
```

## Configuration

Games are configured in `config/games.yaml`:
```yaml
games:
  game-name:
    name: "Full Game Name"
    engine: "Engine Name"
    hot_functions: [...]
    quirks: [...]
    optimization_hints: {...}
```

## Extensibility

To add support for a new game:
1. Create `config/<game-name>.yaml` with game-specific config
2. Add any known hot functions and quirks
3. Test with the real binary
4. Submit a PR

## Future Enhancements

- [ ] AI-powered optimization suggestions (via Ollama/local LLM)
- [ ] Automated regression testing
- [ ] Community game database
- [ ] Visual patch diff viewer
- [ ] Performance profiling dashboard
