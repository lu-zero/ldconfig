//! Cache building API.
//!
//! Provides high-level interface for scanning directories and building cache data.

use crate::config::LibraryConfig;
use crate::internal::cache_format;
use crate::internal::elf::{parse_elf_file, ElfLibrary};
use crate::internal::scanner::{
    deduplicate_libraries, deduplicate_scan_directories, scan_all_libraries,
    should_include_symlink,
};
use crate::internal::symlinks::update_symlinks;
use crate::writer::Cache;
use crate::Error;
use camino::Utf8PathBuf;

/// Options for scanning directories
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Update symlinks in directories
    pub update_symlinks: bool,
    /// Dry run mode (don't make changes)
    pub dry_run: bool,
    /// Verbose output
    pub verbose: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            update_symlinks: true,
            dry_run: false,
            verbose: false,
        }
    }
}

/// Builder for creating cache files
pub struct CacheBuilder {
    libraries: Vec<ElfLibrary>,
    prefix: Option<Utf8PathBuf>,
}

impl CacheBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            libraries: Vec::new(),
            prefix: None,
        }
    }

    /// Set prefix for path stripping
    pub fn with_prefix(mut self, prefix: Utf8PathBuf) -> Self {
        self.prefix = Some(prefix);
        self
    }

    /// Scan directories and collect libraries
    pub fn scan_directories(
        &mut self,
        config: &LibraryConfig,
        options: &ScanOptions,
    ) -> Result<&mut Self, Error> {
        // Deduplicate directories: skip directories that are symlinks to other dirs
        let scan_dirs = deduplicate_scan_directories(config.directories());

        if options.verbose {
            println!("Scanning directories: {:?}", scan_dirs);
        }

        // STEP 1: Single scan - collect all real files and symlinks
        let (real_files, existing_symlinks) = scan_all_libraries(&scan_dirs)?;

        if options.verbose {
            println!(
                "Found {} real files, {} existing symlinks",
                real_files.len(),
                existing_symlinks.len()
            );
        }

        // STEP 2: Update symlinks from real files
        let mut new_symlink_actions = Vec::new();
        if options.update_symlinks && !options.dry_run {
            for dir in &scan_dirs {
                if let Ok(actions) = update_symlinks(dir.as_std_path(), &real_files, options.dry_run)
                {
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

        self.libraries = unique_libraries;

        Ok(self)
    }

    /// Get number of libraries found
    pub fn library_count(&self) -> usize {
        self.libraries.len()
    }

    /// Build cache data (consumes builder)
    pub fn build(self) -> Result<Cache, Error> {
        let data = cache_format::build_cache(&self.libraries, self.prefix.as_deref());
        Ok(Cache::from_bytes(data))
    }
}

impl Default for CacheBuilder {
    fn default() -> Self {
        Self::new()
    }
}
