/// Example: Building a library cache programmatically
///
/// Loads ld.so.conf from inside the given root, scans the configured
/// directories, and builds a cache in memory without touching anything.
///
/// Usage: cargo run --example build_cache -- <root>
use camino::{Utf8Path, Utf8PathBuf};
use ldconfig::{Cache, Error, SearchPaths};
use std::env;

fn main() -> Result<(), Error> {
    let args: Vec<String> = env::args().collect();
    let root = if args.len() > 1 {
        Utf8PathBuf::from(&args[1])
    } else {
        Utf8PathBuf::from("/")
    };

    println!("Building cache for root: {}", root);

    let prefix = (root != "/").then_some(root.as_path());
    let search_paths = SearchPaths::from_file(Utf8Path::new("/etc/ld.so.conf"), prefix)?.with_system();

    println!("Directories to scan: {:?}", &*search_paths);

    let cache = Cache::builder()
        .update_symlinks(false)
        .prefix(root.as_path())
        .build(&search_paths)?;

    println!(
        "Cache: {} entries, {} bytes",
        cache.info().num_entries,
        cache.size()
    );

    Ok(())
}
