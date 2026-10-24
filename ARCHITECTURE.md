# Game Opt Tool — Architecture

This document explains how the tool works, with diagrams and data flow.

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            USER COMMAND                                     │
│                                                                          │
│   game-opt-tool patch --game game.exe --output patched.exe --spec s.json  │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           CLI LAYER (main.rs)                               │
│                                                                          │
│   • Parses arguments with clap                                           │
│   • Dispatches to command handler                                        │
│   • Formats output (text or JSON)                                        │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                    ┌───────────────┼───────────────┐
                    ▼               ▼               ▼
┌───────────────┐ ┌───────────────┐ ┌───────────────┐
│   Patcher     │ │   Backup      │ │  Decompiler   │
│  (patcher/)   │ │   (backup/)   │ │ (decompiler/) │
└───────┬───────┘ └───────┬───────┘ └───────┬───────┘
        │                 │                 │
        ▼                 ▼                 ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         CORE LOGIC                                          │
│                                                                          │
│  PE Parser        │  Backup Manager     │  Ghidra Runner                 │
│  ───────────      │  ──────────         │  ───────────                   │
│  • parse_pe()     │  • create()         │  • analyze()                   │
│  • map_rva()      │  • list_backups()   │  • decompile()                 │
│  • plan_section() │  • verify()         │  • parse_result()              │
│  • compute_cksum  │  • restore()        │                                │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         FILE SYSTEM                                         │
│                                                                          │
│  Input:                        Output:                                   │
│  ├── game.exe                   ├── patched.exe                          │
│  ├── patch.json                 ├── patched.exe.sha256.patch.json        │
│  └── backup.exe.abc123.bak      └── backup.exe.abc123.bak.json           │
│      backup.exe.abc123.bak.json                                │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Data Flow: Patching a Binary

```
Step 1: Read Input
───────────────────
game.exe (254 KB)
    │
    ▼
Step 2: Parse PE Headers
───────────────────────
┌────────────────────────────────────────────────────────────────────┐
│  DOS Header (64 bytes)                                             │
│  ├─ MZ signature at offset 0                                      │
│  ├─ e_lfanew at offset 0x3C (points to PE signature)              │
│  └─ [padding]                                                     │
│                                                                    │
│  PE Signature (4 bytes) @ 0x80                                    │
│  ├─ "PE\0\0"                                                       │
│  └─ [end]                                                          │
│                                                                    │
│  COFF Header (20 bytes) @ 0x84                                    │
│  ├─ Machine: 0x014C (x86)                                         │
│  ├─ NumberOfSections: 6                                            │
│  ├─ SizeOfOptionalHeader: 224                                      │
│  └─ Characteristics: 0x0102                                        │
│                                                                    │
│  Optional Header (224 bytes) @ 0x98                               │
│  ├─ Magic: 0x10B (PE32)                                           │
│  ├─ ImageBase: 0x00400000                                          │
│  ├─ SectionAlignment: 0x1000                                       │
│  ├─ FileAlignment: 0x200                                           │
│  ├─ SizeOfImage: 0x66000                                           │
│  ├─ SizeOfHeaders: 0x1000                                          │
│  ├─ CheckSum: 0x00000000                                           │
│  ├─ NumberOfRvaAndSizes: 16                                        │
│  └─ DataDirectories[16]                                            │
│                                                                    │
│  Section Table (6 × 40 = 240 bytes) @ 0x178                       │
│  ├─ .text: VA=0x1000, VSize=0x246DC, Raw=0x400, Size=0x25000     │
│  ├─ .rdata: VA=0x26000, VSize=0x5DB4, Raw=0x26000, Size=0x6000   │
│  ├─ .data: VA=0x2C000, VSize=0x3000, Raw=0x2C000, Size=0x4000    │
│  ├─ .idata: VA=0x57000, VSize=0x4916, Raw=0x57000, Size=0x5000   │
│  ├─ .rsrc: VA=0x5C000, VSize=0x58D9, Raw=0x5C000, Size=0x6000    │
│  └─ .reloc: VA=0x62000, VSize=0x3863, Raw=0x62000, Size=0x4000   │
└────────────────────────────────────────────────────────────────────┘
    │
    ▼
Step 3: Parse Patch Spec
──────────────────────────
patch.json:
{
  "schema_version": 1,
  "patches": [{
    "address": "0x109214b6",    ← Virtual Address
    "expected": "55 8B EC 6A FF",
    "replace": "90 90 90 90 90"
  }]
}
    │
    ▼
Step 4: Resolve VA → File Offset
──────────────────────────────────
VA 0x109214b6:
  RVA = VA - ImageBase = 0x109214b6 - 0x00400000 = 0x105214b6
  → Matches .text section (VA=0x1000, VSize=0x246DC)
  Wait, that's wrong. Let me recalculate.
  
  Actually: 0x109214b6 - 0x10900000 = 0x214b6 RVA
  → In .text section (VA=0x10901000, so RVA range is 0x1000..0x256DC)
  
  File offset = section_raw_offset + (rva - section_va)
              = 0x1000 + (0x214b6 - 0x1000)
              = 0x1000 + 0x204b6
              = 0x214b6
              
  Hmm, let me check the actual Deus Ex PE structure...
  
  Actually from objdump, .text starts at file offset 0x1000 with VA 0x10901000.
  So RVA 0x214b6 maps to file offset:
    0x1000 + (0x214b6 - 0x1000) = 0x214b6
  
  Wait, that's circular. Let me use the actual calculation:
    RVA = VA - ImageBase = 0x109214b6 - 0x10900000 = 0x214b6
    Section containing RVA 0x214b6: .text (VA=0x10901000)
    Section RVA = 0x214b6 - 0x10901000 = 0x104b6
    
    Hmm, that's not right either. The actual RVA of the entry point should
    be calculated as entry_va - image_base.
    
    Let me just use the tool's actual output:
    - The patch was applied at file offset 0x214b6 (from the JSON report)
    - This means RVA 0x214b6 is in section .text starting at RVA 0x1000
    
    File offset = SectionRawOffset + (RVA - SectionVA)
                = 0x1000 + (0x214b6 - 0x1000)
                = 0x214b6
    
    So the entry point RVA is 0x214b6, and the file offset is also 0x214b6
    because the .text section starts at file offset 0x1000 and RVA 0x1000.
    
    Wait, that means RVA 0x214b6 - SectionVA 0x1000 = 0x204b6 offset in section
    SectionRawOffset 0x1000 + 0x204b6 = 0x214b6 file offset.
    
    Yes! That matches the JSON report showing file_offset: 136374 = 0x214b6.

    Actual calculation from the tool output:
    - patch.address = 0x109214b6 (VA)
    - patch.rva = 0x214b6 (RVA = VA - ImageBase where ImageBase = 0x10900000)
    - patch.file_offset = 0x214b6 (offset in file where .text raw data starts is 0x1000, 
                                     so 0x1000 + (0x214b6 - 0x1000) = 0x214b6)
    - patch.section = ".text"
    - patch.before_hex = "558bec6aff"
    - patch.after_hex = "9090909090"

    The tool successfully applied the patch.
    │
    ▼
Step 5: Validate Patches
──────────────────────────
    ├─ Expected bytes match? YES (0x55 0x8B 0xEC 0x6A 0xFF at offset 0x214b6)
    ├─ Target is executable? YES (.text section has EXECUTE flag)
    ├─ Replace length == Expected length? YES (both 5 bytes)
    └─ No overlaps? YES (only 1 patch)
    │
    ▼
Step 6: Create Output
──────────────────────
    ├─ Copy game.exe → output buffer
    ├─ Apply patches to buffer
    │   buffer[0x214b6..0x214ba] = [0x90, 0x90, 0x90, 0x90, 0x90]
    ├─ (Optional) Append new section if spec has "sections"
    └─ Recompute PE checksum if non-zero
    
    Original file: UNCHANGED
    Output buffer: PATCHED
    │
    ▼
Step 7: Write Output Atomically
─────────────────────────────────
    ├─ Write to temp file: DeusEx.patched.exe.tmp-<pid>-<ns>
    ├─ fsync
    ├─ Rename to output path
    └─ Read back and verify SHA-256
    
    If crash during write: temp file is orphaned, original output untouched
    │
    ▼
Step 8: Write Manifest
───────────────────────
    ├─ Serialize PatchManifest to JSON
    ├─ Write to temp file
    ├─ fsync
    └─ Rename to manifest path
    
    Manifest contains:
    {
      "schema_version": 1,
      "tool_version": "0.1.0",
      "created_unix_secs": 1730000000,
      "input_path": "/tmp/mygame/DeusEx.exe",
      "output_path": "/tmp/mygame/DeusEx.patched.exe",
      "input_sha256": "3e6fdb89...",
      "output_sha256": "4c62ef92...",
      "patches": [{
        "address": 278009014,
        "rva": 136374,
        "file_offset": 136374,
        "section": ".text",
        "before_hex": "558bec6aff",
        "after_hex": "9090909090"
      }],
      "sections": []
    }
    │
    ▼
Done!
```

## Data Flow: Reverting a Patch

```
User runs:
  game-opt-tool unpatch --manifest DeusEx.patched.exe.4c62ef92.patch.json

Step 1: Load Manifest
──────────────────────
    ├─ Read JSON file
    ├─ Deserialize PatchManifest
    └─ Validate schema version = 1
    │
    ▼
Step 2: Verify Target Exists
──────────────────────────────
    ├─ Check target file exists
    ├─ Check not a symlink
    ├─ Check not a directory
    └─ Check it's a regular file
    │
    ▼
Step 3: Verify Target Integrity
─────────────────────────────────
    ├─ Read target file
    ├─ Check size matches manifest.output_file_len
    ├─ Compute SHA-256
    ├─ Compare with manifest.output_sha256
    │
    │   If mismatch → REFUSE (file was corrupted or wrong file)
    │
    └─ Parse PE headers
        ├─ Check number_of_sections matches manifest
        ├─ Check size_of_code matches manifest
        ├─ Check size_of_image matches manifest
        ├─ Check checksum matches manifest
        ├─ Check pe_offset matches manifest
        ├─ Check optional_header_offset matches manifest
        ├─ Check checksum_offset matches manifest
        ├─ Check pe_is_64 matches manifest
        ├─ Check pe_machine matches manifest
        ├─ Check entry_point_rva matches manifest
        └─ Check optional_header_size matches manifest
        
        If any mismatch → REFUSE (PE headers were tampered)
    │
    ▼
Step 4: Restore Original Bytes
───────────────────────────────
    ├─ Create mutable copy of target file
    ├─ For each patch in manifest.patches:
    │   ├─ Parse before_hex → bytes
    │   ├─ Parse after_hex → bytes
    │   ├─ Verify before.len() == after.len()
    │   ├─ Verify restored[offset..offset+len] == after (current bytes match)
    │   └─ Write before bytes to offset
    │
    ├─ For each section in manifest.sections:
    │   ├─ Parse header_before_hex → bytes
    │   ├─ Parse header_after_hex → bytes
    │   ├─ Verify restored[header_offset..header_offset+40] == after
    │   └─ Write before bytes to header_offset
    │
    ├─ If no sections:
    │   └─ Restore checksum to manifest.checksum_before
    │
    ├─ If sections existed:
    │   ├─ Restore number_of_sections to manifest.number_of_sections_before
    │   ├─ Restore size_of_code to manifest.size_of_code_before
    │   ├─ Restore size_of_image to manifest.size_of_image_before
    │   └─ Restore checksum to manifest.checksum_before
    │
    └─ Truncate file to manifest.original_file_len
    │
    ▼
Step 5: Verify Restoration
───────────────────────────
    ├─ Compute SHA-256 of restored file
    ├─ Compare with manifest.input_sha256
    │
    │   If mismatch → REFUSE (restoration failed)
    │
    └─ Success!
    │
    ▼
Step 6: Write Atomically
─────────────────────────
    ├─ Write to temp file
    ├─ fsync
    └─ Rename to target path
    │
    ▼
Done! File is now byte-for-byte identical to original.
```

## Backup System Internals

```
┌─────────────────────────────────────────────────────────────────────┐
│                     BACKUP FILE STRUCTURE                           │
│                                                                     │
│  Original:  DeusEx.exe                                              │
│                                                                     │
│  After 'backup --game DeusEx.exe':                                  │
│                                                                     │
│  ├── DeusEx.exe                          (original, unchanged)      │
│  ├── DeusEx.exe.3e6fdb89b880a60e.123456.bak    (exact copy)        │
│  └── DeusEx.exe.3e6fdb89b880a60e.123456.bak.json (manifest)        │
│                                                                     │
│  Manifest content:                                                  │
│  {                                                                  │
│    "version": 1,                                                    │
│    "original_path": "/tmp/mygame/DeusEx.exe",                       │
│    "backup_path": "/tmp/mygame/DeusEx.exe.3e6fdb89b880a60e.123456.bak", │
│    "sha256": "3e6fdb89b880a60ee984b885a48f262070019572f1531ed70d96108781bae00e", │
│    "size": 253952,                                                  │
│    "created_unix_secs": 1730000000,                                 │
│    "created_unix_nanos": 123456789                                  │
│  }                                                                  │
└─────────────────────────────────────────────────────────────────────┘
```

## File Naming Conventions

| File | Pattern | Purpose |
|------|---------|---------|
| Backup | `<name>.<sha256-prefix-16>.<nanos>.bak` | Exact copy of original |
| Backup manifest | `<name>.<sha256-prefix-16>.<nanos>.bak.json` | Metadata for restore |
| Patch output | `<name>.exe` | Patched binary |
| Patch manifest | `<name>.exe.<output-sha256-prefix-16>.patch.json` | Rollback record |

## Error Handling Strategy

| Error | Handling |
|-------|----------|
| Expected bytes don't match | Fail before writing, no output created |
| Target file corrupted | Fail with clear message, no changes made |
| Disk full during write | Temp file orphaned, original untouched |
| SHA-256 mismatch on restore | Fail with clear message, file unchanged |
| Symlink detected | Fail with message, no write |
| Signed PE (default) | Fail, suggest `allow_signed: true` |
| Missing backup for in-place | Fail, suggest running `backup --game` first |

## Security Model

1. **Input validation**: All user-provided values (addresses, bytes, paths) are validated before use.
2. **Bounds checking**: All file offsets are checked against file length.
3. **Cryptographic verification**: SHA-256 guards every critical transition.
4. **Symlink refusal**: Prevents accidental writes to arbitrary files.
5. **Atomic writes**: Prevents corruption on crash.
6. **Expected bytes**: Patches refuse to apply if the target bytes have changed.

