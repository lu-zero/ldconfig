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
//! use ldconfig::{LibraryConfig, Cache};
//! use camino::Utf8Path;
//!
//! let config = LibraryConfig::from_file("/etc/ld.so.conf", None)?;
//! let cache = Cache::builder()
//!     .prefix(Utf8Path::new("/"))
//!     .build(&config)?;
//! cache.write_to_file("/etc/ld.so.cache")?;
//! # Ok::<(), ldconfig::Error>(())
//! ```

mod internal;

pub mod cache;
pub mod config;
pub mod error;

// Main public API exports
pub use cache::{Cache, CacheEntry, CacheInfo};
pub use config::LibraryConfig;
pub use error::Error;
