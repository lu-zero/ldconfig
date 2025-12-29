// ldconfig - Portable Rust implementation
// MIT OR Apache-2.0, 2025

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
//! ```no_run
//! use ldconfig::{SearchPaths, Cache};
//! use camino::Utf8Path;
//!
//! let search_paths = SearchPaths::from_file("/etc/ld.so.conf", None)?;
//! let cache = Cache::builder()
//!     .prefix(Utf8Path::new("/"))
//!     .build(&search_paths)?;
//! cache.write_to_file("/etc/ld.so.cache")?;
//! # Ok::<(), ldconfig::Error>(())
//! ```

// Internal implementation modules
pub(crate) mod cache_format;
pub(crate) mod elf;
pub(crate) mod hwcap;
pub(crate) mod scanner;
pub(crate) mod symlinks;

pub mod cache;
pub mod config;
pub mod error;

// Main public API exports
pub use cache::{Cache, CacheEntry, CacheInfo};
pub use config::SearchPaths;
pub use error::Error;
