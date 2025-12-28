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
//! use ldconfig::CacheReader;
//!
//! let reader = CacheReader::from_file("/etc/ld.so.cache")?;
//! reader.print(&mut std::io::stdout())?;
//! # Ok::<(), ldconfig::Error>(())
//! ```
//!
//! # Example: Build and write a cache
//!
//! ```no_run
//! use ldconfig::{LibraryConfig, CacheBuilder, ScanOptions};
//!
//! let config = LibraryConfig::from_file("/etc/ld.so.conf", None)?;
//! let cache = CacheBuilder::new()
//!     .scan_directories(&config, &ScanOptions::default())?
//!     .build()?;
//! cache.write_to_file("/etc/ld.so.cache")?;
//! # Ok::<(), ldconfig::Error>(())
//! ```

mod internal;

pub mod builder;
pub mod config;
pub mod error;
pub mod reader;
pub mod writer;

// Backwards compatibility - old modules (will be removed in future version)
#[doc(hidden)]
pub mod cache;
#[doc(hidden)]
pub mod cache_reader;

// Main public API exports
pub use builder::{CacheBuilder, ScanOptions};
pub use config::LibraryConfig;
pub use error::Error;
pub use reader::{CacheEntry, CacheInfo, CacheReader};
pub use writer::Cache;

// Backwards compatibility exports (will be removed in future version)
#[doc(hidden)]
pub use cache::{build_cache, CacheEntry as OldCacheEntry};
#[doc(hidden)]
pub use cache_reader::{
    parse_cache_data, read_cache_file, CacheInfo as OldCacheInfo, CacheEntry as OldReaderEntry,
};
#[doc(hidden)]
pub use config::{
    expand_includes_compat as expand_includes, parse_config_content_compat as parse_config_content,
    parse_config_file_compat as parse_config_file, Config,
};
#[doc(hidden)]
pub use internal::elf::{parse_elf_file, ElfArch, ElfLibrary};
#[doc(hidden)]
pub use internal::hwcap::{detect_hwcap_dirs, scan_hwcap_libraries, HwCap};
#[doc(hidden)]
pub use internal::scanner::{
    deduplicate_libraries, deduplicate_scan_directories, is_dso, scan_all_libraries,
    should_include_symlink, should_scan_library,
};
#[doc(hidden)]
pub use internal::symlinks::{create_symlink, update_symlinks, SymlinkAction, SymlinkActionType};
