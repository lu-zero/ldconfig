/// Example: Compare two cache files
///
/// This example compares two ld.so.cache files to verify compatibility.
/// It cross-validates with the ld-so-cache crate to ensure compatibility.
///
/// Usage: cargo run --example compare_caches <our_cache> <real_cache>

use bpaf::Bpaf;
use ld_so_cache::parsers::parse_ld_cache;
use ldconfig::{Cache, Error};
use std::path::PathBuf;

#[derive(Debug, Clone, Bpaf)]
#[bpaf(options)]
struct Options {
    #[bpaf(positional("our_cache"))]
    /// Our cache file
    our_cache: PathBuf,

    #[bpaf(positional("real_cache"))]
    /// Real cache file
    real_cache: PathBuf,
}

fn compare_caches(our_cache: &Cache, real_cache: &Cache) -> Result<(), Error> {
    let our_info = our_cache.info();
    let real_info = real_cache.info();

    println!("=== Cache Comparison ===");
    println!("Our cache: {} libraries", our_info.num_entries);
    println!("Real cache: {} libraries", real_info.num_entries);

    // Check library count
    if our_info.num_entries == real_info.num_entries {
        println!("✅ Library count matches: {}", our_info.num_entries);
    } else {
        println!(
            "❌ Library count mismatch: {} vs {}",
            our_info.num_entries, real_info.num_entries
        );
    }

    // Check generator
    if our_info.generator.is_some() || real_info.generator.is_some() {
        match (&our_info.generator, &real_info.generator) {
            (Some(our_gen), Some(real_gen)) => {
                println!("Our generator: {}", our_gen);
                println!("Real generator: {}", real_gen);
            }
            (Some(our_gen), None) => {
                println!("Our generator: {}", our_gen);
                println!("Real generator: (none)");
            }
            (None, Some(real_gen)) => {
                println!("Our generator: (none)");
                println!("Real generator: {}", real_gen);
            }
            _ => {}
        }
    }

    // Compare entries
    let our_entries: Vec<_> = our_cache.entries().collect();
    let real_entries: Vec<_> = real_cache.entries().collect();

    let entries_to_compare = std::cmp::min(5, std::cmp::min(our_entries.len(), real_entries.len()));
    println!("\nComparing first {} entries:", entries_to_compare);
    
    for i in 0..entries_to_compare {
        let our_entry = &our_entries[i];
        let real_entry = &real_entries[i];

        if our_entry.soname == real_entry.soname {
            println!("✅ Entry {}: {} (flags match: {})", i, our_entry.soname, our_entry.flags == real_entry.flags);
        } else {
            println!("❌ Entry {} name differs: {} vs {}", i, our_entry.soname, real_entry.soname);
        }

        if our_entry.flags != real_entry.flags {
            println!("   ⚠️  Flags: 0x{:04x} vs 0x{:04x}", our_entry.flags, real_entry.flags);
        }
    }

    Ok(())
}

fn main() -> Result<(), Error> {
    let options = options().run();

    println!("=== Extended Cache Comparison ===");
    println!();

    // Read both cache files using our Cache API
    let our_cache = Cache::from_file(&options.our_cache)?;
    let real_cache = Cache::from_file(&options.real_cache)?;

    println!("--- ldconfig-rs Analysis ---");
    compare_caches(&our_cache, &real_cache)?;

    // Parse using ld-so-cache crate for cross-validation
    let our_data = std::fs::read(&options.our_cache)
        .map_err(|e| Error::CacheRead(format!("Failed to read our cache: {}", e)))?;
    let real_data = std::fs::read(&options.real_cache)
        .map_err(|e| Error::CacheRead(format!("Failed to read real cache: {}", e)))?;

    println!("\n--- ld-so-cache Crate Cross-Validation ---");

    match parse_ld_cache(&our_data) {
        Ok(our_ld_cache) => {
            println!("✅ Our cache parsed successfully with ld-so-cache");
            if let Ok(entries) = our_ld_cache.get_entries() {
                println!("  Libraries: {}", entries.len());

                for (i, entry) in entries.iter().take(3).enumerate() {
                    println!("    {}. {} -> {}", i, entry.library_name, entry.library_path);
                }
            }
        }
        Err(e) => {
            println!("❌ Our cache failed to parse with ld-so-cache: {}", e);
        }
    }

    match parse_ld_cache(&real_data) {
        Ok(real_ld_cache) => {
            println!("✅ Real cache parsed successfully with ld-so-cache");
            if let Ok(entries) = real_ld_cache.get_entries() {
                println!("  Libraries: {}", entries.len());

                for (i, entry) in entries.iter().take(3).enumerate() {
                    println!("    {}. {} -> {}", i, entry.library_name, entry.library_path);
                }
            }
        }
        Err(e) => {
            println!("❌ Real cache failed to parse with ld-so-cache: {}", e);
        }
    }

    // Cross-validate entry counts
    if let (Ok(our_ld_cache), Ok(real_ld_cache)) =
        (parse_ld_cache(&our_data), parse_ld_cache(&real_data)) {
        if let (Ok(our_entries), Ok(real_entries)) =
            (our_ld_cache.get_entries(), real_ld_cache.get_entries()) {

            println!("\n--- Entry Count Validation ---");
            if our_entries.len() == real_entries.len() {
                println!("✅ ld-so-cache library counts match: {}", our_entries.len());
            } else {
                println!(
                    "❌ ld-so-cache library counts differ: {} vs {}",
                    our_entries.len(),
                    real_entries.len()
                );
            }
        }
    }

    Ok(())
}
