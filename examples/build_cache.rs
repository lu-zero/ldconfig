/// Example: Building a library cache programmatically
///
/// This example shows how to use the ldconfig library's high-level API to:
/// 1. Load configuration from ld.so.conf
/// 2. Scan directories for libraries
/// 3. Build a cache file
///
/// Usage: cargo run --example build_cache -- <prefix>
use camino::Utf8PathBuf;
use ldconfig::{Cache, Error, SearchPaths};
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
    let search_paths = if config_path.exists() {
        println!("Loading config from: {}", config_path);
        SearchPaths::from_file(&config_path, Some(prefix.as_path()))?
    } else {
        println!("No config file found, using default directories");
        // Manually apply prefix to default directories
        let default = SearchPaths::default();
        let prefixed_dirs: Vec<_> = default
            .iter()
            .map(|dir| prefix.join(dir.strip_prefix("/").unwrap_or(dir)))
            .collect();
        SearchPaths::new(prefixed_dirs)
    };

    println!("Directories to scan: {:?}", &*search_paths);

    let cache = Cache::builder()
        .update_symlinks(false)
        .dry_run(true)
        .prefix(prefix.as_path())
        .build(&search_paths)?;

    println!("Cache size: {} bytes", cache.size());

    Ok(())
}
