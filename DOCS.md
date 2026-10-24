# Game Opt Tool — Documentation

> **Outdated.** This document describes the pre-revert prototype and includes
> features that do not exist in the current CLI. See [STATUS.md](STATUS.md) for
> the verified state and [README.md](README.md) for current usage.

## Overview
A tool that reverse engineers and optimizes game binaries to improve performance while preserving original behavior. Target games include Deus Ex (2000) and other PE32/x86 executables.

## Quick Start

### Installation
```bash
# 1. Install JDK 21 (required for Ghidra)
export JAVA_HOME=/path/to/jdk21

# 2. Install Ghidra 12.2+ (decompiler)
export GHIDRA_INSTALL_DIR=/path/to/ghidra_12.2_PUBLIC

# 3. Build the tool
cargo build --release --bins

# 4. Run analysis
cargo run --release -- analyze --game /path/to/game.exe
```

### Supported Commands

#### `analyze` — Profile and identify hot functions
```bash
cargo run --release -- analyze --game /path/to/your/game/DeusEx.exe
```

**Output:**
- Game detection (PE/ELF/Mach-O format, architecture)
- Hot functions with frame time percentages
- Performance baseline metrics

#### `profile` — Frame time profiling
```bash
cargo run --release -- profile --game /path/to/game.exe --frames 100
```

**Output:**
- Average FPS, Min FPS, Max FPS
- Average frame time, P99 frame time
- Frame time consistency metrics

#### `optimize` — Decompile and optimize hot functions
```bash
cargo run --release -- optimize --game /path/to/game.exe --output ./optimized/
```

**Requires:** Ghidra installation
**Output:** Decompiled pseudocode, optimized code suggestions

#### `patch` — Apply binary patches via trampolines
```bash
cargo run --release -- patch --game /path/to/game.exe --output ./patched.exe
```

**Output:** Patched executable with optimized code

#### `verify` — Verify behavioral equivalence
```bash
cargo run --release -- verify --original original.exe --patched patched.exe
```

**Output:**
- Behavioral match: true/false
- Frame difference: X.XX%
- Quirks preserved: true/false
- Performance gain: +X.X%

---

## 🎯 Optimization Pipeline (Full Workflow)

### 1. Analyze
```bash
cargo run --release -- analyze --game /path/to/game.exe
```
**Detects:**
- Binary format (PE32, PE32+, ELF, Mach-O)
- Architecture (x86, x64, ARM, ARM64)
- Game detection from path

### 2. Profile (Optional but recommended)
```bash
cargo run --release -- profile --game /path/to/game.exe --frames 100
```
**Measures:**
- Average FPS, Min/Max FPS
- Avg frame time (ms)
- P99 frame time (ms) — worst 1% of frames
- Frame time consistency

### 3. Optimize
```bash
cargo run --release -- optimize --game /path/to/game.exe --output ./optimized/
```
**Process:**
1. Loads config/games.yaml for known hot functions
2. Uses Ghidra headless to decompile functions
3. Applies optimization passes:
   - loop_unrolling — Unroll loops to reduce branch overhead
   - simd_vectorization — Vectorize loops using SIMD instructions
   - cache_friendly_layout — Restructure data for better cache locality
   - constant_folding — Fold compile-time constants
   - dead_code_elimination — Remove unreachable code
   - function_inlining — Inline small functions

### 4. Patch
```bash
cargo run --release -- patch --game /path/to/game.exe --output ./patched.exe
```
**Creates trampoline-based patches:**
- JMP rel32 trampolines for x86
- Preserves original bytes for rollback
- Patches binary at function entry points

### 5. Verify
```bash
cargo run --release -- verify --original /path/to/original.exe --patched /path/to/patched.exe
```
**Checks:**
- Behavioral match (frame-accurate equivalence)
- Quirks preservation (known bugs/quirks kept intact)
- Performance gain (% improvement)
- Frame difference (% visual divergence)

---

## 📁 Project Structure

```
game-opt-tool/
├── Cargo.toml              # Rust project config
├── src/                    # Rust source files
│   ├── main.rs            # CLI entry point
│   ├── cli/               # Command-line interface
│   ├── game/              # Game detection/parsing
│   ├── profiler/          # Frame time measurement
│   ├── optimizer/         # Code optimization engine
│   ├── patcher/           # Binary patching with trampolines
│   ├── verifier/          # Behavioral verification
│   ├── decompiler/        # Ghidra integration
│   └── config/            # Configuration management
├── config/                # Game configurations
│   └── games.yaml         # Game configs and optimization passes
├── .vibe-wise/            # Learning state (vibe-wise)
│   ├── profile.md         # Learner profile
│   ├── progress.md        # Learning progress
│   └── project-map.md     # Project structure map
├── docs/                  # Documentation (this file)
└── LEGAL.md               # Legal guidelines
```

---

## 🎮 Supported Games

The tool works with any 32-bit or 64-bit Windows executable (PE format). Currently configured for:

| Game | Binary | Architecture | Config |
|------|--------|-------------|--------|
| Deus Ex GOTY (2000) | PE32 | x86 | config/games.yaml |

### Adding a New Game

1. **Detect the binary:**
   ```bash
   cargo run --release -- analyze --game /path/to/game.exe
   ```

2. **Add game config to `config/games.yaml`:**
```yaml
games:
  my-game:
    name: "My Game Name"
    version: "1.0"
    engine: "Engine Name"
    binary_format: PE
    architecture: X86
    
    hot_functions:
      - name: "FunctionName"
        estimated_impact: Critical
    
    quirks:
      - name: "Known Quirk"
        description: "Description"
        function: "FunctionName"
        condition: "When condition occurs"
        preservation_required: true
```

3. **Run optimization:**
   ```bash
   cargo run --release -- optimize --game /path/to/game.exe --output ./optimized
   ```

---

## ⚙️ Optimization Passes

| Pass | Description | Enabled |
|------|-------------|---------|
| loop_unrolling | Unroll loops to reduce branch overhead | ✅ |
| simd_vectorization | Vectorize loops using SIMD instructions | ✅ |
| cache_friendly_layout | Restructure data for better cache locality | ✅ |
| constant_folding | Fold compile-time constants | ✅ |
| dead_code_elimination | Remove unreachable code | ✅ |
| function_inlining | Inline small functions | ✅ |

Each pass has configurable parameters in games.yaml.

---

## 🔄 100% Optimization Gate

The tool will continue optimizing until one of these conditions is met:

### Stop Conditions (game is "100% optimized"):

1. **Behavioral match = true** — Verification confirms frame-accurate equivalence
2. **Performance gain ≥ threshold** — User-defined improvement threshold reached
3. **User confirmation** — Manual override via `--confirm-optimized` flag

### Iterative Loop

The tool will automatically:
1. Optimize → Patch → Verify
2. If NOT 100% optimized: repeat with adjusted parameters
3. If 100% optimized: stop and report final metrics

### Example: Full Iterative Pipeline

```bash
# Run until 100% optimized (will loop automatically)
cargo run --release -- run-pipeline \
  --game /path/to/game.exe \
  --until-optimized \
  --max-iterations 10 \
  --performance-threshold 15.0
```

**Output after completion:**
```
=== Optimization Complete ===
Game: /path/to/game.exe
Iterations: 3/10
Final Performance Gain: +18.5%
Behavioral Match: true
Quirks Preserved: true
Frame Difference: 0.03%
Result: OPTIMAL — ready for use
```

---

## 📊 Performance Metrics Tracked

| Metric | Target | Description |
|--------|--------|-------------|
| **P99 frame time** | < 16.7ms | Worst 1% of frames for 60fps smoothness |
| **1% Low FPS** | ≥70% of avg | Minimum playable frame rate |
| **Behavioral match** | 100% | Frame-accurate equivalence |
| **Quirks preserved** | 100% | Known bugs/quirks intact |
| **Performance gain** | +10%+ | Frame time reduction |
| **CPU utilization** | ~50% | Multi-core distribution |

---

## ⚠️ Known Limitations

1. **Ghidra required** for full decompilation
2. **Java 21+** needed for Ghidra headless mode
3. **Optimization passes** are partially implemented (skeleton/prototype)
4. **Binary patching** works for trampoline jmp rel32 on x86
5. **Frame time profiling** uses placeholder data until real profiling is integrated
6. **Only Windows PE binaries** supported (32-bit and 64-bit)

---

## 🛡️ Legal Guidelines

**LEGAL.md content (summary):**
- Personal use only
- Owned games only
- Single-player only
- No distribution of assets
- No DRM circumvention
- Behavioral preservation required
- See LEGAL.md for full text

---

## 📦 Configuration (config/games.yaml)

```yaml
games:
  deus-ex:
    name: "Deus Ex GOTY"
    version: "1.0"
    engine: "BuildEngine"
    binary_format: PE
    architecture: X86
    
    hot_functions:
      - name: "npc_ai_update"
        estimated_impact: Critical
      - name: "physics_step"
        estimated_impact: Critical
      - name: "pathfinding"
        estimated_impact: High
      - name: "render_submit"
        estimated_impact: Medium
    
    quirks:
      - name: "AI Quirk"
        description: "AI behavior quirk"
        function: "npc_ai_update"
        condition: "During combat"
        preservation_required: true

optimizations:
  - name: "loop_unrolling"
    enabled: true
    parameters:
      min_iterations: "4"
      max_unroll: "8"
```

---

## 🐛 Troubleshooting

| Issue | Solution |
|-------|----------|
| "Ghidra not found" | Set GHIDRA_INSTALL_DIR env var |
| "Java not found" | Set JAVA_HOME env var, install JDK 21+ |
| "Failed to parse config/games.yaml" | Ensure YAML is valid, games key exists |
| "No hot functions found" | Add game config to games.yaml |
| "Verification failed" | Check quirks preservation, reduce optimization aggressiveness |
| "Cargo build errors" | Run `cargo update` or check Rust toolchain |

---

## 🔮 Future Roadmap

- [x] Game binary detection and format parsing
- [x] Hot function profiling
- [x] Ghidra integration (decompilation)
- [x] Trampoline-based binary patching
- [x] Behavioral verification framework
- [ ] Real SIMD optimizations (hand-tuned intrinsics)
- [ ] Auto-vectorization pass
- [ ] Multi-game batch optimization
- [ ] Web UI for configuration and results
- [ ] Console game support (PE/ELF/Mach-O)
- [ ] Performance regression detection

---

## 📜 Version History

**v0.1.0** — Initial release
- Game detection and profiling
- Hot function identification
- Configuration system
- Trampoline patcher skeleton
- Verification framework

**v0.1.1** — Optimization pipeline
- Ghidra integration
- Optimization passes
- 100% optimization gate
- Multi-game support

**v0.2.0** (planned) — Full optimization
- Real code optimization
- SIMD vectorization
- Auto-parameter tuning
- Performance regression detection

---