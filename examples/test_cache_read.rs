/// Example: Reading and displaying cache file contents
///
/// This example shows how to use the unified Cache API to read and display
/// the contents of an ld.so.cache file.
///
/// Usage: cargo run --example test_cache_read
use ldconfig::Cache;

fn main() {
    // Read and display our generated cache
    println!("=== Generated Cache ===");
    match Cache::from_file("ld.so.cache") {
        Ok(cache) => {
            let info = cache.info();
            println!("Number of entries: {}", info.num_entries);
            if let Some(ref gen) = info.generator {
                println!("Generator: {}", gen);
            }

            println!("\nFirst 5 entries:");
            for entry in cache.entries().take(5) {
                println!("  {} ({}) => {}", entry.soname, entry.arch, entry.path);
                if entry.hwcap != 0 {
                    println!("    hwcap: 0x{:016x}", entry.hwcap);
                }
            }

            // Find specific libraries (using iterator)
            println!("\nSearching for 'libc':");
            for entry in cache.find("libc") {
                println!("  {} => {}", entry.soname, entry.path);
            }
        }
        Err(e) => {
            println!("Failed to read generated cache: {}", e);
        }
    }

    // Read and display the real system cache
    println!("\n\n=== System Cache ===");
    match Cache::from_file("test-root/etc/ld.so.cache") {
        Ok(cache) => {
            // Use Display trait for full output
            print!("{}", cache);
        }
        Err(e) => {
            println!("Failed to read system cache: {}", e);
        }
    }
}
