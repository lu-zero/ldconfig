//! Cache API.
//!
//! Provides unified interface for reading, querying, and writing ld.so.cache files.
//!
//! # Examples
//!
//! ```no_run
//! use ldconfig::Cache;
//!
//! // Read and display a cache
//! let cache = Cache::from_file("/etc/ld.so.cache")?;
//! println!("{}", cache);  // Uses Display trait
//!
//! // Query entries
//! for entry in cache.entries().take(5) {
//!     println!("{} => {}", entry.soname, entry.path);
//! }
//!
//! // Find specific libraries
//! for entry in cache.find("libc") {
//!     println!("Found: {}", entry.soname);
//! }
//! # Ok::<(), ldconfig::Error>(())
//! ```

use crate::cache_format::{self, CacheInfo as InternalCacheInfo};
use crate::elf::parse_elf_file;
use crate::scanner::{
    deduplicate_libraries, deduplicate_scan_directories, scan_all_libraries, should_include_symlink,
};
use crate::symlinks;
use crate::{Error, SearchPaths};
use bon::bon;
use camino::Utf8Path;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;
use tracing::{debug, info};

/// Information about the cache file
#[derive(Debug, Clone)]
pub struct CacheInfo {
    pub num_entries: usize,
    pub generator: Option<String>,
}

/// A cache entry representing a library
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub soname: String,
    pub path: String,
    pub arch: String,
    pub hwcap: u64,
    pub flags: u32,
}

/// Cache for dynamic linker library information
///
/// This type can be used to:
/// - Read existing cache files from disk or bytes
/// - Query cache contents (entries, search)
/// - Write cache files to disk
/// - Get cache metadata
pub struct Cache {
    data: Vec<u8>,
    info: Option<InternalCacheInfo>,
}

/// Iterator over cache entries
pub struct CacheEntries<'a> {
    cache: &'a Cache,
    entries: Option<std::slice::Iter<'a, crate::cache_format::CacheEntry>>,
}

impl<'a> Iterator for CacheEntries<'a> {
    type Item = CacheEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let entries = self.entries.as_mut()?;

        loop {
            let entry = entries.next()?;
            let soname = self.cache.extract_string(entry.key_offset).ok()?;
            let path = self.cache.extract_string(entry.value_offset).ok()?;
            let arch = decode_arch_flags(entry.flags);

            return Some(CacheEntry {
                soname,
                path,
                arch: arch.to_string(),
                hwcap: entry.hwcap,
                flags: entry.flags,
            });
        }
    }
}

#[bon]
impl Cache {
    #[builder]
    pub fn new(
        /// Directories to scan
        #[builder(finish_fn)]
        search_paths: &SearchPaths,
        /// Update symlinks in directories
        #[builder(default = true)]
        update_symlinks: bool,
        #[builder(default)]
        /// Dry run mode (don't make changes)
        dry_run: bool,
        /// Root prefix
        prefix: &Utf8Path,
    ) -> Result<Self, Error> {
        let scan_dirs = deduplicate_scan_directories(search_paths);

        debug!("Scanning directories: {:?}", scan_dirs);

        // STEP 1: Single scan - collect all real files and symlinks
        let (real_files, existing_symlinks) = scan_all_libraries(&scan_dirs)?;

        debug!(
            "Found {} real files, {} existing symlinks",
            real_files.len(),
            existing_symlinks.len()
        );

        // STEP 2: Update symlinks from real files
        let mut new_symlink_actions = Vec::new();
        if update_symlinks && !dry_run {
            for dir in &scan_dirs {
                if let Ok(actions) = symlinks::update(dir.as_std_path(), &real_files, dry_run) {
                    if !actions.is_empty() {
                        debug!("Symlink actions in {}:", dir);
                        for action in &actions {
                            debug!("  {} -> {}", action.link, action.target);
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
            if let Some(lib) = parse_elf_file(action.link.as_std_path()) {
                cache_entries.push(lib);
            }
        }

        // Deduplicate by (directory, filename)
        let unique_libraries = deduplicate_libraries(&cache_entries);

        info!("Cache entries: {} unique libraries", unique_libraries.len());

        let data = cache_format::build_cache(&unique_libraries, prefix);
        Ok(Cache::from_bytes_raw(data))
    }
}

impl Cache {
    /// Create cache from raw bytes (for writing)
    pub(crate) fn from_bytes_raw(data: Vec<u8>) -> Self {
        Self { data, info: None }
    }

    /// Read and parse cache from file path
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let data = fs::read(path.as_ref())?;
        Self::from_bytes(&data)
    }

    /// Parse cache from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, Error> {
        let info = cache_format::parse_cache(data)?;
        Ok(Self {
            data: data.to_vec(),
            info: Some(info),
        })
    }

    /// Get cache metadata
    pub fn info(&self) -> CacheInfo {
        if let Some(ref info) = self.info {
            CacheInfo {
                num_entries: info.entries.len(),
                generator: info.generator.clone(),
            }
        } else {
            CacheInfo {
                num_entries: 0,
                generator: None,
            }
        }
    }

    /// Get iterator over all entries
    pub fn entries(&self) -> CacheEntries<'_> {
        CacheEntries {
            cache: self,
            entries: self.info.as_ref().map(|info| info.entries.iter()),
        }
    }

    /// Find entries matching a library name (returns iterator)
    pub fn find<'a>(&'a self, name: &'a str) -> impl Iterator<Item = CacheEntry> + 'a {
        self.entries()
            .filter(move |entry| entry.soname.contains(name))
    }

    /// Write cache to file
    pub fn write_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Error> {
        let path = path.as_ref();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write cache file
        let mut file = fs::File::create(path)?;
        file.write_all(&self.data)?;
        file.flush()?;
        file.sync_all()?;

        Ok(())
    }

    /// Get cache as bytes
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Get cache size
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Extract null-terminated string from absolute file offset
    fn extract_string(&self, offset: u32) -> Result<String, Error> {
        let start = offset as usize;
        if start >= self.data.len() {
            return Err(Error::InvalidCacheOffset(offset));
        }

        let slice = &self.data[start..];
        let null_pos = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());

        String::from_utf8(slice[..null_pos].to_vec()).map_err(|_| Error::InvalidCacheUtf8)
    }
}

impl fmt::Display for Cache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref info) = self.info {
            writeln!(f, "{} libs found in cache", info.entries.len())?;

            for entry in &info.entries {
                // Extract strings - if any fail, skip this entry
                let Ok(libname) = self.extract_string(entry.key_offset) else {
                    continue;
                };
                let Ok(libpath) = self.extract_string(entry.value_offset) else {
                    continue;
                };
                let arch_str = decode_arch_flags(entry.flags);

                write!(f, "\t{} ({})", libname, arch_str)?;

                if entry.hwcap != 0 {
                    write!(f, ", hwcap: 0x{:016x}", entry.hwcap)?;
                }

                writeln!(f, " => {}", libpath)?;
            }

            if let Some(ref generator) = info.generator {
                writeln!(f, "Cache generated by: {}", generator)?;
            }
        } else {
            writeln!(f, "Cache not parsed (binary only)")?;
        }

        Ok(())
    }
}

/// Decode architecture from flags (matches ldconfig output format)
fn decode_arch_flags(flags: u32) -> &'static str {
    let arch_bits = (flags >> 8) & 0xff;
    match arch_bits {
        0x00 => "libc6",                // i386/generic ELF
        0x01 => "libc6,SPARC 64-bit",   // SPARC 64-bit
        0x03 => "libc6,x86-64",         // x86_64
        0x04 => "libc6,64bit",          // PowerPC/S390 64-bit
        0x05 => "libc6,64bit",          // PowerPC 64-bit (official)
        0x06 => "libc6,IA-64",          // IA-64
        0x07 => "libc6,MIPS 64-bit",    // MIPS 64-bit
        0x08 => "libc6,x32",            // x32
        0x09 => "libc6,ARM,hard-float", // ARM hard-float
        0x0a => "libc6,AArch64",        // AArch64
        0x0b => "libc6,ARM,soft-float", // ARM soft-float
        0x10 => "libc6,RISC-V 64-bit",  // RISC-V lp64d
        _ => "unknown",
    }
}
