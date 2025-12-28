use bpaf::Bpaf;
use camino::Utf8PathBuf;
use ldconfig::{CacheBuilder, CacheReader, Error, LibraryConfig, ScanOptions};

#[derive(Debug, Clone, Bpaf)]
#[bpaf(options)]
struct Options {
    #[bpaf(short, long)]
    /// Verbose output
    verbose: bool,

    #[bpaf(short('N'), long)]
    /// Dry run - don't make changes
    dry_run: bool,

    #[bpaf(short, long)]
    /// Print cache contents
    print_cache: bool,

    #[bpaf(short('r'), long, argument("PREFIX"), fallback("/".into()))]
    /// Use alternative root prefix (like chroot)
    prefix: Utf8PathBuf,

    #[bpaf(short('C'), long, argument("CACHE"))]
    /// Use cache file path
    cache: Option<Utf8PathBuf>,

    #[bpaf(short('f'), long("config"), argument("CONFIG"))]
    /// Use alternative config file
    config_file: Option<Utf8PathBuf>,
}

fn main() -> Result<(), Error> {
    let options = options().run();

    // Handle print-cache flag
    if options.print_cache {
        return print_cache(&options);
    }

    if options.verbose {
        println!("Using prefix: {}", options.prefix);
    }

    // Determine config file path
    let config_path = options
        .config_file
        .clone()
        .unwrap_or_else(|| options.prefix.join("etc/ld.so.conf"));

    if options.verbose {
        println!("Config file: {}", config_path);
    }

    // Load configuration with prefix handling
    let config = if config_path.exists() {
        if options.verbose {
            println!("Loading configuration from: {}", config_path);
        }
        LibraryConfig::from_file(&config_path, Some(options.prefix.as_path()))?
    } else {
        if options.verbose {
            println!("No config file found, using default configuration");
        }
        // Apply prefix to default directories
        let default = LibraryConfig::default();
        let prefixed_dirs: Vec<_> = default
            .directories()
            .iter()
            .map(|dir| options.prefix.join(dir.strip_prefix("/").unwrap_or(dir)))
            .collect();
        LibraryConfig::from_directories(prefixed_dirs)
    };

    if options.verbose {
        println!("Directories to scan: {:?}", config.directories());
    }

    // Build cache using high-level API
    let scan_options = ScanOptions {
        update_symlinks: !options.dry_run,
        dry_run: options.dry_run,
        verbose: options.verbose,
    };

    let mut builder = CacheBuilder::new();
    if options.prefix.as_str() != "/" {
        builder = builder.with_prefix(options.prefix.clone());
    }

    builder.scan_directories(&config, &scan_options)?;

    let library_count = builder.library_count();

    if options.verbose {
        println!("Found {} libraries", library_count);
    }

    let cache = builder.build()?;

    if options.verbose {
        println!("Built cache with {} bytes", cache.size());
    }

    if !options.dry_run {
        // Determine cache file path
        let cache_path = options
            .cache
            .unwrap_or_else(|| options.prefix.join("etc/ld.so.cache"));

        cache.write_to_file(&cache_path)?;

        if options.verbose {
            println!("Wrote {} bytes to {}", cache.size(), cache_path);
        }
    } else if options.verbose {
        println!("Dry run: would write {} libraries to cache", library_count);
    }

    Ok(())
}

fn print_cache(options: &Options) -> Result<(), Error> {
    // Determine cache file path
    let cache_path = options
        .cache
        .clone()
        .unwrap_or_else(|| options.prefix.join("etc/ld.so.cache"));

    // Read cache using high-level API
    let reader = CacheReader::from_file(&cache_path)?;

    // Print using built-in formatting
    reader.print(&mut std::io::stdout())?;

    Ok(())
}
