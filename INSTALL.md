# Installation Guide

## Quick Start

### Prerequisites
- Rust 1.80+ (install from https://rustup.rs/)
- Git

### Build from Source
```bash
git clone <your-repo>
cd game-opt-tool
cargo build --release
```

The binary will be at `target/release/game-opt-tool`.

### Usage
```bash
# Create a backup
./target/release/game-opt-tool backup --game /path/to/game.exe

# Apply a patch
./target/release/game-opt-tool patch --game game.exe --output patched.exe --spec patch.json

# Revert a patch
./target/release/game-opt-tool unpatch --manifest patched.exe.*.patch.json
```

## Optional: Decompilation Support

To use the `decompile` command, you need:

1. **JDK 25** (full JDK, not JRE)
   - Download from https://adoptium.net/
   - Extract to a permanent location (not /tmp)

2. **Ghidra 12.2**
   - Download from https://ghidra-sre.org/
   - Extract to a permanent location

3. **Set environment variables:**
   ```bash
   export JAVA_HOME=/path/to/jdk25
   export GHIDRA_INSTALL_DIR=/path/to/ghidra
   ```

Then:
```bash
./target/release/game-opt-tool decompile --game game.exe
```

## Platform Notes

### Linux
- Backup/patch commands work immediately after building
- MangoHud optional for frame profiling (sudo pacman -S mangohud)
- Wine optional for running Windows games

### Windows
- Close the game before patching (file locking)
- Use a writable directory (not Program Files)
- Antivirus may flag patched executables

## Storage Requirements

| Component | Size |
|-----------|------|
| Tool binary | ~3 MB |
| Backup (.bak) | Same as game exe |
| Manifest (.json) | 1-2 KB |
| Ghidra (optional) | ~900 MB |
| JDK (optional) | ~300 MB |

## Troubleshooting

| Problem | Solution |
|---------|----------|
| `command not found: cargo` | Install Rust from rustup.rs |
| `Java not found` | Set JAVA_HOME environment variable |
| `Ghidra not found` | Set GHIDRA_INSTALL_DIR environment variable |
| `Permission denied` | Copy game to a writable directory |
| `File locked` | Close the game before patching |

