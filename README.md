# ldconfig - Portable Rust Implementation

[![LICENSE](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![docs.rs](https://img.shields.io/docsrs/ldconfig)](https://docs.rs/ldconfig)
[![dependency status](https://deps.rs/repo/github/lu-zero/ldconfig/status.svg)](https://deps.rs/repo/github/lu-zero/ldconfig)

A Rust implementation of glibc's `ldconfig`: it scans library directories,
keeps the soname symlinks up to date and writes `ld.so.cache`, producing the
same cache the real tool does.

It comes as a command-line tool and as a library for:
- Reading and exploring `ld.so.cache` files
- Parsing `ld.so.conf` configuration files
- Building cache files by scanning library directories
- Writing cache files to disk

One binary handles every supported architecture, so it can build the cache of
a cross-compilation sysroot or a container image from the host, without
running anything inside it.

- [Supported architectures](#supported-architectures)
- [Command-line usage](#command-line-usage)
- [Library usage](#library-usage)
- [Differences from glibc](#differences-from-glibc)
- [Testing](#testing)
- [`docs/architecture.md`](docs/architecture.md) - how it works inside
- [`AGENTS.md`](AGENTS.md) - conventions for contributors, human or not

## Installation

```bash
cargo install ldconfig
```

Linux with glibc is the target; the cache format is glibc's.

## Supported Architectures

This implementation supports the following architectures with proper glibc cache flags:

- **x86-64** - `FLAG_X8664_LIB64`; x32 as `FLAG_X8664_LIBX32`
- **x86** (i386 through i686, all `EM_386` objects) - base ELF flag
- **AArch64** - `FLAG_AARCH64_LIB64`
- **ARM** (EABI v5) - `FLAG_ARM_LIBHF` / `FLAG_ARM_LIBSF` from the float ABI in `e_flags`
- **RISC-V** (RV32/RV64) - `FLAG_RISCV_FLOAT_ABI_SOFT` / `FLAG_RISCV_FLOAT_ABI_DOUBLE` from `e_flags`
- **PowerPC** - `FLAG_POWERPC_LIB64` for 64-bit, base flag for 32-bit
- **LoongArch** - `FLAG_LARCH_FLOAT_ABI_SOFT` / `FLAG_LARCH_FLOAT_ABI_DOUBLE` from `e_flags`
- **MIPS** - o32, n32 and n64, each with its 2008-NaN variant
- **S/390** and **SPARC** - the 64-bit flag for s390x and sparc64, base flag for 32-bit
- **Any other machine** - the base ELF flag, as glibc's generic code does

Libraries of another architecture than the host's are handled (useful with
`-r <sysroot>`), with one limit: the byte order must match the host's. Both
the ELF reader and the cache writer work in native endianness, like glibc's
own `ldconfig`, so a big-endian sysroot cannot be processed on a
little-endian machine.

All architecture flags match the official [glibc ldconfig implementation](https://sourceware.org/git/?p=glibc.git;a=blob;f=sysdeps/generic/ldconfig.h).

`glibc-hwcaps` subdirectories are scanned and written as cache extension
entries (including the x86-64 ISA level from `GNU_PROPERTY_X86_ISA_1_NEEDED`),
matching glibc 2.33+.

## Command-Line Usage

> **This is a real `ldconfig`.** Run without options it repoints soname
> symlinks in the system library directories and replaces `/etc/ld.so.cache`.
> To try it without touching the system, add `-X` (leave symlinks alone) and
> `-C <file>` (write the cache elsewhere), or work in another root with `-r`.

Options follow glibc ldconfig:

| Flag | Description |
|------|-------------|
| `-p` | Print cache contents |
| `-N` | Don't rebuild the cache (but still update symlinks) |
| `-X` | Don't update symbolic links |
| `-n` | Only process directories given on the command line |
| `-r ROOT` | Change to and use ROOT as root directory |
| `-C CACHE` | Use CACHE as cache file |
| `-f CONF` | Use CONF as configuration file |
| `-c FMT` | Use FMT as cache format (only `new` is supported) |
| `-i` | Ignore auxiliary cache file (not implemented, exits with an error) |
| `-l` | Interpret operands as library names (not implemented, exits with an error) |
| `-v` | Verbose output |

Additional directories can be specified as positional arguments.

**Note:** The `-N` flag semantics changed in 0.2.0 from "dry run (no writes at all)" to
"don't rebuild cache" to match glibc behavior. For a true dry run (no cache write and no
symlink updates), use `-N -X`.

### Print cache contents

```bash
# Print the system cache
ldconfig -p

# Print a specific cache file
ldconfig -p -C /path/to/cache
```

### Build/update cache

```bash
# Update the system cache and symlinks (as root)
ldconfig

# Build the cache of another root (a sysroot, an unpacked image)
ldconfig -r /path/to/sysroot

# Scan the system without changing it, writing the cache elsewhere
ldconfig -X -C /tmp/test.cache
```

`-C` names a path inside the root, so with `-r` the cache file lands under it.

From a checkout, replace `ldconfig` with `cargo run --bin ldconfig --`.

## Library Usage

Add to your `Cargo.toml`:
```toml
[dependencies]
ldconfig = "0.2"
```

### Read and display a cache

```rust
use ldconfig::Cache;

let cache = Cache::from_file("/etc/ld.so.cache")?;

// Display the entire cache (uses Display trait)
println!("{}", cache);

// Or iterate over entries
for entry in cache.entries().take(5) {
    println!("{} => {}", entry.soname, entry.path);
}

// Find specific libraries
for entry in cache.find("libc") {
    println!("Found: {} at {}", entry.soname, entry.path);
}
```

### Build and write a cache

```rust
use ldconfig::{SearchPaths, Cache};
use camino::Utf8Path;

// Parse ld.so.conf and add the built-in system directories, like glibc
let search_paths = SearchPaths::from_file("/etc/ld.so.conf", None)?.with_system();

// Build cache by scanning directories
let cache = Cache::builder()
    .prefix(Utf8Path::new("/"))
    .build(&search_paths)?;

// Write to file
cache.write_to_file("/etc/ld.so.cache")?;
```

Building is not read-only by default: like `ldconfig`, it creates and
repoints the soname symlinks in the scanned directories and removes dangling
ones. Links are replaced atomically, so a library never stops resolving while
its link changes. Use `.update_symlinks(false)` (the `-X` flag) or
`.dry_run(true)` to leave the directories untouched.

`SearchPaths::from_file` returns only the directories the config names:

```rust
use ldconfig::SearchPaths;

// Only config directories
let paths = SearchPaths::from_file("/etc/ld.so.conf", None)?;

// Config directories + system directories (glibc-compatible)
let paths = SearchPaths::from_file("/etc/ld.so.conf", None)?.with_system();

// Only system directories (/usr/lib, /usr/lib64, /lib, /lib64)
let paths = SearchPaths::default();
```

## Examples

The `examples/` directory contains complete working examples:

```bash
# Build a cache in memory for a root (default: /), changing nothing
cargo run --example build_cache -- /path/to/sysroot

# Read and query a cache file (default: /etc/ld.so.cache)
cargo run --example test_cache_read -- test.cache

# Compare two caches (with ld-so-cache cross-validation)
cargo run --example compare_caches -- our.cache reference.cache
```

## API Overview

### `Cache` - Reading and writing caches
```rust
pub struct Cache { ... }

impl Cache {
    // Scan and build; options: update_symlinks, dry_run, prefix
    pub fn builder() -> CacheBuilder;   // ... .build(&search_paths)
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Error>;
    pub fn from_bytes(data: &[u8]) -> Result<Self, Error>;
    pub fn entries(&self) -> CacheEntries<'_>;  // Iterator of CacheEntry
    pub fn find(&self, name: &str) -> impl Iterator<Item = CacheEntry>;  // substring match
    pub fn info(&self) -> CacheInfo;
    pub fn write_to_file(&self, path: impl AsRef<Path>) -> Result<(), Error>;  // atomic
    pub fn as_bytes(&self) -> &[u8];
    pub fn size(&self) -> usize;
}

impl fmt::Display for Cache { ... }
```

### `SearchPaths` - Configuration parsing
```rust
pub struct SearchPaths { ... }

impl SearchPaths {
    pub fn from_file(path: impl AsRef<Utf8Path>, prefix: Option<&Utf8Path>) -> Result<Self, Error>;
    pub fn with_system(self) -> Self;   // append /usr/lib, /usr/lib64, /lib, /lib64
    pub fn new(directories: Vec<Utf8PathBuf>) -> Self;
    // Default: only the system directories

    // Also implements Deref<Target = [Utf8PathBuf]> for transparent slice access
}
```

### `chroot_canon` - path resolution inside a root
```rust
pub fn chroot_canon(root: &Utf8Path, name: &Utf8Path) -> Option<Utf8PathBuf>;
```

## Differences from glibc

The cache contents match glibc's. The behaviour differs in a few places:

- **Symlinks are replaced atomically.** glibc unlinks the old link and then
  creates the new one; here the new link is renamed over the old one, so a
  library never stops resolving during an update.
- **All architectures in one binary.** glibc's `ldconfig` only handles the
  machine it was built for.
- **A relative `include` works under a root.** glibc refuses one when `-r` is
  given; here `include ld.so.conf.d/*.conf` resolves inside the root, so a
  stock configuration can be processed from outside.
- **Only the new cache format** (`glibc-ld.so.cache1.1`, glibc 2.2 and later)
  is read and written.
- **No auxiliary cache**, and no library mode (`-l`).

[`docs/architecture.md`](docs/architecture.md) has the full list and the
reasoning.

## Testing

Unit tests cover config parsing, directory scanning, symlink handling, ELF
flag selection, sorting, and the binary format; run them with `cargo test`.
End-to-end validation builds a cache from the host's libraries, without
changing anything, and compares it with the one glibc wrote:

```bash
cargo run --bin ldconfig -- -X -C /tmp/test.cache
cargo run --example compare_caches -- /tmp/test.cache /etc/ld.so.cache
diff <(cargo run --bin ldconfig -- -p -C /tmp/test.cache) <(/sbin/ldconfig -p)
```

Only the first line, which names the cache file, and the `Cache generated by:`
line should differ.

You may download any minimal docker image sporting glibc or use [chroot-stages](https://github.com/lu-zero/crossdev-stages/blob/master/chroot-stage.sh) to download
a Gentoo stage3

```bash
# Test with AArch64 libraries; -C is a path inside the root
cargo run --bin ldconfig -- -X -r <stage3-arm64> -C /test.cache -v

# Test with RISC-V libraries
cargo run --bin ldconfig -- -X -r <stage3-rv64_lp64d> -C /test.cache -v

# Compare against the cache the stage ships
cargo run --example compare_caches -- <stage3-arm64>/test.cache <stage3-arm64>/etc/ld.so.cache
```

Drop `-X` to also exercise the symlink updates inside the root.

The `compare_caches` example uses the [ld-so-cache](https://crates.io/crates/ld-so-cache) crate for cross-validation to ensure compatibility with existing tools.

## Development

This code was written with the assistance of:
- [Claude](https://claude.ai) - AI assistant by Anthropic
- [mistral-vibe](https://github.com/mistralai/mistral-vibe) - AI assistant by Mistral

The code is manually reviewed and should not contain hallucination on release, but single commits in the history can be nonsensical.

[`AGENTS.md`](AGENTS.md) holds the project conventions (checks to run, coding
style, how to test without touching the host) and
[`docs/architecture.md`](docs/architecture.md) describes the design.

## License

MIT
