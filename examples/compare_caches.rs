/// Example: Compare two ld.so.cache files entry by entry.
///
/// Cross-validates both files with the ld-so-cache crate.
///
/// Usage: cargo run --example compare_caches <our_cache> <real_cache>
use anyhow::Error;
use bpaf::Bpaf;
use ld_so_cache::parsers::parse_ld_cache;
use ldconfig::Cache;
use std::collections::BTreeSet;
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

fn entry_set(cache: &Cache) -> BTreeSet<(String, String, u32, u64)> {
    cache
        .entries()
        .map(|e| (e.soname, e.path, e.flags, e.hwcap))
        .collect()
}

fn main() -> Result<(), Error> {
    let options = options().run();

    let ours = Cache::from_file(&options.our_cache)?;
    let real = Cache::from_file(&options.real_cache)?;

    println!(
        "ours: {} entries, generator {:?}",
        ours.info().num_entries,
        ours.info().generator
    );
    println!(
        "real: {} entries, generator {:?}",
        real.info().num_entries,
        real.info().generator
    );

    let our_set = entry_set(&ours);
    let real_set = entry_set(&real);
    let missing: Vec<_> = real_set.difference(&our_set).collect();
    let extra: Vec<_> = our_set.difference(&real_set).collect();

    println!(
        "missing from ours: {}, extra in ours: {}",
        missing.len(),
        extra.len()
    );
    for (soname, path, flags, hwcap) in missing.iter().take(20) {
        println!(
            "  missing: {} => {} (flags {:#06x}, hwcap {:#x})",
            soname, path, flags, hwcap
        );
    }
    for (soname, path, flags, hwcap) in extra.iter().take(20) {
        println!(
            "  extra:   {} => {} (flags {:#06x}, hwcap {:#x})",
            soname, path, flags, hwcap
        );
    }

    // Order comparison: first divergence in the sorted entry list.
    let ours_ordered: Vec<_> = ours.entries().map(|e| (e.soname, e.path)).collect();
    let real_ordered: Vec<_> = real.entries().map(|e| (e.soname, e.path)).collect();
    match ours_ordered
        .iter()
        .zip(real_ordered.iter())
        .position(|(a, b)| a != b)
    {
        Some(i) => println!(
            "first order divergence at {}: ours {:?} vs real {:?}",
            i, ours_ordered[i], real_ordered[i]
        ),
        None if ours_ordered.len() == real_ordered.len() => {
            println!("entry order identical");
        }
        None => println!("entry lists are a prefix of each other"),
    }

    // Cross-validation with the ld-so-cache crate.
    for (label, path) in [("ours", &options.our_cache), ("real", &options.real_cache)] {
        let data = std::fs::read(path)?;
        match parse_ld_cache(&data) {
            Ok(cache) => {
                let n = cache.get_entries().map(|e| e.len()).unwrap_or(0);
                println!("ld-so-cache parses {}: {} entries", label, n);
            }
            Err(e) => println!("ld-so-cache fails to parse {}: {}", label, e),
        }
    }

    Ok(())
}
