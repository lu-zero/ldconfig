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

use crate::internal::cache_format::{self, CacheInfo as InternalCacheInfo};
use crate::Error;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;

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
    entries: Option<std::slice::Iter<'a, crate::internal::cache_format::CacheEntry>>,
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

impl Cache {
    /// Create cache from raw bytes (for writing)
    pub(crate) fn from_bytes_raw(data: Vec<u8>) -> Self {
        Self { data, info: None }
    }

    /// Read and parse cache from file path
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let data = fs::read(path.as_ref())
            .map_err(|e| Error::CacheRead(format!("Failed to read cache: {}", e)))?;

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
        self.entries().filter(move |entry| entry.soname.contains(name))
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
            return Err(Error::CacheRead(format!("Invalid offset: {}", offset)));
        }

        let slice = &self.data[start..];
        let null_pos = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());

        String::from_utf8(slice[..null_pos].to_vec())
            .map_err(|_| Error::CacheRead("Invalid UTF-8 in string".to_string()))
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
        0x00 => "libc6",                  // i386/generic ELF
        0x01 => "libc6,SPARC 64-bit",     // SPARC 64-bit
        0x03 => "libc6,x86-64",           // x86_64
        0x04 => "libc6,64bit",            // PowerPC/S390 64-bit
        0x05 => "libc6,64bit",            // PowerPC 64-bit (official)
        0x06 => "libc6,IA-64",            // IA-64
        0x07 => "libc6,MIPS 64-bit",      // MIPS 64-bit
        0x08 => "libc6,x32",              // x32
        0x09 => "libc6,ARM,hard-float",   // ARM hard-float
        0x0a => "libc6,AArch64",          // AArch64
        0x0b => "libc6,ARM,soft-float",   // ARM soft-float
        0x10 => "libc6,RISC-V 64-bit",    // RISC-V lp64d
        _ => "unknown",
    }
}

// Backwards compatibility exports
use crate::internal::elf::ElfLibrary;
use camino::Utf8Path;

pub fn build_cache(libraries: &[ElfLibrary], prefix: Option<&Utf8Path>) -> Vec<u8> {
    cache_format::build_cache(libraries, prefix)
}
