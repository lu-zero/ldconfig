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
use crate::{atomic_write, symlinks, Error, SearchPaths};
use bon::bon;
use camino::{Utf8Path, Utf8PathBuf};
use std::fmt;
use std::fs;
use std::path::Path;
use tracing::info;

/// Information about the cache file
#[derive(Debug, Clone)]
pub struct CacheInfo {
    /// Number of library entries in the cache.
    pub num_entries: usize,
    /// The tool that wrote the cache, from its generator extension.
    pub generator: Option<String>,
}

/// A cache entry representing a library
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// The name the dynamic loader looks up, e.g. `libz.so.1`.
    pub soname: String,
    /// Where that name resolves, as written in the cache.
    pub path: String,
    /// Flag description as printed by ldconfig -p, e.g. "libc6,x86-64".
    pub arch: String,
    /// Raw hwcap word: legacy hwcap bits, or the extension marker.
    pub hwcap: u64,
    /// glibc-hwcaps subdirectory name for extension entries.
    pub hwcaps: Option<String>,
    /// Raw cache flags: library type and required ABI.
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

        Some(CacheEntry {
            soname: self.cache.extract_string(entry.key_offset),
            path: self.cache.extract_string(entry.value_offset),
            arch: flags_string(entry.flags),
            hwcap: entry.hwcap,
            hwcaps: entry.hwcaps.clone(),
            flags: entry.flags,
        })
    }
}

#[bon]
impl Cache {
    /// Scan `search_paths` and build a cache from the libraries found.
    ///
    /// Like `ldconfig`, this also brings the soname symlinks in those
    /// directories up to date and removes dangling ones, unless
    /// `update_symlinks(false)` or `dry_run(true)` is set. Nothing is
    /// written to the cache file until [`Cache::write_to_file`].
    #[builder]
    pub fn new(
        /// Directories to scan
        #[builder(finish_fn)]
        search_paths: &SearchPaths,
        /// Create, repoint and prune soname symlinks in the scanned
        /// directories (default: true, as `ldconfig` without `-X`)
        #[builder(default = true)]
        update_symlinks: bool,
        #[builder(default)]
        /// Dry run mode: scan only, overriding `update_symlinks`
        dry_run: bool,
        /// Root to operate in, like `ldconfig -r`: paths in `search_paths`
        /// and symlinks are resolved inside it
        #[builder(into, default = "/")]
        prefix: &Utf8Path,
    ) -> Result<Self, Error> {
        let prefix = normalize_prefix(prefix);
        let update_links = update_symlinks && !dry_run;
        let dirs = collect_dirs(search_paths, &prefix);

        let mut entries = Vec::new();
        for dir in &dirs {
            for lib in scan_dir(dir, &prefix, update_links) {
                // Regular directories are cached under the soname, so the
                // entry survives a file rename as long as the link follows;
                // glibc-hwcaps ones name the file itself (search_dir).
                let value_name = match &dir.hwcaps {
                    None => {
                        // A winner that is itself a link needs no link.
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

    /// Entries whose soname contains `name` as a substring
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

    /// The NUL-terminated string at an absolute file offset.
    ///
    /// Offsets were bounds-checked when the cache was parsed; bytes that
    /// are not UTF-8 are replaced, so every entry stays visible.
    fn extract_string(&self, offset: u32) -> String {
        cache_format::read_string(&self.data, offset as usize).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    // A soname that is not UTF-8 used to end the iteration, hiding every
    // entry after it.
    #[test]
    fn entries_survive_a_non_utf8_string() {
        let file = |soname: &str| FileEntry {
            soname: soname.into(),
            path: format!("/usr/lib/{soname}"),
            flags: cache_format::FLAG_ELF_LIBC6,
            isa_level: 0,
            hwcaps: None,
        };
        let mut data = cache_format::build_cache(&[file("libaaa.so.1"), file("libzzz.so.1")]);
        let at = data
            .windows(b"libaaa".len())
            .position(|w| w == b"libaaa")
            .unwrap();
        data[at + 3] = 0xff;

        let cache = Cache::from_bytes(&data).unwrap();
        assert_eq!(cache.info().num_entries, 2);
        let sonames: Vec<String> = cache.entries().map(|e| e.soname).collect();
        assert_eq!(sonames.len(), 2);
        assert!(sonames.iter().any(|s| s == "libzzz.so.1"), "{sonames:?}");
    }
}
