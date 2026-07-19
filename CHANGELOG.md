# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-19

### Added

- Full glibc 2.43 compatibility for CLI options:
  - `-X`: Don't update symbolic links
  - `-n`: Only process directories given on the command line
  - Positional arguments: Additional directories to scan
- `glibc-hwcaps` subdirectory scanning and cache extension entries
- x86-64 ISA level detection from `GNU_PROPERTY_X86_ISA_1_NEEDED`
- Chroot-safe path canonicalization via `chroot_canon` function (port of glibc's `elf/chroot_canon.c`)
- Enhanced architecture flag support matching `sysdeps/generic/ldconfig.h`:
  - x86-64: `FLAG_X8664_LIB64` (x32 as `FLAG_X8664_LIBX32`)
  - x86: Base ELF flag for EM_386 objects
  - AArch64: `FLAG_AARCH64_LIB64`
  - ARM: `FLAG_ARM_LIBHF` / `FLAG_ARM_LIBSF` from e_flags
  - RISC-V: `FLAG_RISCV_FLOAT_ABI_SOFT` / `FLAG_RISCV_FLOAT_ABI_DOUBLE` from e_flags
  - PowerPC: `FLAG_POWERPC_LIB64` for 64-bit, base flag for 32-bit
- Multibyte character handling in config files
- Recursive include directive expansion with depth limit (32)
- Proper handling of comments in config files (via `#`)
- Relative path validation and rejection when building cache
- New cache format validation with proper error messages
- `chroot_canon` function exported from library for programmatic use
- `with_system()` method on `SearchPaths` to append system directories
- `hwcaps` field added to `CacheEntry` for glibc-hwcaps subdirectory names

### Changed

- **API**: `SearchPaths::from_file` now returns only config directories; use `.with_system()` to append system directories
- **Breaking**: `-N` flag semantics changed from "dry run (no writes)" to "don't rebuild cache" to match glibc behavior. For true dry run (no cache write, no symlink updates), use `-N -X`.
- CLI error messages now match glibc format (`ldconfig: <message>`)
- Config file paths are now resolved via `chroot_canon` when `-r` is specified
- Cache file paths are resolved safely within the root directory
- Atomic write now requires parent directory to exist (matches glibc behavior)
- Logging simplified: removed line numbers, cleaner output

### Fixed

- Config parsing no longer panics on multibyte characters
- Symlink resolution in chroot mode cannot escape the root directory
- Cache path is confined to the root directory
- Missing cache file parent directory errors handled gracefully
- Truncated or malformed cache files are rejected without panicking

### Removed

- `hwcap.rs` module (hwcap handling integrated into scanner and ELF inspection)

### Not Implemented

The following glibc ldconfig features are not yet implemented:
- `-c fmt`: Cache format selection (old, new, compat) — only new format is supported (other formats reject with error)
- `-i`: Ignore auxiliary cache file — flag accepted but exits with error
- `-l`: Interpret operands as library names — flag accepted but exits with error

## [0.1.1] - 2025-03-29

### Changed

- Renamed osversion to _osversion in CacheEntry for clarity

## [0.1.0] - 2025-03-29

### Added

- Initial release
- Basic ld.so.cache reading and writing
- Config file parsing
- Library scanning
- Cache building
