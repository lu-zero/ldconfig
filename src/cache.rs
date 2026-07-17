//! Cache API.
//!
//! Provides unified interface for reading, querying, and writing
//! ld.so.cache files.
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

use crate::cache_format::{self, flags_string, CacheInfo as InternalCacheInfo, FileEntry};
use crate::scanner::{collect_dirs, scan_dir};
use crate::{atomic_write, error::Error, symlinks, SearchPaths};
use bon::bon;
use camino::{Utf8Path, Utf8PathBuf};
use std::fmt;
use std::fs;
use std::path::Path;
use tracing::info;

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
    /// Flag description as printed by ldconfig -p, e.g. "libc6,x86-64".
    pub arch: String,
    pub hwcap: u64,
    /// glibc-hwcaps subdirectory name for extension entries.
    pub hwcaps: Option<String>,
    pub flags: u32,
}

impl fmt::Display for CacheEntry {
    /// One `ldconfig -p` line, matching glibc's print_entry.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\t{} ({}", self.soname, self.arch)?;
        if let Some(name) = &self.hwcaps {
            write!(f, ", hwcap: \"{}\"", name)?;
        } else if self.hwcap != 0 {
            write!(f, ", hwcap: {:#018x}", self.hwcap)?;
        }
        write!(f, ") => {}", self.path)
    }
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
    info: InternalCacheInfo,
}

/// Iterator over cache entries
pub struct CacheEntries<'a> {
    cache: &'a Cache,
    entries: std::slice::Iter<'a, cache_format::CacheEntry>,
}

impl<'a> Iterator for CacheEntries<'a> {
    type Item = CacheEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.next()?;
        let soname = self.cache.extract_string(entry.key_offset).ok()?;
        let path = self.cache.extract_string(entry.value_offset).ok()?;

        Some(CacheEntry {
            soname,
            path,
            arch: flags_string(entry.flags),
            hwcap: entry.hwcap,
            hwcaps: entry.hwcaps.clone(),
            flags: entry.flags,
        })
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
        #[builder(into, default = "/")]
        prefix: &Utf8Path,
    ) -> Result<Self, Error> {
        let prefix = normalize_prefix(prefix);
        let update_links = update_symlinks && !dry_run;
        let dirs = collect_dirs(search_paths, &prefix);

        let mut entries = Vec::new();
        for dir in &dirs {
            for lib in scan_dir(dir, &prefix, update_links) {
                // The cached file name is the soname for regular
                // directories (relying on the symlink), the actual file
                // for glibc-hwcaps subdirectories (search_dir).
                let value_name = match &dir.hwcaps {
                    None => {
                        // Don't create links to links.
                        if update_links && !lib.is_link {
                            symlinks::create_link(
                                &prefix,
                                &dir.real,
                                &dir.path,
                                &lib.name,
                                &lib.soname,
                            );
                        }
                        &lib.soname
                    }
                    Some(_) => &lib.name,
                };
                entries.push(FileEntry {
                    path: format!("{}/{}", dir.path, value_name),
                    soname: lib.soname,
                    flags: lib.flags,
                    isa_level: lib.isa_level,
                    hwcaps: dir.hwcaps.clone(),
                });
            }
        }

        info!("Cache entries: {} libraries", entries.len());

        let data = cache_format::build_cache(&entries);
        let info = cache_format::parse_cache(&data)?;
        Ok(Self { data, info })
    }
}

fn normalize_prefix(prefix: &Utf8Path) -> Utf8PathBuf {
    let trimmed = prefix.as_str().trim_end_matches('/');
    if trimmed.is_empty() {
        Utf8PathBuf::from("/")
    } else {
        Utf8PathBuf::from(trimmed)
    }
}

impl Cache {
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
            info,
        })
    }

    /// Get cache metadata
    pub fn info(&self) -> CacheInfo {
        CacheInfo {
            num_entries: self.info.entries.len(),
            generator: self.info.generator.clone(),
        }
    }

    /// Get iterator over all entries
    pub fn entries(&self) -> CacheEntries<'_> {
        CacheEntries {
            cache: self,
            entries: self.info.entries.iter(),
        }
    }

    /// Find entries matching a library name (returns iterator)
    pub fn find<'a>(&'a self, name: &'a str) -> impl Iterator<Item = CacheEntry> + 'a {
        self.entries()
            .filter(move |entry| entry.soname.contains(name))
    }

    /// Write cache to file atomically
    pub fn write_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Error> {
        atomic_write::atomic_write(path, &self.data)?;
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
        writeln!(f, "{} libs found in cache", self.info.entries.len())?;
        for entry in self.entries() {
            writeln!(f, "{}", entry)?;
        }
        if let Some(generator) = &self.info.generator {
            writeln!(f, "Cache generated by: {}", generator)?;
        }
        Ok(())
    }
}
