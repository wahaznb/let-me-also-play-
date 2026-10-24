# Pre-built Binaries

## Download

| Platform | Binary | Size |
|----------|--------|------|
| Windows 10/11 (x64) | `game-opt-tool.exe` | ~3 MB |
| Linux (x86_64) | `game-opt-tool` | ~3 MB |
| macOS (x86_64) | `game-opt-tool` | ~3 MB |

## Windows Usage

```powershell
# 1. Download the binary
# From GitHub Releases or direct link

# 2. Copy game to writable location
xcopy "C:\Program Files\GOG\Deus Ex\System" "D:\deusex-work\" /E /I

# 3. Create backup
.\game-opt-tool.exe backup --game D:\deusex-work\DeusEx.exe

# 4. Create patch spec
echo {schema_version:1,patches:[{address:"0x109214b6",expected:"55 8B EC 6A FF",replace:"90 90 90 90 90"}]} > patch.json

# 5. Apply patch
.\game-opt-tool.exe patch --game D:\deusex-work\DeusEx.exe --output D:\deusex-work\DeusEx.patched.exe --spec patch.json

# 6. Test
start D:\deusex-work\DeusEx.patched.exe
```

## Important Windows Notes

1. **Close the game before patching** — Windows locks open files
2. **Use a writable directory** — Not Program Files
3. **Antivirus may flag** — Add exclusion for your work directory
4. **No frame profiling** — MangoHud is Linux-only
5. **Ghidra optional** — Download JDK 25 + Ghidra 12.2 manually

## Build from Source

If you prefer to build from source:

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build
git clone <repo>
cd game-opt-tool
cargo build --release
```
