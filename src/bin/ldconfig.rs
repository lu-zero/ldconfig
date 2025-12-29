use bpaf::Bpaf;
use camino::Utf8PathBuf;
use ldconfig::{Cache, Error, SearchPaths};
use tracing::{debug, info, warn, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

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

/// Initialize the tracing subscriber with appropriate configuration
///
/// # Arguments
///
/// * `verbose` - If true, sets log level to DEBUG, otherwise INFO
/// * `with_target` - If true, includes target information in log output
pub fn init_logging(verbose: bool) {
    let filter_level = if verbose { Level::DEBUG } else { Level::INFO };

    // Set up environment filter - allow overriding via RUST_LOG env var
    let env_filter = EnvFilter::builder()
        .with_default_directive(filter_level.into())
        .from_env_lossy();

    // Configure the subscriber format
    let fmt_layer = fmt::layer()
        .with_level(verbose)
        .with_target(verbose)
        .with_line_number(verbose)
        .without_time()
        .compact();

    // Initialize the subscriber
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    debug!("Logging initialized with level: {}", filter_level);
}

fn main() -> Result<(), Error> {
    let options = options().run();

    // Initialize logging system
    init_logging(options.verbose);

    // Handle print-cache flag
    if options.print_cache {
        return print_cache(&options);
    }

    debug!("Using prefix: {}", options.prefix);

    // Determine config file path
    let config_path = options
        .config_file
        .clone()
        .unwrap_or_else(|| options.prefix.join("etc/ld.so.conf"));

    debug!("Config file: {}", config_path);

    // Load configuration with prefix handling
    let search_paths = if config_path.exists() {
        info!("Loading configuration from: {}", config_path);
        SearchPaths::from_file(&config_path, Some(options.prefix.as_path()))?
    } else {
        warn!("No config file found, using default configuration");
        // Apply prefix to default directories
        let default = SearchPaths::default();
        let prefixed_dirs: Vec<_> = default
            .iter()
            .map(|dir| options.prefix.join(dir.strip_prefix("/").unwrap_or(dir)))
            .collect();
        SearchPaths::new(prefixed_dirs)
    };

    debug!("Directories to scan: {:?}", &*search_paths);

    let cache = Cache::builder()
        .prefix(options.prefix.as_path())
        .dry_run(options.dry_run)
        .build(&search_paths)?;

    info!("Built cache with {} bytes", cache.size());

    if !options.dry_run {
        // Determine cache file path
        let cache_path = options
            .cache
            .unwrap_or_else(|| options.prefix.join("etc/ld.so.cache"));

        cache.write_to_file(&cache_path)?;

        info!("Wrote {} bytes to {}", cache.size(), cache_path);
    }

    Ok(())
}

fn print_cache(options: &Options) -> Result<(), Error> {
    // Determine cache file path
    let cache_path = options
        .cache
        .clone()
        .unwrap_or_else(|| options.prefix.join("etc/ld.so.cache"));

    // Read cache using unified Cache API
    let cache = Cache::from_file(&cache_path)?;

    // Print using Display trait
    println!("{}", cache);

    Ok(())
}
