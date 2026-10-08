// ldconfig - Portable Rust implementation
// MIT, 2025

//! A portable Rust implementation of ldconfig for managing dynamic linker cache files.
//!
//! This library provides high-level APIs for:
//! - Reading and exploring ld.so.cache files
//! - Parsing ld.so.conf configuration files
//! - Building cache files by scanning library directories
//! - Writing cache files to disk
//!
//! # Example: Read a cache file
//!
//! ```no_run
//! use ldconfig::Cache;
//!
//! let cache = Cache::from_file("/etc/ld.so.cache")?;
//! println!("{}", cache);
//! # Ok::<(), ldconfig::Error>(())
//! ```
//!
//! # Example: Build and write a cache
//!
//! Building scans the directories and, unless told otherwise, also updates
//! the soname symlinks in them, as `ldconfig` does. Pass
//! `.update_symlinks(false)` to only read.
//!
//! ```no_run
//! use ldconfig::{SearchPaths, Cache};
//!
//! // The configured directories plus the built-in system ones, like glibc.
//! let search_paths = SearchPaths::from_file("/etc/ld.so.conf", None)?.with_system();
//! let cache = Cache::builder()
//!     .update_symlinks(false)
//!     .build(&search_paths)?;
//! cache.write_to_file("/etc/ld.so.cache")?;
//! # Ok::<(), ldconfig::Error>(())
//! ```
//!
//! # Foreign roots
//!
//! With a `prefix`, libraries of another architecture are cached as long as
//! it has the host's byte order: both the ELF reader and the cache writer
//! work in native endianness, as glibc's `ldconfig` does.

// Internal implementation modules
pub(crate) mod cache_format;
pub(crate) mod chroot;
pub(crate) mod elf;
pub(crate) mod scanner;
pub(crate) mod symlinks;

pub(crate) mod atomic_write;

mod cache;
mod config;
mod error;

// Main public API exports
pub use cache::{Cache, CacheBuilder, CacheEntry, CacheInfo};
pub use chroot::chroot_canon;
pub use config::SearchPaths;

/// Errors encountered while reading or writing the cache
///
/// The error is made anonymous on purpose since we depend on
/// many third-party crates.
#[derive(thiserror::Error, Debug)]
#[error(transparent)]
pub struct Error(#[from] error::Error);

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.into())
    }
}
