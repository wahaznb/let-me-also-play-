# Cross-Platform Build Guide

## Supported Targets

| Target | Toolchain | CI | Local |
|--------|-----------|-----|-------|
| `x86_64-unknown-linux-gnu` | Native | ✅ | ✅ |
| `x86_64-pc-windows-msvc` | Visual Studio | ✅ | ❌ |
| `x86_64-pc-windows-gnu` | MinGW-w64 | ✅ | ❌ (needs root) |
| `aarch64-apple-darwin` | macOS Xcode | ❌ | ❌ |

## Local Testing

### Linux (CachyOS)
```bash
cargo build --release
cargo test --all-targets
```

### Windows (needs root or WSL)
```bash
# Install mingw-w64
sudo pacman -S mingw-w64-gcc

# Add target
rustup target add x86_64-pc-windows-gnu

# Build
cargo build --release --target x86_64-pc-windows-gnu

# Test
cargo test --all-targets --target x86_64-pc-windows-gnu
```

### Windows (GitHub Actions)
Automatically builds on every push:
- `x86_64-pc-windows-msvc` (Windows runner)
- `x86_64-pc-windows-gnu` (Linux runner + mingw-w64)

## Platform-Specific Code

The code uses `#[cfg]` attributes for platform-specific behavior:

| Module | Unix | Windows |
|--------|------|---------|
| `backup/mod.rs` | `sync_all`, permissions | `remove_file` before rename |
| `decompiler/mod.rs` | `analyzeHeadless` | `analyzeHeadless.bat` |
| `patcher/mod.rs` | symlink tests, permissions | N/A |
| `game/mod.rs` | Linux/macOS detection | Windows detection |

## Known Issues

| Issue | Status |
|-------|--------|
| File locking on Windows | Windows locks open files, may block patching |
| Path separators | Tool accepts both `/` and `\` |
| Case sensitivity | Windows is case-insensitive |
| Antivirus | May flag patched executables |
| No MangoHud | Frame profiling not available on Windows |

## CI Artifacts

After each push, GitHub Actions uploads:
- `game-opt-tool-windows` — Windows MSVC binary
- `game-opt-tool-windows-gnu` — Windows GNU binary
- `game-opt-tool-linux` — Linux binary

Download from Actions tab → latest workflow → artifacts.
