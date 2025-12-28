# ldconfig - Portable Rust Implementation

A Rust implementation of ldconfig functionality for building glibc `ld.so.cache`.

## Supported Architectures

This implementation supports the following architectures with proper glibc cache flags:

- **x86-64** (64-bit Intel/AMD) - `FLAG_X8664_LIB64`
- **AArch64** (ARM 64-bit) - `FLAG_AARCH64_LIB64`
- **RISC-V 64-bit** (lp64d ABI with double-precision FP) - `FLAG_RISCV_FLOAT_ABI_DOUBLE`
- **PowerPC 64-bit** - `FLAG_POWERPC_LIB64`
- **i686** (32-bit x86) - Base ELF flag
- **ARM** (32-bit) - `FLAG_ARM_LIBHF` for hard-float, base flag for soft-float

All architecture flags match the official [glibc ldconfig implementation](https://sourceware.org/git/?p=glibc.git;a=blob;f=sysdeps/generic/ldconfig.h).

## Usage

### Basic ldconfig

```bash
cargo run --bin ldconfig
```

### Testing
Currently there is a comparative test implemented by the `compare_caches` command.

There is a minimalist implementation of the cache reading, but to be sure it works it is possible to leverage the more complete
[ld-so-cache](https://crates.io/crates/ld-so-cache) and have possibly better validation.

```bash
# Without ld-so-cache analysis
cargo run --bin compare_caches our_cache.bin real_cache.bin

# With ld-so-cache analysis
cargo run --bin compare_caches --features ld-so-cache our_cache.bin real_cache.bin
```

## License

MIT
