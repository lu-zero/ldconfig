# ldconfig - Portable Rust Implementation

A Rust implementation of ldconfig for building and managing glibc `ld.so.cache` files.

This library provides both a command-line tool and a high-level API for:
- Reading and exploring `ld.so.cache` files
- Parsing `ld.so.conf` configuration files
- Building cache files by scanning library directories
- Writing cache files to disk

## Supported Architectures

This implementation supports the following architectures with proper glibc cache flags:

- **x86-64** (64-bit Intel/AMD) - `FLAG_X8664_LIB64`
- **AArch64** (ARM 64-bit) - `FLAG_AARCH64_LIB64`
- **RISC-V 64-bit** (lp64d ABI with double-precision FP) - `FLAG_RISCV_FLOAT_ABI_DOUBLE`
- **PowerPC 64-bit** - `FLAG_POWERPC_LIB64`
- **i686** (32-bit x86) - Base ELF flag
- **ARM** (32-bit) - `FLAG_ARM_LIBHF` for hard-float, base flag for soft-float

All architecture flags match the official [glibc ldconfig implementation](https://sourceware.org/git/?p=glibc.git;a=blob;f=sysdeps/generic/ldconfig.h).

## Command-Line Usage

### Print cache contents

```bash
# Print the system cache
cargo run --bin ldconfig -- -p

# Print a specific cache file
cargo run --bin ldconfig -- -p -C /path/to/cache
```

### Build/update cache

```bash
# Update system cache
cargo run --bin ldconfig

# Build cache for a specific root (useful for cross-compilation)
cargo run --bin ldconfig -- -r /path/to/sysroot

# Write to a different cache file (don't overwrite system cache)
cargo run --bin ldconfig -- -r test-root -C test.cache
```

## Library Usage

Add to your `Cargo.toml`:
```toml
[dependencies]
ldconfig = "0.1"
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

// Parse ld.so.conf
let search_paths = SearchPaths::from_file("/etc/ld.so.conf", None)?;

// Build cache by scanning directories
let cache = Cache::builder()
    .prefix(Utf8Path::new("/"))
    .build(&search_paths)?;

// Write to file
cache.write_to_file("/etc/ld.so.cache")?;
```

## Examples

The `examples/` directory contains complete working examples:

```bash
# Build a cache from a sysroot
cargo run --example build_cache -- -r test-root -C test.cache

# Read and query a cache file
cargo run --example test_cache_read -- test.cache

# Compare two caches (with ld-so-cache cross-validation)
cargo run --example compare_caches -- our.cache reference.cache
```

## API Overview

### `Cache` - Reading and writing caches
```rust
pub struct Cache { ... }

impl Cache {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Error>;
    pub fn from_bytes(data: &[u8]) -> Result<Self, Error>;
    pub fn entries(&self) -> CacheEntries<'_>;  // Iterator
    pub fn find(&self, name: &str) -> impl Iterator<Item = CacheEntry>;
    pub fn info(&self) -> CacheInfo;
    pub fn write_to_file(&self, path: impl AsRef<Path>) -> Result<(), Error>;
}

impl fmt::Display for Cache { ... }
```

### `SearchPaths` - Configuration parsing
```rust
pub struct SearchPaths { ... }

impl SearchPaths {
    pub fn from_file(path: impl AsRef<Utf8Path>, prefix: Option<&Utf8Path>) -> Result<Self, Error>;
    pub fn new(directories: Vec<Utf8PathBuf>) -> Self;

    // Also implements Deref<Target = [Utf8PathBuf]> for transparent slice access
}
```

## Testing

The library includes comprehensive testing through examples since it is fairly cumbersome to automate.

You may download any minimal docker image sporting glibc or use [chroot-stages](https://github.com/lu-zero/crossdev-stages/blob/master/chroot-stage.sh) to download
a Gentoo stage3

```bash
# Test with AArch64 libraries
cargo run --bin ldconfig -- -r <stage3-arm64> -C test-aarch64.cache -v

# Test with RISC-V libraries
cargo run --bin ldconfig -- -r <stage3-rv64_lp64d> -C test-riscv.cache -v

# Compare against reference implementation
cargo run --example compare_caches -- test-aarch64.cache <stage3-arm64>/etc/ld.so.cache
```

The `compare_caches` example uses the [ld-so-cache](https://crates.io/crates/ld-so-cache) crate for cross-validation to ensure compatibility with existing tools.

## Development

This code was written with the assistance of:
- [Claude](https://claude.ai) - AI assistant by Anthropic
- [mistral-vibe](https://github.com/mistralai/mistral-vibe) - AI assistant by Mistral

The code is manually reviewed and should not contain hallucination on release, but single commits in the history can be nonsensical.

## License

MIT
