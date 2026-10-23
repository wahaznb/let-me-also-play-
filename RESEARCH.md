# Game Opt Tool — Research & Architecture

> A tool that reverse engineers and optimizes game binaries to improve performance while preserving the original look, feel, and behavior.

---

## Project Vision

**What it does:**
- User points tool at a game directory
- Tool reverse engineers the binary
- Identifies performance bottlenecks
- Rewrites hot functions in optimized code (Rust/C++)
- Patches the game binary with optimized versions
- Verifies behavioral equivalence
- Game runs better, looks identical

**Core principles:**
1. **Preservation first** — The game must look and feel exactly the same
2. **Quirks preserved** — Known bugs/quirks stay unless they cause crashes
3. **No piracy** — Works only on games the user owns, no distribution of assets
4. **Single-player only** — Avoids anti-cheat complexity

---

## Target Games — Research

### Tier 1: Best Candidates (Start Here)

These games have known performance issues, are CPU-bound, and have decompilation-friendly binaries:

| Game | Engine | Why It's a Good Target | Known Issues |
|------|--------|----------------------|--------------|
| **Oblivion Remastered** | Unreal Engine 5 | Already has community patches (LTSharpe's fix), UE5 is decompilable | Single-threaded rendering, slow skeleton uploads, weather rebuild every frame |
| **Skyrim SE/AE** | Creation Engine | Massive modding community, well-understood binary | 256MB address space limit, quest staging stutter, cell streaming |
| **Fallout 4** | Creation Engine | Similar to Skyrim, active modding scene | Script lag, loading stutter, quest system bottlenecks |
| **Star Wars Jedi: Fallen Order/Survivor** | Unreal Engine 4 | UE4 decompiles well, known CPU bottlenecks | GC spikes, single-threaded logic, texture streaming hitches |
| **Horizon Zero Dawn** | Decima Engine | Well-studied binary, custom engine with known issues | CPU-bound AI, poor multithreading, stutter on open areas |
| **Cyberpunk 2077** (pre-2.0) | REDengine 4 | Massive performance problems, CD Projekt fixed some but not all | Ray tracing stutter, CPU bottleneck in cities, poor LOD |
| **Assassin's Creed Odyssey/Odyssey** | AnvilNext 2.0 | Large open world, known performance issues | CPU scaling problems, stutter in cities, draw call issues |
| **Watch Dogs 2** | AnvilNext 2.0 | Similar engine to AC, well-understood | CPU-bound AI, poor multithreading |

### Tier 2: Secondary Targets

| Game | Engine | Notes |
|------|--------|-------|
| **Destiny 2** (single-player content) | Destiny Engine | Has anti-cheat but single-player mode may work |
| **Warframe** | Dreamcatcher Engine | Free-to-play, some single-player content |
| **No Man's Sky** | Hello Engine | Procedural generation is CPU-heavy, good optimization candidate |
| **Elite Dangerous** | Elite Engine | Simulation-heavy, CPU-bound |

### Tier 3: Retro/Classic Games (Highest Success Rate)

| Game | Platform | Why |
|------|----------|-----|
| Any **N64 game** | Nintendo 64 | N64Recomp proves this works, byte-for-byte matching possible |
| Any **PS1 game** | PlayStation 1 | Simpler binaries, well-understood architectures |
| Any **GBA game** | Game Boy Advance | Very simple ARM7TDMI code, easy to decompile |

---

## Technical Architecture

### The Pipeline

```
┌─────────────────────────────────────────────────────────────────────┐
│                     GAME OPT TOOL — PIPELINE                        │
└─────────────────────────────────────────────────────────────────────┘

Phase 1: PROFILING
┌─────────────────────────────────────────────────────────────────────┐
│ Input: Game directory path                                          │
│                                                                     │
│ 1. Locate game executable (.exe, .elf, etc.)                       │
│ 2. Run profiler (RenderDoc, PIX, or custom)                         │
│ 3. Identify hot functions (top 10% of CPU time)                     │
│ 4. Output: Hot function list with timestamps                        │
└──────────────────────────┬──────────────────────────────────────────┘
                           │
                           ▼
Phase 2: REVERSE ENGINEERING
┌─────────────────────────────────────────────────────────────────────┐
│ Input: Hot function addresses                                       │
│                                                                     │
│ 1. Load binary into Ghidra (headless)                               │
│ 2. Decompile target functions                                       │
│ 3. AI (LLM) analyzes pseudocode, suggests optimizations             │
│ 4. Output: Decompiled C-like code + optimization suggestions        │
└──────────────────────────┬──────────────────────────────────────────┘
                           │
                           ▼
Phase 3: OPTIMIZATION
┌─────────────────────────────────────────────────────────────────────┐
│ Input: Decompiled code + optimization suggestions                   │
│                                                                     │
│ 1. Rewrite function in Rust/C++ with optimizations:                 │
│    - Better algorithms (O(n²) → O(n log n))                        │
│    - SIMD vectorization                                             │
│    - Cache-friendly data layout                                     │
│    - Fixed-point instead of float                                   │
│    - Lookup tables                                                  │
│ 2. Compile optimized version                                        │
│ 3. Output: Optimized DLL/shared object                              │
└──────────────────────────┬──────────────────────────────────────────┘
                           │
                           ▼
Phase 4: PATCHING
┌─────────────────────────────────────────────────────────────────────┐
│ Input: Original binary + optimized DLL                              │
│                                                                     │
│ 1. Find function address in original binary                         │
│ 2. Create trampoline (jump to optimized code)                       │
│ 3. Inject optimized function                                        │
│ 4. Preserve ABI (stack layout, registers, calling convention)       │
│ 5. Output: Patched binary or runtime hook                           │
└──────────────────────────┬──────────────────────────────────────────┘
                           │
                           ▼
Phase 5: VERIFICATION
┌─────────────────────────────────────────────────────────────────────┐
│ Input: Original binary + patched binary                             │
│                                                                     │
│ 1. Run both with identical inputs                                   │
│ 2. Compare frame-by-frame output (pixel diff)                       │
│ 3. Compare memory state at checkpoints                              │
│ 4. Measure performance (FPS, frame time, memory usage)              │
│ 5. Verify quirks preserved (known bugs still occur)                 │
│ 6. Output: Pass/Fail report                                         │
└─────────────────────────────────────────────────────────────────────┘
```

### Core Components

```
game-opt-tool/
├── src/
│   ├── profiler/          # Benchmark & profiling module
│   │   ├── renderdoc.rs   # RenderDoc integration
│   │   ├── pix.rs         # PIX integration (Windows)
│   │   └── custom.rs      # Custom profiler fallback
│   │
│   ├── decompiler/        # Ghidra headless integration
│   │   ├── ghidra.rs      # Ghidra Python API wrapper
│   │   ├── disassemble.rs # Disassembly logic
│   │   └── decompile.rs   # Decompilation logic
│   │
│   ├── optimizer/         # Code optimization engine
│   │   ├── analyze.rs     # Static analysis of decompiled code
│   │   ├── suggest.rs     # AI-powered optimization suggestions
│   │   ├── rewrite.rs     # Code rewriting engine
│   │   └── validate.rs    # Correctness validation
│   │
│   ├── patcher/           # Binary patching module
│   │   ├── find_func.rs   # Locate functions in binary
│   │   ├── trampoline.rs  # Create jump trampolines
│   │   ├── inject.rs      # DLL injection or binary patching
│   │   └── abi.rs         # ABI compatibility layer
│   │
│   ├── verifier/          # Verification & testing
│   │   ├── replay.rs      # Input replay system
│   │   ├── compare.rs     # Frame-by-frame comparison
│   │   ├── memdiff.rs     # Memory state comparison
│   │   └── report.rs      # Pass/fail reporting
│   │
│   └── cli/               # Command-line interface
│       ├── analyze.rs     # Analyze command
│       ├── optimize.rs    # Optimize command
│       ├── patch.rs       # Apply patch command
│       └── verify.rs      # Verify command
│
├── config/
│   ├── games.yaml         # Game-specific configurations
│   ├── optimizations.yaml # Available optimization passes
│   └── quirks.yaml        # Known quirks to preserve
│
├── targets/               # Game-specific support
│   ├── oblivion/
│   ├── skyrim/
│   ├── starwarsjedi/
│   └── horizon/
│
└── docs/
    ├── architecture.md
    ├── legal.md
    └── contributing.md
```

---

## Implementation Roadmap

### Phase 1: Proof of Concept (Weeks 1-2)

**Goal:** Make one function in one game faster

1. Set up Ghidra headless mode integration
2. Pick a simple game (e.g., Oblivion Remastered)
3. Find one hot function (e.g., NPC pathfinding)
4. Decompile it manually
5. Write optimized Rust version
6. Patch it in with a trampoline
7. Verify it works

**Deliverable:** Working POC that improves one function in one game

---

### Phase 2: Automation (Weeks 3-6)

**Goal:** Automate the pipeline for one game

1. Build the full pipeline (profile → decompile → optimize → patch → verify)
2. Add game-specific configs for Oblivion
3. Create verification suite (input replay + output comparison)
4. Test on 5-10 hot functions
5. Document results

**Deliverable:** Automated tool that can optimize an entire game

---

### Phase 3: Multi-Game Support (Weeks 7-12)

**Goal:** Support multiple games

1. Add configs for Skyrim, Fallout 4, Star Wars Jedi
2. Build game signature database (how to find functions in each game)
3. Create optimization catalog (common patterns that work across games)
4. Add CLI for end users
5. Test on 20+ functions across 5 games

**Deliverable:** General-purpose tool supporting multiple games

---

### Phase 4: AI Integration (Weeks 13-16)

**Goal:** Use AI to suggest and apply optimizations

1. Integrate local LLM (Ollama + code model)
2. Train/fine-tune on decompilation patterns
3. Add AI-assisted function naming
4. Add AI-verified correctness checks
5. Build interactive mode (human approves AI suggestions)

**Deliverable:** AI-assisted optimization tool

---

### Phase 5: Community & Distribution (Weeks 17-20)

**Goal:** Open source and community adoption

1. Publish on GitHub
2. Add documentation
3. Create game submission process
4. Build community guidelines
5. Launch

**Deliverable:** Public tool with community support

---

## Key Technical Challenges

### 1. Decompilation Quality

**Problem:** Ghidra's decompiler isn't perfect. Variable names are lost, control flow can be confusing.

**Solution:** 
- Use AI to refine pseudocode
- Cross-reference with strings and comments
- Human review for critical functions

### 2. ABI Compatibility

**Problem:** Original game uses specific calling conventions. Your Rust rewrite must match exactly.

**Solution:**
- Study the original calling convention
- Use `#[repr(C)]` in Rust
- Test with trampoline that preserves stack layout

### 3. Verification

**Problem:** How do you prove the optimized code behaves identically?

**Solution:**
- Record inputs (key presses, mouse movements)
- Replay through both original and optimized
- Compare frame buffers pixel-by-pixel
- Check memory state at savepoints

### 4. Quirk Preservation

**Problem:** Some "bugs" are actually features. Float precision quirks, timer rollovers, etc.

**Solution:**
- Build a "quirks database" per game
- Document known quirks
- Test that quirks still occur after optimization

---

## Legal & Ethical Guidelines

### What We Do
- ✅ Work only on games the user owns
- ✅ No DRM circumvention
- ✅ Single-player games only
- ✅ No distribution of game assets
- ✅ User applies the patch themselves

### What We Don't Do
- ❌ Bypass DRM or anti-cheat
- ❌ Distribute game files or assets
- ❌ Work on multiplayer/online games
- ❌ Commercial distribution
- ❌ Share decompiled source code publicly

---

## Next Steps

1. **Pick first target game** — I recommend Oblivion Remastered (already has community patches, well-documented issues)
2. **Set up Ghidra headless** — Install Ghidra, test Python API
3. **Find first hot function** — Use profiler to identify bottleneck
4. **Build MVP pipeline** — Profile → Decompile → Optimize → Patch → Verify

Want me to start building the project structure and first component?
