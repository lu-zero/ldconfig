# ldconfig - Portable Rust Implementation

A Rust implementation of ldconfig functionality for building glibc `ld.so.cache`.

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
