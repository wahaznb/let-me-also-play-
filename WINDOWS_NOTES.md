# Windows Testing Notes

## What We Know

The code uses `#[cfg(windows)]` in several places:
- `src/backup/mod.rs` — Windows-specific file replacement
- `src/decompiler/mod.rs` — Windows path handling
- `src/main.rs` — Windows-specific code paths
- `src/patcher/mod.rs` — Unix-specific code (permissions, symlinks)

## What We Don't Know

1. Does `cargo build --target x86_64-pc-windows-msvc` succeed?
2. Does the binary run on Windows 10/11?
3. Does PE parsing work identically?
4. Does backup/restore work?
5. Does patching work?
6. Does file locking block operations?
7. Does the atomic write work?

## Testing Plan

### Step 1: Cross-compile
```bash
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

### Step 2: Test on Windows VM
- Install Windows 10/11
- Copy game to writable directory
- Run the binary
- Test backup/patch/revert

### Step 3: Fix Issues
- File locking problems
- Path separator issues
- Permission issues
- Antivirus false positives

### Step 4: Add CI
- GitHub Actions workflow
- Build on every push
- Upload Windows artifact

## Known Issues

| Issue | Status |
|-------|--------|
| File locking | Windows locks open files, may block patching |
| Path separators | Tool accepts both / and \ |
| Case sensitivity | Windows is case-insensitive |
| Antivirus | May flag patched executables |
| No MangoHud | Frame profiling not available on Windows |
| No Wine needed | Games run natively |

## Quick Test Script

```powershell
# Run this on Windows to verify basic functionality

# 1. Check binary runs
.\game-opt-tool.exe --help

# 2. Check doctor
.\game-opt-tool.exe doctor

# 3. Copy game (adjust path)
$game = "C:\Program Files\GOG\Deus Ex\System\DeusEx.exe"
$work = "D:\deusex-work\"
New-Item -Path $work -ItemType Directory -Force
Copy-Item $game $work

# 4. Backup
.\game-opt-tool.exe backup --game "$work\DeusEx.exe"

# 5. Patch
.\game-opt-tool.exe patch --game "$work\DeusEx.exe" --output "$work\DeusEx.patched.exe" --spec patch.json

# 6. Verify
.\game-opt-tool.exe unpatch --manifest "$work\DeusEx.patched.exe.*.patch.json"
```
