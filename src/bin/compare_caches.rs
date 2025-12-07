use bpaf::Bpaf;
use ldconfig::{parse_cache_data, LdconfigError};
use std::path::PathBuf;

#[cfg(feature = "ld-so-cache")]
use ld_so_cache::parsers::parse_ld_cache;

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

fn compare_caches(our_cache: &[u8], real_cache: &[u8]) -> Result<(), LdconfigError> {
    let our_info = parse_cache_data(our_cache)?;
    let real_info = parse_cache_data(real_cache)?;

    println!("=== Cache Comparison ===");
    println!(
        "Our cache: {} libraries, {} string bytes",
        our_info.header.nlibs, our_info.header.len_strings
    );
    println!(
        "Real cache: {} libraries, {} string bytes",
        real_info.header.nlibs, real_info.header.len_strings
    );

    // Check magic
    if our_info.header.magic != real_info.header.magic {
        println!(
            "❌ Magic mismatch: {} vs {}",
            our_info.header.magic, real_info.header.magic
        );
    } else {
        println!("✅ Magic matches: {}", our_info.header.magic);
    }

    // Check header structure
    if our_info.header.nlibs == real_info.header.nlibs {
        println!("✅ Library count matches: {}", our_info.header.nlibs);
    } else {
        println!(
            "❌ Library count mismatch: {} vs {}",
            our_info.header.nlibs, real_info.header.nlibs
        );
    }

    // Compare a few entries
    let entries_to_compare = std::cmp::min(5, our_info.entries.len());
    for i in 0..entries_to_compare {
        if i < real_info.entries.len() {
            let our_entry = &our_info.entries[i];
            let real_entry = &real_info.entries[i];

            if our_entry.flags == real_entry.flags {
                println!("✅ Entry {} flags match: {}", i, our_entry.flags);
            } else {
                println!(
                    "❌ Entry {} flags mismatch: {} vs {}",
                    i, our_entry.flags, real_entry.flags
                );
            }
        }
    }

    Ok(())
}

fn main() -> Result<(), LdconfigError> {
    let options = options().run();

    // Read both cache files
    let our_data = std::fs::read(&options.our_cache)
        .map_err(|e| LdconfigError::CacheWrite(format!("Failed to read our cache: {}", e)))?;

    let real_data = std::fs::read(&options.real_cache)
        .map_err(|e| LdconfigError::CacheWrite(format!("Failed to read real cache: {}", e)))?;

    println!("=== Extended Cache Comparison ===");
    println!("Using both our implementation and ld-so-cache crate");
    println!();

    // Local implementation
    #[cfg(not(feature = "ld-so-cache"))]
    {
        compare_caches(&our_data, &real_data)?;
    }

    // Parse using ld-so-cache crate
    #[cfg(feature = "ld-so-cache")]
    {
        println!("--- ld-so-cache Crate Analysis ---");

        match parse_ld_cache(&our_data) {
            Ok(our_cache) => {
                println!("✅ Our cache parsed successfully with ld-so-cache");
                if let Ok(entries) = our_cache.get_entries() {
                    println!("  Libraries: {}", entries.len());

                    // Show first few entries
                    for (i, entry) in entries.iter().enumerate() {
                        println!(
                            "    {}. {} -> {}",
                            i, entry.library_name, entry.library_path
                        );
                    }
                }
            }
            Err(e) => {
                println!("❌ Our cache failed to parse with ld-so-cache: {}", e);
            }
        }

        match parse_ld_cache(&real_data) {
            Ok(real_cache) => {
                println!("✅ Real cache parsed successfully with ld-so-cache");
                if let Ok(entries) = real_cache.get_entries() {
                    println!("  Libraries: {}", entries.len());

                    // Show first few entries
                    for (i, entry) in entries.iter().enumerate() {
                        println!(
                            "    {}. {} -> {}",
                            i, entry.library_name, entry.library_path
                        );
                    }
                }
            }
            Err(e) => {
                println!("❌ Real cache failed to parse with ld-so-cache: {}", e);
            }
        }

        println!();
        println!("--- Cross-Validation ---");

        // Try to parse both with both implementations
        let our_parsed = parse_ld_cache(&our_data);
        let real_parsed = parse_ld_cache(&real_data);

        match (our_parsed, real_parsed) {
            (Ok(our_cache), Ok(real_cache)) => {
                let our_entries = our_cache.get_entries().unwrap_or_default();
                let real_entries = real_cache.get_entries().unwrap_or_default();

                if our_entries.len() == real_entries.len() {
                    println!("✅ Library counts match: {}", our_entries.len());
                } else {
                    println!(
                        "❌ Library counts differ: {} vs {}",
                        our_entries.len(),
                        real_entries.len()
                    );
                }

                // Compare a few entries
                for (i, (our_entry, real_entry)) in
                    our_entries.iter().zip(real_entries.iter()).enumerate()
                {
                    if our_entry.library_name == real_entry.library_name {
                        println!("✅ Library {} name matches: {}", i, our_entry.library_name);
                    } else {
                        println!(
                            "❌ Library {} name differs: {} vs {}",
                            i, our_entry.library_name, real_entry.library_name
                        );
                    }
                }
            }
            (Err(e), _) => {
                println!("❌ Our cache failed cross-validation: {}", e);
            }
            (_, Err(e)) => {
                println!("❌ Real cache failed cross-validation: {}", e);
            }
        }
    }

    Ok(())
}
