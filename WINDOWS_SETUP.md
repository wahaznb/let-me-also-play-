# Windows Setup Guide

This tool runs on Windows 10/11 (x86_64). Below are the exact steps for a fresh install.

## Quick Start (Minimal)

For backup/restore and patching only (no Ghidra):

```powershell
# 1. Install Rust (if not already installed)
# Download from https://rustup.rs/ and run:
# rustup-init.exe

# 2. Clone and build
git clone <your-repo>
cd game-opt-tool
cargo build --release

# 3. Copy game to writable location
# (GOG/Epic installs are read-only)
xcopy "C:\Program Files (x86)\GOG Galaxy\Games\Deus Ex GOTY\System" "D:\deusex-work\" /E /I

# 4. Create backup
.\target\release\game-opt-tool.exe backup --game D:\deusex-work\DeusEx.exe

# 5. Create patch spec
echo {schema_version:1,patches:[{address:"0x109214b6",expected:"55 8B EC 6A FF",replace:"90 90 90 90 90"}]} > patch.json

# 6. Apply patch
.\target\release\game-opt-tool.exe patch --game D:\deusex-work\DeusEx.exe --output D:\deusex-work\DeusEx.patched.exe --spec patch.json

# 7. Test
start D:\deusex-work\DeusEx.patched.exe
```

## Full Setup (with Ghidra decompilation)

Ghidra requires Java 25. On Windows, download the JDK:

```powershell
# 1. Download JDK 25 (Adoptium)
$url = "https://api.adoptium.net/v3/binary/latest/25/ga/windows/x64/jdk/hotspot/normal/eclipse"
Invoke-WebRequest -Uri $url -OutFile "$env:USERPROFILE\jdk25.zip"

# 2. Extract
Expand-Archive "$env:USERPROFILE\jdk25.zip" "$env:USERPROFILE\jdk25"
Rename-Item "$env:USERPROFILE\jdk25\jdk-25*" "$env:USERPROFILE\jdk25"

# 3. Download Ghidra 12.2
$url = "https://github.com/NationalSecurityAgency/ghidra/releases/download/Ghidra_12.2_build/ghidra_12.2_20250123.zip"
Invoke-WebRequest -Uri $url -OutFile "$env:USERPROFILE\ghidra.zip"
Expand-Archive "$env:USERPROFILE\ghidra.zip" "$env:USERPROFILE"
Rename-Item "$env:USERPROFILE\ghidra_12.2_PUBLIC" "$env:USERPROFILE\ghidra"

# 4. Set environment variables
$env:JAVA_HOME = "$env:USERPROFILE\jdk25"
$env:PATH = "$env:JAVA_HOME\bin;$env:PATH"
$env:GHIDRA_INSTALL_DIR = "$env:USERPROFILE\ghidra"

# Verify
java -version
& "$env:GHIDRA_INSTALL_DIR\support\analyzeHeadless"
```

## Platform Differences

### What's the same
- PE parsing, patching, and unpatching work identically
- Backup/restore logic is platform-agnostic
- JSON manifest format is the same
- SHA-256 verification works the same

### What's different

| Feature | Linux | Windows |
|---------|-------|---------|
| Line endings | `\n` | `\r\n` (not an issue for binary patches) |
| Path separators | `/` | `\` (tool accepts both) |
| Case sensitivity | `DeusEx.exe` ≠ `deusex.exe` | `DeusEx.exe` = `deusex.exe` |
| Atomic writes | `rename()` is atomic | `MoveFileEx()` with `MOVEFILE_REPLACE_EXISTING` |
| File locking | Limited | Exclusive locks on open files |
| Wine | Not needed | Not needed (runs natively) |
| MangoHud | Works | N/A (use RTSS or FRAPS instead) |
| Ghidra | Requires JDK 25 | Requires JDK 25 |

### File locking caveat

Windows locks files that are open. If DeusEx.exe is running:
- `backup` will fail (can't read locked file)
- `patch` will fail (can't write output)

**Solution**: Close the game first, or copy to a different path.

### Permissions

Windows may require admin for:
- Writing to `C:\Program Files` (use a different directory)
-某些 AV software may flag modified executables

**Recommendation**: Use a directory like `D:\deusex-work\` or `C:\Users\You\deusex-work\`.

## Common Issues

### "Access denied" when backing up
```powershell
# The game is running. Close it first.
taskkill /F /IM DeusEx.exe
# Or copy to a different location
xcopy "C:\GOG Galaxy\Games\Deus Ex\System" "D:\deusex\" /E
```

### "File is locked" error
```powershell
# Check if file is open
handle.exe DeusEx.exe  # from Sysinternals
# Or just reboot and try before launching the game
```

### Java/Ghidra not found
```powershell
# Set environment variables permanently
[Environment]::SetEnvironmentVariable("JAVA_HOME", "$env:USERPROFILE\jdk25", "User")
[Environment]::SetEnvironmentVariable("GHIDRA_INSTALL_DIR", "$env:USERPROFILE\ghidra", "User")
# Restart terminal
```

### Path with spaces
```powershell
# Quotes around paths with spaces
.\game-opt-tool.exe backup --game "D:\My Games\Deus Ex\DeusEx.exe"
```

## Storage Requirements

| Component | Space | Notes |
|-----------|-------|-------|
| Tool binary | ~3 MB | `target/release/game-opt-tool.exe` |
| Game copy | Size of exe | ~250 MB for Deus Ex |
| Backup (.bak) | Same as game | Exact copy |
| Backup manifest | ~500 B | JSON file |
| Patched exe | Same as game | Slightly larger if code cave added |
| Patch manifest | ~1-2 KB | JSON file |
| Ghidra (optional) | ~1.5 GB | If using decompile command |
| JDK 25 (optional) | ~300 MB | If using decompile command |

## Security Notes for Windows

1. **Antivirus**: Modified executables may trigger false positives. Add an exclusion for your work directory.
2. **User Account Control**: No admin rights needed unless writing to `Program Files`.
3. **File associations**: `.json` files open in Notepad by default. Use a proper editor.

## Verification

After setup, run:
```powershell
.\target\release\game-opt-tool.exe doctor --game "D:\deusex-work\DeusEx.exe"
```

Expected output:
```
game-opt-tool doctor
====================
[ok]   config: config\games.yaml parses (3 games)

External tools:
[ok]   wine: not needed on Windows
[warn] mangohud: not found (frame-time capture, optional)

Game target:
[ok]   detected: DeusEx (PE, x86)
[fail] backups: directory is not writable  ← fix: use writable dir
[warn] backups: none yet
```

