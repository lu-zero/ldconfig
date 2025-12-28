use bpaf::Bpaf;
use camino::Utf8PathBuf;
use ldconfig::{
    build_cache, expand_includes, parse_cache_data, parse_config_file, parse_elf_file,
    update_symlinks, Config, ElfLibrary, LdconfigError,
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

fn main() -> Result<(), LdconfigError> {
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

    // Scan for libraries
    let mut libraries = scan_libraries(&scan_dirs)?;

    if options.verbose {
        println!("Found {} base libraries", libraries.len());
    }

    // Scan hwcap subdirectories for optimized library variants
    let hwcap_libraries = scan_hwcap_directories(&scan_dirs)?;
    if options.verbose && !hwcap_libraries.is_empty() {
        println!("Found {} hwcap-optimized libraries", hwcap_libraries.len());
    }
    libraries.extend(hwcap_libraries);

    if options.verbose {
        println!(
            "Total libraries (including hwcap variants): {}",
            libraries.len()
        );
    }

    // Deduplicate libraries by SONAME, keeping only unique entries
    // The real ldconfig keeps multiple entries for the same SONAME if they
    // come from different directories, but we need to deduplicate within
    // the same directory to avoid redundant entries.
    let unique_libraries = deduplicate_libraries(&libraries);

    if options.verbose {
        println!(
            "After deduplication: {} unique libraries",
            unique_libraries.len()
        );
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

        // Update symlinks
        for dir in &scan_dirs {
            if let Ok(actions) = update_symlinks(dir.as_std_path(), &libraries, options.dry_run) {
                if options.verbose && !actions.is_empty() {
                    println!("Symlink actions in {}:", dir);
                    for action in actions {
                        println!("  {} -> {}", action.link, action.target);
                    }
                }
            }
        }
    } else if options.verbose {
        println!("Dry run: would write {} entries to cache", libraries.len());
    }

    Ok(())
}

fn print_cache(options: &Options) -> Result<(), LdconfigError> {
    // Determine cache file path
    let cache_path = options.cache.clone().unwrap_or_else(|| {
        options.prefix.join("etc/ld.so.cache")
    });

    // Read and parse cache
    let data = std::fs::read(&cache_path)
        .map_err(|e| LdconfigError::CacheWrite(format!("Failed to read cache: {}", e)))?;

    let cache_info = parse_cache_data(&data)?;

    // Print header
    println!(
        "{} libs found in cache `{}'",
        cache_info.entries.len(),
        cache_path
    );

    // Helper function to extract null-terminated string from absolute file offset
    let extract_string = |offset: u32| -> Result<String, LdconfigError> {
        let start = offset as usize;
        if start >= data.len() {
            return Err(LdconfigError::CacheRead(format!(
                "Invalid offset: {}",
                offset
            )));
        }

        let slice = &data[start..];
        let null_pos = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());

        String::from_utf8(slice[..null_pos].to_vec()).map_err(|_| {
            LdconfigError::CacheRead("Invalid UTF-8 in string".to_string())
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

/// Deduplicate scan directories by removing directories that are symlinks
/// to canonical paths already in the list. Keep the CANONICAL path, not the symlink.
fn deduplicate_scan_directories(dirs: &[Utf8PathBuf]) -> Vec<Utf8PathBuf> {
    use std::collections::HashMap;

    let mut canonical_to_first: HashMap<Utf8PathBuf, Utf8PathBuf> = HashMap::new();

    for dir in dirs {
        // Get canonical path
        let canonical = if let Ok(canon) = dir.as_std_path().canonicalize() {
            Utf8PathBuf::try_from(canon).unwrap_or_else(|_| dir.clone())
        } else {
            dir.clone()
        };

        // Keep the canonical path (not the symlink)
        canonical_to_first.entry(canonical.clone()).or_insert(canonical);
    }

    canonical_to_first.into_values().collect()
}

/// Deduplicate libraries by (directory, filename) pair
/// This removes exact duplicates but keeps all symlinks and matching real files
fn deduplicate_libraries(libraries: &[ElfLibrary]) -> Vec<ElfLibrary> {
    use std::collections::HashMap;

    let mut unique_libs: HashMap<(Utf8PathBuf, String), ElfLibrary> = HashMap::new();

    for lib in libraries {
        let dir = lib.path.parent().unwrap_or_else(|| "".as_ref()).to_owned();
        let filename = lib.path.file_name().unwrap_or(lib.path.as_str()).to_string();
        let key = (dir, filename);

        // Keep first occurrence
        unique_libs.entry(key).or_insert_with(|| lib.clone());
    }

    unique_libs.into_values().collect()
}

fn scan_hwcap_directories(dirs: &[Utf8PathBuf]) -> Result<Vec<ElfLibrary>, LdconfigError> {
    use ldconfig::detect_hwcap_dirs;

    let mut hwcap_libraries = Vec::new();

    for dir in dirs {
        if !dir.exists() {
            continue;
        }

        // Detect hwcap subdirectories (e.g., /lib/x86-64-v3/, /lib/haswell/)
        let hwcap_dirs = detect_hwcap_dirs(dir.as_std_path())?;

        for (hwcap_path, hwcap) in hwcap_dirs {
            for entry in std::fs::read_dir(&hwcap_path)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_file() && should_scan_library(&path) {
                    if let Ok(mut lib) = parse_elf_file(&path) {
                        let is_symlink = std::fs::symlink_metadata(&path)
                            .map(|m| m.file_type().is_symlink())
                            .unwrap_or(false);
                        let filename = path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("");

                        let should_include = if is_symlink {
                            // For symlinks, check the pattern
                            if filename.ends_with(".so") && !filename.contains(".so.") {
                                // Bare .so symlink: include if target has same base name + .so.VERSION pattern
                                if let Ok(target) = std::fs::read_link(&path) {
                                    let target_name = target.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("");
                                    let base = filename.trim_end_matches(".so");
                                    // Include if target is like libfoo.so.X (standard pattern)
                                    // Exclude if target is like libfoo-X.so (dash-version) or libbar.so (different base)
                                    target_name.starts_with(&format!("{}.", base)) && target_name.contains(".so.")
                                } else {
                                    false
                                }
                            } else {
                                // Versioned symlink (.so.X): include only if filename matches SONAME
                                filename == lib.soname
                            }
                        } else {
                            // Real file: include only if filename matches SONAME
                            filename == lib.soname
                        };

                        if should_include {
                            // Override hwcap from path detection with proper architecture-aware value
                            let arch = lib.arch;
                            lib.hwcap = Some(hwcap.to_bitmask(arch));
                            hwcap_libraries.push(lib);
                        }
                    }
                }
            }
        }
    }

    Ok(hwcap_libraries)
}

fn scan_libraries(dirs: &[Utf8PathBuf]) -> Result<Vec<ElfLibrary>, LdconfigError> {
    let mut libraries = Vec::new();

    for dir in dirs {
        if !dir.exists() {
            continue;
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && should_scan_library(&path) {
                if let Ok(lib) = parse_elf_file(&path) {
                    let is_symlink = std::fs::symlink_metadata(&path)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false);
                    let filename = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");

                    let should_include = if is_symlink {
                        // For symlinks, check the pattern
                        if filename.ends_with(".so") && !filename.contains(".so.") {
                            // Bare .so symlink: include if target has same base name + .so.VERSION pattern
                            if let Ok(target) = std::fs::read_link(&path) {
                                let target_name = target.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("");
                                let base = filename.trim_end_matches(".so");
                                // Include if target is like libfoo.so.X (standard pattern)
                                // Exclude if target is like libfoo-X.so (dash-version) or libbar.so (different base)
                                target_name.starts_with(&format!("{}.", base)) && target_name.contains(".so.")
                            } else {
                                false
                            }
                        } else {
                            // Versioned symlink (.so.X): include only if filename matches SONAME
                            filename == lib.soname
                        }
                    } else {
                        // Real file: include only if filename matches SONAME
                        filename == lib.soname
                    };

                    if should_include {
                        libraries.push(lib);
                    }
                }
            }
        }
    }

    Ok(libraries)
}

fn should_scan_library(path: &std::path::Path) -> bool {
    // Check if this looks like a shared library
    if let Some(ext) = path.extension() {
        if ext == "so" || path.file_name().unwrap().to_string_lossy().contains(".so.") {
            return true;
        }
    }
    false
}
