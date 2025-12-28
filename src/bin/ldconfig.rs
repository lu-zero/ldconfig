use bpaf::Bpaf;
use camino::Utf8PathBuf;
use ldconfig::{
    build_cache, deduplicate_libraries, deduplicate_scan_directories, expand_includes,
    parse_cache_data, parse_config_file, parse_elf_file, scan_all_libraries,
    should_include_symlink, update_symlinks, Config, Error,
};

#[derive(Debug, Clone, Bpaf)]
#[bpaf(options)]
struct Options {
    #[bpaf(short, long)]
    /// Verbose output
    verbose: bool,

    #[bpaf(short, long)]
    /// Dry run - don't make changes
    dry_run: bool,

    #[bpaf(short, long)]
    /// Print cache contents
    print_cache: bool,

    #[bpaf(short, long, argument("PREFIX"), fallback("/".into()))]
    /// Use alternative root prefix (like chroot)
    prefix: Utf8PathBuf,

    #[bpaf(short('C'), long, argument("CACHE"))]
    /// Use cache file path
    cache: Option<Utf8PathBuf>,

    #[bpaf(short('c'), long, argument("CONFIG"))]
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
    let config_path = if let Some(custom_config) = &options.config_file {
        if options.verbose {
            println!("Using custom config file: {}", custom_config);
        }
        custom_config.clone()
    } else {
        let default_config_path = options.prefix.join("etc/ld.so.conf");
        if options.verbose {
            println!("Looking for config file at: {}", default_config_path);
        }
        default_config_path
    };

    // Load configuration
    let mut config = if config_path.exists() {
        if options.verbose {
            println!("Loading configuration from: {}", config_path);
        }
        parse_config_file(&config_path)?
    } else {
        if options.verbose {
            println!("No config file found, using default configuration");
        }
        Config::default()
    };

    // Apply prefix to all directories in config
    config.directories = config
        .directories
        .into_iter()
        .map(|path: Utf8PathBuf| {
            if path.is_absolute() {
                options.prefix.join(path.strip_prefix("/").unwrap_or(&path))
            } else {
                options.prefix.join(path)
            }
        })
        .collect();

    if options.verbose {
        println!(
            "Directories after prefix application: {:?}",
            config.directories
        );
    }

    // Handle include patterns
    if !config.include_patterns.is_empty() {
        if options.verbose {
            println!("Include patterns: {:?}", config.include_patterns);
        }

        // Resolve include patterns relative to config file directory
        let config_dir = config_path.parent().unwrap_or_else(|| "/etc".as_ref());

        let mut temp_config = config.clone();
        temp_config.include_patterns = temp_config
            .include_patterns
            .into_iter()
            .map(|pattern| {
                // Relative patterns are relative to config file directory
                if pattern.starts_with("/") {
                    // Absolute pattern - just apply prefix
                    options
                        .prefix
                        .join(pattern.strip_prefix("/").unwrap_or(&pattern))
                        .to_string()
                } else {
                    // Relative pattern - resolve relative to config directory
                    config_dir.join(pattern).to_string()
                }
            })
            .collect();

        if options.verbose {
            println!(
                "Prefixed include patterns: {:?}",
                temp_config.include_patterns
            );
        }

        // Expand includes
        let expanded_dirs = expand_includes(&temp_config)?;
        if options.verbose {
            println!("Expanded dirs: {:?}", expanded_dirs);
        }

        // ADD expanded directories to existing ones (don't replace!)
        let additional_dirs: Vec<Utf8PathBuf> = expanded_dirs
            .into_iter()
            .map(|path: Utf8PathBuf| {
                if path.is_absolute() {
                    options.prefix.join(path.strip_prefix("/").unwrap_or(&path))
                } else {
                    options.prefix.join(path)
                }
            })
            .collect();

        config.directories.extend(additional_dirs);

        if options.verbose {
            println!(
                "Directories after include expansion: {:?}",
                config.directories
            );
        }
    }

    // Deduplicate directories: skip directories that are symlinks to other dirs already in the list
    let scan_dirs = deduplicate_scan_directories(&config.directories);

    if options.verbose {
        println!("Scanning directories: {:?}", scan_dirs);
    }

    // STEP 1: Single scan - collect all real files and symlinks
    let (real_files, existing_symlinks) = scan_all_libraries(&scan_dirs)?;

    if options.verbose {
        println!("Found {} real files, {} existing symlinks", real_files.len(), existing_symlinks.len());
    }

    // STEP 2: Update symlinks from real files
    let mut new_symlink_actions = Vec::new();
    if !options.dry_run {
        for dir in &scan_dirs {
            if let Ok(actions) = update_symlinks(dir.as_std_path(), &real_files, options.dry_run) {
                if options.verbose && !actions.is_empty() {
                    println!("Symlink actions in {}:", dir);
                    for action in &actions {
                        println!("  {} -> {}", action.link, action.target);
                    }
                }
                new_symlink_actions.extend(actions);
            }
        }
    }

    // STEP 3: Build cache entries from real files + symlinks
    let mut cache_entries = Vec::new();

    // Add real files where filename == SONAME
    for lib in &real_files {
        let filename = lib.path.file_name().unwrap_or("");
        if filename == lib.soname {
            cache_entries.push(lib.clone());
        }
    }

    // Add existing symlinks (with filtering)
    for lib in &existing_symlinks {
        let filename = lib.path.file_name().unwrap_or("");
        if should_include_symlink(filename, &lib.soname, &lib.path) {
            cache_entries.push(lib.clone());
        }
    }

    // Add newly created symlinks
    for action in &new_symlink_actions {
        if let Ok(lib) = parse_elf_file(action.link.as_std_path()) {
            cache_entries.push(lib);
        }
    }

    // Deduplicate by (directory, filename)
    let unique_libraries = deduplicate_libraries(&cache_entries);

    if options.verbose {
        println!("Cache entries: {} unique libraries", unique_libraries.len());
    }

    // Build cache with prefix for path stripping
    let cache = build_cache(&unique_libraries, Some(options.prefix.as_path()));

    if options.verbose {
        println!("Built cache with {} bytes", cache.len());
        println!("Cache magic: {}", String::from_utf8_lossy(&cache[..20]));
    }

    if !options.dry_run {
        // Determine cache file path
        let cache_path = options.cache.map_or_else(
            || options.prefix.join("etc/ld.so.cache"),
            |c| c.to_path_buf(),
        );

        // Ensure parent directory exists
        if let Some(parent) = cache_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write cache file
        let mut file = std::fs::File::create(&cache_path)?;
        use std::io::Write;
        file.write_all(&cache)?;
        file.flush()?;
        file.sync_all()?;

        if options.verbose {
            println!("Wrote {} bytes to {}", cache.len(), cache_path);
        }
    } else if options.verbose {
        println!("Dry run: would write {} entries to cache", unique_libraries.len());
    }

    Ok(())
}

fn print_cache(options: &Options) -> Result<(), Error> {
    // Determine cache file path
    let cache_path = options.cache.clone().unwrap_or_else(|| {
        options.prefix.join("etc/ld.so.cache")
    });

    // Read and parse cache
    let data = std::fs::read(&cache_path)
        .map_err(|e| Error::CacheWrite(format!("Failed to read cache: {}", e)))?;

    let cache_info = parse_cache_data(&data)?;

    // Print header
    println!(
        "{} libs found in cache `{}'",
        cache_info.entries.len(),
        cache_path
    );

    // Helper function to extract null-terminated string from absolute file offset
    let extract_string = |offset: u32| -> Result<String, Error> {
        let start = offset as usize;
        if start >= data.len() {
            return Err(Error::CacheRead(format!(
                "Invalid offset: {}",
                offset
            )));
        }

        let slice = &data[start..];
        let null_pos = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());

        String::from_utf8(slice[..null_pos].to_vec()).map_err(|_| {
            Error::CacheRead("Invalid UTF-8 in string".to_string())
        })
    };

    // Helper function to decode architecture from flags
    let decode_arch = |flags: u32| -> &'static str {
        let arch_bits = (flags >> 8) & 0xf;
        match arch_bits {
            0 => "libc6",           // i386
            3 => "libc6,x86-64",    // x86_64
            8 => "libc6,x32",       // x32
            10 => "libc6,AArch64",  // aarch64
            5 => "libc6,riscv64",   // riscv64
            4 => "libc6,64bit",     // ppc64
            6 => "libc6,IA-64",     // ia64
            9 => "libc6,ARM,hard-float", // arm hf
            _ => "unknown",
        }
    };

    // Print each entry
    for entry in &cache_info.entries {
        let libname = extract_string(entry.key_offset)?;
        let libpath = extract_string(entry.value_offset)?;
        let arch_str = decode_arch(entry.flags as u32);

        // Format: "    libname (arch) => /path/to/lib"
        print!("\t{} ({})", libname, arch_str);

        // Add hwcap info if present
        if entry.hwcap != 0 {
            print!(", hwcap: 0x{:016x}", entry.hwcap);
        }

        println!(" => {}", libpath);
    }

    // Print generator if available
    if let Some(ref generator) = cache_info.generator {
        println!("Cache generated by: {}", generator);
    }

    Ok(())
}
