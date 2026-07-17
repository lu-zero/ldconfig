/// Example: Reading and displaying cache file contents
///
/// Usage: cargo run --example test_cache_read [cache_path]
use ldconfig::Cache;
use std::env;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/etc/ld.so.cache".to_string());

    match Cache::from_file(&path) {
        Ok(cache) => {
            let info = cache.info();
            println!("Number of entries: {}", info.num_entries);
            if let Some(generator) = &info.generator {
                println!("Generator: {}", generator);
            }

            println!("\nFirst 5 entries:");
            for entry in cache.entries().take(5) {
                println!("{}", entry);
            }

            println!("\nSearching for 'libc':");
            for entry in cache.find("libc") {
                println!("  {} => {}", entry.soname, entry.path);
            }
        }
        Err(e) => {
            println!("Failed to read {}: {}", path, e);
        }
    }
}
