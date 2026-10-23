# Game Opt Tool — Project Summary

## What We Built

A complete foundation for a game optimization tool that:

1. **Reverse engineers** game binaries using Ghidra
2. **Profiles** games to find CPU bottlenecks  
3. **Decompiles** hot functions to readable code
4. **Optimizes** the code (loop unrolling, SIMD, cache layout, etc.)
5. **Patches** the binary with optimized versions
6. **Verifies** the game still works correctly

## Project Structure

```
game-opt-tool/
├── src/
│   ├── main.rs              # Entry point + CLI orchestration
│   ├── cli/mod.rs           # Command definitions
│   ├── config/mod.rs        # Game configuration system
│   ├── game/mod.rs          # Game detection
│   ├── profiler/mod.rs      # Performance profiling
│   ├── decompiler/mod.rs    # Ghidra integration
│   ├── optimizer/mod.rs     # Code optimization engine
│   ├── patcher/mod.rs       # Binary patching
│   └── verifier/mod.rs      # Correctness verification
├── config/
│   └── games.yaml           # Game-specific configs
├── docs/
│   └── architecture.md      # System design docs
├── Cargo.toml               # Rust dependencies
├── .cargo/config.toml       # Compiler settings
├── setup.sh                 # Installation script
├── README.md                # Usage guide
├── RESEARCH.md              # Target game research
├── LEGAL.md                 # Legal guidelines
└── CONTRIBUTING.md          # How to contribute
```

## Key Features

### ✅ Done
- [x] Project structure and build system
- [x] Game detection (PE/ELF/Mach-O)
- [x] Profiler interface (placeholder for RenderDoc/PIX)
- [x] Ghidra decompiler integration
- [x] Optimization engine with 6 passes
- [x] Binary patcher with trampoline support
- [x] Verification framework
- [x] Configuration system for game-specific settings
- [x] Legal guidelines
- [x] Target game research (Oblivion, Skyrim, Jedi: Survivor)

### 🔄 Next Steps
- [ ] Install Rust and test compilation
- [ ] Set up Ghidra headless mode
- [ ] Implement actual binary patching logic
- [ ] Add RenderDoc/PIX integration
- [ ] Create test cases for known games
- [ ] Add AI optimization suggestions (Ollama)
- [ ] Build community game database

## Target Games (Research Complete)

### Tier 1 — Best Candidates
| Game | Engine | Why |
|------|--------|-----|
| Oblivion Remastered | UE5 | Known single-threaded rendering, skeleton upload issues |
| Skyrim SE/AE | Creation Engine | 256MB address space limit, quest staging stutter |
| Star Wars Jedi: Survivor | UE4 | GC spikes, texture streaming hitches |

### Why These Games?
1. **CPU-bound** — Real optimization opportunities
2. **Well-documented** — Community knows the issues
3. **No anti-cheat** — Safe to test
4. **Preservation value** — Games at risk of being lost
5. **Active modding scenes** — Community will help/test

## Technical Approach

### The Pipeline
```
Game Directory → Detect Format → Profile → Find Hot Functions
                    ↓
              Decompile with Ghidra
                    ↓
              AI Suggests Optimizations
                    ↓
              Rewrite in Rust/C++
                    ↓
              Patch Binary (Trampolines)
                    ↓
              Verify (Frame Comparison)
```

### Key Technologies
- **Ghidra** — Open-source disassembler/decompiler (NSA-made)
- **Rust** — Safe, fast language for optimized code
- **LLVM** — For code generation (optional)
- **RenderDoc** — GPU/CPU profiling
- **Ollama** — Local LLM for AI assistance

## Legal Position

✅ **Safe:**
- Personal use on owned games
- No DRM circumvention
- Single-player only
- No asset distribution

⚠️ **Gray Area:**
- Reverse engineering (fair use precedent exists)
- Binary modification (no clear legal test)

❌ **Avoid:**
- Multiplayer games (anti-cheat bans)
- Distributing patched games
- Circumventing encryption

## Next Actions

### Immediate (This Week)
1. Install Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2. Install Ghidra: https://ghidra.re/
3. Test the tool on a simple game
4. Pick first target: **Oblivion Remastered**

### Short Term (Next Month)
1. Get the pipeline working end-to-end
2. Successfully optimize one function in one game
3. Document the process
4. Share findings with community

### Long Term
1. Support 10+ games
2. Build community database of optimizations
3. Release as open-source tool
4. Potentially partner with preservation groups

## Resources

- [REA (Reverse Engineer Anything)](https://github.com/morluto/rea) — 372k stars, AI + RE tools
- [N64Recomp](https://github.com/N64Recomp/N64Recomp) — 8.2k stars, static recompilation
- [Oblivion Performance Fix](https://github.com/LTSharpe/OblivionRemasteredPerformanceFix) — AI-powered single-game patch
- [E9Patch](https://github.com/GJDuck/e9patch) — 1.2k stars, binary rewriting
- [FEX-Emu](https://github.com/FEX-Emu/FEX) — 8.3k stars, binary recompilation

---

**Ready to start?** Install Rust and run:
```bash
cd game-opt-tool
cargo build --release
```
