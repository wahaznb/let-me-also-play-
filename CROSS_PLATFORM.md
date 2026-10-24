# Cross-Platform Build Guide

## Supported Targets

| Target | Toolchain | CI | Local | Notes |
|--------|-----------|-----|-------|-------|
| `x86_64-unknown-linux-gnu` | Native | ✅ | ✅ | CachyOS, Ubuntu |
| `x86_64-pc-windows-msvc` | Visual Studio | ✅ | ❌ | Windows 10/11 |
| `x86_64-pc-windows-gnu` | MinGW-w64 | ✅ | ❌ (root) | Windows 10/11 |
| `x86_64-apple-darwin` | macOS Xcode | ✅ | ❌ | Intel Mac |
| `aarch64-apple-darwin` | macOS Xcode | ✅ | ❌ | Apple Silicon Mac |

## Why macOS Was Re-added

macOS was ditched because:
- No local testing (no Mac available)
- No Wine installed
- No MangoHud (Linux-only)

But GitHub Actions can build and test on macOS runners automatically. The code uses `#[cfg(unix)]` which includes macOS, so it should compile. CI will tell us if it actually works.

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

### macOS (needs macOS machine)
```bash
# Add target
rustup target add x86_64-apple-darwin

# Build
cargo build --release --target x86_64-apple-darwin

# Test
cargo test --all-targets --target x86_64-apple-darwin
```

## Platform-Specific Code

The code uses `#[cfg]` attributes for platform-specific behavior:

| Module | Unix (`#[cfg(unix)]`) | Windows (`#[cfg(windows)]`) |
|--------|----------------------|---------------------------|
| `backup/mod.rs` | `sync_all`, permissions | `remove_file` before rename |
| `decompiler/mod.rs` | `analyzeHeadless` | `analyzeHeadless.bat` |
| `patcher/mod.rs` | symlink tests, permissions | N/A |
| `game/mod.rs` | Linux/macOS detection | Windows detection |

macOS is covered by `#[cfg(unix)]` — same as Linux.

## Known Issues

| Issue | Status |
|-------|--------|
| File locking on Windows | Windows locks open files, may block patching |
| Path separators | Tool accepts both `/` and `\` |
| Case sensitivity | Windows is case-insensitive |
| Antivirus | May flag patched executables |
| No MangoHud | Frame profiling not available on Windows or macOS |
| Wine on macOS | Not installed, Windows games need it |

## CI Artifacts

After each push, GitHub Actions uploads:
- `game-opt-tool-windows` — Windows MSVC binary
- `game-opt-tool-windows-gnu` — Windows GNU binary
- `game-opt-tool-linux` — Linux binary
- `game-opt-tool-macos` — macOS Intel binary
- `game-opt-tool-macos-arm64` — macOS Apple Silicon binary

Download from Actions tab → latest workflow → artifacts.
