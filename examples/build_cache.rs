/// Example: Building a library cache programmatically
///
/// This example shows how to use the ldconfig library's high-level API to:
/// 1. Load configuration from ld.so.conf
/// 2. Scan directories for libraries
/// 3. Build a cache file
///
/// Usage: cargo run --example build_cache -- <prefix>
use camino::Utf8PathBuf;
use ldconfig::{CacheBuilder, Error, LibraryConfig, ScanOptions};
use std::env;

fn main() -> Result<(), Error> {
    let args: Vec<String> = env::args().collect();
    let prefix = if args.len() > 1 {
        Utf8PathBuf::from(&args[1])
    } else {
        Utf8PathBuf::from("/")
    };

    println!("Building cache for prefix: {}", prefix);

    // Step 1: Load configuration with automatic prefix handling
    let config_path = prefix.join("etc/ld.so.conf");
    let config = if config_path.exists() {
        println!("Loading config from: {}", config_path);
        LibraryConfig::from_file(&config_path, Some(prefix.as_path()))?
    } else {
        println!("No config file found, using default directories");
        // Manually apply prefix to default directories
        let default = LibraryConfig::default();
        let prefixed_dirs: Vec<_> = default
            .directories()
            .iter()
            .map(|dir| prefix.join(dir.strip_prefix("/").unwrap_or(dir)))
            .collect();
        LibraryConfig::from_directories(prefixed_dirs)
    };

    println!("Directories to scan: {:?}", config.directories());

    // Step 2: Build cache using high-level API
    let scan_options = ScanOptions {
        update_symlinks: false, // Dry run for example
        dry_run: true,
        verbose: true,
    };

    let mut builder = CacheBuilder::new();
    if prefix.as_str() != "/" {
        builder = builder.with_prefix(prefix.clone());
    }

    builder.scan_directories(&config, &scan_options)?;

    println!("Cache will contain {} unique libraries", builder.library_count());

    let cache = builder.build()?;
    println!("Cache size: {} bytes", cache.size());

    Ok(())
}
